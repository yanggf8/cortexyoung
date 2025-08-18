import { simpleGit, SimpleGit } from 'simple-git';
import { warn as timestampedWarn } from './logging-utils';
import * as fs from 'fs/promises';
import * as path from 'path';

export interface ScanResult {
  files: string[];
  lastCommit?: string;
  totalFiles: number;
}

export interface FileChange {
  filePath: string;
  status: 'added' | 'modified' | 'deleted';
  lastCommit: string;
  commitAuthor: string;
  commitMessage: string;
  commitDate: string;
}

export class GitScanner {
  private git: SimpleGit;

  constructor(private repositoryPath: string) {
    this.git = simpleGit(repositoryPath);
  }

  async scanRepository(mode: 'full' | 'incremental', sinceCommit?: string): Promise<ScanResult> {
    try {
      if (mode === 'incremental' && sinceCommit) {
        return await this.scanIncremental(sinceCommit);
      } else {
        return await this.scanFull();
      }
    } catch (error) {
      throw new Error(`Git scan failed: ${error instanceof Error ? error.message : 'Unknown error'}`);
    }
  }

  private async scanFull(): Promise<ScanResult> {
    // Get all tracked files
    let files = await this.git.raw(['ls-files']).then(result => 
      result.split('\n').filter(Boolean)
    );

    // Include untracked files for dual-mode support (default: true, can be disabled)
    const includeUntracked = process.env.CORTEX_INCLUDE_UNTRACKED !== 'false';
    if (includeUntracked) {
      try {
        // Get untracked files
        const untrackedFiles = await this.git.raw(['ls-files', '--others', '--exclude-standard']).then(result => 
          result.split('\n').filter(Boolean)
        );
        
        // Combine tracked and untracked files
        files = [...files, ...untrackedFiles];
        
        if (untrackedFiles.length > 0) {
          console.log(`[GitScanner] Including ${untrackedFiles.length} untracked files in scan`);
        }
      } catch (error) {
        timestampedWarn(`[GitScanner] Failed to get untracked files: ${error}`);
      }
    }

    // Filter for code and documentation files and check if files exist
    const relevantFiles = await this.filterExistingFiles(
      files.filter(file => this.isRelevantFile(file))
    );

    // Get latest commit
    const log = await this.git.log({ maxCount: 1 });
    const lastCommit = log.latest?.hash;

    return {
      files: relevantFiles,
      lastCommit,
      totalFiles: relevantFiles.length
    };
  }

  private async scanIncremental(sinceCommit: string): Promise<ScanResult> {
    // Get changed files since the specified commit
    const diff = await this.git.diffSummary([`${sinceCommit}..HEAD`]);
    const changedFiles = diff.files.map(file => file.file);

    // Filter for relevant files and check if files exist
    const relevantFiles = await this.filterExistingFiles(
      changedFiles.filter(file => this.isRelevantFile(file))
    );

    // Get latest commit
    const log = await this.git.log({ maxCount: 1 });
    const lastCommit = log.latest?.hash;

    return {
      files: relevantFiles,
      lastCommit,
      totalFiles: relevantFiles.length
    };
  }

  async getFileChanges(files: string[]): Promise<FileChange[]> {
    const changes: FileChange[] = [];

    for (const file of files) {
      try {
        // Get the latest commit for this file
        const log = await this.git.log({ file, maxCount: 1 });
        const latestCommit = log.latest;

        if (latestCommit) {
          changes.push({
            filePath: file,
            status: 'modified', // Simplified for now
            lastCommit: latestCommit.hash,
            commitAuthor: latestCommit.author_email,
            commitMessage: latestCommit.message,
            commitDate: latestCommit.date
          });
        }
      } catch (error) {
        // Skip files that can't be analyzed
        timestampedWarn(`Could not get commit info for ${file}: ${error}`);
      }
    }

    return changes;
  }

  async getCoChangeFiles(file: string): Promise<string[]> {
    try {
      // Get commits that modified this file
      const log = await this.git.log({ file, maxCount: 10 });
      const commitHashes = log.all.map(commit => commit.hash);

      const coChangeFiles = new Set<string>();

      // For each commit, find other files modified in the same commit
      for (const hash of commitHashes) {
        const show = await this.git.show([hash, '--name-only', '--format=']);
        const filesInCommit = show.split('\n').filter(Boolean);
        
        filesInCommit.forEach(f => {
          if (f !== file && this.isRelevantFile(f)) {
            coChangeFiles.add(f);
          }
        });
      }

      return Array.from(coChangeFiles);
    } catch (error) {
      timestampedWarn(`Could not get co-change files for ${file}: ${error}`);
      return [];
    }
  }

  private async filterExistingFiles(files: string[]): Promise<string[]> {
    const existingFiles: string[] = [];
    
    for (const file of files) {
      try {
        const fullPath = path.join(this.repositoryPath, file);
        await fs.access(fullPath, fs.constants.F_OK);
        existingFiles.push(file);
      } catch (error: any) {
        if (error.code === 'ENOENT') {
          timestampedWarn(`Skipping deleted file: ${file}`);
        } else {
          timestampedWarn(`Error checking file ${file}: ${error.message}`);
        }
      }
    }
    
    return existingFiles;
  }

  private isRelevantFile(filePath: string): boolean {
    const ext = path.extname(filePath).toLowerCase();
    const relevantExtensions = [
      '.js', '.jsx', '.ts', '.tsx',
      '.py', '.go', '.rs', '.java',
      '.cpp', '.c', '.h', '.hpp',
      '.md', '.rst', '.txt',
      '.json', '.yaml', '.yml', '.toml',
      '.html', '.css', '.scss'
    ];

    // Skip certain directories
    const skipDirs = ['node_modules', '.git', 'dist', 'build', 'coverage', '.next', '.cache'];
    if (skipDirs.some(dir => filePath.includes(dir))) {
      return false;
    }

    return relevantExtensions.includes(ext);
  }

  async readFile(filePath: string): Promise<string> {
    const fullPath = path.join(this.repositoryPath, filePath);
    try {
      return await fs.readFile(fullPath, 'utf-8');
    } catch (error: any) {
      if (error.code === 'ENOENT') {
        throw new Error(`File not found: ${filePath} (may have been deleted but not committed)`);
      }
      throw error;
    }
  }

  async getCurrentBranch(): Promise<string> {
    try {
      const branch = await this.git.branchLocal();
      return branch.current;
    } catch (error) {
      timestampedWarn(`Could not get current branch: ${error}`);
      return 'unknown';
    }
  }

  async getLatestCommit(): Promise<string> {
    try {
      const log = await this.git.log({ maxCount: 1 });
      return log.latest?.hash || 'unknown';
    } catch (error) {
      timestampedWarn(`Could not get latest commit: ${error}`);
      return 'unknown';
    }
  }

  async getCommitDistance(fromCommit: string, toCommit: string): Promise<number> {
    try {
      const result = await this.git.raw(['rev-list', '--count', `${fromCommit}..${toCommit}`]);
      return parseInt(result.trim()) || 0;
    } catch (error) {
      timestampedWarn(`Could not get commit distance from ${fromCommit} to ${toCommit}: ${error}`);
      return 0;
    }
  }
}