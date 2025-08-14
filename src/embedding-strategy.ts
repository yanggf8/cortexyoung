import { CodeChunk } from './types';
import { ProcessPoolEmbedder } from './process-pool-embedder';
import { CachedEmbedder } from './cached-embedder';
import { log } from './logging-utils';

export type EmbeddingStrategy = 'process-pool' | 'cached' | 'auto' | 'original'; // 'original' deprecated, redirects to 'cached'

export interface EmbeddingStrategyConfig {
  strategy: EmbeddingStrategy;
  batchSize?: number;
  processCount?: number;
  timeoutMs?: number;
}

export interface EmbeddingResult {
  chunks: CodeChunk[];
  strategy: EmbeddingStrategy;
  performance: {
    totalTime: number;
    averageBatchTime: number;
    totalBatches: number;
    chunksPerSecond: number;
    peakMemoryMB: number;
    cacheStats?: {
      hits: number;
      misses: number;
      hitRate: number;
    };
  };
}

export class EmbeddingStrategyManager {
  private processPoolEmbedder?: ProcessPoolEmbedder;
  private cachedEmbedder?: CachedEmbedder;
  private repositoryPath: string;

  constructor(repositoryPath: string) {
    this.repositoryPath = repositoryPath;
    // Note: originalEmbedder removed - ProcessPool handles all cases
  }

  /**
   * Determine the best embedding strategy based on environment and chunk count
   */
  private determineAutoStrategy(chunkCount: number): EmbeddingStrategy {
    // Simplified auto-selection: ProcessPool starts with 1 process, so no need for original strategy
    // Use cached strategy for cache benefits, fallback to process-pool for large datasets
    
    // Use cached strategy for datasets that can benefit from caching
    if (chunkCount < 500) {
      return 'cached';
    }

    // Use process pool for large datasets (will scale beyond single process)
    return 'process-pool';
  }

