import fastq from 'fastq';
import { spawn, ChildProcess } from 'child_process';
import * as path from 'path';
import * as os from 'os';
import { CodeChunk } from './types';

interface EmbeddingTask {
  chunks: CodeChunk[];
  originalIndices: number[]; // Track original indices for each chunk
  batchIndex: number;
  timestamp: number;
}

interface EmbeddingResult {
  embedding: number[];
  originalIndex: number;
  timestamp: number;
  stats: {
    duration: number;
    memoryDelta: number;
    processId: number;
  };
}

interface ProcessInstance {
  id: number;
  process: ChildProcess;
  isReady: boolean;
  isAvailable: boolean;
  lastUsed: number;
  pendingRequests: Map<string, { resolve: Function; reject: Function; timeout: NodeJS.Timeout }>;
  messageBuffer: string; // Buffer for incomplete JSON messages
}

export class ProcessPoolEmbedder {
  private processes: ProcessInstance[] = [];
  private queue: fastq.queueAsPromised<EmbeddingTask, EmbeddingResult[]>;
  private processCount: number;
  private isInitialized = false;

  constructor(processCount?: number) {
    // Calculate optimal process count - reserve cores for system, or use provided count
    const totalCores = os.cpus().length;
    
    if (processCount) {
      // Use provided count
      this.processCount = processCount;
    } else {
      // Auto-calculate: reserve 2 cores for system work
      if (totalCores <= 2) {
        this.processCount = 1;
        console.warn(`⚠️  WARNING: Only ${totalCores} CPU core(s) detected. Using 1 worker process.`);
        console.warn(`⚠️  System performance may be impacted. Recommend 4+ cores for optimal performance.`);
      } else {
        this.processCount = totalCores - 2; // Reserve 2 cores for system
      }
    }
    
    console.log(`🏭 Process Pool: ${this.processCount} external Node.js processes (${totalCores} CPU cores, ${totalCores <= 2 ? 'using all cores' : 'reserved 2 for system'})`);
    console.log(`✅ Strategy: Complete process isolation - no ONNX Runtime thread safety issues`);
    
    // Create fastq queue - consumer count matches process count
    this.queue = fastq.promise(this.processEmbeddingTask.bind(this), this.processCount);
  }

  async initialize(): Promise<void> {
    if (this.isInitialized) return;

    console.log(`🔧 Spawning ${this.processCount} external Node.js processes...`);
    
    // Create processes
    await this.createProcesses();
    
    // Wait for all processes to be ready
    await this.waitForProcessesReady();
    
    this.isInitialized = true;
    console.log(`🎉 Process pool initialized with ${this.processes.length} ready processes`);
  }

  private async createProcesses(): Promise<void> {
    const processScript = path.join(__dirname, 'external-embedding-process.js');

    for (let i = 0; i < this.processCount; i++) {
      console.log(`⏳ Spawning external process ${i + 1}/${this.processCount}...`);
      
      // Spawn external Node.js process
      const childProcess = spawn('node', [processScript], {
        stdio: ['pipe', 'pipe', 'pipe'], // stdin, stdout, stderr
        env: { ...process.env, NODE_ENV: 'production' }
      });

      const processInstance: ProcessInstance = {
        id: i,
        process: childProcess,
        isReady: false,
        isAvailable: true,
        lastUsed: 0,
        pendingRequests: new Map(),
        messageBuffer: '' // Initialize message buffer
      };

      // Handle process stdout (responses) with proper JSON parsing
      childProcess.stdout?.on('data', (data) => {
        // Append to buffer
        processInstance.messageBuffer += data.toString();
        
        // Try to parse complete JSON messages
        let lines = processInstance.messageBuffer.split('\n');
        
        // Keep the last incomplete line in buffer
        processInstance.messageBuffer = lines.pop() || '';
        
        // Process complete lines
        for (const line of lines) {
          if (line.trim()) {
            try {
              const message = JSON.parse(line);
              this.handleProcessMessage(processInstance, message);
            } catch (error) {
              const errorMessage = error instanceof Error ? error.message : String(error);
              console.error(`❌ Failed to parse JSON from process ${i}:`, errorMessage);
              console.error(`❌ Problematic line: ${line.substring(0, 200)}...`);
            }
          }
        }
      });

      // Handle process stderr (logs)
      childProcess.stderr?.on('data', (data) => {
        // Forward stderr to our stderr for debugging
        process.stderr.write(data);
      });

      // Handle process exit
      childProcess.on('exit', (code, signal) => {
        console.error(`❌ Process ${i} exited with code ${code}, signal ${signal}`);
        processInstance.isReady = false;
        processInstance.isAvailable = false;
        
        // Reject any pending requests
        processInstance.pendingRequests.forEach(({ reject, timeout }) => {
          clearTimeout(timeout);
          reject(new Error(`Process ${i} exited unexpectedly`));
        });
        processInstance.pendingRequests.clear();
      });

      // Handle process errors
      childProcess.on('error', (error) => {
        console.error(`❌ Process ${i} error:`, error);
        processInstance.isReady = false;
        processInstance.isAvailable = false;
      });

      this.processes.push(processInstance);

      // Initialize process with staggered timing
      setTimeout(() => {
        this.sendToProcess(processInstance, {
          type: 'init',
          data: { processId: i }
        });
      }, i * 1000); // 1 second delay between each process
    }
  }

