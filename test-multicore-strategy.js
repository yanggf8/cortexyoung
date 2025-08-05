const os = require('os');

// Test our multi-core utilization strategy
function calculateOptimalConcurrency(cpuCores, memoryGB, totalBatches, batchSize) {
  console.log(`🧠 System Resources: ${cpuCores} CPU cores, ${memoryGB.toFixed(1)}GB RAM`);
  
  // 1. CPU-based limit: Reserve 1-2 cores for system
  const cpuLimit = Math.max(1, cpuCores - 2);
  console.log(`🔧 CPU Limit: ${cpuLimit} (reserved 2 cores for system)`);
  
  // 2. Memory-based limit: Estimate ~80MB per batch (conservative)
  const estimatedMemoryPerBatch = (batchSize * 384 * 4) / (1024 * 1024); // 384D float32 vectors
  const memoryLimit = Math.max(1, Math.floor((memoryGB * 0.5 * 1024) / estimatedMemoryPerBatch)); // Use 50% of RAM
  console.log(`🔧 Memory Limit: ${memoryLimit} (${estimatedMemoryPerBatch.toFixed(1)}MB per batch, using 50% RAM)`);
  
  // 3. BGE model consideration: fastembed already uses internal threading
  const modelLimit = Math.min(4, cpuCores); // BGE works well with 2-4 concurrent instances max
  console.log(`🔧 Model Limit: ${modelLimit} (BGE optimal concurrency)`);
  
  // 4. Batch count limit: Don't create more concurrent processes than batches
  const batchLimit = Math.min(totalBatches, 6); // Never more than 6 concurrent
  console.log(`🔧 Batch Limit: ${batchLimit} (total batches: ${totalBatches})`);
  
  const optimalConcurrency = Math.min(cpuLimit, memoryLimit, modelLimit, batchLimit);
  console.log(`\n✅ Optimal Concurrency: ${optimalConcurrency}`);
  
  return optimalConcurrency;
}

// Test different scenarios
console.log('='.repeat(60));
console.log('MULTI-CORE UTILIZATION STRATEGY TEST');
console.log('='.repeat(60));

const realCpuCores = os.cpus().length;
const realMemoryGB = os.totalmem() / (1024 * 1024 * 1024);

console.log('\n🔍 Current System:');
calculateOptimalConcurrency(realCpuCores, realMemoryGB, 3, 800); // Our typical scenario

console.log('\n🔍 Low-end System (4 cores, 8GB):');
calculateOptimalConcurrency(4, 8, 3, 800);

console.log('\n🔍 High-end System (16 cores, 32GB):');
calculateOptimalConcurrency(16, 32, 3, 800);

console.log('\n🔍 Many Small Batches (8 cores, 16GB, 10 batches):');
calculateOptimalConcurrency(8, 16, 10, 200);

console.log('\n📊 Key Principles:');
console.log('- Never use all CPU cores (reserve for system)');
console.log('- Limit memory usage to 50% of available RAM');
console.log('- BGE model has internal threading, avoid over-parallelization'); 
console.log('- Monitor memory deltas per batch to detect issues');
console.log('- Throttle when concurrency limit reached');