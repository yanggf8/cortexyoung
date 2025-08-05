import fastq from 'fastq';
import { Worker } from 'worker_threads';
import * as path from 'path';
import * as os from 'os';
import { CodeChunk } from './types';

interface EmbeddingTask {
  chunk: CodeChunk;
  originalIndex: number;
  timestamp: number;
}

interface EmbeddingResult {
  embedding: number[];
  originalIndex: number;
  timestamp: number;
  stats: {
    duration: number;
    memoryDelta: number;
    workerId: number;
  };
}

interface WorkerInstance {
  id: number;
  worker: Worker;
  isReady: boolean;
  isAvailable: boolean;
  lastUsed: number;
}

export class WorkerPoolEmbedder {
  private workers: WorkerInstance[] = [];
  private queue: fastq.queueAsPromised<EmbeddingTask, EmbeddingResult>;
  private workerCount: number;
  private isInitialized = false;

  constructor() {
    // Calculate optimal worker count - be much more conservative
    const totalCores = os.cpus().length;
    // ONNX Runtime has issues with high concurrency - limit to 2-3 workers max
    this.workerCount = Math.min(3, Math.max(1, Math.floor(totalCores / 4)));
    
    console.log(`🏊 Worker Pool: ${this.workerCount} workers (${totalCores} CPU cores, conservative for ONNX safety)`);
    console.log(`✅ Strategy: Limited workers to avoid ONNX Runtime concurrency issues`);
    
    // Create fastq queue - consumer count matches worker count
    this.queue = fastq.promise(this.processEmbeddingTask.bind(this), this.workerCount);
  }

  async initialize(): Promise<void> {
    if (this.isInitialized) return;

    console.log(`🔧 Initializing worker pool with ${this.workerCount} workers...`);
    
    // Create workers
    await this.createWorkers();
    
    // Wait for all workers to be ready
    await this.waitForWorkersReady();
    
    this.isInitialized = true;
    console.log(`🎉 Worker pool initialized with ${this.workers.length} ready workers`);
  }

  private async createWorkers(): Promise<void> {
    const workerScript = path.join(__dirname, 'isolated-embedding-worker.js');

    for (let i = 0; i < this.workerCount; i++) {
      console.log(`⏳ Creating worker ${i + 1}/${this.workerCount}...`);
      
      const worker = new Worker(workerScript);
      const workerInstance: WorkerInstance = {
        id: i,
        worker,
        isReady: false,
        isAvailable: true,
        lastUsed: 0
      };

      // Handle worker messages
      worker.on('message', (message) => {
        if (message.type === 'init_complete') {
          if (message.success) {
            workerInstance.isReady = true;
            console.log(`✅ Worker ${i} ready with isolated FastEmbedding`);
          } else {
            console.error(`❌ Worker ${i} initialization failed:`, message.error);
          }
        }
      });

      worker.on('error', (error) => {
        console.error(`❌ Worker ${i} error:`, error);
        workerInstance.isReady = false;
        workerInstance.isAvailable = false;
      });

      worker.on('exit', (code) => {
        if (code !== 0) {
          console.error(`❌ Worker ${i} exited with code ${code}`);
        }
      });

      this.workers.push(workerInstance);

      // Initialize worker with longer staggered timing to avoid ONNX conflicts
      setTimeout(() => {
        worker.postMessage({
          type: 'init',
          data: { workerId: i }
        });
      }, i * 2000); // 2 second delay between each worker (was 1 second)
    }
  }

