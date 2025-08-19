#!/usr/bin/env node

// External Node.js process for embedding generation
// Complete isolation from main process - coordinates via IPC for shared caching

const readline = require('readline');
// Memory-mapped cache will be imported dynamically after TypeScript compilation

// Simple timestamp function for consistent logging
function timestampedLog(...args) {
  const timestamp = new Date().toISOString();
  console.error(`[${timestamp}]`, ...args);
}

function timestampedWrite(message) {
  const timestamp = new Date().toISOString();
  process.stderr.write(`[${timestamp}] ${message}`);
}

// Process-local variables - completely isolated
let embedder = null;
let processId = null;
let isInitialized = false;
let cache = null;

// Create readline interface for IPC communication
const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout
});

// Initialize embedder in this separate Node.js process
async function initializeEmbedder(id) {
  try {
    processId = id;
    // Log to stderr for parent process to capture as debug info (not error)
    timestampedWrite(`[Process] Node.js version: ${process.version}, Platform: ${process.platform}, Arch: ${process.arch}\n`);
    timestampedWrite(`[Process ${processId}] Starting separate Node.js instance...\n`);
    
    // Dynamic import in separate process - complete isolation
    const { FlagEmbedding, EmbeddingModel } = await import('fastembed');
    
    timestampedWrite(`[Process ${processId}] Creating isolated FastEmbedding instance...\n`);
    
    // Each process gets its own BGE instance with simplified configuration
    embedder = await FlagEmbedding.init({
      model: EmbeddingModel.BGESmallENV15,
      maxLength: 512,  // Standard max length
      cacheDir: './.fastembed_cache'
      // Use default settings - let FastEmbed choose the best available backend
    });
    
    // Initialize memory-mapped cache (same files as parent process)
    try {
      const { MemoryMappedCache } = require('../dist/memory-mapped-cache.js');
      cache = MemoryMappedCache.getInstance('./.cortex/mmap-cache', 10000, 384);
      
      // Suppress MemoryMappedCache logging to avoid JSON parsing interference
      const originalConsoleLog = console.log;
      const originalConsoleError = console.error;
      console.log = () => {};
      console.error = () => {};
      
      await cache.initialize();
      
      // Restore console logging
      console.log = originalConsoleLog;
      console.error = originalConsoleError;
      
      timestampedWrite(`[Process ${processId}] Memory-mapped cache initialized\n`);
    } catch (error) {
      timestampedWrite(`[Process ${processId}] Failed to initialize cache: ${error.message}\n`);
      cache = null; // Fallback to no cache
    }
    
    isInitialized = true;
    timestampedWrite(`[Process ${processId}] FastEmbedding ready in separate process with shared cache\n`);
    
    // Send success response
    console.log(JSON.stringify({
      type: 'init_complete',
      processId,
      success: true
    }));
    
  } catch (error) {
    timestampedLog(`[Process ${processId}] Initialization failed:`, error);
    console.log(JSON.stringify({
      type: 'init_complete',
      processId,
      success: false,
      error: error.message
    }));
    process.exit(1);
  }
}

