import { FlagEmbedding, EmbeddingModel } from 'fastembed';
import { Mutex } from 'async-mutex';
import fastq from 'fastq';
import * as os from 'os';
import { CodeChunk } from './types';

interface EmbeddingTask {
  texts: string[];
  batchIndex: number;
  totalBatches: number;
}

interface EmbeddingResult {
  embeddings: number[][];
  batchIndex: number;
  stats: {
    duration: number;
    memoryDelta: number;
    chunksProcessed: number;
  };
}

interface SessionSlot {
  embedder: any;
  mutex: Mutex;
  id: number;
}

/**
 * Session Pool Embedder using Global Sessions with Mutex Protection
 * Avoids ONNX Runtime thread safety issues by using a pool of sessions
 * with mutex-protected access instead of worker threads
 */
export class SessionPoolEmbedder {
  private sessions: SessionSlot[] = [];
  private queue: fastq.queueAsPromised<EmbeddingTask, EmbeddingResult>;
  private numSessions: number;
  private initialized = false;

  constructor() {
    const totalCores = os.cpus().length;
    // Use fewer sessions than cores to avoid ONNX Runtime issues
    this.numSessions = Math.min(4, totalCores <= 2 ? 1 : Math.max(1, totalCores - 2));
    
    console.log(`🚀 Session Pool Embedder: ${this.numSessions} sessions (${totalCores} CPU cores)`);
    console.log(`✅ Option 2: Global sessions with mutex protection (ONNX-safe)`);
    
    // FastQ queue with concurrency = number of sessions
    this.queue = fastq.promise(this.processEmbeddingTask.bind(this), this.numSessions);
  }

  async initialize(): Promise<void> {
    if (this.initialized) return;
    
    console.log('🔄 Initializing session pool...');
    const startTime = Date.now();
    
    // Create all sessions sequentially to avoid conflicts
    for (let i = 0; i < this.numSessions; i++) {
      console.log(`📡 Initializing session ${i + 1}/${this.numSessions}...`);
      
      const embedder = await FlagEmbedding.init({
        model: EmbeddingModel.BGESmallENV15,
        maxLength: 512,
        cacheDir: './.fastembed_cache'
      });
      
      const sessionSlot: SessionSlot = {
        embedder,
        mutex: new Mutex(),
        id: i
      };
      
      this.sessions.push(sessionSlot);
      console.log(`✅ Session ${i + 1} ready`);
    }
    
    const initDuration = Date.now() - startTime;
    console.log(`✅ All ${this.numSessions} sessions initialized in ${initDuration}ms`);
    this.initialized = true;
  }

  /**
   * FastQ task processor - automatically gets a session from the pool
   */
  private async processEmbeddingTask(task: EmbeddingTask): Promise<EmbeddingResult> {
    // Round-robin session selection for load balancing
    const sessionIndex = task.batchIndex % this.numSessions;
    const session = this.sessions[sessionIndex];
    
    const startTime = Date.now();
    const beforeMemory = process.memoryUsage();
    
    try {
      // Mutex-protected inference to avoid ONNX Runtime conflicts
      const embeddings = await session.mutex.runExclusive(async () => {
        const results: number[][] = [];
        const embeddingStream = session.embedder.embed(task.texts);
        
        for await (const batch of embeddingStream) {
          results.push(...batch.map((emb: Float32Array) => Array.from(emb)));
        }
        
        return results;
      });
      
      const afterMemory = process.memoryUsage();
      const memoryDelta = Math.round((afterMemory.heapUsed - beforeMemory.heapUsed) / 1024 / 1024);
      const duration = Date.now() - startTime;
      
      console.log(`✅ Batch ${task.batchIndex + 1}/${task.totalBatches} completed by session ${session.id} in ${duration}ms`);
      
      return {
        embeddings,
        batchIndex: task.batchIndex,
        stats: {
          duration,
          memoryDelta,
          chunksProcessed: task.texts.length
        }
      };
      
    } catch (error) {
      console.error(`❌ Session ${session.id} error processing batch ${task.batchIndex}:`, error);
      throw error;
    }
  }

  /**
   * Main method: Process all chunks using session pool
   */
  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Initialize session pool
    await this.initialize();
    
    // Adaptive batch sizing based on chunk count and sessions
    const optimalBatchSize = Math.max(100, Math.min(500, Math.ceil(chunks.length / (this.numSessions * 4))));
    const batches: CodeChunk[][] = [];
    
    // Split into batches
    for (let i = 0; i < chunks.length; i += optimalBatchSize) {
      batches.push(chunks.slice(i, i + optimalBatchSize));
    }
    
    console.log(`📊 Processing ${chunks.length} chunks in ${batches.length} batches (${optimalBatchSize} per batch)`);
    
    // Add all tasks to FastQ queue
    const taskPromises: Promise<EmbeddingResult>[] = [];
    
    for (let i = 0; i < batches.length; i++) {
      const batch = batches[i];
      const texts = batch.map(chunk => this.createOptimizedEmbeddingText(chunk));
      
      const taskPromise = this.queue.push({
        texts,
        batchIndex: i,
        totalBatches: batches.length
      });
      
      taskPromises.push(taskPromise);
    }
    
    console.log(`⏳ Processing ${taskPromises.length} tasks with session pool...`);
    
    // Execute all tasks concurrently (up to numSessions in parallel)
    const results = await Promise.all(taskPromises);
    
    // Sort results by batch index and reconstruct chunks
    results.sort((a, b) => a.batchIndex - b.batchIndex);
    
    const embeddedChunks: CodeChunk[] = [];
    for (let i = 0; i < results.length; i++) {
      const result = results[i];
      const batch = batches[result.batchIndex];
      
      for (let j = 0; j < batch.length; j++) {
        embeddedChunks.push({
          ...batch[j],
          embedding: result.embeddings[j] || []
        });
      }
    }
    
    this.printStats();
    return embeddedChunks;
  }

  private createOptimizedEmbeddingText(chunk: CodeChunk): string {
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

  private printStats(): void {
    console.log('\n📊 Session Pool Embedding Statistics:');
    console.log('━'.repeat(50));
    console.log(`Queue: ${this.queue.length()} pending, ${this.queue.running()} running`);
    console.log(`Sessions: ${this.numSessions} total, ${this.sessions.length} initialized`);
    console.log(`Memory: ${Math.round(process.memoryUsage().heapUsed / 1024 / 1024)}MB heap`);
    console.log('━'.repeat(50));
  }

  async shutdown(): Promise<void> {
    console.log('🛑 Shutting down session pool embedder...');
    
    // Wait for queue to drain
    await this.queue.drained();
    
    console.log('✅ Session pool embedder shut down');
  }
}