  private handleProcessMessage(processInstance: ProcessInstance, message: any): void {
    const { type, batchId } = message;
    
    if (type === 'init_complete') {
      if (message.success) {
        processInstance.isReady = true;
        console.log(`✅ Process ${processInstance.id} ready with isolated FastEmbedding`);
      } else {
        console.error(`❌ Process ${processInstance.id} initialization failed:`, message.error);
      }
    } else if (type === 'embed_complete' && batchId) {
      const pending = processInstance.pendingRequests.get(batchId);
      if (pending) {
        clearTimeout(pending.timeout);
        processInstance.pendingRequests.delete(batchId);
        
        if (message.success) {
          pending.resolve(message);
        } else {
          pending.reject(new Error(message.error));
        }
      }
    } else if (type === 'error') {
      console.error(`❌ Process ${processInstance.id} error:`, message.error);
    }
  }

  private sendToProcess(processInstance: ProcessInstance, message: any): void {
    if (processInstance.process.stdin) {
      processInstance.process.stdin.write(JSON.stringify(message) + '\n');
    }
  }

  private async waitForProcessesReady(): Promise<void> {
    const maxWaitTime = 120000; // 2 minutes max wait (processes are slower to start)
    const startTime = Date.now();
    
    while (Date.now() - startTime < maxWaitTime) {
      const readyProcesses = this.processes.filter(p => p.isReady).length;
      
      if (readyProcesses === this.processCount) {
        console.log(`✅ All ${this.processCount} processes ready`);
        return;
      }
      
      console.log(`⏳ Waiting for processes: ${readyProcesses}/${this.processCount} ready...`);
      await new Promise(resolve => setTimeout(resolve, 2000));
    }
    
    throw new Error(`Timeout: Only ${this.processes.filter(p => p.isReady).length}/${this.processCount} processes ready`);
  }

  // FastQ consumer function - processes a batch of chunks per task
  private async processEmbeddingTask(task: EmbeddingTask): Promise<EmbeddingResult[]> {
    // Find available process (round-robin style)
    const availableProcess = this.processes
      .filter(p => p.isReady && p.isAvailable)
      .sort((a, b) => a.lastUsed - b.lastUsed)[0];

    if (!availableProcess) {
      throw new Error('No available processes in pool');
    }

    // Mark process as busy
    availableProcess.isAvailable = false;
    availableProcess.lastUsed = Date.now();

    try {
      // Create optimized embedding texts for the entire batch
      const embeddingTexts = task.chunks.map(chunk => this.createOptimizedEmbeddingText(chunk));
      
      console.log(`🔄 Process ${availableProcess.id} processing batch ${task.batchIndex} (${task.chunks.length} chunks)`);
      
      // Send entire batch to external process
      const batchId = `batch-${task.batchIndex}-${Date.now()}`;
      const result = await new Promise<any>((resolve, reject) => {
        const timeout = setTimeout(() => {
          availableProcess.pendingRequests.delete(batchId);
          reject(new Error(`Process ${availableProcess.id} timeout`));
        }, 120000); // 2 minute timeout for batch processing

        availableProcess.pendingRequests.set(batchId, { resolve, reject, timeout });

        this.sendToProcess(availableProcess, {
          type: 'embed_batch',
          batchId,
          data: { texts: embeddingTexts }
        });
      });

      console.log(`✅ Process ${availableProcess.id} completed batch ${task.batchIndex} (${task.chunks.length} chunks) in ${result.stats.duration}ms`);
      
      // Map results back to individual chunks with original indices
      const batchResults: EmbeddingResult[] = [];
      
      for (let i = 0; i < task.chunks.length; i++) {
        batchResults.push({
          embedding: result.embeddings[i],
          originalIndex: task.originalIndices[i], // Use tracked original index
          timestamp: task.timestamp,
          stats: {
            duration: result.stats.duration,
            memoryDelta: result.stats.memoryDelta,
            processId: availableProcess.id
          }
        });
      }

      return batchResults;

    } finally {
      // Mark process as available again
      availableProcess.isAvailable = true;
    }
  }