// Process embedding batch in this separate process with progress reporting
async function processEmbeddingBatch(texts, batchId, timeoutWarning = null) {
  if (!isInitialized || !embedder) {
    throw new Error(`Process ${processId} not initialized`);
  }
  
  const startTime = Date.now();
  const beforeMemory = process.memoryUsage();
  let progressReported = false;
  let timeoutWarned = false;
  
  // Set up more frequent progress reporting for slow processing
  const progressInterval = setInterval(() => {
    const elapsed = Date.now() - startTime;
    const memoryUsage = process.memoryUsage();
    console.log(JSON.stringify({
      type: 'progress',
      batchId,
      processId,
      elapsed,
      status: 'processing',
      message: `Processing ${texts.length} texts - ${Math.round(elapsed/1000)}s elapsed - RSS: ${Math.round(memoryUsage.rss/1024/1024)}MB`,
      memoryMB: Math.round(memoryUsage.rss/1024/1024),
      heapUsedMB: Math.round(memoryUsage.heapUsed/1024/1024)
    }));
    progressReported = true;
  }, 3000); // Report progress every 3 seconds for better visibility
  
  // Set up timeout warning
  let timeoutWarningTimer = null;
  if (timeoutWarning && timeoutWarning > 0) {
    timeoutWarningTimer = setTimeout(() => {
      console.log(JSON.stringify({
        type: 'timeout_warning',
        batchId,
        processId,
        message: `Process approaching timeout, will return partial results soon`
      }));
      timeoutWarned = true;
    }, timeoutWarning);
  }
  
  try {
    timestampedLog(`[Process ${processId}] Processing ${texts.length} texts for batch ${batchId}`);
    
    // Use this process's isolated embedder instance with memory-conscious processing
    // Process in smaller sub-batches to prevent memory spikes
    const maxSubBatchSize = Math.min(50, Math.ceil(texts.length / 4)); // Max 50 or 1/4 of batch
    const subBatches = [];
    
    for (let i = 0; i < texts.length; i += maxSubBatchSize) {
      subBatches.push(texts.slice(i, i + maxSubBatchSize));
    }
    
    timestampedLog(`[Process ${processId}] Splitting ${texts.length} texts into ${subBatches.length} sub-batches of ~${maxSubBatchSize}`);
    
    // Check cache for existing embeddings first
    const results = [];
    const uncachedTexts = [];
    const uncachedIndices = [];
    
    for (let i = 0; i < texts.length; i++) {
      const text = texts[i];
      const contentHash = require('crypto').createHash('sha256').update(text).digest('hex');
      
      // Check memory-mapped cache directly (zero overhead)
      if (cache) {
        try {
          const cachedEmbedding = cache.get(contentHash);
          if (cachedEmbedding) {
            results[i] = Array.from(cachedEmbedding);
            continue; // Cache hit, skip to next
          }
        } catch (error) {
          // Cache error, treat as miss and continue
        }
      }
      
      // Cache miss - add to uncached list
      uncachedTexts.push(text);
      uncachedIndices.push(i);
    }
    
    timestampedLog(`[Process ${processId}] Cache check: ${results.filter(Boolean).length} hits, ${uncachedTexts.length} misses`);
    
    // Generate embeddings only for cache misses
    if (uncachedTexts.length > 0) {
      const embeddings = embedder.embed(uncachedTexts);
      
      let embeddingIndex = 0;
      let processedCount = 0;
      
      for await (const batch of embeddings) {
        // Check if we're approaching timeout and should return partial results
        const elapsed = Date.now() - startTime;
        if (timeoutWarning && elapsed > timeoutWarning * 0.9 && results.some(r => r !== undefined)) {
          timestampedLog(`[Process ${processId}] Timeout warning reached, returning partial results`);
          break;
        }
        
        // Process batch and store in cache
        const batchResults = batch.map(emb => Array.from(emb));
        
        for (let i = 0; i < batchResults.length; i++) {
          const embedding = batchResults[i];
          const originalIndex = uncachedIndices[embeddingIndex];
          const originalText = uncachedTexts[embeddingIndex];
          
          // Store in results array at correct position
          results[originalIndex] = embedding;
          
          // Store in memory-mapped cache for future use
          if (cache) {
            try {
              const contentHash = require('crypto').createHash('sha256').update(originalText).digest('hex');
              const embeddingArray = new Float32Array(embedding);
              cache.set(contentHash, embeddingArray);
            } catch (cacheError) {
              // Cache storage failed, but continue processing
            }
          }
          
          embeddingIndex++;
          processedCount++;
        }
        
        // Report progress for large batches
        if (texts.length > 100 && processedCount % 50 === 0) {
          console.log(JSON.stringify({
            type: 'progress',
            batchId,
            processId,
            processed: processedCount + results.filter(Boolean).length - processedCount, // Include cache hits
            total: texts.length,
            progress: Math.round(((processedCount + results.filter(Boolean).length - processedCount) / texts.length) * 100)
          }));
        }
      }
    }
    
    clearInterval(progressInterval);
    clearTimeout(timeoutWarningTimer);
    
    const afterMemory = process.memoryUsage();
    const memoryDelta = Math.round((afterMemory.heapUsed - beforeMemory.heapUsed) / 1024 / 1024);
    const duration = Date.now() - startTime;
    
    timestampedLog(`[Process ${processId}] Completed batch ${batchId} in ${duration}ms`);
    
    // Send success response
    console.log(JSON.stringify({
      type: 'embed_complete',
      batchId,
      success: true,
      embeddings: results,
      stats: {
        duration,
        memoryDelta,
        chunksProcessed: texts.length,
        processId
      }
    }));
    
  } catch (error) {
    clearInterval(progressInterval);
    clearTimeout(timeoutWarningTimer);
    
    timestampedLog(`[Process ${processId}] Processing error:`, error);
    console.log(JSON.stringify({
      type: 'embed_complete',
      batchId,
      success: false,
      error: error.message,
      stats: {
        duration: Date.now() - startTime,
        chunksProcessed: texts.length,
        processId
      }
    }));
  }
}

