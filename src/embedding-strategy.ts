import { CodeChunk } from './types';
import { EmbeddingGenerator } from './embedder';
import { ProcessPoolEmbedder } from './process-pool-embedder';
import { CachedEmbedder } from './cached-embedder';

export type EmbeddingStrategy = 'original' | 'process-pool' | 'cached' | 'auto';

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
  private originalEmbedder: EmbeddingGenerator;
  private processPoolEmbedder?: ProcessPoolEmbedder;
  private cachedEmbedder?: CachedEmbedder;
  private repositoryPath: string;

  constructor(repositoryPath: string) {
    this.repositoryPath = repositoryPath;
    this.originalEmbedder = new EmbeddingGenerator();
  }

  /**
   * Determine the best embedding strategy based on environment and chunk count
   */
  private determineAutoStrategy(chunkCount: number): EmbeddingStrategy {
    // Auto-selection logic based on chunk count and system resources
    const os = require('os');
    const cpuCount = os.cpus().length;
    const totalMemoryGB = os.totalmem() / (1024 * 1024 * 1024);

    // Use original strategy for very small datasets
    if (chunkCount < 50) {
      return 'original';
    }

    // Use cached strategy for medium datasets
    if (chunkCount >= 50 && chunkCount < 500) {
      return 'cached';
    }

    // Use process pool for large datasets with sufficient resources
    if (cpuCount >= 4 && totalMemoryGB >= 4 && chunkCount >= 500) {
      return 'process-pool';
    }

    // Default to cached for good performance with cache benefits
    return 'cached';
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
      console.log(`🤖 Auto-selected embedding strategy: ${strategy} (${chunks.length} chunks)`);
    }

    console.log(`🚀 Using ${strategy} embedding strategy for ${chunks.length} chunks`);

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
        case 'original':
          result = await this.generateWithOriginal(chunks, config);
          batchInfo = this.calculateOriginalBatchInfo(chunks, config.batchSize || 100);
          break;

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

  /**
   * Generate embeddings using the original single-threaded approach
   */
  private async generateWithOriginal(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<CodeChunk[]> {
    // Initialize model
    const modelInfo = await this.originalEmbedder.getModelInfo();
    console.log(`📊 Original Strategy: Model ${modelInfo.name} ready (${modelInfo.dimension}D)`);

    const batchSize = config.batchSize || 100;
    const embeddedChunks: CodeChunk[] = [];
    const totalBatches = Math.ceil(chunks.length / batchSize);

    console.log(`📊 Processing ${chunks.length} chunks in ${totalBatches} batches of ${batchSize}...`);

    for (let i = 0; i < chunks.length; i += batchSize) {
      const batchStartTime = Date.now();
      const batch = chunks.slice(i, i + batchSize);
      const texts = batch.map(chunk => this.createEmbeddingText(chunk));

      try {
        const embeddings = await this.originalEmbedder.embedBatch(texts);

        for (let j = 0; j < batch.length; j++) {
          embeddedChunks.push({
            ...batch[j],
            embedding: embeddings[j] || []
          });
        }

        const batchTime = Date.now() - batchStartTime;
        const currentBatch = Math.floor(i / batchSize) + 1;
        const progress = (currentBatch / totalBatches) * 100;

        console.log(`📊 [Original] Batch ${currentBatch}/${totalBatches}: ${progress.toFixed(1)}% (${batchTime}ms)`);

      } catch (error) {
        console.warn(`Failed to generate embeddings for batch starting at ${i}:`, error);
        embeddedChunks.push(...batch);
      }
    }

    return embeddedChunks;
  }

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
      await this.cachedEmbedder.initialize();
      
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

  private calculateOriginalBatchInfo(chunks: CodeChunk[], batchSize: number) {
    const totalBatches = Math.ceil(chunks.length / batchSize);
    // Estimate based on typical batch times
    const estimatedBatchTime = batchSize * 15; // ~15ms per chunk estimate
    return {
      totalBatches,
      averageBatchTime: estimatedBatchTime
    };
  }

  private calculateProcessPoolBatchInfo(chunks: CodeChunk[]) {
    // ProcessPool typically processes in ~50 chunk batches
    const typicalBatchSize = 50;
    const totalBatches = Math.ceil(chunks.length / typicalBatchSize);
    const estimatedBatchTime = 57000; // ~57s per 50-chunk batch based on documentation
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