import chokidar from 'chokidar';
import { EventEmitter } from 'eventemitter3';
import { log, warn, error } from './logging-utils';
import { ContentAnalyzer, ContentAnalysis } from './content-analyzer';
import { UsageEvidenceCollector } from './usage-evidence-collector';

interface FileWatcherConfig {
  repositoryPath: string;
  ignorePatterns: string[];
  debounceMs: number;
  batchSize: number;
  adaptiveThreshold: number;
  enableGitIgnore: boolean;
  enableContentAnalysis: boolean;
  analysisThreshold: number; // Minimum importance score to index
  gitFilesOnly: boolean; // Only process git-tracked files
  allowedNonGitPatterns: string[]; // Specific non-git patterns to allow
  collectUsageEvidence: boolean; // Collect evidence of valuable files during usage
}

interface FileChangeEvent {
  type: 'add' | 'change' | 'unlink' | 'addDir' | 'unlinkDir';
  path: string;
  timestamp: number;
  size?: number;
  relativePath: string;
  contentAnalysis?: ContentAnalysis;
  shouldIndex?: boolean;
  indexingPriority?: 'critical' | 'high' | 'medium' | 'low' | 'skip';
  filterReason?: string;
}

export class FileWatcher extends EventEmitter {
  private watcher: chokidar.FSWatcher | null = null;
  private config: FileWatcherConfig;
  private isWatching: boolean = false;
  private contentAnalyzer: ContentAnalyzer;
  private analysisCache: Map<string, { analysis: ContentAnalysis; timestamp: number }> = new Map();
  private evidenceCollector: UsageEvidenceCollector;

  constructor(config: Partial<FileWatcherConfig>) {
    super();
    this.config = {
      repositoryPath: process.cwd(),
      ignorePatterns: [
        '**/node_modules/**',
        '**/.git/**',
        '**/.cortex/**',
        '**/dist/**',
        '**/build/**',
        '**/*.log',
        '**/*.tmp',
        '**/coverage/**',
        '**/.nyc_output/**'
      ],
      debounceMs: 500,
      batchSize: 10,
      adaptiveThreshold: 20,
      enableGitIgnore: true,
      enableContentAnalysis: true,
      analysisThreshold: 20, // Index files with importance >= 20
      gitFilesOnly: true, // Start with git files only
      allowedNonGitPatterns: [], // Can be expanded later if needed
      collectUsageEvidence: true, // Learn from real usage patterns
      ...config
    };
    this.contentAnalyzer = new ContentAnalyzer();
    this.evidenceCollector = new UsageEvidenceCollector(this.config.repositoryPath);
  }

  async initialize(): Promise<void> {
    log('[FileWatcher] Initializing intelligent file watcher...');
    
    // Validate repository path
    const fs = await import('fs/promises');
    try {
      await fs.access(this.config.repositoryPath);
    } catch (err) {
      throw new Error(`Repository path does not exist: ${this.config.repositoryPath}`);
    }
    
    log(`[FileWatcher] Configured for path=${this.config.repositoryPath}`);
    log(`[FileWatcher] Content analysis: ${this.config.enableContentAnalysis ? 'enabled' : 'disabled'}`);
    log(`[FileWatcher] Analysis threshold: ${this.config.analysisThreshold}/100`);
    log(`[FileWatcher] Ignore patterns: ${this.config.ignorePatterns.join(', ')}`);
  }

  async start(): Promise<void> {
    if (this.isWatching) {
      warn('[FileWatcher] Already watching, skipping start');
      return;
    }

    log('[FileWatcher] Starting intelligent file watcher...');
    
    this.watcher = chokidar.watch(this.config.repositoryPath, {
      ignored: this.config.ignorePatterns,
      ignoreInitial: true,
      persistent: true,
      ignorePermissionErrors: true,
      usePolling: false, // Use native events when possible
      atomic: true, // Wait for write operations to complete
      awaitWriteFinish: {
        stabilityThreshold: 100,
        pollInterval: 50
      }
    });

    // Set up event handlers
    this.setupEventHandlers();
    
    this.isWatching = true;
    log('[FileWatcher] Intelligent file watcher started successfully');
  }