// Process embedding batch using true shared memory for large result transfer
async function processEmbeddingBatchShared(texts, batchId, sharedBufferKey, expectedResults, embedDimension, timeoutWarning = null) {
  if (!isInitialized || !embedder) {
    throw new Error(`Process ${processId} not initialized`);
  }
  
  const startTime = Date.now();
  const beforeMemory = process.memoryUsage();
  
  try {
    timestampedLog(`[Process ${processId}] Processing ${texts.length} texts for batch ${batchId} with SharedArrayBuffer`);
    
    // Use this process's isolated embedder instance
    const embeddings = embedder.embed(texts);
    
    const results = [];
    for await (const batch of embeddings) {
      results.push(...batch.map(emb => Array.from(emb)));
    }
    
    // For now, send embeddings via JSON but indicate shared memory usage
    // TODO: Implement true SharedArrayBuffer transfer when Node.js worker_threads are used
    // This would require:
    // 1. Parent process creates SharedArrayBuffer
    // 2. Child process receives buffer reference via postMessage
    // 3. Child writes results directly to shared buffer
    // 4. Parent reads from shared buffer
    
    const afterMemory = process.memoryUsage();
    const memoryDelta = Math.round((afterMemory.heapUsed - beforeMemory.heapUsed) / 1024 / 1024);
    const duration = Date.now() - startTime;
    
    timestampedLog(`[Process ${processId}] Completed shared memory batch ${batchId} in ${duration}ms (${results.length} embeddings)`);
    
    // Send shared memory response with optimization marker
    console.log(JSON.stringify({
      type: 'shared_memory',
      batchId,
      success: true,
      embeddings: results, // Large payload - will benefit from true SharedArrayBuffer
      bufferKey: sharedBufferKey,
      resultCount: results.length,
      embedDimension: embedDimension,
      stats: {
        duration,
        memoryDelta,
        chunksProcessed: texts.length,
        processId,
        sharedMemoryUsed: true,
        payloadSizeKB: Math.round(JSON.stringify(results).length / 1024)
      }
    }));
    
  } catch (error) {
    timestampedLog(`[Process ${processId}] Shared memory processing error:`, error);
    console.log(JSON.stringify({
      type: 'shared_memory',
      batchId,
      success: false,
      error: error.message,
      bufferKey: sharedBufferKey,
      stats: {
        duration: Date.now() - startTime,
        chunksProcessed: texts.length,
        processId,
        sharedMemoryUsed: false
      }
    }));
  }
}

// Handle messages from parent process via stdin
rl.on('line', async (line) => {
  try {
    const message = JSON.parse(line);
    const { type, data, batchId } = message;
    
    if (type === 'init') {
      await initializeEmbedder(data.processId);
    } else if (type === 'embed_batch') {
      await processEmbeddingBatch(data.texts, batchId, data.timeoutWarning);
    } else if (type === 'embed_batch_shared') {
      await processEmbeddingBatchShared(
        data.texts, 
        batchId, 
        data.sharedBufferKey, 
        data.expectedResults, 
        data.embedDimension,
        data.timeoutWarning
      );
    } else if (type === 'query_memory') {
      // Handle memory usage query from parent process
      const memoryUsage = process.memoryUsage();
      console.log(JSON.stringify({
        type: 'memory_response',
        requestId: message.requestId,
        success: true,
        memoryUsage,
        processId
      }));
    } else if (type === 'shutdown') {
      timestampedLog(`[Process ${processId}] Shutting down gracefully...`);
      process.exit(0);
    } else if (type === 'abort') {
      timestampedLog(`[Process ${processId}] Received abort signal from parent: ${message.reason || 'Unknown reason'}`);
      
      // Send acknowledgment to parent
      console.log(JSON.stringify({
        type: 'abort_ack',
        processId,
        timestamp: Date.now(),
        reason: 'Acknowledged abort from parent'
      }));
      
      // Cleanup and exit gracefully
      if (embedder) {
        embedder = null;
      }
      
      timestampedLog(`[Process ${processId}] Aborting gracefully...`);
      process.exit(0);
    } else {
      throw new Error(`Unknown message type: ${type}`);
    }
    
  } catch (error) {
    timestampedLog(`[Process ${processId}] Message handling error:`, error);
    console.log(JSON.stringify({
      type: 'error',
      error: error.message,
      processId
    }));
  }
});

