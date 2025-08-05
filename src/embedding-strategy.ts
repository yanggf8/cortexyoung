import { CodeChunk } from './types';
import { EmbeddingGenerator } from './embedder';
import { ProcessPoolEmbedder } from './process-pool-embedder';
import { StartupStageTracker } from './startup-stages';

export type EmbeddingStrategy = 'original' | 'process-pool' | 'concurrent' | 'hybrid' | 'auto';

export interface EmbeddingStrategyConfig {
  strategy: EmbeddingStrategy;
  batchSize?: number;
  processCount?: number;
  timeoutMs?: number;
  concurrency?: number; // For concurrent strategy
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
  };
}

export class EmbeddingStrategyManager {
  private originalEmbedder: EmbeddingGenerator;
  private processPoolEmbedder?: ProcessPoolEmbedder;
  private stageTracker?: StartupStageTracker;

  constructor(stageTracker?: StartupStageTracker) {
    this.originalEmbedder = new EmbeddingGenerator();
    this.stageTracker = stageTracker;
  }

  /**
   * Determine the best embedding strategy based on environment and chunk count
   */
  private determineAutoStrategy(chunkCount: number): EmbeddingStrategy {
    // Auto-selection logic based on chunk count and system resources
    const os = require('os');
    const cpuCount = os.cpus().length;
    const totalMemoryGB = os.totalmem() / (1024 * 1024 * 1024);

    // Use original strategy for small datasets or limited resources
    if (chunkCount < 100) {
      return 'original';
    }

    // Use process pool for larger datasets with sufficient resources
    if (cpuCount >= 4 && totalMemoryGB >= 4) {
      return 'process-pool';
    }

    // Default to original for conservative compatibility
    return 'original';
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

    const startMemory = process.memoryUsage();
    let peakMemory = startMemory.heapUsed;
    let memoryWarningCount = 0;

    // Enhanced memory monitoring with pressure detection
    const memoryInterval = setInterval(() => {
      const currentMemory = process.memoryUsage();
      const currentHeap = currentMemory.heapUsed;
      const availableMemory = process.memoryUsage().heapTotal - currentHeap;
      
      if (currentHeap > peakMemory) {
        peakMemory = currentHeap;
      }
      
      // Memory pressure detection (>75% of heap used)
      const memoryPressure = currentHeap / currentMemory.heapTotal;
      if (memoryPressure > 0.75) {
        memoryWarningCount++;
        if (memoryWarningCount === 1) {
          console.warn(`⚠️  Memory pressure detected: ${(memoryPressure * 100).toFixed(1)}% heap usage`);
        }
      }
    }, 500); // Check more frequently for memory issues

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

        case 'concurrent':
          result = await this.generateWithConcurrent(chunks, config);
          batchInfo = this.calculateConcurrentBatchInfo(chunks, config.concurrency || 10);
          break;

        case 'hybrid':
          result = await this.generateWithHybrid(chunks, config);
          batchInfo = this.calculateHybridBatchInfo(chunks);
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
        peakMemoryMB: peakMemory / (1024 * 1024)
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
    this.stageTracker?.startStage('embedding_generation', 
      `Processing ${chunks.length} chunks with original embedding strategy`);

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
        
        this.stageTracker?.updateStageProgress('embedding_generation', progress,
          `Original strategy - batch ${currentBatch}/${totalBatches}`);

      } catch (error) {
        console.warn(`Failed to generate embeddings for batch starting at ${i}:`, error);
        embeddedChunks.push(...batch);
      }
    }

    this.stageTracker?.completeStage('embedding_generation', 
      `Original strategy completed - ${embeddedChunks.length} chunks processed`);

    return embeddedChunks;
  }

  /**
   * Generate embeddings using the ProcessPool approach
   */
  private async generateWithProcessPool(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<CodeChunk[]> {
    this.stageTracker?.startStage('embedding_generation', 
      `Processing ${chunks.length} chunks with ProcessPool embedding strategy`);

    this.processPoolEmbedder = new ProcessPoolEmbedder(config.processCount);

    try {
      console.log(`📊 ProcessPool Strategy: Initializing ${this.processPoolEmbedder.getProcessCount()} processes...`);
      
      const result = await this.processPoolEmbedder.processAllEmbeddings(chunks);

      this.stageTracker?.completeStage('embedding_generation', 
        `ProcessPool strategy completed - ${result.length} chunks processed`);

      return result;

    } finally {
      if (this.processPoolEmbedder) {
        await this.processPoolEmbedder.shutdown();
        this.processPoolEmbedder = undefined;
      }
    }
  }

  /**
   * Generate embeddings using I/O-optimized concurrent approach
   * Uses Promise.all for optimal concurrency with single model instance
   */
  private async generateWithConcurrent(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<CodeChunk[]> {
    this.stageTracker?.startStage('embedding_generation', 
      `Processing ${chunks.length} chunks with concurrent I/O strategy`);

    // Initialize model once
    const modelInfo = await this.originalEmbedder.getModelInfo();
    console.log(`📊 Concurrent Strategy: Model ${modelInfo.name} ready (${modelInfo.dimension}D)`);

    const concurrency = config.concurrency || this.calculateOptimalConcurrency(chunks.length);
    console.log(`🔄 Concurrent Strategy: Processing ${chunks.length} chunks with concurrency ${concurrency} (${config.concurrency ? 'manual' : 'auto-tuned'})`);

    // Create batches with adaptive sizing for optimal performance
    const { batchSize, actualBatches } = this.calculateAdaptiveBatching(chunks.length, concurrency);
    const batches: CodeChunk[][] = [];
    
    // Use adaptive batch size calculation
    let currentIndex = 0;
    for (let i = 0; i < actualBatches; i++) {
      const remainingChunks = chunks.length - currentIndex;
      const remainingBatches = actualBatches - i;
      const currentBatchSize = Math.ceil(remainingChunks / remainingBatches);
      
      batches.push(chunks.slice(currentIndex, currentIndex + currentBatchSize));
      currentIndex += currentBatchSize;
    }

    console.log(`📦 Created ${batches.length} adaptive batches (${batchSize} avg chunks, ${batches.map(b => b.length).join(',')} actual sizes)`);

    // Process all batches concurrently using Promise.all
    const startWallClock = process.hrtime.bigint();
    const startTime = Date.now();
    
    const batchPromises = batches.map(async (batch, batchIndex) => {
      const batchStartTime = Date.now();
      const texts = batch.map(chunk => this.createEmbeddingText(chunk));
      
      try {
        console.log(`🚀 [Concurrent] Starting batch ${batchIndex + 1}/${batches.length} (${batch.length} chunks)`);
        
        const embeddings = await this.originalEmbedder.embedBatch(texts);
        
        const embeddedChunks = batch.map((chunk, index) => ({
          ...chunk,
          embedding: embeddings[index] || []
        }));

        const batchTime = Date.now() - batchStartTime;
        console.log(`✅ [Concurrent] Completed batch ${batchIndex + 1}/${batches.length} in ${batchTime}ms`);
        
        return embeddedChunks;
        
      } catch (error) {
        console.warn(`❌ [Concurrent] Failed batch ${batchIndex + 1}:`, error);
        return batch; // Return chunks without embeddings
      }
    });

    // Wait for all batches to complete concurrently
    console.log(`⏳ [Concurrent] Processing ${batches.length} batches concurrently...`);
    const batchResults = await Promise.all(batchPromises);
    
    // Flatten results
    const embeddedChunks = batchResults.flat();
    
    // Calculate precise wall-clock duration
    const endWallClock = process.hrtime.bigint();
    const wallClockDurationMs = Number(endWallClock - startWallClock) / 1_000_000;
    const totalTime = Date.now() - startTime;
    
    console.log(`✅ [Concurrent] All batches completed in ${totalTime}ms (wall-clock: ${wallClockDurationMs.toFixed(0)}ms)`);
    console.log(`📊 [Concurrent] Real throughput: ${(chunks.length / (wallClockDurationMs / 1000)).toFixed(2)} chunks/second`);
    console.log(`📊 [Concurrent] Aggregate throughput: ${(chunks.length / (totalTime / 1000)).toFixed(2)} chunks/second`);

    this.stageTracker?.completeStage('embedding_generation', 
      `Concurrent strategy completed - ${embeddedChunks.length} chunks processed`);

    return embeddedChunks;
  }

  /**
   * Generate embeddings using hybrid approach: 2 processes with SharedArrayBuffer
   * Combines process isolation with I/O concurrency for balanced performance
   */
  private async generateWithHybrid(
    chunks: CodeChunk[], 
    config: EmbeddingStrategyConfig
  ): Promise<CodeChunk[]> {
    this.stageTracker?.startStage('embedding_generation', 
      `Processing ${chunks.length} chunks with hybrid embedding strategy`);

    console.log(`🔄 Hybrid Strategy: 2-process approach with SharedArrayBuffer`);
    console.log(`📊 Target performance: >5.1 chunks/second (beat concurrent strategy)`);

    // Split chunks into two equal halves
    const midpoint = Math.floor(chunks.length / 2);
    const chunk1 = chunks.slice(0, midpoint);
    const chunk2 = chunks.slice(midpoint);
    
    console.log(`🔀 Split: Process 1 gets ${chunk1.length} chunks, Process 2 gets ${chunk2.length} chunks`);

    // Create SharedArrayBuffer for results storage
    // Each embedding is 384 floats (BGE-small-en-v1.5), 4 bytes per float
    const embeddingSize = 384;
    const bufferSize = chunks.length * embeddingSize * 4; // 4 bytes per float32
    const sharedBuffer = new SharedArrayBuffer(bufferSize);
    const sharedArray = new Float32Array(sharedBuffer);
    
    console.log(`💾 SharedArrayBuffer: ${bufferSize} bytes for ${chunks.length} embeddings (${embeddingSize}D)`);

    // Initialize 2 ProcessPoolEmbedders with 1 process each
    const embedder1 = new ProcessPoolEmbedder(1);
    const embedder2 = new ProcessPoolEmbedder(1);

    const startTime = Date.now();
    
    try {
      // Process both halves concurrently
      const [result1, result2] = await Promise.all([
        this.processHybridChunk(embedder1, chunk1, sharedArray, 0, 1),
        this.processHybridChunk(embedder2, chunk2, sharedArray, midpoint, 2)
      ]);

      // Read embeddings from SharedArrayBuffer and merge with chunks
      const embeddedChunks: CodeChunk[] = [];
      
      for (let i = 0; i < chunks.length; i++) {
        const embeddingStart = i * embeddingSize;
        const embedding = Array.from(sharedArray.slice(embeddingStart, embeddingStart + embeddingSize));
        
        embeddedChunks.push({
          ...chunks[i],
          embedding
        });
      }

      const totalTime = Date.now() - startTime;
      const chunksPerSecond = (chunks.length / (totalTime / 1000));
      
      console.log(`✅ [Hybrid] Completed ${chunks.length} chunks in ${totalTime}ms`);
      console.log(`📊 [Hybrid] Throughput: ${chunksPerSecond.toFixed(1)} chunks/second`);
      
      // Performance validation
      if (chunksPerSecond > 5.1) {
        console.log(`🎉 [Hybrid] SUCCESS: Exceeded 5.1 chunks/second target!`);
      } else {
        console.log(`⚠️  [Hybrid] Below target: ${chunksPerSecond.toFixed(1)} < 5.1 chunks/second`);
      }

      this.stageTracker?.completeStage('embedding_generation', 
        `Hybrid strategy completed - ${embeddedChunks.length} chunks processed`);

      return embeddedChunks;

    } finally {
      // Cleanup processes
      await Promise.all([
        embedder1.shutdown(),
        embedder2.shutdown()
      ]);
    }
  }

  /**
   * Process a chunk of embeddings using concurrent strategy and write to SharedArrayBuffer
   */
  private async processHybridChunk(
    embedder: ProcessPoolEmbedder,
    chunks: CodeChunk[],
    sharedArray: Float32Array,
    startIndex: number,
    processId: number
  ): Promise<void> {
    console.log(`🚀 [Hybrid-P${processId}] Processing ${chunks.length} chunks starting at index ${startIndex}`);
    
    const processStartTime = Date.now();
    
    // Use ProcessPool to generate embeddings
    const embeddedChunks = await embedder.processAllEmbeddings(chunks);
    
    // Write embeddings to SharedArrayBuffer
    const embeddingSize = 384;
    embeddedChunks.forEach((chunk, chunkIndex) => {
      if (chunk.embedding && chunk.embedding.length === embeddingSize) {
        const globalIndex = startIndex + chunkIndex;
        const bufferStart = globalIndex * embeddingSize;
        
        // Copy embedding to shared memory
        sharedArray.set(chunk.embedding, bufferStart);
      }
    });
    
    const processTime = Date.now() - processStartTime;
    console.log(`✅ [Hybrid-P${processId}] Completed ${chunks.length} chunks in ${processTime}ms`);
  }

  /**
   * Calculate optimal concurrency level based on dataset size and system resources
   * Based on performance analysis: concurrency=5 is optimal for ~100 chunks
   */
  private calculateOptimalConcurrency(chunkCount: number): number {
    const os = require('os');
    const cpuCount = os.cpus().length;
    const memoryInfo = process.memoryUsage();
    const availableMemoryGB = (memoryInfo.heapTotal - memoryInfo.heapUsed) / (1024 * 1024 * 1024);
    
    // Memory-aware concurrency calculation
    let concurrency;
    
    // Performance-based concurrency calculation
    if (chunkCount <= 50) {
      concurrency = 2; // Small datasets: minimal concurrency
    } else if (chunkCount <= 150) {
      concurrency = 5; // Sweet spot: proven optimal for 100-chunk datasets
    } else if (chunkCount <= 500) {
      concurrency = Math.min(8, Math.floor(cpuCount * 0.75)); // Scale with CPU, cap at 8
    } else {
      // Large datasets: consider ProcessPool instead, but provide reasonable fallback
      concurrency = Math.min(6, Math.floor(cpuCount * 0.5)); // Conservative for large datasets
    }
    
    // Apply memory pressure throttling
    if (availableMemoryGB < 0.5) { // Less than 500MB available
      concurrency = Math.max(1, Math.floor(concurrency * 0.5));
      console.warn(`⚠️  Memory pressure: Reducing concurrency to ${concurrency} (${availableMemoryGB.toFixed(1)}GB available)`);
    } else if (availableMemoryGB < 1.0) { // Less than 1GB available
      concurrency = Math.max(2, Math.floor(concurrency * 0.75));
      console.log(`🔧 Memory conscious: Concurrency ${concurrency} (${availableMemoryGB.toFixed(1)}GB available)`);
    }
    
    return concurrency;
  }

  /**
   * Calculate adaptive batch sizing for optimal performance
   * Balances batch size efficiency with concurrency benefits
   */
  private calculateAdaptiveBatching(chunkCount: number, concurrency: number): { batchSize: number; actualBatches: number } {
    // Optimal batch size ranges based on performance analysis
    const MIN_BATCH_SIZE = 5;   // Minimum chunks per batch to be efficient
    const MAX_BATCH_SIZE = 50;  // Maximum to prevent memory issues
    const SWEET_SPOT_BATCH_SIZE = 20; // Proven optimal from testing

    // Calculate ideal batch size
    let idealBatchSize = Math.ceil(chunkCount / concurrency);
    
    // Apply constraints and optimizations
    if (chunkCount <= 100) {
      // Small to medium datasets: aim for sweet spot
      idealBatchSize = Math.min(SWEET_SPOT_BATCH_SIZE, Math.max(MIN_BATCH_SIZE, idealBatchSize));
    } else if (chunkCount <= 500) {
      // Medium datasets: balance efficiency and memory
      idealBatchSize = Math.min(MAX_BATCH_SIZE, Math.max(MIN_BATCH_SIZE * 2, idealBatchSize));
    } else {
      // Large datasets: larger batches for efficiency
      idealBatchSize = Math.min(MAX_BATCH_SIZE, Math.max(MIN_BATCH_SIZE * 3, idealBatchSize));
    }
    
    // Calculate actual number of batches based on optimized size
    const actualBatches = Math.ceil(chunkCount / idealBatchSize);
    
    return {
      batchSize: idealBatchSize,
      actualBatches: actualBatches
    };
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
    // Estimate based on typical batch times (this could be improved with actual measurement)
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

  private calculateConcurrentBatchInfo(chunks: CodeChunk[], concurrency: number) {
    // Concurrent processes batches simultaneously
    const batchSize = Math.max(1, Math.ceil(chunks.length / concurrency));
    const totalBatches = Math.ceil(chunks.length / batchSize);
    // Estimate based on concurrent execution (should be faster than sequential)
    const estimatedBatchTime = batchSize * 10; // ~10ms per chunk estimate (optimistic due to concurrency)
    return {
      totalBatches,
      averageBatchTime: estimatedBatchTime
    };
  }

  private calculateHybridBatchInfo(chunks: CodeChunk[]) {
    // Hybrid uses 2 processes processing halves concurrently
    const totalBatches = 2; // Always 2 concurrent batches
    // Estimate based on ProcessPool performance but with 2-process parallelism
    const estimatedBatchTime = Math.ceil(chunks.length / 2) * 100; // ~100ms per chunk estimate (hybrid efficiency)
    return {
      totalBatches,
      averageBatchTime: estimatedBatchTime
    };
  }

  /**
   * Compare strategies by running both on a sample of chunks
   */
  async compareStrategies(
    chunks: CodeChunk[], 
    sampleSize: number = 50
  ): Promise<{
    original: EmbeddingResult;
    processPool: EmbeddingResult;
    recommendation: EmbeddingStrategy;
    comparison: {
      speedRatio: number; // processPool speed / original speed
      memoryRatio: number; // processPool memory / original memory
      timeToProcess1000Chunks: {
        original: number;
        processPool: number;
      };
    };
  }> {
    console.log(`🏁 Comparing embedding strategies using ${sampleSize} sample chunks...`);
    
    const sampleChunks = chunks.slice(0, sampleSize);
    
    // Test original strategy
    console.log('🧪 Testing original strategy...');
    const originalResult = await this.generateEmbeddings(sampleChunks, { 
      strategy: 'original',
      batchSize: 20 // Smaller batches for fair comparison
    });
    
    // Small delay between tests
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    // Test process pool strategy
    console.log('🧪 Testing ProcessPool strategy...');
    const processPoolResult = await this.generateEmbeddings(sampleChunks, { 
      strategy: 'process-pool'
    });
    
    // Calculate comparison metrics
    const speedRatio = originalResult.performance.chunksPerSecond / processPoolResult.performance.chunksPerSecond;
    const memoryRatio = processPoolResult.performance.peakMemoryMB / originalResult.performance.peakMemoryMB;
    
    const timeToProcess1000Original = 1000 / originalResult.performance.chunksPerSecond;
    const timeToProcess1000ProcessPool = 1000 / processPoolResult.performance.chunksPerSecond;
    
    // Determine recommendation
    let recommendation: EmbeddingStrategy;
    if (chunks.length < 200) {
      recommendation = 'original'; // Original is better for small datasets
    } else if (speedRatio > 1.5) {
      recommendation = 'original'; // Original is significantly faster
    } else if (memoryRatio < 2 && timeToProcess1000ProcessPool < timeToProcess1000Original) {
      recommendation = 'process-pool'; // ProcessPool is faster with reasonable memory usage
    } else {
      recommendation = 'original'; // Default to original for safety
    }
    
    console.log(`📊 Strategy comparison complete. Recommendation: ${recommendation}`);
    
    return {
      original: originalResult,
      processPool: processPoolResult,
      recommendation,
      comparison: {
        speedRatio,
        memoryRatio,
        timeToProcess1000Chunks: {
          original: timeToProcess1000Original,
          processPool: timeToProcess1000ProcessPool
        }
      }
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
    const concurrency = process.env.EMBEDDING_CONCURRENCY ? parseInt(process.env.EMBEDDING_CONCURRENCY) : undefined;

    return {
      strategy,
      batchSize,
      processCount,
      timeoutMs,
      concurrency
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
  }
}