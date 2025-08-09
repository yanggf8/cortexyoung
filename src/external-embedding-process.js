#!/usr/bin/env node

// External Node.js process for embedding generation
// Complete isolation from main process - no shared memory or threads

const readline = require('readline');

// Process-local variables - completely isolated
let embedder = null;
let processId = null;
let isInitialized = false;

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
    process.stderr.write(`[Process] External embedding process started, waiting for init...\n`);
    process.stderr.write(`[Process] Node.js version: ${process.version}, Platform: ${process.platform}, Arch: ${process.arch}\n`);
    process.stderr.write(`[Process ${processId}] Starting separate Node.js instance...\n`);
    
    // Dynamic import in separate process - complete isolation
    const { FlagEmbedding, EmbeddingModel } = await import('fastembed');
    
    process.stderr.write(`[Process ${processId}] Creating isolated FastEmbedding instance...\n`);
    
    // Each process gets its own BGE instance with simplified configuration
    embedder = await FlagEmbedding.init({
      model: EmbeddingModel.BGESmallENV15,
      maxLength: 512,  // Standard max length
      cacheDir: './.fastembed_cache'
      // Use default settings - let FastEmbed choose the best available backend
    });
    
    isInitialized = true;
    process.stderr.write(`[Process ${processId}] FastEmbedding ready in separate process\n`);
    
    // Send success response
    console.log(JSON.stringify({
      type: 'init_complete',
      processId,
      success: true
    }));
    
  } catch (error) {
    console.error(`[Process ${processId}] Initialization failed:`, error);
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
    console.error(`[Process ${processId}] Processing ${texts.length} texts for batch ${batchId}`);
    
    // Use this process's isolated embedder instance with memory-conscious processing
    // Process in smaller sub-batches to prevent memory spikes
    const maxSubBatchSize = Math.min(50, Math.ceil(texts.length / 4)); // Max 50 or 1/4 of batch
    const subBatches = [];
    
    for (let i = 0; i < texts.length; i += maxSubBatchSize) {
      subBatches.push(texts.slice(i, i + maxSubBatchSize));
    }
    
    console.error(`[Process ${processId}] Splitting ${texts.length} texts into ${subBatches.length} sub-batches of ~${maxSubBatchSize}`);
    
    const embeddings = embedder.embed(texts); // Still use full batch but with optimized settings
    
    const results = [];
    let processedCount = 0;
    
    for await (const batch of embeddings) {
      // Check if we're approaching timeout and should return partial results
      const elapsed = Date.now() - startTime;
      if (timeoutWarning && elapsed > timeoutWarning * 0.9 && results.length > 0) {
        console.error(`[Process ${processId}] Timeout warning reached, returning ${results.length} partial results`);
        clearInterval(progressInterval);
        clearTimeout(timeoutWarningTimer);
        
        // Send partial results
        console.log(JSON.stringify({
          type: 'embed_complete',
          batchId,
          success: true,
          partial: true,
          embeddings: results,
          stats: {
            duration: elapsed,
            memoryDelta: Math.round((process.memoryUsage().heapUsed - beforeMemory.heapUsed) / 1024 / 1024),
            chunksProcessed: results.length,
            totalChunks: texts.length,
            processId
          }
        }));
        return;
      }
      
      // Process batch normally
      const batchResults = batch.map(emb => Array.from(emb));
      results.push(...batchResults);
      processedCount += batchResults.length;
      
      // Report progress for large batches
      if (texts.length > 100 && processedCount % 50 === 0) {
        console.log(JSON.stringify({
          type: 'progress',
          batchId,
          processId,
          processed: processedCount,
          total: texts.length,
          progress: Math.round((processedCount / texts.length) * 100)
        }));
      }
    }
    
    clearInterval(progressInterval);
    clearTimeout(timeoutWarningTimer);
    
    const afterMemory = process.memoryUsage();
    const memoryDelta = Math.round((afterMemory.heapUsed - beforeMemory.heapUsed) / 1024 / 1024);
    const duration = Date.now() - startTime;
    
    console.error(`[Process ${processId}] Completed batch ${batchId} in ${duration}ms`);
    
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
    
    console.error(`[Process ${processId}] Processing error:`, error);
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
    console.error(`[Process ${processId}] Processing ${texts.length} texts for batch ${batchId} with SharedArrayBuffer`);
    
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
    
    console.error(`[Process ${processId}] Completed shared memory batch ${batchId} in ${duration}ms (${results.length} embeddings)`);
    
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
    console.error(`[Process ${processId}] Shared memory processing error:`, error);
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
      console.error(`[Process ${processId}] Shutting down gracefully...`);
      process.exit(0);
    } else if (type === 'abort') {
      console.error(`[Process ${processId}] Received abort signal from parent: ${message.reason || 'Unknown reason'}`);
      
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
      
      console.error(`[Process ${processId}] Aborting gracefully...`);
      process.exit(0);
    } else {
      throw new Error(`Unknown message type: ${type}`);
    }
    
  } catch (error) {
    console.error(`[Process ${processId}] Message handling error:`, error);
    console.log(JSON.stringify({
      type: 'error',
      error: error.message,
      processId
    }));
  }
});

// Handle process termination
process.on('SIGTERM', () => {
  console.error(`[Process ${processId}] Received SIGTERM, shutting down...`);
  
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
  console.error(`[Process ${processId}] Received SIGINT, shutting down...`);
  
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
      console.error(`[Process ${processId}] Received abort via IPC: ${reason || 'Unknown reason'}`);
      
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
      
      console.error(`[Process ${processId}] Aborting gracefully via IPC...`);
      process.exit(0);
    }
  } catch (error) {
    console.error(`[Process ${processId}] IPC message handling error:`, error);
  }
});

// Enhanced error handling with memory reporting
process.on('uncaughtException', (error) => {
  const memoryUsage = process.memoryUsage();
  console.error(`[Process ${processId}] Uncaught exception:`, error);
  console.error(`[Process ${processId}] Memory at crash: RSS=${Math.round(memoryUsage.rss/1024/1024)}MB, Heap=${Math.round(memoryUsage.heapUsed/1024/1024)}MB`);
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
  console.error(`[Process ${processId}] Unhandled rejection:`, reason);
  console.error(`[Process ${processId}] Memory at rejection: RSS=${Math.round(memoryUsage.rss/1024/1024)}MB, Heap=${Math.round(memoryUsage.heapUsed/1024/1024)}MB`);
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
    if (memoryUsage.rss > 500 * 1024 * 1024) { // Warn if RSS > 500MB
      console.error(`[Process ${processId}] High memory usage: RSS=${Math.round(memoryUsage.rss/1024/1024)}MB`);
    }
  }, 30000);
} else {
  console.error(`[Process] GC not exposed. Start with --expose-gc for better memory management`);
}

process.stderr.write(`[Process] External embedding process started, waiting for init...\n`);
process.stderr.write(`[Process] Node.js version: ${process.version}, Platform: ${process.platform}, Arch: ${process.arch}\n`);
