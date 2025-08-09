const { parentPort } = require('worker_threads');
const { FlagEmbedding, EmbeddingModel } = require('fastembed');

let embedder = null;
let workerId = null;

async function initializeEmbedder() {
  if (!embedder) {
    console.log(`[Worker ${workerId}] Initializing BGE model with separate InferenceSession...`);
    // Option 1: Each worker creates its own InferenceSession
    embedder = await FlagEmbedding.init({
      model: EmbeddingModel.BGESmallENV15,
      maxLength: 400,
      cacheDir: './.fastembed_cache'
    });
    console.log(`[Worker ${workerId}] Model ready with dedicated InferenceSession`);
  }
  return embedder;
}

parentPort?.on('message', async (message) => {
  const { type, data, batchId } = message;
  
  if (type === 'init') {
    workerId = data.workerId;
    parentPort?.postMessage({
      type: 'init_complete',
      workerId,
      batchId
    });
    return;
  }
  
  if (type === 'embed_batch') {
    const startTime = Date.now();
    const beforeMemory = process.memoryUsage();
    
    try {
      const embedder = await initializeEmbedder();
      const embeddings = embedder.embed(data.texts);
      
      const results = [];
      for await (const batch of embeddings) {
        results.push(...batch.map(emb => Array.from(emb)));
      }
      
      const afterMemory = process.memoryUsage();
      const memoryDelta = Math.round((afterMemory.heapUsed - beforeMemory.heapUsed) / 1024 / 1024);
      const duration = Date.now() - startTime;
      
      parentPort?.postMessage({
        type: 'embed_complete',
        batchId,
        workerId,
        success: true,
        embeddings: results,
        stats: {
          duration,
          memoryDelta,
          chunksProcessed: data.texts.length
        }
      });
      
    } catch (error) {
      parentPort?.postMessage({
        type: 'embed_complete',
        batchId,
        workerId,
        success: false,
        error: error.message,
        stats: {
          duration: Date.now() - startTime,
          chunksProcessed: data.texts.length
        }
      });
    }
  }
});