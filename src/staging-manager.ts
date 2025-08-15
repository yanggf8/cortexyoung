import * as path from 'path';
import * as fs from 'fs/promises';
import { exec } from 'child_process';
import { promisify } from 'util';
import { log, warn } from './logging-utils';

const execAsync = promisify(exec);

export interface StagedFile {
  filePath: string;
  isGitTracked: boolean;
  lastModified: Date;
  fileSize: number;
  status: 'new' | 'modified' | 'staged' | 'indexed';
}

export interface StagingConfig {
  includeUntrackedFiles: boolean;
  maxUntrackedFiles: number;
  maxFileSizeKB: number;
  excludePatterns: string[];
}

export class StagingManager {
  private stagedFiles = new Map<string, StagedFile>();
  private repositoryPath: string;
  private config: StagingConfig;

  constructor(repositoryPath: string, config?: Partial<StagingConfig>) {
    this.repositoryPath = repositoryPath;
    this.config = {
      includeUntrackedFiles: true,
      maxUntrackedFiles: 100,
      maxFileSizeKB: 1024, // 1MB max file size
      excludePatterns: [
        'node_modules/**',
        '.git/**', 
        '*.log',
        'dist/**',
        'build/**',
        '*.tmp',
        '*.temp'
      ],
      ...config
    };
  }

  /**
   * Check if a file is git-tracked
   */
  async isGitTracked(filePath: string): Promise<boolean> {
    try {
      const relativePath = path.relative(this.repositoryPath, filePath);
      const { stdout } = await execAsync(`git ls-files --error-unmatch "${relativePath}"`, {
        cwd: this.repositoryPath
      });
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Check if a file should be excluded based on patterns
   */
  private shouldExcludeFile(filePath: string): boolean {
    const relativePath = path.relative(this.repositoryPath, filePath);
    
    return this.config.excludePatterns.some(pattern => {
      // Simple glob-like matching
      const regexPattern = pattern
        .replace(/\*\*/g, '.*')
        .replace(/\*/g, '[^/]*')
        .replace(/\?/g, '[^/]');
      
      const regex = new RegExp(`^${regexPattern}$`);
      return regex.test(relativePath);
    });
  }

  /**
   * Check if a file meets staging criteria
   */
  async shouldStageFile(filePath: string): Promise<boolean> {
    try {
      // Check exclusion patterns
      if (this.shouldExcludeFile(filePath)) {
        return false;
      }

      // Check file size
      const stats = await fs.stat(filePath);
      if (stats.size > this.config.maxFileSizeKB * 1024) {
        return false;
      }

      // Check if it's a text file (basic check)
      const isTextFile = await this.isTextFile(filePath);
      if (!isTextFile) {
        return false;
      }

      return true;
    } catch {
      return false;
    }
  }

  /**
   * Basic text file detection
   */
  private async isTextFile(filePath: string): Promise<boolean> {
    try {
      // Check by extension first
      const ext = path.extname(filePath).toLowerCase();
      const textExtensions = [
        '.ts', '.js', '.tsx', '.jsx', '.py', '.java', '.cpp', '.c', '.h',
        '.cs', '.php', '.rb', '.go', '.rs', '.swift', '.kt', '.scala',
        '.html', '.css', '.scss', '.less', '.vue', '.svelte',
        '.json', '.xml', '.yaml', '.yml', '.toml', '.ini',
        '.md', '.txt', '.rst', '.tex', '.sql'
      ];
      
      if (textExtensions.includes(ext)) {
        return true;
      }

      // Check first 512 bytes for binary content
      const buffer = Buffer.alloc(512);
      const file = await fs.open(filePath, 'r');
      const { bytesRead } = await file.read(buffer, 0, 512, 0);
      await file.close();

      // Check for null bytes (binary indicator)
      for (let i = 0; i < bytesRead; i++) {
        if (buffer[i] === 0) {
          return false;
        }
      }

      return true;
    } catch {
      return false;
    }
  }

  /**
   * Stage a file for indexing
   */
  async stageFile(filePath: string): Promise<boolean> {
    try {
      if (!await this.shouldStageFile(filePath)) {
        return false;
      }

      const stats = await fs.stat(filePath);
      const isTracked = await this.isGitTracked(filePath);
      
      // Check limits for untracked files
      if (!isTracked) {
        const untrackedCount = Array.from(this.stagedFiles.values())
          .filter(f => !f.isGitTracked).length;
        
        if (untrackedCount >= this.config.maxUntrackedFiles) {
          warn(`[StagingManager] Maximum untracked files limit reached (${this.config.maxUntrackedFiles})`);
          return false;
        }
      }

      const relativePath = path.relative(this.repositoryPath, filePath);
      const stagedFile: StagedFile = {
        filePath: relativePath,
        isGitTracked: isTracked,
        lastModified: stats.mtime,
        fileSize: stats.size,
        status: isTracked ? 'modified' : 'new'
      };

      this.stagedFiles.set(relativePath, stagedFile);
      
      log(`[StagingManager] Staged file: ${relativePath} (${isTracked ? 'tracked' : 'untracked'})`);
      return true;
    } catch (error) {
      warn(`[StagingManager] Failed to stage file ${filePath}:`, error);
      return false;
    }
  }

  /**
   * Remove a file from staging
   */
  unstageFile(filePath: string): boolean {
    const relativePath = path.relative(this.repositoryPath, filePath);
    const removed = this.stagedFiles.delete(relativePath);
    
    if (removed) {
      log(`[StagingManager] Unstaged file: ${relativePath}`);
    }
    
    return removed;
  }

  /**
   * Mark a file as indexed
   */
  markFileIndexed(filePath: string): void {
    const relativePath = path.relative(this.repositoryPath, filePath);
    const stagedFile = this.stagedFiles.get(relativePath);
    
    if (stagedFile) {
      stagedFile.status = 'indexed';
      log(`[StagingManager] Marked file as indexed: ${relativePath}`);
    }
  }

  /**
   * Get all staged files
   */
  getStagedFiles(): StagedFile[] {
    return Array.from(this.stagedFiles.values());
  }

  /**
   * Get staged files by status
   */
  getFilesByStatus(status: StagedFile['status']): StagedFile[] {
    return this.getStagedFiles().filter(f => f.status === status);
  }

  /**
   * Get staging statistics
   */
  getStats(): {
    totalStaged: number;
    gitTracked: number;
    untracked: number;
    byStatus: Record<StagedFile['status'], number>;
  } {
    const files = this.getStagedFiles();
    
    const stats = {
      totalStaged: files.length,
      gitTracked: files.filter(f => f.isGitTracked).length,
      untracked: files.filter(f => !f.isGitTracked).length,
      byStatus: {
        new: 0,
        modified: 0,
        staged: 0,
        indexed: 0
      } as Record<StagedFile['status'], number>
    };

    files.forEach(file => {
      stats.byStatus[file.status]++;
    });

    return stats;
  }

  /**
   * Clear all staged files
   */
  clearStaging(): void {
    const count = this.stagedFiles.size;
    this.stagedFiles.clear();
    log(`[StagingManager] Cleared ${count} staged files`);
  }

  /**
   * Get files that need indexing (new or modified)
   */
  getFilesNeedingIndex(): StagedFile[] {
    return this.getStagedFiles().filter(f => 
      f.status === 'new' || f.status === 'modified' || f.status === 'staged'
    );
  }
}