  // Main method: Process all chunks using process pool with batching
  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Initialize process pool first
    await this.initialize();
    
    console.log(`📊 Processing ${chunks.length} chunks using ${this.processCount} external processes`);
    
    // Precise timing measurement
    const startWallClockTime = process.hrtime.bigint();
    const currentTimestamp = Date.now();
    
    // Calculate batch size: distribute chunks evenly across processes
    // But limit batch size to prevent overly large JSON messages
    const idealBatchSize = Math.ceil(chunks.length / this.processCount);
    const maxBatchSize = 50; // Limit to prevent JSON parsing issues
    const batchSize = Math.min(idealBatchSize, maxBatchSize);
    
    console.log(`📦 Batch size: ${batchSize} chunks per batch (ideal: ${idealBatchSize}, max: ${maxBatchSize})`);
    
    // Create batches for each process with original index tracking
    const batches: { chunks: CodeChunk[]; originalIndices: number[] }[] = [];
    for (let i = 0; i < chunks.length; i += batchSize) {
      const batchChunks = chunks.slice(i, i + batchSize);
      const batchIndices = Array.from({ length: batchChunks.length }, (_, idx) => i + idx);
      batches.push({
        chunks: batchChunks,
        originalIndices: batchIndices
      });
    }
    
    console.log(`🔄 Created ${batches.length} batches for ${this.processCount} processes`);
    
    // Create tasks for each batch
    const tasks: Promise<EmbeddingResult[]>[] = [];
    
    for (let i = 0; i < batches.length; i++) {
      const batch = batches[i];
      const taskPromise = this.queue.push({
        chunks: batch.chunks,
        originalIndices: batch.originalIndices,
        batchIndex: i,
        timestamp: currentTimestamp
      });
      
      tasks.push(taskPromise);
    }

    console.log(`⏳ Processing ${tasks.length} batches with process pool...`);
    
    // Wait for all tasks to complete
    const batchResults = await Promise.all(tasks);
    
    // Flatten and sort results by original index to maintain order
    const allResults: EmbeddingResult[] = [];
    batchResults.forEach(batchResult => {
      allResults.push(...batchResult);
    });
    
    allResults.sort((a, b) => a.originalIndex - b.originalIndex);
    
    // Calculate precise wall-clock duration
    const endWallClockTime = process.hrtime.bigint();
    const wallClockDurationMs = Number(endWallClockTime - startWallClockTime) / 1_000_000;
    
    // Merge embeddings back with original chunks
    const embeddedChunks: CodeChunk[] = allResults.map(result => ({
      ...chunks[result.originalIndex],
      embedding: result.embedding,
      indexed_at: result.timestamp
    }));