  private async waitForWorkersReady(): Promise<void> {
    const maxWaitTime = 60000; // 60 seconds max wait
    const startTime = Date.now();
    
    while (Date.now() - startTime < maxWaitTime) {
      const readyWorkers = this.workers.filter(w => w.isReady).length;
      
      if (readyWorkers === this.workerCount) {
        console.log(`✅ All ${this.workerCount} workers ready`);
        return;
      }
      
      console.log(`⏳ Waiting for workers: ${readyWorkers}/${this.workerCount} ready...`);
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
    
    throw new Error(`Timeout: Only ${this.workers.filter(w => w.isReady).length}/${this.workerCount} workers ready`);
  }

  // FastQ consumer function - processes one chunk per task
  private async processEmbeddingTask(task: EmbeddingTask): Promise<EmbeddingResult> {
    // Find available worker (round-robin style)
    const availableWorker = this.workers
      .filter(w => w.isReady && w.isAvailable)
      .sort((a, b) => a.lastUsed - b.lastUsed)[0];

    if (!availableWorker) {
      throw new Error('No available workers in pool');
    }

    // Mark worker as busy
    availableWorker.isAvailable = false;
    availableWorker.lastUsed = Date.now();

    try {
      // Create optimized embedding text (consistent across all embedding generation)
      const embeddingText = this.createOptimizedEmbeddingText(task.chunk);
      
      console.log(`🔄 Worker ${availableWorker.id} processing chunk ${task.originalIndex} (${embeddingText.length} chars)`);
      
      // Send task to worker
      const result = await new Promise<EmbeddingResult>((resolve, reject) => {
        const timeout = setTimeout(() => {
          reject(new Error(`Worker ${availableWorker.id} timeout`));
        }, 30000); // 30 second timeout per chunk

        availableWorker.worker.once('message', (message) => {
          clearTimeout(timeout);
          
          if (message.type === 'embed_complete') {
            if (message.success) {
              resolve({
                embedding: message.embeddings[0], // Single chunk = single embedding
                originalIndex: task.originalIndex,
                timestamp: task.timestamp,
                stats: {
                  duration: message.stats.duration,
                  memoryDelta: message.stats.memoryDelta,
                  workerId: availableWorker.id
                }
              });
            } else {
              reject(new Error(`Worker ${availableWorker.id} error: ${message.error}`));
            }
          }
        });

        availableWorker.worker.postMessage({
          type: 'embed_batch',
          batchId: `chunk-${task.originalIndex}`,
          data: { texts: [embeddingText] } // Single chunk as array
        });
      });

      console.log(`✅ Worker ${availableWorker.id} completed chunk ${task.originalIndex} in ${result.stats.duration}ms`);
      return result;

    } finally {
      // Mark worker as available again
      availableWorker.isAvailable = true;
    }
  }

  // Main method: Process all chunks using worker pool
  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Initialize worker pool first
    await this.initialize();
    
    console.log(`📊 Processing ${chunks.length} chunks using ${this.workerCount} workers`);
    
    const currentTimestamp = Date.now();
    
    // Create tasks for each chunk with original index preservation
    const tasks: Promise<EmbeddingResult>[] = [];
    
    for (let i = 0; i < chunks.length; i++) {
      const taskPromise = this.queue.push({
        chunk: chunks[i],
        originalIndex: i,
        timestamp: currentTimestamp
      });
      
      tasks.push(taskPromise);
    }

    console.log(`⏳ Processing ${tasks.length} tasks with worker pool...`);
    
    // Wait for all tasks to complete
    const results = await Promise.all(tasks);
    
    // Sort results by original index to maintain order
    results.sort((a, b) => a.originalIndex - b.originalIndex);
    
    // Merge embeddings back with original chunks
    const embeddedChunks: CodeChunk[] = results.map(result => ({
      ...chunks[result.originalIndex],
      embedding: result.embedding,
      // Add timestamp for versioning
      indexed_at: result.timestamp
    }));

    this.printStats(results);
    return embeddedChunks;
  }

  // Optimized embedding text generation - consistent across all embedding generation
  private createOptimizedEmbeddingText(chunk: CodeChunk): string {
    const parts = [];
    
    // Add symbol name if available (most important for semantic search)
    if (chunk.symbol_name) {
      parts.push(chunk.symbol_name);
    }
    
    // Add chunk type for context
    parts.push(chunk.chunk_type);
    
    // Add main content
    parts.push(chunk.content);
    
    // Add limited import context (top 3 most relevant)
    if (chunk.relationships.imports.length > 0) {
      parts.push(chunk.relationships.imports.slice(0, 3).join(' '));
    }
    
    return parts.join(' ');
  }

  private printStats(results: EmbeddingResult[]): void {
    console.log('\n📊 Worker Pool Statistics:');
    console.log('━'.repeat(50));
    
    // Worker usage stats
    const workerUsage = new Map<number, number>();
    results.forEach(result => {
      const count = workerUsage.get(result.stats.workerId) || 0;
      workerUsage.set(result.stats.workerId, count + 1);
    });
    
    console.log('Worker Usage:');
    workerUsage.forEach((count, workerId) => {
      console.log(`  Worker ${workerId}: ${count} chunks`);
    });
    
    // Performance stats
    const totalDuration = results.reduce((sum, r) => sum + r.stats.duration, 0);
    const avgDuration = Math.round(totalDuration / results.length);
    const maxDuration = Math.max(...results.map(r => r.stats.duration));
    const minDuration = Math.min(...results.map(r => r.stats.duration));
    
    console.log(`Queue: ${this.queue.length()} pending, ${this.queue.running()} running`);
    console.log(`Performance: ${avgDuration}ms avg (${minDuration}-${maxDuration}ms range)`);
    console.log(`Workers: ${this.workers.length} total, ${this.workers.filter(w => w.isReady).length} ready`);
    console.log(`Total chunks: ${results.length}, Timestamp: ${results[0]?.timestamp}`);
    console.log('━'.repeat(50));
  }

  async shutdown(): Promise<void> {
    console.log('🛑 Shutting down worker pool...');
    
    // Wait for queue to drain
    await this.queue.drained();
    
    // Terminate all workers
    await Promise.all(this.workers.map(async (workerInstance) => {
      try {
        await workerInstance.worker.terminate();
        console.log(`✅ Worker ${workerInstance.id} terminated`);
      } catch (error) {
        console.error(`❌ Error terminating worker ${workerInstance.id}:`, error);
      }
    }));
    
    console.log('✅ Worker pool shut down');
  }

  // Health check method
  getPoolStatus() {
    return {
      workerCount: this.workerCount,
      initialized: this.isInitialized,
      readyWorkers: this.workers.filter(w => w.isReady).length,
      availableWorkers: this.workers.filter(w => w.isReady && w.isAvailable).length,
      queueLength: this.queue.length(),
      queueRunning: this.queue.running()
    };
  }
}
