import fastq from 'fastq';
import { Worker } from 'worker_threads';
import * as path from 'path';
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

export class FastQEmbedder {
  private workers: Worker[] = [];
  private workerAvailable: boolean[] = [];
  private numWorkers: number;
  private queue: fastq.queueAsPromised<EmbeddingTask, EmbeddingResult>;
  private results: EmbeddingResult[] = [];

  constructor() {
    // Option 1: One InferenceSession per worker/thread (best for true parallelism)
    const totalCores = os.cpus().length;
    // Each worker gets its own BGE model instance = separate InferenceSession
    this.numWorkers = Math.min(3, totalCores <= 2 ? 1 : Math.max(1, totalCores - 2));
    
    console.log(`🚀 FastQ Embedder: ${this.numWorkers} workers (${totalCores} CPU cores)`);
    console.log(`✅ Option 1: One InferenceSession per worker (true parallelism)`);
    
    // Create fastq queue - each worker will have its own ONNX session
    this.queue = fastq.promise(this.processEmbeddingTask.bind(this), this.numWorkers);
  }

  async initialize(): Promise<void> {
    await this.initializeWorkers();
    
    // Wait for all workers to be ready
    while (this.workerAvailable.some(available => !available)) {
      console.log('⏳ Waiting for workers to initialize...');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    
    console.log('✅ All workers ready');
  }

  private async initializeWorkers(): Promise<void> {
    const workerScript = path.join(__dirname, 'embedding-worker.js');

    for (let i = 0; i < this.numWorkers; i++) {
      const worker = new Worker(workerScript);
      this.workers[i] = worker;
      this.workerAvailable[i] = false;

      worker.on('message', (message) => {
        if (message.type === 'init_complete') {
          this.workerAvailable[i] = true;
          console.log(`✅ Worker ${i} ready`);
        }
      });

      worker.on('error', (error) => {
        console.error(`❌ Worker ${i} error:`, error);
        this.workerAvailable[i] = false;
      });

      // Staggered initialization to avoid ONNX Runtime conflicts
      setTimeout(() => {
        worker.postMessage({
          type: 'init',
          data: { workerId: i },
          batchId: `init-${i}`
        });
      }, i * 1000); // 1 second delay between each worker
    }
  }

  // This is the worker function that fastq calls
  private async processEmbeddingTask(task: EmbeddingTask): Promise<EmbeddingResult> {
    // Find available worker
    const availableWorkerIndex = this.workerAvailable.findIndex(available => available);
    if (availableWorkerIndex === -1) {
      throw new Error('No available workers');
    }

    const worker = this.workers[availableWorkerIndex];
    this.workerAvailable[availableWorkerIndex] = false;

    try {
      const result = await new Promise<EmbeddingResult>((resolve, reject) => {
        const timeout = setTimeout(() => {
          reject(new Error('Worker timeout'));
        }, 60000);

        worker.once('message', (message) => {
          clearTimeout(timeout);
          
          if (message.success) {
            resolve({
              embeddings: message.embeddings,
              batchIndex: task.batchIndex,
              stats: message.stats
            });
          } else {
            reject(new Error(message.error));
          }
        });

        worker.postMessage({
          type: 'embed_batch',
          batchId: `batch-${task.batchIndex}`,
          data: { texts: task.texts }
        });
      });

      console.log(`✅ Batch ${task.batchIndex + 1}/${task.totalBatches} completed in ${result.stats.duration}ms`);
      return result;

    } finally {
      this.workerAvailable[availableWorkerIndex] = true;
    }
  }

  // Main method: Process all chunks using fastq
  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Initialize workers first
    await this.initialize();
    
    const batchSize = 400;
    const batches: CodeChunk[][] = [];
    
    // Split into batches
    for (let i = 0; i < chunks.length; i += batchSize) {
      batches.push(chunks.slice(i, i + batchSize));
    }

    console.log(`📊 Queuing ${chunks.length} chunks in ${batches.length} batches`);

    // Add all tasks to fastq queue - this is beautifully simple!
    const tasks: Promise<EmbeddingResult>[] = [];
    
    for (let i = 0; i < batches.length; i++) {
      const batch = batches[i];
      const texts = batch.map(chunk => this.createOptimizedEmbeddingText(chunk));
      
      // fastq handles all the complexity for us
      const taskPromise = this.queue.push({
        texts,
        batchIndex: i,
        totalBatches: batches.length
      });
      
      tasks.push(taskPromise);
    }

    console.log(`⏳ Processing ${tasks.length} tasks with fastq...`);
    
    // Wait for all tasks to complete
    const results = await Promise.all(tasks);
    
    // Sort results by batch index and combine with chunks
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
    console.log('\n📊 FastQ Embedding Statistics:');
    console.log('━'.repeat(50));
    console.log(`Queue: ${this.queue.length()} pending, ${this.queue.running()} running`);
    console.log(`Workers: ${this.numWorkers} total, ${this.workerAvailable.filter(a => a).length} available`);
    console.log('━'.repeat(50));
  }

  async shutdown(): Promise<void> {
    console.log('🛑 Shutting down FastQ embedder...');
    
    // Wait for queue to drain
    await this.queue.drained();
    
    // Terminate workers
    await Promise.all(this.workers.map(worker => worker.terminate()));
    
    console.log('✅ FastQ embedder shut down');
  }
}