    this.printPreciseBenchmarkStats(batchResults, wallClockDurationMs, chunks.length);
    return embeddedChunks;
  }

  // Optimized embedding text generation
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

  private printPreciseBenchmarkStats(batchResults: EmbeddingResult[][], wallClockDurationMs: number, totalChunks: number): void {
    console.log('\n📊 ProcessPool Precise Benchmark Results:');
    console.log('━'.repeat(60));
    
    // Flatten results for analysis
    const allResults = batchResults.flat();
    
    // Calculate real wall-clock throughput
    const wallClockThroughput = totalChunks / (wallClockDurationMs / 1000);
    
    console.log('🕐 **REAL DURATION & THROUGHPUT**:');
    console.log(`  Wall-clock duration: ${wallClockDurationMs.toFixed(0)}ms (${(wallClockDurationMs/1000).toFixed(1)}s)`);
    console.log(`  Total throughput: ${wallClockThroughput.toFixed(2)} chunks/second`);
    console.log(`  Total chunks processed: ${totalChunks}`);
    console.log('');
    
    // Per-worker analysis
    const workerStats = new Map<number, {
      batches: number;
      chunks: number;
      totalProcessingTime: number;
      avgBatchTime: number;
      throughputPerWorker: number;
    }>();
    
    batchResults.forEach((batch, batchIndex) => {
      if (batch.length > 0) {
        const processId = batch[0].stats.processId;
        const batchDuration = batch[0].stats.duration;
        const chunkCount = batch.length;
        
        if (!workerStats.has(processId)) {
          workerStats.set(processId, {
            batches: 0,
            chunks: 0,
            totalProcessingTime: 0,
            avgBatchTime: 0,
            throughputPerWorker: 0
          });
        }
        
        const stats = workerStats.get(processId)!;
        stats.batches += 1;
        stats.chunks += chunkCount;
        stats.totalProcessingTime += batchDuration;
      }
    });
    
    // Calculate per-worker throughput
    workerStats.forEach((stats, processId) => {
      stats.avgBatchTime = stats.totalProcessingTime / stats.batches;
      stats.throughputPerWorker = stats.chunks / (stats.totalProcessingTime / 1000);
    });
    
    console.log('👥 **PER-WORKER PERFORMANCE**:');
    workerStats.forEach((stats, processId) => {
      console.log(`  Process ${processId}:`);
      console.log(`    • Chunks: ${stats.chunks}`);
      console.log(`    • Batches: ${stats.batches}`);
      console.log(`    • Processing time: ${stats.totalProcessingTime.toFixed(0)}ms`);
      console.log(`    • Avg batch time: ${stats.avgBatchTime.toFixed(0)}ms`);
      console.log(`    • Worker throughput: ${stats.throughputPerWorker.toFixed(2)} chunks/second`);
      console.log('');
    });
    
    // Concurrency validation
    const maxWorkerTime = Math.max(...Array.from(workerStats.values()).map(s => s.totalProcessingTime));
    const totalWorkerTime = Array.from(workerStats.values()).reduce((sum, s) => sum + s.totalProcessingTime, 0);
    const concurrencyEfficiency = (totalWorkerTime / wallClockDurationMs);
    const theoreticalMaxConcurrency = Math.floor(concurrencyEfficiency);
    
    console.log('🔄 **CONCURRENCY ANALYSIS**:');
    console.log(`  Active workers: ${workerStats.size}/${this.processCount}`);
    console.log(`  Max worker time: ${maxWorkerTime.toFixed(0)}ms`);
    console.log(`  Total worker time: ${totalWorkerTime.toFixed(0)}ms`);
    console.log(`  Concurrency efficiency: ${concurrencyEfficiency.toFixed(1)}x`);
    console.log(`  Parallel speedup: ${(totalWorkerTime / wallClockDurationMs).toFixed(1)}x vs sequential`);
    
    if (concurrencyEfficiency < workerStats.size * 0.7) {  
      console.log(`  ⚠️  Low concurrency efficiency - workers may be idle or contending`);
    } else {
      console.log(`  ✅ Good concurrency efficiency - workers running in parallel`);
    }
    
    console.log('');
    console.log('📈 **SUMMARY**:');
    console.log(`  • Real wall-clock: ${wallClockThroughput.toFixed(2)} chunks/second`);
    console.log(`  • Per-worker avg: ${(Array.from(workerStats.values()).reduce((sum, s) => sum + s.throughputPerWorker, 0) / workerStats.size).toFixed(2)} chunks/second`);
    console.log(`  • Processes utilized: ${workerStats.size}/${this.processCount}`);
    console.log(`  • Parallel efficiency: ${(concurrencyEfficiency / workerStats.size * 100).toFixed(1)}%`);
    
    console.log('━'.repeat(60));
  }

  getProcessCount(): number {
    return this.processCount;
  }

  async shutdown(): Promise<void> {
    console.log('🛑 Shutting down process pool...');
    
    // Wait for queue to drain
    await this.queue.drained();
    
    // Terminate all processes
    await Promise.all(this.processes.map(async (processInstance) => {
      try {
        // Send shutdown message
        this.sendToProcess(processInstance, { type: 'shutdown' });
        
        // Wait a bit for graceful shutdown
        await new Promise(resolve => setTimeout(resolve, 1000));
        
        // Force kill if still running
        if (!processInstance.process.killed) {
          processInstance.process.kill('SIGTERM');
        }
        
        console.log(`✅ Process ${processInstance.id} terminated`);
      } catch (error) {
        console.error(`❌ Error terminating process ${processInstance.id}:`, error);
      }
    }));
    
    console.log('✅ Process pool shut down');
  }

  // Health check method
  getPoolStatus() {
    return {
      processCount: this.processCount,
      initialized: this.isInitialized,
      readyProcesses: this.processes.filter(p => p.isReady).length,
      availableProcesses: this.processes.filter(p => p.isReady && p.isAvailable).length,
      queueLength: this.queue.length(),
      queueRunning: this.queue.running()
    };
  }
}