  async stop(): Promise<void> {
    if (!this.isWatching || !this.watcher) {
      return;
    }

    log('[FileWatcher] Stopping file watcher...');
    
    await this.watcher.close();
    this.watcher = null;
    this.isWatching = false;
    
    // Clear analysis cache
    this.analysisCache.clear();
    
    // Cleanup evidence collector and save final evidence
    if (this.evidenceCollector) {
      await this.evidenceCollector.cleanup();
    }
    
    log('[FileWatcher] File watcher stopped');
  }

  private setupEventHandlers(): void {
    if (!this.watcher) return;

    this.watcher
      .on('add', (path, stats) => this.handleFileEvent('add', path, stats))
      .on('change', (path, stats) => this.handleFileEvent('change', path, stats))
      .on('unlink', (path) => this.handleFileEvent('unlink', path))
      .on('addDir', (path, stats) => this.handleFileEvent('addDir', path, stats))
      .on('unlinkDir', (path) => this.handleFileEvent('unlinkDir', path))
      .on('error', (err) => {
        error('[FileWatcher] Watcher error:', err);
        this.emit('error', err);
      })
      .on('ready', () => {
        log('[FileWatcher] Initial scan complete, ready for changes');
        this.emit('ready');
      });
  }

  private async handleFileEvent(
    type: FileChangeEvent['type'], 
    path: string, 
    stats?: any
  ): Promise<void> {
    const relativePath = this.getRelativePath(path);
    
    const event: FileChangeEvent = {
      type,
      path,
      relativePath,
      timestamp: Date.now(),
      size: stats?.size
    };

    // For file additions and changes, perform content analysis
    if ((type === 'add' || type === 'change') && this.isAnalyzableFile(path)) {
      if (this.config.enableContentAnalysis) {
        try {
          const analysis = await this.getContentAnalysis(path);
          const filterDecision = this.makeFilteringDecision(analysis, relativePath);
          
          event.contentAnalysis = analysis;
          event.shouldIndex = filterDecision.shouldIndex;
          event.indexingPriority = filterDecision.priority;
          event.filterReason = filterDecision.reason;
          
          // Collect usage evidence
          if (this.config.collectUsageEvidence) {
            const isGitTracked = await this.isGitTracked(relativePath);
            this.evidenceCollector.recordFileChange(relativePath, isGitTracked, event.size || 0);
            this.evidenceCollector.recordContentQuality(relativePath, analysis.estimatedImportance);
          }
          
          log(`[FileWatcher] File ${type}: ${relativePath} (${filterDecision.priority}, ${analysis.estimatedImportance}/100)`);
        } catch (error) {
          warn(`[FileWatcher] Failed to analyze ${relativePath}:`, error);
          // Fallback to basic filtering
          const basicDecision = await this.makeBasicFilteringDecision(relativePath);
          event.shouldIndex = basicDecision.shouldIndex;
          event.indexingPriority = basicDecision.priority;
          event.filterReason = basicDecision.reason;
          
          log(`[FileWatcher] File ${type}: ${relativePath} (basic filter: ${basicDecision.priority})`);
        }
      } else {
        // Content analysis disabled, use basic filtering
        const basicDecision = await this.makeBasicFilteringDecision(relativePath);
        event.shouldIndex = basicDecision.shouldIndex;
        event.indexingPriority = basicDecision.priority;
        event.filterReason = basicDecision.reason;
        
        log(`[FileWatcher] File ${type}: ${relativePath} (${basicDecision.priority})`);
      }
    } else {
      // For deletions or non-analyzable files, use basic filtering
      const basicDecision = await this.makeBasicFilteringDecision(relativePath);
      event.shouldIndex = basicDecision.shouldIndex;
      event.indexingPriority = basicDecision.priority;
      event.filterReason = basicDecision.reason;
      
      log(`[FileWatcher] File ${type}: ${relativePath} (${basicDecision.priority})`);
    }

    this.emit('fileChange', event);
  }

