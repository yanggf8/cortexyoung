import chokidar from 'chokidar';
import { EventEmitter } from 'events';
import { join } from 'path';
import type { EnhancedProjectDetector } from './project-detector.js';
import type { CLAUDEMdMaintainer } from './claude-md-maintainer.js';

export interface ContextWatcherEvents {
  'context-updated': [];
  'pattern-changed': [string]; // file path that changed
  'error': [Error];
}

export class ContextWatcher extends EventEmitter<ContextWatcherEvents> {
  private detector: EnhancedProjectDetector;
  private maintainer: CLAUDEMdMaintainer;
  private projectRoot: string;
  private watcher: chokidar.FSWatcher | null = null;
  private debounceTimer: NodeJS.Timeout | null = null;
  private isProcessingUpdate = false;

  constructor(
    detector: EnhancedProjectDetector,
    maintainer: CLAUDEMdMaintainer,
    projectRoot: string = process.cwd()
  ) {
    super();
    this.detector = detector;
    this.maintainer = maintainer;
    this.projectRoot = projectRoot;
  }

  /**
   * Start watching project files for changes
   */
  startWatching(): void {
    const watchPatterns = this.getWatchPatterns();
    
    this.watcher = chokidar.watch(watchPatterns, {
      cwd: this.projectRoot,
      ignored: this.getIgnorePatterns(),
      persistent: true,
      ignoreInitial: false, // Process existing files on startup
      followSymlinks: false,
      awaitWriteFinish: {
        stabilityThreshold: 100,
        pollInterval: 50
      }
    });

    this.watcher.on('change', (path) => this.handleFileChange(path, 'change'));
    this.watcher.on('add', (path) => this.handleFileChange(path, 'add'));
    this.watcher.on('unlink', (path) => this.handleFileChange(path, 'unlink'));
    this.watcher.on('error', (error) => this.handleError(error));
    this.watcher.on('ready', () => {
      console.log('🔍 ContextWatcher started - monitoring project structure + implementation patterns');
      // Initial context update on startup
      this.updateContextAfterDelay();
    });
  }

  /**
   * Stop watching files
   */
  stopWatching(): void {
    if (this.watcher) {
      this.watcher.close();
      this.watcher = null;
    }
    
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    
    console.log('🔍 ContextWatcher stopped');
  }

  /**
   * Force immediate context update (useful for testing)
   */
  async forceUpdate(): Promise<void> {
    await this.updateCLAUDEMdContext();
  }

  /**
   * Get patterns to watch for changes
   */
  private getWatchPatterns(): string[] {
    return [
      // Project structure files
      'package.json',
      'tsconfig.json',
      'go.mod',
      'requirements.txt',
      'pyproject.toml',
      'Cargo.toml',
      
      // Implementation pattern files - more specific patterns
      'src/**/*.{ts,js,py,go,rs}',
      'lib/**/*.{ts,js,py,go,rs}',
      'app/**/*.{ts,js,py,go,rs}',
      
      // Specific pattern-relevant directories
      '**/auth/**/*.{ts,js,py,go,rs}',
      '**/middleware/**/*.{ts,js,py,go,rs}',
      '**/controllers/**/*.{ts,js,py,go,rs}',
      '**/routes/**/*.{ts,js,py,go,rs}',
      '**/handlers/**/*.{ts,js,py,go,rs}',
      '**/guards/**/*.{ts,js,py,go,rs}',
      '**/error*/**/*.{ts,js,py,go,rs}',
      '**/exception*/**/*.{ts,js,py,go,rs}'
    ];
  }

  /**
   * Get patterns to ignore during watching
   */
  private getIgnorePatterns(): string[] {
    return [
      '**/node_modules/**',
      '**/.git/**',
      '**/dist/**',
      '**/build/**',
      '**/target/**',
      '**/__pycache__/**',
      '**/venv/**',
      '**/env/**',
      '**/.env*',
      '**/CLAUDE.md', // Ignore our own writes to prevent loops
      '**/*.claudecat.tmp', // Ignore temporary files
      '**/*.log',
      '**/*.lock',
      '**/coverage/**',
      '**/.nyc_output/**'
    ];
  }

