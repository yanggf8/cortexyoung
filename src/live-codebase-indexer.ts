import { CodebaseIndexer } from './indexer';
import { FileWatcher, FileChangeEvent } from './file-watcher';
import { ActivityDetector } from './activity-detector';
import { ChangeProcessor } from './change-processor';
import { log, warn, error } from './logging-utils';
import { StartupStageTracker } from './startup-stages';

interface LiveIndexingConfig {
  enableContentAnalysis: boolean;
  analysisThreshold: number;
  debounceMs: number;
  batchSize: number;
  maxConcurrentFiles: number;
  suspendOnHighActivity: boolean;
}

interface LiveIndexingStats {
  filesWatched: number;
  changesProcessed: number;
  averageProcessingTime: number;
  cacheHitRate: number;
  lastProcessedFile: string | null;
  isActive: boolean;
}

export class LiveCodebaseIndexer extends CodebaseIndexer {
  private fileWatcher!: FileWatcher;
  private activityDetector!: ActivityDetector;
  private changeProcessor!: ChangeProcessor;
  private config: LiveIndexingConfig;
  private isLiveModeEnabled: boolean = false;
  private stats: LiveIndexingStats;

  constructor(repositoryPath: string, stageTracker?: StartupStageTracker) {
    super(repositoryPath, stageTracker);
    
    this.config = {
      enableContentAnalysis: true,
      analysisThreshold: 20,
      debounceMs: 500,
      batchSize: 10,
      maxConcurrentFiles: 5,
      suspendOnHighActivity: true
    };

    this.stats = {
      filesWatched: 0,
      changesProcessed: 0,
      averageProcessingTime: 0,
      cacheHitRate: 0,
      lastProcessedFile: null,
      isActive: false
    };

    this.initializeComponents();
  }

  private initializeComponents(): void {
    this.activityDetector = new ActivityDetector();
    
    this.changeProcessor = new ChangeProcessor(this.activityDetector, {
      debounceMs: this.config.debounceMs,
      batchSize: this.config.batchSize,
      maxQueueSize: 100
    });

    this.fileWatcher = new FileWatcher({
      repositoryPath: (this as any).repositoryPath,
      enableContentAnalysis: this.config.enableContentAnalysis,
      analysisThreshold: this.config.analysisThreshold,
      debounceMs: this.config.debounceMs
    });

    this.setupEventHandlers();
  }

  private setupEventHandlers(): void {
    // Handle individual file changes
    this.fileWatcher.on('fileChange', (event: FileChangeEvent) => {
      this.stats.filesWatched++;
      log(`[LiveIndexer] File change detected: ${event.relativePath} (${event.indexingPriority})`);
      
      // Process through our intelligent change processor
      this.changeProcessor.processChange(event);
    });

    // Handle indexable file changes (high-priority events)
    this.fileWatcher.on('indexableFileChange', (event: FileChangeEvent) => {
      log(`[LiveIndexer] High-priority file change: ${event.relativePath} (importance: ${event.contentAnalysis?.estimatedImportance}/100)`);
    });

    // Handle processing batches
    (process as any).on('cortex:fileChangeBatch', async (batch: any) => {
      await this.processBatch(batch);
    });

    // Handle errors
    this.fileWatcher.on('error', (err: Error) => {
      error('[LiveIndexer] File watcher error:', err);
    });
  }

  async enableLiveMode(): Promise<void> {
    if (this.isLiveModeEnabled) {
      warn('[LiveIndexer] Live mode already enabled');
      return;
    }

    log('[LiveIndexer] Enabling intelligent live indexing mode...');

    try {
      // Initialize file watcher
      await this.fileWatcher.initialize();
      await this.fileWatcher.start();

      // Enable real-time updates in base indexer
      await this.enableRealTimeUpdates();

      this.isLiveModeEnabled = true;
      this.stats.isActive = true;

      log('[LiveIndexer] ✅ Live mode enabled - intelligent real-time indexing active');
      log(`[LiveIndexer] Configuration: analysis=${this.config.enableContentAnalysis}, threshold=${this.config.analysisThreshold}/100`);
    } catch (err) {
      error('[LiveIndexer] Failed to enable live mode:', err);
      throw err;
    }
  }

  async disableLiveMode(): Promise<void> {
    if (!this.isLiveModeEnabled) {
      return;
    }

    log('[LiveIndexer] Disabling live mode...');

    try {
      // Stop file watcher
      await this.fileWatcher.stop();

      // Process any pending changes
      await this.changeProcessor.processAllPending();

      // Disable real-time updates in base indexer
      await this.disableRealTimeUpdates();

      this.isLiveModeEnabled = false;
      this.stats.isActive = false;

      log('[LiveIndexer] ✅ Live mode disabled');
    } catch (err) {
      error('[LiveIndexer] Error disabling live mode:', err);
      throw err;
    }
  }

