import fastq from 'fastq';
import { spawn, ChildProcess } from 'child_process';
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
}

export class ProcessPoolEmbedder {
  private processes: ProcessInstance[] = [];
  private queue: fastq.queueAsPromised<EmbeddingTask, EmbeddingResult>;
  private processCount: number;
  private isInitialized = false;

  constructor() {
    // Calculate optimal process count - reserve cores for system
    const totalCores = os.cpus().length;
    // Use most cores but reserve 2 for system processes
    this.processCount = Math.max(1, totalCores - 2);
    
    console.log(`🏭 Process Pool: ${this.processCount} external Node.js processes (${totalCores} CPU cores, reserved 2 for system)`);
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
        pendingRequests: new Map()
      };

      // Handle process stdout (responses)
      childProcess.stdout?.on('data', (data) => {
        const lines = data.toString().split('\n').filter((line: string) => line.trim());
        
        for (const line of lines) {
          try {
            const message = JSON.parse(line);
            this.handleProcessMessage(processInstance, message);
          } catch (error) {
            console.error(`❌ Failed to parse message from process ${i}:`, line);
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

  // FastQ consumer function - processes one chunk per task
  private async processEmbeddingTask(task: EmbeddingTask): Promise<EmbeddingResult> {
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
      // Create optimized embedding text
      const embeddingText = this.createOptimizedEmbeddingText(task.chunk);
      
      console.log(`🔄 Process ${availableProcess.id} processing chunk ${task.originalIndex} (${embeddingText.length} chars)`);
      
      // Send task to external process
      const batchId = `chunk-${task.originalIndex}-${Date.now()}`;
      const result = await new Promise<any>((resolve, reject) => {
        const timeout = setTimeout(() => {
          availableProcess.pendingRequests.delete(batchId);
          reject(new Error(`Process ${availableProcess.id} timeout`));
        }, 60000); // 60 second timeout per chunk

        availableProcess.pendingRequests.set(batchId, { resolve, reject, timeout });

        this.sendToProcess(availableProcess, {
          type: 'embed_batch',
          batchId,
          data: { texts: [embeddingText] }
        });
      });

      console.log(`✅ Process ${availableProcess.id} completed chunk ${task.originalIndex} in ${result.stats.duration}ms`);
      
      return {
        embedding: result.embeddings[0], // Single chunk = single embedding
        originalIndex: task.originalIndex,
        timestamp: task.timestamp,
        stats: {
          duration: result.stats.duration,
          memoryDelta: result.stats.memoryDelta,
          processId: availableProcess.id
        }
      };

    } finally {
      // Mark process as available again
      availableProcess.isAvailable = true;
    }
  }

  // Main method: Process all chunks using process pool
  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Initialize process pool first
    await this.initialize();
    
    console.log(`📊 Processing ${chunks.length} chunks using ${this.processCount} external processes`);
    
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

    console.log(`⏳ Processing ${tasks.length} tasks with process pool...`);
    
    // Wait for all tasks to complete
    const results = await Promise.all(tasks);
    
    // Sort results by original index to maintain order
    results.sort((a, b) => a.originalIndex - b.originalIndex);
    
    // Merge embeddings back with original chunks
    const embeddedChunks: CodeChunk[] = results.map(result => ({
      ...chunks[result.originalIndex],
      embedding: result.embedding,
      indexed_at: result.timestamp
    }));

    this.printStats(results);
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

  private printStats(results: EmbeddingResult[]): void {
    console.log('\n📊 Process Pool Statistics:');
    console.log('━'.repeat(50));
    
    // Process usage stats
    const processUsage = new Map<number, number>();
    results.forEach(result => {
      const count = processUsage.get(result.stats.processId) || 0;
      processUsage.set(result.stats.processId, count + 1);
    });
    
    console.log('Process Usage:');
    processUsage.forEach((count, processId) => {
      console.log(`  Process ${processId}: ${count} chunks`);
    });
    
    // Performance stats
    const totalDuration = results.reduce((sum, r) => sum + r.stats.duration, 0);
    const avgDuration = Math.round(totalDuration / results.length);
    const maxDuration = Math.max(...results.map(r => r.stats.duration));
    const minDuration = Math.min(...results.map(r => r.stats.duration));
    
    console.log(`Queue: ${this.queue.length()} pending, ${this.queue.running()} running`);
    console.log(`Performance: ${avgDuration}ms avg (${minDuration}-${maxDuration}ms range)`);
    console.log(`Processes: ${this.processes.length} total, ${this.processes.filter(p => p.isReady).length} ready`);
    console.log(`Total chunks: ${results.length}, Timestamp: ${results[0]?.timestamp}`);
    console.log('━'.repeat(50));
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