  private getRelativePath(absolutePath: string): string {
    const path = require('path');
    return path.relative(this.config.repositoryPath, absolutePath);
  }

  private isAnalyzableFile(filePath: string): boolean {
    const ext = require('path').extname(filePath).toLowerCase();
    const analyzableExtensions = ['.ts', '.js', '.jsx', '.tsx', '.py', '.md', '.json', '.yml', '.yaml'];
    return analyzableExtensions.includes(ext);
  }

  private async getContentAnalysis(filePath: string): Promise<ContentAnalysis> {
    // Check cache first
    const cached = this.analysisCache.get(filePath);
    if (cached && Date.now() - cached.timestamp < 60000) { // 1 minute cache
      return cached.analysis;
    }

    // Perform fresh analysis
    const analysis = await this.contentAnalyzer.analyzeFile(filePath);
    
    // Cache the result
    this.analysisCache.set(filePath, {
      analysis,
      timestamp: Date.now()
    });

    // Clean old cache entries periodically
    if (this.analysisCache.size > 1000) {
      this.cleanAnalysisCache();
    }

    return analysis;
  }

  private makeFilteringDecision(analysis: ContentAnalysis, relativePath: string): {
    shouldIndex: boolean;
    priority: 'critical' | 'high' | 'medium' | 'low' | 'skip';
    reason: string;
  } {
    const importance = analysis.estimatedImportance;
    
    // Critical files (always index with highest priority)
    if (importance >= 80) {
      return {
        shouldIndex: true,
        priority: 'critical',
        reason: `High importance (${importance}/100), ${analysis.semanticValue} semantic value`
      };
    }
    
    // High priority files
    if (importance >= 65) {
      return {
        shouldIndex: true,
        priority: 'high',
        reason: `Good importance (${importance}/100), ${analysis.fileType} file`
      };
    }
    
    // Medium priority files
    if (importance >= 40) {
      return {
        shouldIndex: true,
        priority: 'medium',
        reason: `Medium importance (${importance}/100), ${analysis.language} file`
      };
    }
    
    // Low priority files (index but with low priority)
    if (importance >= this.config.analysisThreshold) {
      return {
        shouldIndex: true,
        priority: 'low',
        reason: `Low importance (${importance}/100), may be useful`
      };
    }
    
    // Skip files with very low importance
    return {
      shouldIndex: false,
      priority: 'skip',
      reason: `Very low importance (${importance}/100), below threshold (${this.config.analysisThreshold})`
    };
  }

  private async makeBasicFilteringDecision(relativePath: string): Promise<{
    shouldIndex: boolean;
    priority: 'critical' | 'high' | 'medium' | 'low' | 'skip';
    reason: string;
  }> {
    const path = require('path');
    const ext = path.extname(relativePath).toLowerCase();
    const fileName = path.basename(relativePath).toLowerCase();
    
    // Check if file is git-tracked (if git-only mode is enabled)
    if (this.config.gitFilesOnly) {
      const isGitTracked = await this.isGitTracked(relativePath);
      const isAllowedNonGit = this.config.allowedNonGitPatterns.some(pattern => 
        relativePath.match(new RegExp(pattern.replace(/\*/g, '.*')))
      );
      
      if (!isGitTracked && !isAllowedNonGit) {
        return {
          shouldIndex: false,
          priority: 'skip',
          reason: 'Not git-tracked (git-only mode enabled)'
        };
      }
    }
    
    // Critical files by name
    if (fileName.includes('index') || fileName.includes('main') || fileName.includes('app')) {
      return {
        shouldIndex: true,
        priority: 'critical',
        reason: 'Key file by name pattern'
      };
    }
    
    // High priority by extension
    if (['.ts', '.tsx'].includes(ext)) {
      return {
        shouldIndex: true,
        priority: 'high',
        reason: 'TypeScript source file'
      };
    }
    
    // Medium priority
    if (['.js', '.jsx', '.py'].includes(ext)) {
      return {
        shouldIndex: true,
        priority: 'medium',
        reason: 'Source code file'
      };
    }
    
    // Low priority
    if (['.md', '.json', '.yml', '.yaml'].includes(ext)) {
      return {
        shouldIndex: true,
        priority: 'low',
        reason: 'Documentation or config file'
      };
    }
    
    // Skip unknown file types
    return {
      shouldIndex: false,
      priority: 'skip',
      reason: 'Unknown file type or excluded pattern'
    };
  }

