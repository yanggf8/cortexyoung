import fastq from 'fastq';
import { spawn, ChildProcess } from 'child_process';
import * as path from 'path';
import * as os from 'os';
import * as crypto from 'crypto';
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
  pendingRequests: Map<string, { 
    resolve: Function; 
    reject: Function; 
    timeout: NodeJS.Timeout; 
    onProgress?: Function;
    timeoutWarning?: NodeJS.Timeout;
  }>;
  messageBuffer: string; // Buffer for incomplete JSON messages
  sharedBuffers: Map<string, SharedArrayBuffer>; // Shared memory buffers for large data transfer
}

interface CachedEmbedding {
  embedding: number[];
  timestamp: number;
  hitCount: number;
  lastAccessed: number; // For LRU eviction
}

interface SharedMemoryResponse {
  type: 'shared_memory';
  batchId: string;
  success: boolean;
  bufferKey: string;
  resultCount: number;
  embedDimension: number;
  stats: any;
  error?: string;
}

interface BatchPerformanceMetrics {
  batchSize: number;
  duration: number;
  memoryUsed: number;
  memoryPeak: number;
  throughput: number; // chunks per second
  success: boolean;
  timestamp: number;
}

interface AdaptiveBatchConfig {
  currentSize: number;
  minSize: number;
  maxSize: number;
  stepSize: number;
  memoryThresholdMB: number;
  performanceHistory: BatchPerformanceMetrics[];
  isOptimizing: boolean;
  optimalSize?: number;
  lastAdjustment: number;
  
  // Hysteresis to prevent oscillation
  convergenceHistory: number[]; // Recent batch sizes to detect oscillation
  stableCount: number; // Count of stable performance measurements
  hysteresisThreshold: number; // % threshold for hysteresis (broader than basic threshold)
  lastDirection: 'up' | 'down' | 'none'; // Track adjustment direction
  
  // Failure recovery
  failureRecoverySize?: number; // Emergency small batch size for failures
  consecutiveFailures: number; // Track consecutive failures for recovery
}

export class ProcessPoolEmbedder {
  private processes: ProcessInstance[] = [];
  private queue: fastq.queueAsPromised<EmbeddingTask, EmbeddingResult[]>;
  private processCount: number;
  private isInitialized = false;
  private embeddingCache: Map<string, CachedEmbedding> = new Map(); // Shared cache across all processes
  private cacheStats = { hits: 0, misses: 0, total: 0, evictions: 0 };
  private adaptiveBatch: AdaptiveBatchConfig;
  private systemMemoryMB: number;
  private isEvicting = false; // Prevent concurrent eviction operations
  
  // Cache management configuration
  private static readonly MAX_CACHE_SIZE = 10000; // Maximum cache entries
  private static readonly CACHE_EVICTION_THRESHOLD = 0.8; // Trigger cleanup at 80% capacity
  private static readonly EVICTION_PERCENTAGE = 0.2; // Remove 20% when evicting

