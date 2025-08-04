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
    console.error(`[Process ${processId}] Starting separate Node.js instance...`);
    
    // Dynamic import in separate process - complete isolation
    const { FlagEmbedding, EmbeddingModel } = await import('fastembed');
    
    console.error(`[Process ${processId}] Creating isolated FastEmbedding instance...`);
    
    // Each process gets its own complete BGE instance
    embedder = await FlagEmbedding.init({
      model: EmbeddingModel.BGESmallENV15,
      maxLength: 512,
      cacheDir: './.fastembed_cache'
    });
    
    isInitialized = true;
    console.error(`[Process ${processId}] FastEmbedding ready in separate process`);
    
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

// Process embedding batch in this separate process
async function processEmbeddingBatch(texts, batchId) {
  if (!isInitialized || !embedder) {
    throw new Error(`Process ${processId} not initialized`);
  }
  
  const startTime = Date.now();
  const beforeMemory = process.memoryUsage();
  
  try {
    console.error(`[Process ${processId}] Processing ${texts.length} texts for batch ${batchId}`);
    
    // Use this process's isolated embedder instance
    // No concurrency issues - each process is completely separate
    const embeddings = embedder.embed(texts);
    
    const results = [];
    for await (const batch of embeddings) {
      results.push(...batch.map(emb => Array.from(emb)));
    }
    
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

// Handle messages from parent process via stdin
rl.on('line', async (line) => {
  try {
    const message = JSON.parse(line);
    const { type, data, batchId } = message;
    
    if (type === 'init') {
      await initializeEmbedder(data.processId);
    } else if (type === 'embed_batch') {
      await processEmbeddingBatch(data.texts, batchId);
    } else if (type === 'shutdown') {
      console.error(`[Process ${processId}] Shutting down gracefully...`);
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
  if (embedder) {
    embedder = null;
  }
  process.exit(0);
});

process.on('SIGINT', () => {
  console.error(`[Process ${processId}] Received SIGINT, shutting down...`);
  if (embedder) {
    embedder = null;
  }
  process.exit(0);
});

// Handle uncaught errors
process.on('uncaughtException', (error) => {
  console.error(`[Process ${processId}] Uncaught exception:`, error);
  console.log(JSON.stringify({
    type: 'error',
    error: error.message,
    processId
  }));
  process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
  console.error(`[Process ${processId}] Unhandled rejection:`, reason);
  console.log(JSON.stringify({
    type: 'error',
    error: String(reason),
    processId
  }));
  process.exit(1);
});

console.error(`[Process] External embedding process started, waiting for init...`);