  private async processBatch(batch: any): Promise<void> {
    const startTime = Date.now();
    
    try {
      log(`[LiveIndexer] Processing batch: ${batch.events.length} files, priority: ${batch.highestPriority}`);

      // Group events by type for efficient processing
      const additions: FileChangeEvent[] = [];
      const modifications: FileChangeEvent[] = [];
      const deletions: FileChangeEvent[] = [];

      for (const event of batch.events) {
        switch (event.type) {
          case 'add':
            additions.push(event);
            break;
          case 'change':
            modifications.push(event);
            break;
          case 'unlink':
            deletions.push(event);
            break;
        }
      }

      // Process deletions first
      for (const event of deletions) {
        await this.handleFileChange(event.path, 'deleted');
      }

      // Process additions and modifications
      const filesToProcess = [...additions, ...modifications];
      
      if (filesToProcess.length > 0) {
        // Use intelligent batching based on priority
        if (batch.highestPriority === 'critical') {
          // Process critical files immediately, one by one
          for (const event of filesToProcess) {
            await this.handleFileChange(event.path, event.type);
          }
        } else {
          // Batch process medium/low priority files
          await this.batchProcessFiles(filesToProcess);
        }
      }

      // Update statistics
      const processingTime = Date.now() - startTime;
      this.stats.changesProcessed += batch.events.length;
      this.stats.averageProcessingTime = 
        (this.stats.averageProcessingTime + processingTime) / 2;
      this.stats.lastProcessedFile = batch.events[batch.events.length - 1]?.relativePath || null;

      log(`[LiveIndexer] Batch processed in ${processingTime}ms`);

    } catch (err) {
      error('[LiveIndexer] Error processing batch:', err);
    }
  }

  private async batchProcessFiles(events: FileChangeEvent[]): Promise<void> {
    // Process files in parallel batches for efficiency
    const batchSize = this.config.maxConcurrentFiles;
    
    for (let i = 0; i < events.length; i += batchSize) {
      const batch = events.slice(i, i + batchSize);
      
      const promises = batch.map(event => 
        this.handleFileChange(event.path, event.type)
          .catch(err => warn(`[LiveIndexer] Failed to process ${event.relativePath}:`, err))
      );
      
      await Promise.all(promises);
    }
  }

  // Enhanced file change handling with intelligence
  async handleFileChange(filePath: string, changeType: string): Promise<void> {
    // Call parent's handleFileChange but with enhanced logging
    const relativePath = require('path').relative((this as any).repositoryPath, filePath);
    
    log(`[LiveIndexer] Processing file change: ${changeType} ${relativePath}`);
    
    try {
      await super.handleFileChange(filePath, changeType);
      log(`[LiveIndexer] ✅ Successfully processed ${relativePath}`);
    } catch (err) {
      warn(`[LiveIndexer] ❌ Failed to process ${relativePath}:`, err);
      throw err;
    }
  }

  // Configuration methods
  updateConfig(updates: Partial<LiveIndexingConfig>): void {
    this.config = { ...this.config, ...updates };
    
    // Update components with new config
    if (updates.debounceMs !== undefined) {
      this.changeProcessor.setDebounceTime(updates.debounceMs);
    }
    
    if (updates.analysisThreshold !== undefined) {
      this.fileWatcher.setAnalysisThreshold(updates.analysisThreshold);
    }
    
    log(`[LiveIndexer] Configuration updated: ${Object.keys(updates).join(', ')}`);
  }

  // Monitoring and statistics
  getLiveStats(): LiveIndexingStats & {
    activityState: any;
    queueStats: any;
    cacheStats: any;
  } {
    const activityState = this.activityDetector.getActivityState();
    const queueStats = this.changeProcessor.getQueueStats();
    const cacheStats = this.fileWatcher.getAnalysisCacheStats();

    return {
      ...this.stats,
      activityState,
      queueStats,
      cacheStats
    };
  }

  getConfig(): LiveIndexingConfig {
    return { ...this.config };
  }

  isLiveMode(): boolean {
    return this.isLiveModeEnabled;
  }

  // Force immediate processing of pending changes
  async flushPendingChanges(): Promise<void> {
    log('[LiveIndexer] Flushing all pending changes...');
    await this.changeProcessor.processAllPending();
    log('[LiveIndexer] All pending changes processed');
  }

  // Analyze existing files for initial intelligence
  async analyzeExistingFiles(): Promise<void> {
    if (!this.isLiveModeEnabled) {
      throw new Error('Live mode must be enabled before analyzing existing files');
    }

    log('[LiveIndexer] Analyzing existing files for content intelligence...');
    
    // Get all files from git scanner
    const scanResult = await (this as any).gitScanner.scanRepository('full');
    
    // Analyze files in batches
    const analyses = await this.fileWatcher.analyzeExistingFiles(scanResult.files);
    
    // Get filtering statistics
    const filterStats = this.fileWatcher.getFilteringStats(analyses);
    
    log(`[LiveIndexer] Analysis complete:`);
    log(`  Total files: ${filterStats.total}`);
    log(`  Would index: ${filterStats.wouldIndex} (${((filterStats.wouldIndex / filterStats.total) * 100).toFixed(1)}%)`);
    log(`  Average importance: ${filterStats.averageImportance.toFixed(1)}/100`);
    log(`  Priority distribution:`, filterStats.byPriority);
  }

  // Cleanup
  async cleanup(reason: string = 'cleanup'): Promise<void> {
    await this.disableLiveMode();
    await super.cleanup(reason);
  }
}

export { LiveIndexingConfig, LiveIndexingStats };