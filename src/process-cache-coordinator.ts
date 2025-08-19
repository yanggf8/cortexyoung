import { SharedMemoryCache } from './shared-memory-cache';
import { log, warn } from './logging-utils';

/**
 * Coordinates caching between main process SharedArrayBuffer
 * and child process local caches via IPC
 */
export class ProcessCacheCoordinator {
  private sharedCache: SharedMemoryCache;
  private childProcesses: Set<any> = new Set();

  constructor(sharedCache: SharedMemoryCache) {
    this.sharedCache = sharedCache;
  }

  /**
   * Register a child process for cache coordination
   */
  registerChildProcess(childProcess: any): void {
    this.childProcesses.add(childProcess);
    
    // Send cache initialization message
    childProcess.send({
      type: 'cache_init',
      config: {
        maxEntries: 1000, // Smaller cache for child processes
        embeddingDim: 384
      }
    });

    // Handle cache requests from child process
    childProcess.on('message', (message: any) => {
      this.handleChildMessage(childProcess, message);
    });

    log(`[CacheCoordinator] Registered child process for cache coordination`);
  }

  /**
   * Unregister a child process
   */
  unregisterChildProcess(childProcess: any): void {
    this.childProcesses.delete(childProcess);
  }

  /**
   * Handle messages from child processes
   */
  private handleChildMessage(childProcess: any, message: any): void {
    switch (message.type) {
      case 'cache_get':
        this.handleCacheGet(childProcess, message);
        break;
      case 'cache_set':
        this.handleCacheSet(message);
        break;
      case 'cache_stats':
        this.handleCacheStatsRequest(childProcess);
        break;
    }
  }

  /**
   * Handle cache get request from child process
   */
  private handleCacheGet(childProcess: any, message: any): void {
    const { contentHash, requestId } = message;
    const embedding = this.sharedCache.get(contentHash);
    
    childProcess.send({
      type: 'cache_get_response',
      requestId,
      embedding: embedding ? Array.from(embedding) : null
    });
  }

  /**
   * Handle cache set from child process
   */
  private handleCacheSet(message: any): void {
    const { contentHash, embedding } = message;
    const embeddingArray = new Float32Array(embedding);
    this.sharedCache.set(contentHash, embeddingArray);
  }

  /**
   * Handle cache stats request
   */
  private handleCacheStatsRequest(childProcess: any): void {
    const stats = this.sharedCache.getStats();
    childProcess.send({
      type: 'cache_stats_response',
      stats
    });
  }

  /**
   * Broadcast cache clear to all child processes
   */
  broadcastCacheClear(): void {
    this.sharedCache.clear();
    for (const childProcess of this.childProcesses) {
      childProcess.send({ type: 'cache_clear' });
    }
  }

  /**
   * Get overall cache statistics
   */
  getOverallStats() {
    const mainStats = this.sharedCache.getStats();
    return {
      main: mainStats,
      childProcessCount: this.childProcesses.size,
      coordination: 'IPC-based'
    };
  }
}