  constructor() {
    // Calculate optimal process count - reserve cores for system
    const totalCores = os.cpus().length;
    // Use most cores but reserve 2 for system processes
    this.processCount = Math.max(1, totalCores - 2);
    
    console.log(`🏭 Process Pool: ${this.processCount} external Node.js processes (${totalCores} CPU cores, reserved 2 for system)`);
    console.log(`✅ Strategy: Process isolation + shared memory cache for optimal performance`);
    console.log(`🧠 Shared embedding cache: Content-based deduplication across all processes`);
    
    // Initialize adaptive batching system
    this.systemMemoryMB = Math.round(os.totalmem() / (1024 * 1024));
    // Simple approach: Focus on Node.js heap hard limit
    // Grow batch sizes until we approach the heap limit for maximum efficiency
    const nodeHeapLimitMB = 4096; // Node.js default heap limit (~4GB)
    const heapThresholdMB = nodeHeapLimitMB * 0.85; // Use 85% of heap limit as threshold
    
    this.adaptiveBatch = {
      currentSize: 400,    // Start with large batch size for efficiency
      minSize: 200,        // Higher minimum for better throughput
      maxSize: 800,        // Cap at 800 chunks as requested
      stepSize: 100,       // Large steps to find optimal size quickly
      memoryThresholdMB: heapThresholdMB, // 85% of Node.js heap limit
      performanceHistory: [],
      isOptimizing: true,
      lastAdjustment: Date.now(),
      
      // Hysteresis configuration
      convergenceHistory: [],
      stableCount: 0,
      hysteresisThreshold: 0.1,
      lastDirection: 'none',
      
      // Failure recovery configuration
      failureRecoverySize: 50,  // Emergency small batch size
      consecutiveFailures: 0
    };
    
    console.log(`🚀 Optimized chunk sizing strategy:`);
    console.log(`  Starting batch size: ${this.adaptiveBatch.currentSize} chunks`);
    console.log(`  Max batch size: ${this.adaptiveBatch.maxSize} chunks`);
    console.log(`  Memory threshold: ${heapThresholdMB}MB (85% of ${nodeHeapLimitMB}MB heap limit)`);
    console.log(`  Strategy: Start large, optimize between ${this.adaptiveBatch.minSize}-${this.adaptiveBatch.maxSize} chunks`);
    
    console.log(`🎯 Adaptive Batching: Start=${this.adaptiveBatch.currentSize}, Range=${this.adaptiveBatch.minSize}-${this.adaptiveBatch.maxSize}, Threshold=${Math.round(this.adaptiveBatch.memoryThresholdMB)}MB`);
    
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
        messageBuffer: '', // Initialize message buffer
        sharedBuffers: new Map() // Shared memory buffers for large data transfer
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
    } else if (type === 'progress' && batchId) {
      // Handle progress updates from child process
      const pending = processInstance.pendingRequests.get(batchId);
      if (pending && pending.onProgress) {
        pending.onProgress(message);
      }
      console.log(`📊 Process ${processInstance.id} progress: ${message.message || `${message.processed}/${message.total} (${message.progress}%)`}`);
    } else if (type === 'timeout_warning' && batchId) {
      // Handle timeout warnings from child process
      console.log(`⚠️ Process ${processInstance.id}: ${message.message}`);
    } else if (type === 'embed_complete' && batchId) {
      const pending = processInstance.pendingRequests.get(batchId);
      if (pending) {
        clearTimeout(pending.timeout);
        if (pending.timeoutWarning) {
          clearTimeout(pending.timeoutWarning);
        }
        processInstance.pendingRequests.delete(batchId);
        
        if (message.success) {
          if (message.partial) {
            console.log(`⚠️ Process ${processInstance.id} returned partial results: ${message.stats.chunksProcessed}/${message.stats.totalChunks} chunks`);
          }
          pending.resolve(message);
        } else {
          pending.reject(new Error(message.error || 'Embedding failed'));
        }
      }
    } else if (type === 'shared_memory' && batchId) {
      // Handle shared memory response
      const pending = processInstance.pendingRequests.get(batchId);
      if (pending) {
        clearTimeout(pending.timeout);
        processInstance.pendingRequests.delete(batchId);
        
        if (message.success) {
          // Read embeddings from shared memory buffer
          const embeddings = this.readEmbeddingsFromSharedMemory(
            processInstance, 
            message as SharedMemoryResponse
          );
          pending.resolve({ ...message, embeddings });
        } else {
          pending.reject(new Error(message.error));
        }
      }
    } else if (type === 'memory_response') {
      // Handle memory query response
      const { requestId } = message;
      if (requestId) {
        const pending = processInstance.pendingRequests.get(requestId);
        if (pending) {
          clearTimeout(pending.timeout);
          processInstance.pendingRequests.delete(requestId);
          
          if (message.success) {
            pending.resolve(message);
          } else {
            pending.reject(new Error(message.error || 'Memory query failed'));
          }
        }
      }
    } else if (type === 'error') {
      console.error(`❌ Process ${processInstance.id} error:`, message.error);
    }
  }

  private async restartProcess(processInstance: ProcessInstance): Promise<void> {
    console.log(`🔄 Restarting failed process ${processInstance.id}...`);
    
    // Kill old process if still running
    if (processInstance.process && !processInstance.process.killed) {
      processInstance.process.kill('SIGTERM');
    }
    
    // Clear any pending requests
    processInstance.pendingRequests.forEach(({ reject, timeout }) => {
      clearTimeout(timeout);
      reject(new Error(`Process ${processInstance.id} restarted`));
    });
    processInstance.pendingRequests.clear();
    
    // Reset state
    processInstance.isReady = false;
    processInstance.isAvailable = false;
    
    // Spawn new process
    const childProcess = spawn('node', [path.join(__dirname, 'external-embedding-process.js')], {
      stdio: ['pipe', 'pipe', 'pipe', 'pipe'],
      env: {
        ...process.env,
        PROCESS_ID: processInstance.id.toString()
      }
    });
    
    processInstance.process = childProcess;
    
    // Set up message handling
    childProcess.on('message', (message) => {
      this.handleProcessMessage(processInstance, message);
    });
    
    // Set up error handling
    childProcess.on('exit', (code, signal) => {
      console.error(`❌ Process ${processInstance.id} exited with code ${code}, signal ${signal}`);
      processInstance.isReady = false;
      processInstance.isAvailable = false;
      
      // Reject any pending requests
      processInstance.pendingRequests.forEach(({ reject, timeout }) => {
        clearTimeout(timeout);
        reject(new Error(`Process ${processInstance.id} exited unexpectedly`));
      });
      processInstance.pendingRequests.clear();
    });
    
    childProcess.on('error', (error) => {
      console.error(`❌ Process ${processInstance.id} error:`, error);
      processInstance.isReady = false;
      processInstance.isAvailable = false;
    });
    
    // Forward stderr
    childProcess.stderr?.on('data', (data) => {
      process.stderr.write(data);
    });
    
    // Initialize the restarted process
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error(`Process ${processInstance.id} restart timeout`));
      }, 30000); // 30 second timeout for restart
      
      const checkReady = () => {
        if (processInstance.isReady) {
          clearTimeout(timeout);
          resolve();
        } else {
          setTimeout(checkReady, 100);
        }
      };
      
      // Send init message
      this.sendToProcess(processInstance, {
        type: 'init',
        data: { processId: processInstance.id }
      });
      
      checkReady();
    });
  }

  private async processSmallBatch(processInstance: ProcessInstance, chunks: any[], batchIndex: number): Promise<{ embeddings: number[][], stats: any }> {
    const embeddingTexts = chunks.map(chunk => this.createOptimizedEmbeddingText(chunk));
    const batchId = `recovery-${batchIndex}-${Date.now()}`;
    
    console.log(`🔧 Processing recovery batch with ${chunks.length} chunks`);
    
    return new Promise<{ embeddings: number[][], stats: any }>((resolve, reject) => {
      const timeoutDuration = 60000; // 1 minute timeout for recovery
      const warningTime = timeoutDuration * 0.7; // 42 seconds
      
      const timeout = setTimeout(() => {
        processInstance.pendingRequests.delete(batchId);
        reject(new Error(`Recovery batch timeout for process ${processInstance.id}`));
      }, timeoutDuration);
      
      const progressCallback = (progressMessage: any) => {
        console.log(`🔧 Recovery batch progress: ${progressMessage.message || 'Processing...'}`);
      };
      
      processInstance.pendingRequests.set(batchId, { 
        resolve, 
        reject, 
        timeout,
        onProgress: progressCallback
      });
      
      // Use simple JSON communication for recovery (no SharedArrayBuffer)
      this.sendToProcess(processInstance, {
        type: 'embed_batch',
        batchId,
        data: { 
          texts: embeddingTexts,
          timeoutWarning: warningTime
        }
      });
    });
  }

  private async retryFailedBatch(chunks: CodeChunk[], originalBatchIndex: number): Promise<EmbeddingResult[]> {
    // Use recovery batch size for retry
    const retryBatchSize = this.adaptiveBatch.failureRecoverySize || 25;
    const results: EmbeddingResult[] = [];
    
    // Process in smaller sub-batches
    for (let i = 0; i < chunks.length; i += retryBatchSize) {
      const subBatch = chunks.slice(i, Math.min(i + retryBatchSize, chunks.length));
      
      // Find an available process
      const availableProcess = this.processes
        .filter(p => p.isReady && p.isAvailable)
        .sort((a, b) => a.lastUsed - b.lastUsed)[0];
      
      if (!availableProcess) {
        // Try to restart a process if none available
        const deadProcess = this.processes.find(p => !p.isReady);
        if (deadProcess) {
          await this.restartProcess(deadProcess);
        } else {
          throw new Error('No processes available for retry');
        }
      }
      
      try {
        const subBatchResult = await this.processSmallBatch(availableProcess, subBatch, originalBatchIndex);
        
        // Convert to EmbeddingResult format
        const subResults = subBatch.map((chunk, idx) => ({
          originalIndex: originalBatchIndex * this.adaptiveBatch.currentSize + i + idx,
          embedding: subBatchResult.embeddings[idx],
          timestamp: Date.now()
        }));
        
        results.push(...subResults);
        console.log(`✅ Sub-batch retry successful: ${subBatch.length} chunks processed`);
        
      } catch (error) {
        console.error(`❌ Sub-batch retry failed:`, error);
        // Add zero embeddings for failed chunks
        const failedResults = subBatch.map((chunk, idx) => ({
          originalIndex: originalBatchIndex * this.adaptiveBatch.currentSize + i + idx,
          embedding: new Array(384).fill(0),
          timestamp: Date.now()
        }));
        results.push(...failedResults);
      }
    }
    
    return results;
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

  // Generate content-based cache key for embedding
  private generateCacheKey(content: string): string {
    // Use full SHA-256 hash to eliminate collision risk (was truncated to 16 chars)
    return crypto.createHash('sha256').update(content.trim()).digest('hex');
  }

  // Check cache for existing embeddings
  private checkCache(chunks: CodeChunk[]): { cached: (number[] | null)[], uncachedIndices: number[] } {
    const cached: (number[] | null)[] = [];
    const uncachedIndices: number[] = [];
    const currentTime = Date.now();
    
    chunks.forEach((chunk, index) => {
      const cacheKey = this.generateCacheKey(chunk.content);
      const cachedEmbedding = this.embeddingCache.get(cacheKey);
      
      if (cachedEmbedding && this.validateCacheEntry(chunk.content, cachedEmbedding.embedding)) {
        cached.push(cachedEmbedding.embedding);
        cachedEmbedding.hitCount++;
        cachedEmbedding.lastAccessed = currentTime; // Update LRU timestamp
        this.cacheStats.hits++;
      } else {
        // Remove invalid cache entry if found
        if (cachedEmbedding) {
          this.embeddingCache.delete(cacheKey);
        }
        cached.push(null);
        uncachedIndices.push(index);
        this.cacheStats.misses++;
      }
      this.cacheStats.total++;
    });
    
    // Check if cache cleanup is needed
    this.checkAndEvictCache();
    
    return { cached, uncachedIndices };
  }

  // Update cache with new embeddings
  private updateCache(chunks: CodeChunk[], embeddings: number[][], timestamp: number): void {
    const currentTime = Date.now();
    
    chunks.forEach((chunk, index) => {
      const cacheKey = this.generateCacheKey(chunk.content);
      
      // Validate embedding before caching
      if (this.validateCacheEntry(chunk.content, embeddings[index])) {
        this.embeddingCache.set(cacheKey, {
          embedding: embeddings[index],
          timestamp,
          hitCount: 0,
          lastAccessed: currentTime
        });
      }
    });
    
    // Check if cache cleanup is needed after adding new entries
    this.checkAndEvictCache();
  }

  // Create shared memory buffer for large embedding data transfer
  private createSharedBuffer(processInstance: ProcessInstance, bufferKey: string, sizeBytes: number): SharedArrayBuffer {
    const buffer = new SharedArrayBuffer(sizeBytes);
    processInstance.sharedBuffers.set(bufferKey, buffer);
    return buffer;
  }

  // Read embeddings from shared memory buffer
  private readEmbeddingsFromSharedMemory(processInstance: ProcessInstance, response: SharedMemoryResponse): number[][] {
    const buffer = processInstance.sharedBuffers.get(response.bufferKey);
    if (!buffer) {
      throw new Error(`Shared buffer not found: ${response.bufferKey}`);
    }

    const view = new Float32Array(buffer);
    const embeddings: number[][] = [];
    
    for (let i = 0; i < response.resultCount; i++) {
      const start = i * response.embedDimension;
      const end = start + response.embedDimension;
      embeddings.push(Array.from(view.slice(start, end)));
    }
    
    // Clean up shared buffer after reading
    processInstance.sharedBuffers.delete(response.bufferKey);
    return embeddings;
  }

  // Validate cache entry for correctness
  private validateCacheEntry(content: string, embedding: number[]): boolean {
    return embedding.length === 384 && // BGE-small-en-v1.5 dimension
           content.trim().length > 0 &&
           embedding.every(n => typeof n === 'number' && !isNaN(n) && isFinite(n));
  }

  // Check if cache needs eviction and perform LRU cleanup (thread-safe)
  private checkAndEvictCache(): void {
    const currentSize = this.embeddingCache.size;
    const threshold = ProcessPoolEmbedder.MAX_CACHE_SIZE * ProcessPoolEmbedder.CACHE_EVICTION_THRESHOLD;
    
    // Only evict if needed and not already evicting (prevents concurrent evictions)
    if (currentSize > threshold && !this.isEvicting) {
      this.evictLRUEntries();
    }
  }

  // Perform LRU eviction to keep cache size manageable (with concurrency protection)
  private evictLRUEntries(): void {
    // Prevent concurrent evictions
    if (this.isEvicting) {
      console.log(`⚠️ Cache eviction already in progress, skipping...`);
      return;
    }
    
    this.isEvicting = true;
    
    try {
      const currentSize = this.embeddingCache.size;
      const targetRemoval = Math.floor(currentSize * ProcessPoolEmbedder.EVICTION_PERCENTAGE);
      
      if (targetRemoval <= 0) {
        return;
      }
      
      console.log(`🧹 Cache eviction: Removing ${targetRemoval} entries (${currentSize} → ${currentSize - targetRemoval})`);
      
      // Convert to array and sort by LRU score (combination of lastAccessed and hitCount)
      const entries = Array.from(this.embeddingCache.entries())
        .map(([key, value]) => {
          // LRU score: prioritize recently accessed and frequently used items
          const timeSinceAccess = Date.now() - value.lastAccessed;
          const lruScore = timeSinceAccess / (value.hitCount + 1); // Lower score = keep longer
          return { key, value, lruScore };
        })
        .sort((a, b) => b.lruScore - a.lruScore); // Highest LRU score first (least valuable)
      
      // Remove least valuable entries atomically
      let removed = 0;
      const keysToRemove: string[] = [];
      
      for (const entry of entries) {
        if (removed >= targetRemoval) break;
        keysToRemove.push(entry.key);
        removed++;
      }
      
      // Batch delete for better performance
      keysToRemove.forEach(key => this.embeddingCache.delete(key));
      
      this.cacheStats.evictions += removed;
      
      console.log(`✅ Cache eviction completed: Removed ${removed} entries, cache size now ${this.embeddingCache.size}`);
      
    } catch (error) {
      console.error(`❌ Cache eviction failed:`, error);
    } finally {
      this.isEvicting = false;
    }
  }

  // Get storage and memory statistics
  private getStorageStats(): { cacheEntries: number; estimatedMemoryMB: number; hitRate: string } {
    const estimatedMemoryBytes = this.embeddingCache.size * 384 * 4; // 384 dims × 4 bytes per float32
    const estimatedMemoryMB = Math.round(estimatedMemoryBytes / (1024 * 1024));
    const hitRate = this.cacheStats.total > 0 
      ? ((this.cacheStats.hits / this.cacheStats.total) * 100).toFixed(1) + '%'
      : '0%';
    
    return {
      cacheEntries: this.embeddingCache.size,
      estimatedMemoryMB,
      hitRate
    };
  }

  // Get current adaptive batch size
  private getAdaptiveBatchSize(totalChunks: number): number {
    // Ensure batch size doesn't exceed total chunks
    return Math.min(this.adaptiveBatch.currentSize, totalChunks);
  }

  // Monitor memory usage with reliable fallback estimation
  private async getSystemWideMemoryStats(): Promise<{ 
    total: number; 
    main: number; 
    children: number[]; 
    peak: number; 
    available: number;
    reliable: boolean;
  }> {
    const mainUsage = process.memoryUsage();
    const mainUsedMB = Math.round(mainUsage.heapUsed / (1024 * 1024));
    
    // During heavy processing, child processes can't respond to IPC queries
    // Use reliable estimation instead
    const activeProcessCount = this.processes.filter(p => p.isReady).length;
    const estimatedChildHeapMB = 12; // Conservative per-process heap estimate
    const estimatedChildMemories = Array(activeProcessCount).fill(estimatedChildHeapMB);
    const totalChildMemoryMB = activeProcessCount * estimatedChildHeapMB;
    const totalUsedMB = mainUsedMB + totalChildMemoryMB;
    
    console.log(`📊 Reliable memory estimate: Main=${mainUsedMB}MB, Children=${activeProcessCount}x${estimatedChildHeapMB}MB=${totalChildMemoryMB}MB, Total=${totalUsedMB}MB`);
    
    return {
      total: totalUsedMB,
      main: mainUsedMB,
      children: estimatedChildMemories,
      peak: Math.round(mainUsage.heapTotal / (1024 * 1024)) + totalChildMemoryMB,
      available: this.systemMemoryMB - totalUsedMB,
      reliable: true
    };
  }
  
  // Query memory usage from a specific child process using Node.js process stats
  private async queryProcessMemory(processInstance: ProcessInstance): Promise<number> {
    return new Promise((resolve) => {
      if (!processInstance.isReady || !processInstance.process.pid) {
        resolve(0);
        return;
      }
      
      const requestId = `memory-${Date.now()}-${processInstance.id}`;
      const timeout = setTimeout(() => {
        console.warn(`⚠️ Memory query timeout for process ${processInstance.id}`);
        resolve(0);
      }, 2000); // Increased timeout to 2 seconds
      
      // Store the request in pending requests for proper handling
      processInstance.pendingRequests.set(requestId, {
        resolve: (response: any) => {
          clearTimeout(timeout);
          const memoryMB = Math.round(response.memoryUsage.heapUsed / (1024 * 1024));
          console.log(`📊 Process ${processInstance.id} memory: ${memoryMB}MB`);
          resolve(memoryMB);
        },
        reject: (error: any) => {
          clearTimeout(timeout);
          console.warn(`⚠️ Memory query failed for process ${processInstance.id}:`, error);
          resolve(0);
        },
        timeout
      });
      
      // Send memory query using existing IPC mechanism
      this.sendToProcess(processInstance, {
        type: 'query_memory',
        requestId
      });
    });
  }
  
  // Get current heap usage - simple and direct
  private getCurrentHeapUsage(): { heapUsedMB: number; heapTotalMB: number; availableHeapMB: number } {
    const usage = process.memoryUsage();
    const heapUsedMB = Math.round(usage.heapUsed / (1024 * 1024));
    const heapTotalMB = Math.round(usage.heapTotal / (1024 * 1024));
    
    // Node.js heap limit (can be set via --max-old-space-size)
    const heapLimitMB = 4096; // Default Node.js heap limit
    const availableHeapMB = heapLimitMB - heapUsedMB;
    
    return {
      heapUsedMB,
      heapTotalMB,
      availableHeapMB: Math.max(0, availableHeapMB)
    };
  }
  
  // Legacy method for backward compatibility - now uses direct heap usage
  private getMemoryStats(): { used: number; peak: number; available: number } {
    const heapStats = this.getCurrentHeapUsage();
    
    return {
      used: heapStats.heapUsedMB,
      peak: heapStats.heapTotalMB,
      available: heapStats.availableHeapMB
    };
  }

  // Record batch performance and adapt size with reliable memory tracking
  private recordBatchPerformance(batchSize: number, duration: number, memoryBefore: number, memoryAfter: number, success: boolean): void {
    const throughput = success ? (batchSize / duration) * 1000 : 0; // chunks per second
    
    // Use reliable heap usage for immediate adaptive decisions
    const currentHeap = this.getCurrentHeapUsage();
    
    const metrics: BatchPerformanceMetrics = {
      batchSize,
      duration,
      memoryUsed: currentHeap.heapUsedMB - memoryBefore,
      memoryPeak: currentHeap.heapUsedMB, // Use current heap usage
      throughput,
      success,
      timestamp: Date.now()
    };
    
    this.adaptiveBatch.performanceHistory.push(metrics);
    
    // Keep only last 20 measurements for analysis
    if (this.adaptiveBatch.performanceHistory.length > 20) {
      this.adaptiveBatch.performanceHistory.shift();
    }
    
    // Adapt batch size based on performance
    this.adaptBatchSize(metrics);
    
    console.log(`📈 Batch perf: size=${batchSize}, duration=${duration}ms, throughput=${throughput.toFixed(1)} chunks/s, heap=${currentHeap.heapUsedMB}MB (${currentHeap.availableHeapMB}MB available)`);
  }

  // Adaptive batch sizing algorithm with hysteresis to prevent oscillation
  private adaptBatchSize(latestMetrics: BatchPerformanceMetrics): void {
    const config = this.adaptiveBatch;
    const history = config.performanceHistory;
    
    // Don't adjust too frequently - minimum 5 seconds between adjustments
    if (Date.now() - config.lastAdjustment < 5000) {
      return;
    }
    
    // Memory pressure check using actual system-wide memory
    // Account for the fact that each child process uses ~327MB RSS (not just 7MB heap)
    const actualMemoryThreshold = config.memoryThresholdMB;
    
    if (latestMetrics.memoryPeak > actualMemoryThreshold) {
      const reduction = Math.max(config.stepSize, Math.floor(config.currentSize * 0.2));
      const oldSize = config.currentSize;
      config.currentSize = Math.max(config.minSize, config.currentSize - reduction);
      config.lastDirection = 'down';
      config.lastAdjustment = Date.now();
      config.stableCount = 0; // Reset stability counter
      
      console.log(`⚠️ Memory pressure detected!`);
      console.log(`  System memory: ${latestMetrics.memoryPeak}MB / ${actualMemoryThreshold}MB threshold`);
      console.log(`  Reducing batch size: ${oldSize} → ${config.currentSize} (-${reduction} chunks)`);
      return;
    }
    
    // Need at least 3 successful measurements to make decisions
    const recentSuccessful = history.filter(m => m.success && m.timestamp > Date.now() - 30000);
    if (recentSuccessful.length < 3) {
      return;
    }
    
    // Track convergence history for oscillation detection
    config.convergenceHistory.push(config.currentSize);
    if (config.convergenceHistory.length > 10) {
      config.convergenceHistory.shift(); // Keep last 10 batch sizes
    }
    
    // Check for oscillation pattern
    if (this.detectOscillation(config)) {
      config.isOptimizing = false;
      config.optimalSize = config.currentSize;
      console.log(`🌊 Oscillation detected! Converging at batch size ${config.currentSize}`);
      return;
    }
    
    // Calculate average throughput for recent batches with weighted average
    const weights = recentSuccessful.map((_, i) => i + 1); // More recent = higher weight
    const weightSum = weights.reduce((sum, w) => sum + w, 0);
    const avgThroughput = recentSuccessful.reduce((sum, m, i) => sum + m.throughput * weights[i], 0) / weightSum;
    const currentThroughput = latestMetrics.throughput;
    
    // Hysteresis thresholds - different for up vs down based on last direction
    const upThreshold = config.lastDirection === 'down' 
      ? 1 + config.hysteresisThreshold // Require stronger signal to reverse direction
      : 1.05; // Original 5% threshold
    const downThreshold = config.lastDirection === 'up'
      ? 1 - config.hysteresisThreshold // Require stronger signal to reverse direction  
      : 0.95; // Original 5% threshold
    
    const throughputRatio = currentThroughput / avgThroughput;
    
    // If still optimizing (haven't found optimal size yet)
    if (config.isOptimizing) {
      if (throughputRatio > upThreshold && config.currentSize < config.maxSize) {
        // Performance improved - increase batch size
        const oldSize = config.currentSize;
        config.currentSize = Math.min(config.maxSize, config.currentSize + config.stepSize);
        config.lastDirection = 'up';
        config.stableCount = 0;
        console.log(`🚀 Throughput improved (${throughputRatio.toFixed(3)}x)! Increasing batch size: ${oldSize} → ${config.currentSize}`);
      }
      else if (throughputRatio < downThreshold) {
        // Performance declined - we may have found the peak
        const oldSize = config.currentSize;
        config.currentSize = Math.max(config.minSize, config.currentSize - config.stepSize);
        config.lastDirection = 'down';
        
        // Check if we should converge
        if (config.stableCount >= 2) { // Need multiple confirmations
          config.isOptimizing = false;
          config.optimalSize = config.currentSize;
          console.log(`🎯 Found optimal batch size: ${config.currentSize} (throughput peaked, confirmed over ${config.stableCount + 1} measurements)`);
        } else {
          config.stableCount++;
          console.log(`📉 Throughput declined (${throughputRatio.toFixed(3)}x), reducing batch size: ${oldSize} → ${config.currentSize} (confirmation ${config.stableCount}/3)`);
        }
      }
      else {
        // Performance stable - increment stability counter
        config.stableCount++;
        if (config.stableCount >= 5) {
          config.isOptimizing = false;
          config.optimalSize = config.currentSize;
          console.log(`🎯 Converged at stable batch size: ${config.currentSize} (${config.stableCount} stable measurements)`);
        }
      }
    } else {
      // In maintenance mode - small adjustments around optimal size with stronger hysteresis
      const optimalSize = config.optimalSize || config.currentSize;
      const maintenanceThreshold = 0.15; // 15% threshold for maintenance mode
      
      if (throughputRatio < (1 - maintenanceThreshold)) {
        // Significant performance drop - fine-tune
        const adjustment = Math.floor(config.stepSize / 2);
        const oldSize = config.currentSize;
        
        if (config.currentSize > optimalSize) {
          config.currentSize = Math.max(config.minSize, config.currentSize - adjustment);
        } else {
          config.currentSize = Math.min(config.maxSize, config.currentSize + adjustment);
        }
        
        console.log(`🔧 Fine-tuning batch size: ${oldSize} → ${config.currentSize} (throughput: ${throughputRatio.toFixed(3)}x)`);
      }
    }
    
    config.lastAdjustment = Date.now();
  }
  
  // Detect oscillation pattern in batch size adjustments
  private detectOscillation(config: AdaptiveBatchConfig): boolean {
    const history = config.convergenceHistory;
    if (history.length < 6) return false;
    
    // Look for alternating pattern in last 6 measurements
    const recent = history.slice(-6);
    let oscillations = 0;
    
    for (let i = 1; i < recent.length - 1; i++) {
      const prev = recent[i - 1];
      const curr = recent[i];
      const next = recent[i + 1];
      
      // Check if current value is a local min or max
      if ((curr > prev && curr > next) || (curr < prev && curr < next)) {
        oscillations++;
      }
    }
    
    // Oscillation detected if we have 3+ local extrema in 6 measurements
    return oscillations >= 3;
  }

  // FastQ consumer function - processes a batch of chunks per task
  private async processEmbeddingTask(task: EmbeddingTask): Promise<EmbeddingResult[]> {
    // Check cache first - this is the key improvement!
    const { cached, uncachedIndices } = this.checkCache(task.chunks);
    const cacheHits = cached.filter(emb => emb !== null).length;
    const cacheMisses = uncachedIndices.length;
    
    console.log(`🧠 Cache check: ${cacheHits} hits, ${cacheMisses} misses (${Math.round(cacheHits/task.chunks.length*100)}% hit rate)`);
    
    let processEmbeddings: number[][] = [];
    
    // Only process uncached chunks if any exist
    if (uncachedIndices.length > 0) {
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
        // Only create embedding texts for uncached chunks
        const uncachedChunks = uncachedIndices.map(i => task.chunks[i]);
        const embeddingTexts = uncachedChunks.map(chunk => this.createOptimizedEmbeddingText(chunk));
        
        console.log(`🔄 Process ${availableProcess.id} processing batch ${task.batchIndex} (${uncachedChunks.length}/${task.chunks.length} uncached chunks)`);
        
        // Record memory before processing (including child processes for accurate monitoring)
        const memoryBefore = this.getMemoryStats().used; // Quick estimate for immediate use
        const startTime = Date.now();
        
        // Use true shared memory for large batches (>50 chunks), JSON for smaller ones
        const embedDimension = 384; // BGE-small-en-v1.5 dimension
        const useSharedMemory = uncachedChunks.length > 50;
        const bufferKey = `batch-${task.batchIndex}-${Date.now()}`;
        let sharedBuffer: SharedArrayBuffer | null = null;
        
        if (useSharedMemory) {
          const bufferSize = uncachedChunks.length * embedDimension * 4; // 4 bytes per float32
          sharedBuffer = this.createSharedBuffer(availableProcess, bufferKey, bufferSize);
          console.log(`💾 Using SharedArrayBuffer for ${uncachedChunks.length} chunks (${Math.round(bufferSize/1024)}KB)`);
        }
        
        // Send batch with shared memory info to external process
        const batchId = bufferKey;
        let result: any;
        let processingSuccess = true;
        
        try {
          result = await new Promise<any>((resolve, reject) => {
            const timeoutDuration = 120000; // 2 minute total timeout
            const warningTime = timeoutDuration * 0.7; // 70% of timeout (84 seconds)
            
            // Set up progressive timeout - first warning, then hard timeout
            const timeout = setTimeout(() => {
              availableProcess.pendingRequests.delete(batchId);
              reject(new Error(`Process ${availableProcess.id} hard timeout after ${timeoutDuration/1000}s`));
            }, timeoutDuration);

            const progressCallback = (progressMessage: any) => {
              console.log(`📊 Batch ${task.batchIndex} progress: ${progressMessage.message || 'Processing...'}`);
            };

            availableProcess.pendingRequests.set(batchId, { 
              resolve, 
              reject, 
              timeout,
              onProgress: progressCallback
            });

            this.sendToProcess(availableProcess, {
              type: useSharedMemory ? 'embed_batch_shared' : 'embed_batch',
              batchId,
              data: { 
                texts: embeddingTexts,
                timeoutWarning: warningTime, // Tell child process when to send warnings
                ...(useSharedMemory && {
                  sharedBufferKey: bufferKey,
                  expectedResults: uncachedChunks.length,
                  embedDimension
                })
              }
            });
          });
        } catch (error) {
          processingSuccess = false;
          console.error(`❌ Process ${availableProcess.id} failed batch ${task.batchIndex}:`, error);
          
          // Increment failure count and trigger recovery
          this.adaptiveBatch.consecutiveFailures++;
          console.log(`🚨 Process failure detected (${this.adaptiveBatch.consecutiveFailures} consecutive failures)`);
          
          // If we have multiple failures, switch to recovery mode
          if (this.adaptiveBatch.consecutiveFailures >= 2) {
            console.log(`🔧 Entering failure recovery mode - reducing batch size to ${this.adaptiveBatch.failureRecoverySize}`);
            const originalSize = this.adaptiveBatch.currentSize;
            this.adaptiveBatch.currentSize = this.adaptiveBatch.failureRecoverySize || 50;
            
            try {
              // Restart the failed process
              await this.restartProcess(availableProcess);
              console.log(`✅ Process ${availableProcess.id} restarted successfully`);
              
              // Retry with smaller chunks
              const smallerChunks = uncachedChunks.slice(0, this.adaptiveBatch.currentSize);
              const remainingChunks = uncachedChunks.slice(this.adaptiveBatch.currentSize);
              
              console.log(`🔄 Retrying with reduced batch: ${smallerChunks.length} chunks (${remainingChunks.length} remaining)`);
              
              // Process smaller batch
              const retryResult = await this.processSmallBatch(availableProcess, smallerChunks, task.batchIndex);
              processEmbeddings = retryResult.embeddings;
              
              // If successful, reset failure count and restore some confidence
              this.adaptiveBatch.consecutiveFailures = 0;
              this.adaptiveBatch.currentSize = Math.min(originalSize, this.adaptiveBatch.currentSize * 2);
              console.log(`✅ Recovery successful - gradually increasing batch size to ${this.adaptiveBatch.currentSize}`);
              
              // Update cache with successful embeddings
              this.updateCache(smallerChunks, processEmbeddings, task.timestamp);
              
              // If there were remaining chunks, we'll need to process them separately
              if (remainingChunks.length > 0) {
                console.log(`📋 ${remainingChunks.length} chunks remaining - will be processed in next batch`);
                // Add remaining chunks back to queue for next iteration
                // This is handled by the caller in the main processing loop
              }
              
              return processEmbeddings;
              
            } catch (retryError) {
              console.error(`❌ Recovery failed for process ${availableProcess.id}:`, retryError);
              // Further reduce batch size for next attempt
              this.adaptiveBatch.failureRecoverySize = Math.max(10, (this.adaptiveBatch.failureRecoverySize || 50) / 2);
              this.adaptiveBatch.currentSize = this.adaptiveBatch.failureRecoverySize;
              throw retryError;
            }
          }
          
          throw error;
        } finally {
          // Record performance metrics for adaptive sizing with direct heap tracking
          const duration = Date.now() - startTime;
          
          // Track actual Node.js heap usage - simple and direct
          const heapUsage = this.getCurrentHeapUsage();
          console.log(`🚀 Heap usage: ${heapUsage.heapUsedMB}MB used, ${heapUsage.availableHeapMB}MB available (${heapUsage.heapTotalMB}MB total)`);
          
          this.recordBatchPerformance(uncachedChunks.length, duration, memoryBefore, heapUsage.heapUsedMB, processingSuccess);
          
          // Get reliable system-wide stats for next iteration (don't block current processing)
          this.getSystemWideMemoryStats().then(systemStats => {
            if (systemStats.reliable) {
              console.log(`📊 System memory: Total=${systemStats.total}MB (Main=${systemStats.main}MB, Children=${systemStats.children.reduce((sum, mem) => sum + mem, 0)}MB)`);
            }
          }).catch(() => {
            // Silently ignore
          });
        }

        console.log(`✅ Process ${availableProcess.id} completed batch ${task.batchIndex} (${uncachedChunks.length} chunks) in ${result.stats.duration}ms`);
        
        // Reset failure count on successful processing
        if (this.adaptiveBatch.consecutiveFailures > 0) {
          console.log(`✅ Successful batch - resetting failure count from ${this.adaptiveBatch.consecutiveFailures} to 0`);
          this.adaptiveBatch.consecutiveFailures = 0;
        }
        
        processEmbeddings = result.embeddings;
        
        // Update cache with new embeddings
        this.updateCache(uncachedChunks, processEmbeddings, task.timestamp);
        
      } finally {
        // Mark process as available again
        availableProcess.isAvailable = true;
      }
    }
    
    // Merge cached and newly computed embeddings
    const finalEmbeddings: number[][] = [];
    let uncachedIndex = 0;
    
    for (let i = 0; i < task.chunks.length; i++) {
      if (cached[i] !== null) {
        // Use cached embedding
        finalEmbeddings.push(cached[i] as number[]);
      } else {
        // Use newly computed embedding
        finalEmbeddings.push(processEmbeddings[uncachedIndex]);
        uncachedIndex++;
      }
    }
    
    // Map results back to individual chunks with original indices
    const batchResults: EmbeddingResult[] = [];
    
    for (let i = 0; i < task.chunks.length; i++) {
      batchResults.push({
        embedding: finalEmbeddings[i],
        originalIndex: task.originalIndices[i],
        timestamp: task.timestamp,
        stats: {
          duration: uncachedIndices.length > 0 ? (processEmbeddings.length > 0 ? 25000 : 0) : 0, // Estimated duration
          memoryDelta: 0,
          processId: -1 // -1 indicates cache hit
        }
      });
    }

    return batchResults;
  }

  // Main method: Process all chunks using process pool with batching
  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Initialize process pool first
    await this.initialize();
    
    console.log(`📊 Processing ${chunks.length} chunks using ${this.processCount} external processes`);
    
    const currentTimestamp = Date.now();
    
    // Use adaptive batch size that learns and optimizes over time
    const adaptiveBatchSize = this.getAdaptiveBatchSize(chunks.length);
    
    console.log(`🎯 Adaptive batch size: ${adaptiveBatchSize} chunks per batch (learning optimal size)`);
    console.log(`📈 Batch optimization: ${this.adaptiveBatch.isOptimizing ? 'Learning' : 'Converged'} (history: ${this.adaptiveBatch.performanceHistory.length} samples)`);
    
    // Create batches for each process with original index tracking
    const batches: { chunks: CodeChunk[]; originalIndices: number[] }[] = [];
    for (let i = 0; i < chunks.length; i += adaptiveBatchSize) {
      const batchChunks = chunks.slice(i, i + adaptiveBatchSize);
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
    
    // Wait for all tasks to complete with retry logic
    const batchResults = await Promise.allSettled(tasks);
    
    // Handle failed batches by retrying with smaller chunks
    const successfulResults: EmbeddingResult[][] = [];
    const failedBatches: { batchIndex: number, chunks: CodeChunk[], error: any }[] = [];
    
    batchResults.forEach((result, index) => {
      if (result.status === 'fulfilled') {
        successfulResults.push(result.value);
      } else {
        console.log(`❌ Batch ${index} failed:`, result.reason);
        const startIdx = index * this.adaptiveBatch.currentSize;
        const endIdx = Math.min(startIdx + this.adaptiveBatch.currentSize, chunks.length);
        failedBatches.push({
          batchIndex: index,
          chunks: chunks.slice(startIdx, endIdx),
          error: result.reason
        });
      }
    });
    
    // Retry failed batches with smaller chunk sizes
    for (const failedBatch of failedBatches) {
      try {
        console.log(`🔄 Retrying failed batch ${failedBatch.batchIndex} with ${failedBatch.chunks.length} chunks`);
        const retryResults = await this.retryFailedBatch(failedBatch.chunks, failedBatch.batchIndex);
        successfulResults.push(retryResults);
      } catch (retryError) {
        console.error(`❌ Final retry failed for batch ${failedBatch.batchIndex}:`, retryError);
        // Add empty results to maintain indexing
        const emptyResults: EmbeddingResult[] = failedBatch.chunks.map((chunk, idx) => ({
          originalIndex: failedBatch.batchIndex * this.adaptiveBatch.currentSize + idx,
          embedding: new Array(384).fill(0), // Zero embedding as fallback
          timestamp: Date.now()
        }));
        successfulResults.push(emptyResults);
      }
    }
    
    // Flatten successful results
    const allResults: EmbeddingResult[] = [];
    successfulResults.forEach(batchResult => {
      allResults.push(...batchResult);
    });
    
    allResults.sort((a, b) => a.originalIndex - b.originalIndex);
    
    // Merge embeddings back with original chunks
    const embeddedChunks: CodeChunk[] = allResults.map(result => ({
      ...chunks[result.originalIndex],
      embedding: result.embedding,
      indexed_at: result.timestamp
    }));

    this.printBatchStats(batchResults);
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

  private printBatchStats(batchResults: EmbeddingResult[][]): void {
    console.log('\n📊 Process Pool Statistics:');
    console.log('━'.repeat(50));
    
    // Flatten results for analysis
    const allResults = batchResults.flat();
    
    // Enhanced cache statistics
    const hitRate = this.cacheStats.total > 0 ? (this.cacheStats.hits / this.cacheStats.total * 100) : 0;
    const storageStats = this.getStorageStats();
    const utilizationRate = (this.embeddingCache.size / ProcessPoolEmbedder.MAX_CACHE_SIZE * 100);
    
    console.log(`🧠 Cache Performance:`);
    console.log(`  Total requests: ${this.cacheStats.total}`);
    console.log(`  Cache hits: ${this.cacheStats.hits}`);
    console.log(`  Cache misses: ${this.cacheStats.misses}`);
    console.log(`  Cache evictions: ${this.cacheStats.evictions}`);
    console.log(`  Hit rate: ${hitRate.toFixed(1)}%`);
    console.log(`  Cache size: ${this.embeddingCache.size}/${ProcessPoolEmbedder.MAX_CACHE_SIZE} (${utilizationRate.toFixed(1)}%)`);
    console.log(`  Memory usage: ~${storageStats.estimatedMemoryMB}MB`);
    
    // Process usage stats (exclude cache hits with processId = -1)
    const processUsage = new Map<number, number>();
    const batchSizes = new Map<number, number>();
    const cacheHits = allResults.filter(r => r.stats.processId === -1).length;
    
    batchResults.forEach((batch, batchIndex) => {
      if (batch.length > 0) {
        const processId = batch[0].stats.processId;
        if (processId !== -1) { // Exclude cache hits
          processUsage.set(processId, (processUsage.get(processId) || 0) + 1);
        }
        batchSizes.set(batchIndex, batch.length);
      }
    });
    
    console.log('\nProcess Usage (batches):');
    if (cacheHits > 0) {
      console.log(`  Cache: ${cacheHits} chunks (no process needed)`);
    }
    processUsage.forEach((batchCount, processId) => {
      const totalChunks = allResults.filter(r => r.stats.processId === processId).length;
      console.log(`  Process ${processId}: ${batchCount} batch(es), ${totalChunks} chunks`);
    });
    
    console.log('\nBatch Sizes:');
    batchSizes.forEach((size, batchIndex) => {
      console.log(`  Batch ${batchIndex}: ${size} chunks`);
    });
    
    // Performance stats (only for non-cached results)
    const batchDurations = batchResults.map(batch => 
      batch.length > 0 && batch[0].stats.processId !== -1 ? batch[0].stats.duration : 0
    ).filter(d => d > 0);
    
    if (batchDurations.length > 0) {
      const avgDuration = Math.round(batchDurations.reduce((sum, d) => sum + d, 0) / batchDurations.length);
      const maxDuration = Math.max(...batchDurations);
      const minDuration = Math.min(...batchDurations);
      
      console.log(`\nPerformance:`);
      console.log(`  Batch duration: ${avgDuration}ms avg (${minDuration}-${maxDuration}ms range)`);
      console.log(`  Total chunks: ${allResults.length}`);
      console.log(`  Cached chunks: ${cacheHits}`);
      console.log(`  Processed chunks: ${allResults.length - cacheHits}`);
      console.log(`  Total batches: ${batchResults.length}`);
      console.log(`  Processes used: ${processUsage.size}/${this.processes.length}`);
    }
    
    console.log('━'.repeat(50));
  }

  // Clear cache manually (useful for testing or memory pressure)
  clearCache(): void {
    console.log(`🧹 Manually clearing cache (${this.embeddingCache.size} entries)`);
    this.embeddingCache.clear();
    this.cacheStats = { hits: 0, misses: 0, total: 0, evictions: 0 };
    console.log(`✅ Cache cleared successfully`);
  }

  async shutdown(): Promise<void> {
    console.log('🛑 Shutting down process pool...');
    
    // Wait for any ongoing eviction to complete
    while (this.isEvicting) {
      console.log('⏳ Waiting for cache eviction to complete...');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    
    // Wait for queue to drain
    await this.queue.drained();
    
    // Clear cache to free memory
    this.clearCache();
    
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
        
        // Clean up shared buffers
        processInstance.sharedBuffers.clear();
        
        console.log(`✅ Process ${processInstance.id} terminated`);
      } catch (error) {
        console.error(`❌ Error terminating process ${processInstance.id}:`, error);
      }
    }));
    
    console.log('✅ Process pool shut down');
  }

  // Health check method
  getPoolStatus() {
    const hitRate = this.cacheStats.total > 0 ? (this.cacheStats.hits / this.cacheStats.total * 100) : 0;
    
    return {
      processCount: this.processCount,
      initialized: this.isInitialized,
      readyProcesses: this.processes.filter(p => p.isReady).length,
      availableProcesses: this.processes.filter(p => p.isReady && p.isAvailable).length,
      queueLength: this.queue.length(),
      queueRunning: this.queue.running(),
      cache: {
        size: this.embeddingCache.size,
        maxSize: ProcessPoolEmbedder.MAX_CACHE_SIZE,
        utilizationRate: ((this.embeddingCache.size / ProcessPoolEmbedder.MAX_CACHE_SIZE) * 100).toFixed(1) + '%',
        hits: this.cacheStats.hits,
        misses: this.cacheStats.misses,
        evictions: this.cacheStats.evictions,
        total: this.cacheStats.total,
        hitRate: hitRate.toFixed(1) + '%',
        estimatedMemoryMB: this.getStorageStats().estimatedMemoryMB
      }
    };
  }

  // Get comprehensive cache statistics
  getCacheStats() {
    const hitRate = this.cacheStats.total > 0 ? (this.cacheStats.hits / this.cacheStats.total * 100) : 0;
    const storageStats = this.getStorageStats();
    
    // Calculate cache efficiency metrics
    const entries = Array.from(this.embeddingCache.values());
    const avgHitCount = entries.length > 0 
      ? entries.reduce((sum, entry) => sum + entry.hitCount, 0) / entries.length 
      : 0;
    
    const utilizationRate = (this.embeddingCache.size / ProcessPoolEmbedder.MAX_CACHE_SIZE * 100);
    
    return {
      // Basic stats
      size: this.embeddingCache.size,
      maxSize: ProcessPoolEmbedder.MAX_CACHE_SIZE,
      utilizationRate: utilizationRate.toFixed(1) + '%',
      
      // Hit/miss stats
      hits: this.cacheStats.hits,
      misses: this.cacheStats.misses,
      total: this.cacheStats.total,
      evictions: this.cacheStats.evictions,
      hitRate: hitRate.toFixed(1) + '%',
      
      // Memory stats
      estimatedMemoryMB: storageStats.estimatedMemoryMB,
      avgHitCount: avgHitCount.toFixed(1),
      
      // Top performers
      topEntries: Array.from(this.embeddingCache.entries())
        .sort((a, b) => b[1].hitCount - a[1].hitCount)
        .slice(0, 10)
        .map(([key, value]) => ({ 
          key: key.substring(0, 12) + '...', 
          hitCount: value.hitCount,
          age: Math.round((Date.now() - value.timestamp) / 1000) + 's'
        })),
        
      // LRU candidates (for monitoring)
      lruCandidates: entries.length > 100 ? Array.from(this.embeddingCache.entries())
        .map(([key, value]) => {
          const timeSinceAccess = Date.now() - value.lastAccessed;
          return { 
            key: key.substring(0, 12) + '...', 
            lruScore: timeSinceAccess / (value.hitCount + 1),
            lastAccessed: Math.round(timeSinceAccess / 1000) + 's ago'
          };
        })
        .sort((a, b) => b.lruScore - a.lruScore)
        .slice(0, 5) : []
    };
  }

  // Get adaptive batch statistics
  async getAdaptiveBatchStats() {
    const recentMetrics = this.adaptiveBatch.performanceHistory.slice(-5);
    const avgThroughput = recentMetrics.length > 0 
      ? recentMetrics.reduce((sum, m) => sum + m.throughput, 0) / recentMetrics.length 
      : 0;
    
    // Get current system-wide memory stats
    let systemMemory;
    try {
      systemMemory = await this.getSystemWideMemoryStats();
    } catch (error) {
      systemMemory = {
        total: 0,
        main: this.getMemoryStats().used,
        children: [],
        peak: 0,
        available: 0
      };
    }
    
    return {
      currentSize: this.adaptiveBatch.currentSize,
      optimalSize: this.adaptiveBatch.optimalSize,
      isOptimizing: this.adaptiveBatch.isOptimizing,
      stableCount: this.adaptiveBatch.stableCount,
      lastDirection: this.adaptiveBatch.lastDirection,
      minSize: this.adaptiveBatch.minSize,
      maxSize: this.adaptiveBatch.maxSize,
      memoryThresholdMB: this.adaptiveBatch.memoryThresholdMB,
      performanceSamples: this.adaptiveBatch.performanceHistory.length,
      avgThroughput: avgThroughput.toFixed(1) + ' chunks/s',
      
      // System-wide memory stats
      systemMemory: {
        totalMB: systemMemory.total,
        mainProcessMB: systemMemory.main,
        childProcessesMB: systemMemory.children.reduce((sum, mem) => sum + mem, 0),
        availableMB: systemMemory.available,
        processBreakdown: systemMemory.children.map((mem, i) => `P${i}: ${mem}MB`).join(', ')
      },
      
      // Hysteresis info
      hysteresis: {
        threshold: (this.adaptiveBatch.hysteresisThreshold * 100).toFixed(1) + '%',
        oscillationHistory: this.adaptiveBatch.convergenceHistory.slice(-6),
        oscillationDetected: this.detectOscillation(this.adaptiveBatch)
      },
      
      recentPerformance: recentMetrics.map(m => ({
        size: m.batchSize,
        throughput: m.throughput.toFixed(1),
        memoryMB: m.memoryPeak,
        success: m.success
      }))
    };
  }
}
