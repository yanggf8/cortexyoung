import chokidar from 'chokidar';
import { readFile } from 'fs/promises';
import * as path from 'path';
import { log, warn } from './logging-utils';
import { CodebaseIndexer } from './indexer';
import { StagingManager } from './staging-manager';

interface SemanticChange {
  filePath: string;
  changeType: 'structure' | 'content' | 'deleted';
  affectedChunks: string[];
}

export class SemanticWatcher {
  private watcher: chokidar.FSWatcher | null = null;
  private indexer: CodebaseIndexer;
  private stagingManager: StagingManager;
  private isActive = false;
  
  // Simple semantic patterns that affect Claude Code's understanding
  private semanticPatterns = [
    /^(import|export|from)\s/m,           // Import/export changes
    /^(class|interface|type|enum)\s/m,    // Type definitions
    /^(function|const|let|var)\s.*=/m,    // Function/variable declarations
    /^(async\s+)?function\s/m,            // Function definitions
    /\/\*\*[\s\S]*?\*\//g,               // JSDoc comments (semantic)
  ];

  constructor(repositoryPath: string, indexer: CodebaseIndexer) {
    this.indexer = indexer;
    this.stagingManager = new StagingManager(repositoryPath, {
      includeUntrackedFiles: true,
      maxUntrackedFiles: 50,
      maxFileSizeKB: 2048 // 2MB max
    });
    
    this.watcher = chokidar.watch(repositoryPath, {
      ignored: [
        '**/node_modules/**',
        '**/.git/**', 
        '**/.cortex/**',
        '**/dist/**',
        '**/*.log'
      ],
      ignoreInitial: true,
      awaitWriteFinish: { stabilityThreshold: 200 }
    });
  }

  async start(): Promise<void> {
    if (!this.watcher || this.isActive) return;
    
    log('[SemanticWatcher] Starting semantic file watcher...');
    
    this.watcher
      .on('change', (path: string) => this.handleFileChange(path))
      .on('add', (path: string) => this.handleFileChange(path))  // Handle new files too
      .on('unlink', (path: string) => this.handleFileDelete(path))
      .on('error', (err: Error) => warn('[SemanticWatcher] Error:', err));
    
    this.isActive = true;
    log('[SemanticWatcher] Semantic watcher active with dual-mode tracking');
    
    // Log staging configuration for visibility
    const stats = this.stagingManager.getStats();
    log(`[SemanticWatcher] Staging configuration: untracked files support=${this.stagingManager['config'].includeUntrackedFiles}, max=${this.stagingManager['config'].maxUntrackedFiles}`);
  }

  async stop(): Promise<void> {
    if (this.watcher) {
      await this.watcher.close();
      this.isActive = false;
      log('[SemanticWatcher] Semantic watcher stopped');
    }
  }

  private async handleFileChange(filePath: string): Promise<void> {
    try {
      log(`[SemanticWatcher] Processing file change: ${filePath}`);
      
      // Check if file is git-tracked
      const isGitTracked = await this.stagingManager.isGitTracked(filePath);
      
      if (isGitTracked) {
        // Git-tracked files: Direct processing (bypass staging)
        log(`[SemanticWatcher] Git-tracked file detected, processing directly: ${filePath}`);
        
        const content = await readFile(filePath, 'utf-8');
        const semanticChange = await this.analyzeSemanticChange(filePath, content);
        
        if (semanticChange) {
          log(`[SemanticWatcher] Semantic change detected in git-tracked file: ${filePath}`);
          await this.processSemanticChange(semanticChange);
        } else {
          log(`[SemanticWatcher] No semantic changes in git-tracked file, re-indexing: ${filePath}`);
          await this.indexer.handleFileChange(filePath, 'content');
        }
        
        log(`[SemanticWatcher] Git-tracked file processed: ${filePath}`);
      } else {
        // Untracked files: Use staging manager
        const staged = await this.stagingManager.stageFile(filePath);
        if (!staged) {
          log(`[SemanticWatcher] Untracked file not staged (excluded or doesn't meet criteria): ${filePath}`);
          return;
        }

        log(`[SemanticWatcher] Untracked file staged successfully: ${filePath}`);

        const content = await readFile(filePath, 'utf-8');
        const semanticChange = await this.analyzeSemanticChange(filePath, content);
        
        if (semanticChange) {
          log(`[SemanticWatcher] Semantic change detected in untracked file: ${filePath}`);
          await this.processSemanticChange(semanticChange);
        } else {
          log(`[SemanticWatcher] No semantic changes in untracked file, indexing: ${filePath}`);
          await this.indexer.handleFileChange(filePath, 'content');
        }
        
        this.stagingManager.markFileIndexed(filePath);
        log(`[SemanticWatcher] Untracked file marked as indexed: ${filePath}`);
      }
    } catch (error) {
      // File might be deleted or inaccessible, ignore
      log(`[SemanticWatcher] Error processing file change for ${filePath}: ${error}`);
    }
  }

  private async handleFileDelete(filePath: string): Promise<void> {
    log(`[SemanticWatcher] File deleted: ${filePath}`);
    
    // Remove from staging if it was staged
    this.stagingManager.unstageFile(filePath);
    
    const semanticChange: SemanticChange = {
      filePath,
      changeType: 'deleted',
      affectedChunks: [] // Will be determined by indexer
    };
    
    await this.processSemanticChange(semanticChange);
  }

  private async analyzeSemanticChange(filePath: string, content: string): Promise<SemanticChange | null> {
    // Skip non-code files
    if (!this.isCodeFile(filePath)) return null;
    
    // Check if content has semantic patterns
    const hasSemanticContent = this.semanticPatterns.some(pattern => 
      pattern.test(content)
    );
    
    if (!hasSemanticContent) {
      // Check if it's a significant content change (not just whitespace/comments)
      const significantContent = content
        .replace(/\/\*[\s\S]*?\*\//g, '') // Remove block comments
        .replace(/\/\/.*$/gm, '')         // Remove line comments  
        .replace(/\s+/g, ' ')             // Normalize whitespace
        .trim();
      
      if (significantContent.length < 50) return null; // Too small to be semantic
    }
    
    return {
      filePath,
      changeType: 'structure', // Assume structure change for simplicity
      affectedChunks: [] // Will be determined by indexer
    };
  }

  private isCodeFile(filePath: string): boolean {
    // Expanded to match staging manager's text file detection
    const codeExtensions = [
      '.ts', '.js', '.tsx', '.jsx', '.py', '.java', '.cpp', '.c', '.h',
      '.cs', '.php', '.rb', '.go', '.rs', '.swift', '.kt', '.scala',
      '.html', '.css', '.scss', '.less', '.vue', '.svelte',
      '.json', '.xml', '.yaml', '.yml', '.toml', '.ini',
      '.md', '.txt', '.rst', '.tex', '.sql'
    ];
    return codeExtensions.some(ext => filePath.endsWith(ext));
  }

  private async processSemanticChange(change: SemanticChange): Promise<void> {
    // Use existing indexer for incremental updates
    await this.indexer.handleFileChange(change.filePath, change.changeType);
    
    // Emit event for MCP tools to refresh context
    (process as any).emit('cortex:contextInvalidated', {
      filePath: change.filePath,
      changeType: change.changeType,
      timestamp: Date.now()
    });
  }

  isWatching(): boolean {
    return this.isActive;
  }

  getStagingStats(): any {
    return this.stagingManager.getStats();
  }

  getStagedFiles(): any[] {
    return this.stagingManager.getStagedFiles();
  }

  getFilesNeedingIndex(): any[] {
    return this.stagingManager.getFilesNeedingIndex();
  }
}