  private cleanAnalysisCache(): void {
    const now = Date.now();
    const maxAge = 5 * 60 * 1000; // 5 minutes
    
    for (const [filePath, cached] of this.analysisCache.entries()) {
      if (now - cached.timestamp > maxAge) {
        this.analysisCache.delete(filePath);
      }
    }
    
    log(`[FileWatcher] Cleaned analysis cache, ${this.analysisCache.size} entries remaining`);
  }

  // Enhanced event emission with filtering information
  emit(event: 'fileChange', change: FileChangeEvent): boolean;
  emit(event: 'indexableFileChange', change: FileChangeEvent): boolean;
  emit(event: 'ready'): boolean;
  emit(event: 'error', error: Error): boolean;
  emit(event: string | symbol, ...args: any[]): boolean {
    // If it's a file change event and the file should be indexed, also emit indexableFileChange
    if (event === 'fileChange' && args[0]?.shouldIndex) {
      super.emit('indexableFileChange', args[0]);
    }
    
    return super.emit(event, ...args);
  }

  // Configuration methods
  getConfig(): FileWatcherConfig {
    return { ...this.config };
  }

  updateConfig(updates: Partial<FileWatcherConfig>): void {
    this.config = { ...this.config, ...updates };
    log(`[FileWatcher] Configuration updated: ${Object.keys(updates).join(', ')}`);
  }

  setAnalysisThreshold(threshold: number): void {
    this.config.analysisThreshold = Math.max(0, Math.min(100, threshold));
    log(`[FileWatcher] Analysis threshold set to ${this.config.analysisThreshold}/100`);
  }

  // Status and monitoring methods
  isActive(): boolean {
    return this.isWatching;
  }

  getAnalysisCacheStats(): {
    size: number;
    oldestEntry: Date | null;
    newestEntry: Date | null;
    hitRate?: number;
  } {
    if (this.analysisCache.size === 0) {
      return { size: 0, oldestEntry: null, newestEntry: null };
    }

    let oldest = Date.now();
    let newest = 0;

    for (const cached of this.analysisCache.values()) {
      oldest = Math.min(oldest, cached.timestamp);
      newest = Math.max(newest, cached.timestamp);
    }

    return {
      size: this.analysisCache.size,
      oldestEntry: new Date(oldest),
      newestEntry: new Date(newest)
    };
  }

  // Batch analysis for existing files
  async analyzeExistingFiles(filePaths: string[]): Promise<Map<string, ContentAnalysis>> {
    log(`[FileWatcher] Analyzing ${filePaths.length} existing files...`);
    
    const analyzableFiles = filePaths.filter(fp => this.isAnalyzableFile(fp));
    const results = await this.contentAnalyzer.analyzeFiles(analyzableFiles);
    
    // Cache the results
    const now = Date.now();
    for (const [filePath, analysis] of results.entries()) {
      this.analysisCache.set(filePath, { analysis, timestamp: now });
    }
    
    log(`[FileWatcher] Analyzed ${results.size} files, cached results`);
    return results;
  }