  /**
   * Generate embeddings using the specified strategy
   */
  async generateEmbeddings(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<EmbeddingResult> {
    const startTime = Date.now();
    let strategy = config.strategy;

    // Resolve auto strategy
    if (strategy === 'auto') {
      strategy = this.determineAutoStrategy(chunks.length);
      log(`[EmbeddingStrategy] Auto-selected strategy=${strategy} chunks=${chunks.length}`);
    }

    log(`[EmbeddingStrategy] Using strategy=${strategy} chunks=${chunks.length}`);

    let result: CodeChunk[];
    let batchInfo: { totalBatches: number; averageBatchTime: number };
    let cacheStats: { hits: number; misses: number; hitRate: number } | undefined;

    const startMemory = process.memoryUsage();
    let peakMemory = startMemory.heapUsed;
    let memoryWarningCount = 0;

    // Enhanced memory monitoring with pressure detection
    const memoryInterval = setInterval(() => {
      const currentMemory = process.memoryUsage();
      const currentHeap = currentMemory.heapUsed;
      
      if (currentHeap > peakMemory) {
        peakMemory = currentHeap;
      }
      
      // Memory pressure detection using available system memory
      const os = require('os');
      const totalSystemMemory = os.totalmem();
      const availableMemory = os.freemem();
      const availableMemoryGB = availableMemory / (1024 * 1024 * 1024);
      
      // Trigger warning when available memory drops below 2GB (conservative threshold)
      if (availableMemoryGB < 2.0) {
        memoryWarningCount++;
        if (memoryWarningCount === 1) {
          const totalGB = (totalSystemMemory / (1024 * 1024 * 1024)).toFixed(1);
          const usedGB = (parseFloat(totalGB) - availableMemoryGB).toFixed(1);
          console.warn(`⚠️  Memory pressure detected: Only ${availableMemoryGB.toFixed(1)}GB available (${usedGB}GB used of ${totalGB}GB total)`);
        }
      }
    }, 500);

    try {
      switch (strategy) {
        case 'process-pool':
          result = await this.generateWithProcessPool(chunks, config);
          batchInfo = this.calculateProcessPoolBatchInfo(chunks);
          break;

        case 'cached':
          const cachedResult = await this.generateWithCached(chunks, config);
          result = cachedResult.chunks;
          batchInfo = cachedResult.batchInfo;
          cacheStats = cachedResult.cacheStats;
          break;

        case 'original':
          // Legacy support - redirect to cached strategy (ProcessPool with 1 process)
          console.log(`⚠️ 'original' strategy deprecated - using 'cached' strategy instead`);
          const legacyResult = await this.generateWithCached(chunks, config);
          result = legacyResult.chunks;
          batchInfo = legacyResult.batchInfo;
          cacheStats = legacyResult.cacheStats;
          break;

        default:
          throw new Error(`Unknown embedding strategy: ${strategy}`);
      }
    } finally {
      clearInterval(memoryInterval);
    }

    const totalTime = Date.now() - startTime;
    const chunksPerSecond = chunks.length / (totalTime / 1000);

    return {
      chunks: result,
      strategy,
      performance: {
        totalTime,
        averageBatchTime: batchInfo.averageBatchTime,
        totalBatches: batchInfo.totalBatches,
        chunksPerSecond,
        peakMemoryMB: peakMemory / (1024 * 1024),
        cacheStats
      }
    };
  }

  // Note: generateWithOriginal removed - ProcessPool with 1 process handles small workloads

  /**
   * Generate embeddings using the ProcessPool approach
   */
  private async generateWithProcessPool(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<CodeChunk[]> {
    this.processPoolEmbedder = new ProcessPoolEmbedder();

    try {
      console.log(`📊 ProcessPool Strategy: Initializing processes...`);
      
      const result = await this.processPoolEmbedder.processAllEmbeddings(chunks);
      
      return result;

    } finally {
      if (this.processPoolEmbedder) {
        await this.processPoolEmbedder.shutdown();
        this.processPoolEmbedder = undefined;
      }
    }
  }

  /**
   * Generate embeddings using the cached approach
   */
  private async generateWithCached(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<{
    chunks: CodeChunk[]; 
    batchInfo: { totalBatches: number; averageBatchTime: number };
    cacheStats: { hits: number; misses: number; hitRate: number };
  }> {
    this.cachedEmbedder = new CachedEmbedder(this.repositoryPath);

    try {
      console.log(`📊 Cached Strategy: Initializing with intelligent embedding cache...`);
      await this.cachedEmbedder.initialize(chunks.length);
      
      const result = await this.cachedEmbedder.processAllEmbeddings(chunks);
      const stats = await this.cachedEmbedder.getEmbeddingCacheStats();

      return {
        chunks: result,
        batchInfo: this.calculateCachedBatchInfo(chunks, stats),
        cacheStats: {
          hits: stats.cache_hits,
          misses: stats.cache_misses,
          hitRate: stats.hit_rate
        }
      };

    } finally {
      if (this.cachedEmbedder) {
        await this.cachedEmbedder.shutdown();
        this.cachedEmbedder = undefined;
      }
    }
  }

  private createEmbeddingText(chunk: CodeChunk): string {
    // Consistent embedding text generation across strategies
    const parts = [];
    
    if (chunk.symbol_name) {
      parts.push(chunk.symbol_name);
    }
    
    parts.push(chunk.chunk_type);
    parts.push(chunk.content);
    
    if (chunk.relationships.imports.length > 0) {
      parts.push(chunk.relationships.imports.slice(0, 3).join(' '));
    }
    
    return parts.join(' ');
  }

  // Note: calculateOriginalBatchInfo removed - original strategy deprecated

  private calculateProcessPoolBatchInfo(chunks: CodeChunk[]) {
    // ProcessPool uses fixed 400-chunk batches optimized for BGE-small-en-v1.5
    const fixedBatchSize = 400;
    const totalBatches = Math.ceil(chunks.length / fixedBatchSize);
    const estimatedBatchTime = fixedBatchSize * 100; // ~100ms per chunk estimate for ProcessPool
    return {
      totalBatches,
      averageBatchTime: estimatedBatchTime
    };
  }

  private calculateCachedBatchInfo(chunks: CodeChunk[], stats: any) {
    // Cached strategy performance depends on cache hit rate
    const effectiveChunks = stats.cache_misses; // Only uncached chunks need processing
    const typicalBatchSize = 50;
    const totalBatches = Math.ceil(effectiveChunks / typicalBatchSize) || 1;
    // Much faster due to cache hits
    const estimatedBatchTime = effectiveChunks > 0 ? (effectiveChunks * 100) : 100; // ~100ms per chunk for uncached
    return {
      totalBatches,
      averageBatchTime: estimatedBatchTime
    };
  }

  /**
   * Get strategy configuration from environment variables
   */
  static getConfigFromEnv(): EmbeddingStrategyConfig {
    const strategy = (process.env.EMBEDDING_STRATEGY as EmbeddingStrategy) || 'auto';
    const batchSize = process.env.EMBEDDING_BATCH_SIZE ? parseInt(process.env.EMBEDDING_BATCH_SIZE) : undefined;
    const processCount = process.env.EMBEDDING_PROCESS_COUNT ? parseInt(process.env.EMBEDDING_PROCESS_COUNT) : undefined;
    const timeoutMs = process.env.EMBEDDING_TIMEOUT_MS ? parseInt(process.env.EMBEDDING_TIMEOUT_MS) : undefined;

    return {
      strategy,
      batchSize,
      processCount,
      timeoutMs
    };
  }

  /**
   * Cleanup resources
   */
  async cleanup(): Promise<void> {
    if (this.processPoolEmbedder) {
      await this.processPoolEmbedder.shutdown();
      this.processPoolEmbedder = undefined;
    }
    if (this.cachedEmbedder) {
      await this.cachedEmbedder.shutdown();
      this.cachedEmbedder = undefined;
    }
  }
}