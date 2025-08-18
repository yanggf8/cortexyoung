import { FileChangeEvent } from './file-watcher';
import { ActivityDetector, ActivityState } from './activity-detector';
import { log, warn } from './logging-utils';

interface ProcessorConfig {
  debounceMs: number;
  batchSize: number;
  maxQueueSize: number;
  priorityWeights: {
    critical: number;
    high: number;
    medium: number;
    low: number;
  };
}

interface ProcessingBatch {
  events: FileChangeEvent[];
  totalImportance: number;
  highestPriority: string;
  processingStarted: Date;
}

export class ChangeProcessor {
  private changeQueue: FileChangeEvent[] = [];
  private debounceTimers: Map<string, NodeJS.Timeout> = new Map();
  private activityDetector: ActivityDetector;
  private config: ProcessorConfig;
  private isProcessing: boolean = false;
  private processingStats = {
    totalProcessed: 0,
    totalSkipped: 0,
    averageProcessingTime: 0,
    lastBatchSize: 0
  };

  constructor(
    activityDetector: ActivityDetector,
    config: Partial<ProcessorConfig> = {}
  ) {
    this.activityDetector = activityDetector;
    this.config = {
      debounceMs: 500,
      batchSize: 10,
      maxQueueSize: 100,
      priorityWeights: {
        critical: 4,
        high: 3,
        medium: 2,
        low: 1
      },
      ...config
    };
  }

  async processChange(event: FileChangeEvent): Promise<void> {
    // Record activity
    this.activityDetector.recordChange(event.relativePath);
    
    // Check if we should suspend processing due to high activity
    const activityState = this.activityDetector.getActivityState();
    if (activityState.suspendProcessing) {
      warn(`[ChangeProcessor] High activity detected (${activityState.changeRate.toFixed(1)}/s), suspending processing for ${event.relativePath}`);
      this.processingStats.totalSkipped++;
      return;
    }
    
    // Skip files that shouldn't be indexed
    if (event.shouldIndex === false) {
      log(`[ChangeProcessor] Skipping ${event.relativePath}: ${event.filterReason}`);
      this.processingStats.totalSkipped++;
      return;
    }
    
    // Clear existing debounce timer for this file
    const existingTimer = this.debounceTimers.get(event.relativePath);
    if (existingTimer) {
      clearTimeout(existingTimer);
    }
    
    // Add to queue with priority-based insertion
    this.addToQueue(event);
    
    // Determine debounce time based on activity and priority
    const debounceTime = this.calculateDebounceTime(event, activityState);
    
    // Set up debounced processing
    const timer = setTimeout(() => {
      this.debouncedProcess(event.relativePath);
      this.debounceTimers.delete(event.relativePath);
    }, debounceTime);
    
    this.debounceTimers.set(event.relativePath, timer);
  }

  private calculateDebounceTime(event: FileChangeEvent, activityState: ActivityState): number {
    let baseDebounce = this.config.debounceMs;
    
    // Adjust based on activity intensity
    switch (activityState.intensity) {
      case 'high':
        baseDebounce *= 3; // Wait longer during high activity
        break;
      case 'medium':
        baseDebounce *= 1.5;
        break;
      case 'low':
        baseDebounce *= 0.8; // Process faster during low activity
        break;
    }
    
    // Adjust based on file priority
    if (event.indexingPriority === 'critical') {
      baseDebounce *= 0.5; // Process critical files faster
    } else if (event.indexingPriority === 'low') {
      baseDebounce *= 2; // Wait longer for low priority files
    }
    
    return Math.max(100, Math.min(baseDebounce, 10000)); // Clamp between 100ms and 10s
  }

  private addToQueue(event: FileChangeEvent): void {
    // Remove any existing events for the same file to avoid duplicates
    this.changeQueue = this.changeQueue.filter(
      e => e.relativePath !== event.relativePath
    );
    
    // Insert event in priority order
    const priority = this.getPriorityWeight(event.indexingPriority || 'low');
    let insertIndex = this.changeQueue.length;
    
    for (let i = 0; i < this.changeQueue.length; i++) {
      const existingPriority = this.getPriorityWeight(this.changeQueue[i].indexingPriority || 'low');
      if (priority > existingPriority) {
        insertIndex = i;
        break;
      }
    }
    
    this.changeQueue.splice(insertIndex, 0, event);
    
    // Enforce queue size limit
    if (this.changeQueue.length > this.config.maxQueueSize) {
      warn(`[ChangeProcessor] Queue size limit exceeded, dropping lowest priority events`);
      // Remove lowest priority events
      this.changeQueue = this.changeQueue.slice(0, this.config.maxQueueSize);
    }
  }

  private getPriorityWeight(priority: string): number {
    return this.config.priorityWeights[priority as keyof typeof this.config.priorityWeights] || 1;
  }

  private async debouncedProcess(filePath: string): Promise<void> {
    if (this.isProcessing) {
      // Schedule for later processing
      setTimeout(() => this.debouncedProcess(filePath), 100);
      return;
    }

    // Find events for this file
    const fileEvents = this.changeQueue.filter(e => e.relativePath === filePath);
    if (fileEvents.length === 0) return;

    // Remove processed events from queue
    this.changeQueue = this.changeQueue.filter(e => e.relativePath !== filePath);

    // Process the events
    await this.processFileEvents(fileEvents);
  }