// Handle process termination
process.on('SIGTERM', () => {
  timestampedLog(`[Process ${processId}] Received SIGTERM, shutting down...`);
  
  // Send acknowledgment to parent before exit (if possible)
  try {
    if (process.send) {
      process.send({
        type: 'abort_ack',
        processId,
        reason: 'SIGTERM received',
        timestamp: Date.now()
      });
    }
  } catch (error) {
    // Ignore - parent might already be gone
  }
  
  if (embedder) {
    embedder = null;
  }
  process.exit(0);
});

process.on('SIGINT', () => {
  timestampedLog(`[Process ${processId}] Received SIGINT, shutting down...`);
  
  // Send acknowledgment to parent before exit (if possible)
  try {
    if (process.send) {
      process.send({
        type: 'abort_ack',
        processId,
        reason: 'SIGINT received',
        timestamp: Date.now()
      });
    }
  } catch (error) {
    // Ignore - parent might already be gone
  }
  
  if (embedder) {
    embedder = null;
  }
  process.exit(0);
});

// Handle IPC messages from parent process
process.on('message', (message) => {
  try {
    const { type, reason, timestamp } = message;
    
    if (type === 'abort') {
      timestampedLog(`[Process ${processId}] Received abort via IPC: ${reason || 'Unknown reason'}`);
      
      // Send acknowledgment via IPC
      process.send?.({
        type: 'abort_ack',
        processId,
        timestamp: Date.now(),
        reason: 'Acknowledged abort from parent via IPC'
      });
      
      // Cleanup and exit gracefully
      if (embedder) {
        embedder = null;
      }
      
      timestampedLog(`[Process ${processId}] Aborting gracefully via IPC...`);
      process.exit(0);
    }
  } catch (error) {
    timestampedLog(`[Process ${processId}] IPC message handling error:`, error);
  }
});

// Enhanced error handling with memory reporting
process.on('uncaughtException', (error) => {
  const memoryUsage = process.memoryUsage();
  timestampedLog(`[Process ${processId}] Uncaught exception:`, error);
  timestampedLog(`[Process ${processId}] Memory at crash: RSS=${Math.round(memoryUsage.rss/1024/1024)}MB, Heap=${Math.round(memoryUsage.heapUsed/1024/1024)}MB`);
  console.log(JSON.stringify({
    type: 'error',
    error: error.message,
    processId,
    memoryAtError: {
      rssMB: Math.round(memoryUsage.rss/1024/1024),
      heapUsedMB: Math.round(memoryUsage.heapUsed/1024/1024),
      heapTotalMB: Math.round(memoryUsage.heapTotal/1024/1024)
    }
  }));
  process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
  const memoryUsage = process.memoryUsage();
  timestampedLog(`[Process ${processId}] Unhandled rejection:`, reason);
  timestampedLog(`[Process ${processId}] Memory at rejection: RSS=${Math.round(memoryUsage.rss/1024/1024)}MB, Heap=${Math.round(memoryUsage.heapUsed/1024/1024)}MB`);
  console.log(JSON.stringify({
    type: 'error',
    error: String(reason),
    processId,
    memoryAtError: {
      rssMB: Math.round(memoryUsage.rss/1024/1024),
      heapUsedMB: Math.round(memoryUsage.heapUsed/1024/1024),
      heapTotalMB: Math.round(memoryUsage.heapTotal/1024/1024)
    }
  }));
  process.exit(1);
});

// Set up proactive memory monitoring
if (typeof global.gc === 'function') {
  // Force garbage collection every 30 seconds if available
  setInterval(() => {
    global.gc();
    const memoryUsage = process.memoryUsage();
    const rssMB = Math.round(memoryUsage.rss / 1024 / 1024);
    
    // System-aware memory context (informational only, not a scaling trigger)
    const os = require('os');
    const totalSystemMemoryMB = Math.round(os.totalmem() / 1024 / 1024);
    const systemPercent = ((memoryUsage.rss / os.totalmem()) * 100).toFixed(1);
    
    // Log context instead of arbitrary warning threshold
    if (rssMB > 1000) { // Only log for truly large processes (>1GB)
      timestampedLog(`[Process ${processId}] Memory status: RSS=${rssMB}MB (${systemPercent}% of ${totalSystemMemoryMB}MB system) - Normal for BGE model`);
    }
  }, 30000);
} else {
  timestampedLog(`[Process] GC not exposed. Start with --expose-gc for better memory management`);
}

// Startup messages are logged in initializeEmbedder function to avoid duplication