  /**
   * Handle individual file change events
   */
  private handleFileChange(path: string, eventType: 'change' | 'add' | 'unlink'): void {
    const fullPath = join(this.projectRoot, path);
    
    // Check if this is a pattern-relevant file
    if (this.isPatternRelevantFile(path)) {
      console.log(`📁 Pattern-relevant file ${eventType}: ${path}`);
      this.emit('pattern-changed', fullPath);
      
      // Debounced update - wait for no new events for 500ms
      this.updateContextAfterDelay();
    } else if (this.isStructureRelevantFile(path)) {
      console.log(`📦 Structure file ${eventType}: ${path}`);
      
      // Structure changes get faster updates
      this.updateContextAfterDelay(1000);
    }
  }

  /**
   * Check if file is relevant to implementation patterns
   */
  private isPatternRelevantFile(path: string): boolean {
    const patternKeywords = [
      '/auth/', '/middleware/', '/guards/',
      '/controllers/', '/routes/', '/handlers/',
      '/error', '/exception'
    ];
    
    const fileExtensions = ['.ts', '.js', '.py', '.go', '.rs'];
    
    return patternKeywords.some(keyword => path.includes(keyword)) &&
           fileExtensions.some(ext => path.endsWith(ext));
  }

  /**
   * Check if file is relevant to project structure
   */
  private isStructureRelevantFile(path: string): boolean {
    const structureFiles = [
      'package.json', 'tsconfig.json', 'go.mod', 
      'requirements.txt', 'pyproject.toml', 'Cargo.toml'
    ];
    
    return structureFiles.some(file => path.endsWith(file));
  }

  /**
   * Update context after a delay (debouncing)
   */
  private updateContextAfterDelay(delay: number = 500): void {
    // Clear existing timer
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
    }
    
    // Set new timer
    this.debounceTimer = setTimeout(() => {
      this.updateCLAUDEMdContext();
    }, delay);
  }

  /**
   * Update CLAUDE.md with current context
   */
  private async updateCLAUDEMdContext(): Promise<void> {
    // Prevent concurrent updates
    if (this.isProcessingUpdate) {
      console.log('⏳ Context update already in progress, skipping...');
      return;
    }

    this.isProcessingUpdate = true;
    
    try {
      console.log('🔄 Updating project context...');
      
      const newContext = this.detector.detectCurrentContext();
      await this.maintainer.updateProjectContext(newContext);
      
      this.emit('context-updated');
      console.log('✅ Project context update completed');
      
    } catch (error) {
      console.error('❌ Failed to update project context:', error);
      this.emit('error', error instanceof Error ? error : new Error(String(error)));
    } finally {
      this.isProcessingUpdate = false;
    }
  }

  /**
   * Handle watcher errors
   */
  private handleError(error: Error): void {
    console.error('❌ ContextWatcher error:', error);
    this.emit('error', error);
  }

  /**
   * Get current watch status
   */
  getStatus(): {
    isWatching: boolean;
    watchedPaths: string[];
    isProcessingUpdate: boolean;
  } {
    return {
      isWatching: this.watcher !== null,
      watchedPaths: this.watcher?.getWatched() ? Object.keys(this.watcher.getWatched()) : [],
      isProcessingUpdate: this.isProcessingUpdate
    };
  }

  /**
   * Get watch statistics
   */
  getStats(): {
    watcherReady: boolean;
    watchedFileCount: number;
    lastUpdateTime?: Date;
  } {
    const watched = this.watcher?.getWatched() || {};
    const fileCount = Object.values(watched).reduce((total, files) => total + files.length, 0);
    
    return {
      watcherReady: this.watcher !== null,
      watchedFileCount: fileCount,
      lastUpdateTime: this.debounceTimer ? undefined : new Date()
    };
  }
}