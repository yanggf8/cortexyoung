// CRITICAL: Each worker gets its own complete FastEmbedding instance + Mutex
// Mutex ensures only one embedding operation per worker at a time
// Complete isolation prevents ONNX thread safety issues

const { parentPort } = require('worker_threads');

// Worker-local variables - completely isolated per worker
let embedder = null;
let workerId = null;
let isInitialized = false;
let workerMutex = null;

// Import both FastEmbedding and Mutex in worker scope for complete isolation
async function createIsolatedEmbedder() {
  try {
    // Dynamic imports to ensure fresh module instances per worker
    const { FlagEmbedding, EmbeddingModel } = await import('fastembed');
    const { Mutex } = await import('async-mutex');
    
    console.log(`[Worker ${workerId}] Creating isolated FastEmbedding + Mutex...`);
    
    // Create worker-specific mutex to prevent concurrent embedding calls
    workerMutex = new Mutex();
    
    // Each worker creates its own complete BGE instance
    // This includes separate ONNX InferenceSession, avoiding thread safety issues
    const isolatedEmbedder = await FlagEmbedding.init({
      model: EmbeddingModel.BGESmallENV15,
      maxLength: 400,
      cacheDir: './.fastembed_cache'  // Shared cache for model files, separate sessions
    });
    
    console.log(`[Worker ${workerId}] Isolated FastEmbedding + Mutex ready`);
    return isolatedEmbedder;
    
  } catch (error) {
    console.error(`[Worker ${workerId}] Failed to create isolated embedder:`, error);
    throw error;
  }
}

// Initialize worker when requested
async function initializeWorker(id) {
  try {
    workerId = id;
    console.log(`[Worker ${workerId}] Initializing with complete isolation + mutex...`);
    
    // Create completely isolated embedder instance with mutex
    embedder = await createIsolatedEmbedder();
    isInitialized = true;
    
    parentPort?.postMessage({
      type: 'init_complete',
      workerId,
      success: true
    });
    
  } catch (error) {
    console.error(`[Worker ${workerId}] Initialization failed:`, error);
    parentPort?.postMessage({
      type: 'init_complete',
      workerId,
      success: false,
      error: error.message
    });
  }
}

// Process embedding batch with mutex protection
// This ensures sequential processing even in async environment
async function processEmbeddingBatch(texts, batchId) {
  if (!isInitialized || !embedder || !workerMutex) {
    throw new Error(`Worker ${workerId} not properly initialized`);
  }
  
  // CRITICAL: Use mutex to ensure only one embedding operation at a time
  // This blocks newer consumer instances until current one completes
  return await workerMutex.runExclusive(async () => {
    const startTime = Date.now();
    const beforeMemory = process.memoryUsage();
    
    try {
      console.log(`[Worker ${workerId}] 🔒 Mutex acquired - processing ${texts.length} texts for batch ${batchId}`);
      
      // Use this worker's isolated embedder instance
      // Protected by mutex - no concurrent access possible
      // Each worker has its own ONNX InferenceSession
      const embeddings = embedder.embed(texts);
      
      const results = [];
      for await (const batch of embeddings) {
        results.push(...batch.map(emb => Array.from(emb)));
      }
      
      const afterMemory = process.memoryUsage();
      const memoryDelta = Math.round((afterMemory.heapUsed - beforeMemory.heapUsed) / 1024 / 1024);
      const duration = Date.now() - startTime;
      
      console.log(`[Worker ${workerId}] ✅ Completed batch ${batchId} in ${duration}ms - mutex released`);
      
      return {
        embeddings: results,
        stats: {
          duration,
          memoryDelta,
          chunksProcessed: texts.length,
          workerId
        }
      };
      
    } catch (error) {
      console.error(`[Worker ${workerId}] ❌ Processing error in mutex:`, error);
      throw error;
    }
    // Mutex automatically released here by runExclusive
  });
}

// Message handler - clean isolation with mutex protection
parentPort?.on('message', async (message) => {
  const { type, data, batchId } = message;
  
  try {
    if (type === 'init') {
      await initializeWorker(data.workerId);
      return;
    }
    
    if (type === 'embed_batch') {
      // This call is protected by mutex - will block if another task is running
      // Ensures sequential processing per worker even in async FastQ environment
      const result = await processEmbeddingBatch(data.texts, batchId);
      
      parentPort?.postMessage({
        type: 'embed_complete',
        batchId,
        success: true,
        ...result
      });
      
    } else {
      throw new Error(`Unknown message type: ${type}`);
    }
    
  } catch (error) {
    parentPort?.postMessage({
      type: 'embed_complete',
      batchId,
      success: false,
      error: error.message,
      stats: {
        duration: 0,
        chunksProcessed: 0,
        workerId
      }
    });
  }
});

// Graceful shutdown
process.on('SIGTERM', () => {
  console.log(`[Worker ${workerId}] Shutting down gracefully...`);
  if (embedder) {
    embedder = null;
  }
  if (workerMutex) {
    workerMutex = null;
  }
  process.exit(0);
});

// Handle uncaught errors
process.on('uncaughtException', (error) => {
  console.error(`[Worker ${workerId}] Uncaught exception:`, error);
  process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
  console.error(`[Worker ${workerId}] Unhandled rejection at:`, promise, 'reason:', reason);
  process.exit(1);
});