  private async processFileEvents(events: FileChangeEvent[]): Promise<void> {
    if (events.length === 0) return;

    this.isProcessing = true;
    const startTime = Date.now();
    
    try {
      // Group events by file and take the latest event for each file
      const latestEvents = new Map<string, FileChangeEvent>();
      
      for (const event of events) {
        const existing = latestEvents.get(event.relativePath);
        if (!existing || event.timestamp > existing.timestamp) {
          latestEvents.set(event.relativePath, event);
        }
      }

      const finalEvents = Array.from(latestEvents.values());
      
      if (finalEvents.length > 0) {
        // Create processing batch with metadata
        const batch = this.createProcessingBatch(finalEvents);
        
        log(`[ChangeProcessor] Processing batch: ${finalEvents.length} files, priority: ${batch.highestPriority}, importance: ${batch.totalImportance.toFixed(1)}`);
        
        // Emit batch processing event with enhanced metadata
        (process as any).emit('cortex:fileChangeBatch', batch);
        
        // Update statistics
        this.processingStats.totalProcessed += finalEvents.length;
        this.processingStats.lastBatchSize = finalEvents.length;
        
        const processingTime = Date.now() - startTime;
        this.processingStats.averageProcessingTime = 
          (this.processingStats.averageProcessingTime + processingTime) / 2;
      }
      
    } catch (error) {
      warn(`[ChangeProcessor] Error processing file events:`, error);
    } finally {
      this.isProcessing = false;
    }
  }

  private createProcessingBatch(events: FileChangeEvent[]): ProcessingBatch {
    let totalImportance = 0;
    let highestPriorityWeight = 0;
    let highestPriority = 'low';
    
    for (const event of events) {
      // Calculate total importance
      if (event.contentAnalysis?.estimatedImportance) {
        totalImportance += event.contentAnalysis.estimatedImportance;
      }
      
      // Find highest priority
      const priorityWeight = this.getPriorityWeight(event.indexingPriority || 'low');
      if (priorityWeight > highestPriorityWeight) {
        highestPriorityWeight = priorityWeight;
        highestPriority = event.indexingPriority || 'low';
      }
    }
    
    return {
      events,
      totalImportance,
      highestPriority,
      processingStarted: new Date()
    };
  }

  async processAllPending(): Promise<void> {
    log('[ChangeProcessor] Processing all pending changes...');
    
    // Clear all debounce timers
    for (const timer of this.debounceTimers.values()) {
      clearTimeout(timer);
    }
    this.debounceTimers.clear();

    // Process all queued events immediately
    if (this.changeQueue.length > 0) {
      const allEvents = [...this.changeQueue];
      this.changeQueue = [];
      await this.processFileEvents(allEvents);
    }
    
    log('[ChangeProcessor] All pending changes processed');
  }

  // Configuration methods
  updateConfig(updates: Partial<ProcessorConfig>): void {
    this.config = { ...this.config, ...updates };
    log(`[ChangeProcessor] Configuration updated: ${Object.keys(updates).join(', ')}`);
  }

  setDebounceTime(ms: number): void {
    this.config.debounceMs = Math.max(50, Math.min(ms, 30000)); // Clamp between 50ms and 30s
    log(`[ChangeProcessor] Debounce time set to ${this.config.debounceMs}ms`);
  }

  // Status and monitoring methods
  getQueueStats(): {
    queueSize: number;
    pendingDebounces: number;
    isProcessing: boolean;
    priorityDistribution: Record<string, number>;
    averageImportance: number;
  } {
    const priorityDistribution: Record<string, number> = {
      critical: 0,
      high: 0,
      medium: 0,
      low: 0,
      skip: 0
    };
    
    let totalImportance = 0;
    let importanceCount = 0;
    
    for (const event of this.changeQueue) {
      const priority = event.indexingPriority || 'low';
      priorityDistribution[priority]++;
      
      if (event.contentAnalysis?.estimatedImportance) {
        totalImportance += event.contentAnalysis.estimatedImportance;
        importanceCount++;
      }
    }
    
    return {
      queueSize: this.changeQueue.length,
      pendingDebounces: this.debounceTimers.size,
      isProcessing: this.isProcessing,
      priorityDistribution,
      averageImportance: importanceCount > 0 ? totalImportance / importanceCount : 0
    };
  }

  getProcessingStats(): typeof this.processingStats {
    return { ...this.processingStats };
  }

  clearQueue(): void {
    this.changeQueue = [];
    
    for (const timer of this.debounceTimers.values()) {
      clearTimeout(timer);
    }
    this.debounceTimers.clear();
    
    log('[ChangeProcessor] Queue and timers cleared');
  }

  resetStats(): void {
    this.processingStats = {
      totalProcessed: 0,
      totalSkipped: 0,
      averageProcessingTime: 0,
      lastBatchSize: 0
    };
    log('[ChangeProcessor] Processing statistics reset');
  }
}

export { ProcessorConfig, ProcessingBatch };