  // Get filtering statistics
  getFilteringStats(analyses: Map<string, ContentAnalysis>): {
    total: number;
    wouldIndex: number;
    byPriority: Record<string, number>;
    byFileType: Record<string, number>;
    averageImportance: number;
  } {
    const stats = {
      total: analyses.size,
      wouldIndex: 0,
      byPriority: { critical: 0, high: 0, medium: 0, low: 0, skip: 0 },
      byFileType: {} as Record<string, number>,
      averageImportance: 0
    };

    let totalImportance = 0;

    for (const [filePath, analysis] of analyses.entries()) {
      const decision = this.makeFilteringDecision(analysis, filePath);
      
      if (decision.shouldIndex) {
        stats.wouldIndex++;
      }
      
      stats.byPriority[decision.priority]++;
      
      const fileType = analysis.fileType;
      stats.byFileType[fileType] = ((stats.byFileType as any)[fileType] || 0) + 1;
      
      totalImportance += analysis.estimatedImportance;
    }

    stats.averageImportance = analyses.size > 0 ? totalImportance / analyses.size : 0;

    return stats;
  }

  clearAnalysisCache(): void {
    this.analysisCache.clear();
    log('[FileWatcher] Analysis cache cleared');
  }

  private async isGitTracked(relativePath: string): Promise<boolean> {
    try {
      const { execSync } = require('child_process');
      const fullPath = require('path').join(this.config.repositoryPath, relativePath);
      
      // Use git ls-files to check if file is tracked
      execSync(`git ls-files --error-unmatch "${fullPath}"`, { 
        cwd: this.config.repositoryPath,
        stdio: 'pipe' // Suppress output
      });
      return true;
    } catch (error) {
      // File is not tracked by git
      return false;
    }
  }

  // Configuration methods for git filtering
  enableGitOnlyMode(): void {
    this.config.gitFilesOnly = true;
    log('[FileWatcher] Git-only mode enabled');
  }

  disableGitOnlyMode(): void {
    this.config.gitFilesOnly = false;
    log('[FileWatcher] Git-only mode disabled');
  }

  addAllowedNonGitPattern(pattern: string): void {
    if (!this.config.allowedNonGitPatterns.includes(pattern)) {
      this.config.allowedNonGitPatterns.push(pattern);
      log(`[FileWatcher] Added allowed non-git pattern: ${pattern}`);
    }
  }

  removeAllowedNonGitPattern(pattern: string): void {
    const index = this.config.allowedNonGitPatterns.indexOf(pattern);
    if (index > -1) {
      this.config.allowedNonGitPatterns.splice(index, 1);
      log(`[FileWatcher] Removed allowed non-git pattern: ${pattern}`);
    }
  }

  // Evidence-based methods
  recordSearchHit(filePath: string, wasClicked: boolean = true): void {
    if (this.config.collectUsageEvidence) {
      this.evidenceCollector.recordSearchHit(filePath, wasClicked);
    }
  }

  getUsageEvidenceReport(): any {
    return this.evidenceCollector.generateEvidenceReport();
  }

  getEvidenceStats(): any {
    return this.evidenceCollector.getStats();
  }

  logEvidenceSummary(): void {
    if (this.config.collectUsageEvidence) {
      this.evidenceCollector.logEvidenceSummary();
    }
  }

  // Apply evidence-based recommendations
  async applyEvidenceBasedRecommendations(minScore: number = 5): Promise<void> {
    if (!this.config.collectUsageEvidence) {
      log('[FileWatcher] Evidence collection disabled, cannot apply recommendations');
      return;
    }

    const report = this.evidenceCollector.generateEvidenceReport();
    
    if (report.valuableNonGitFiles.length > 0) {
      log(`[FileWatcher] Found ${report.valuableNonGitFiles.length} valuable non-git files based on usage evidence`);
      
      // Add recommended patterns
      for (const pattern of report.recommendedPatterns) {
        this.addAllowedNonGitPattern(pattern);
      }
      
      log(`[FileWatcher] Applied ${report.recommendedPatterns.length} evidence-based patterns`);
      
      // Optionally disable git-only mode if we have strong evidence
      if (report.valuableNonGitFiles.length > 10) {
        log('[FileWatcher] Strong evidence for non-git files, consider disabling git-only mode');
      }
    } else {
      log('[FileWatcher] No valuable non-git files found in usage evidence');
    }
  }
}

// Export types for use in other modules
export { FileWatcherConfig, FileChangeEvent };