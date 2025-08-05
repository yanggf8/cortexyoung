#!/usr/bin/env node

/**
 * Precise Benchmark Comparison
 * 
 * Provides accurate benchmarking with:
 * 1. Real wall-clock duration (not aggregated batch times)
 * 2. Per-worker throughput for ProcessPool
 * 3. Total system throughput
 * 4. True concurrency validation
 */

const { EmbeddingStrategyManager } = require('./dist/embedding-strategy');
const { GitScanner } = require('./dist/git-scanner');
const { SmartChunker } = require('./dist/chunker');

async function runPreciseBenchmarks() {
  console.log('⏱️  Precise Strategy Benchmarking');
  console.log('=' .repeat(70));
  console.log('🎯 Goal: Accurate performance measurement with wall-clock timing');
  console.log('📊 Testing: ProcessPool vs Concurrent with precise metrics');
  console.log('');

  try {
    // Prepare consistent test data
    console.log('📁 Preparing test chunks...');
    const gitScanner = new GitScanner('.');
    const scanResult = await gitScanner.scanRepository('full');
    const files = scanResult.files;
    
    const codeFiles = files.filter(filePath => 
      filePath.match(/\.(js|ts|jsx|tsx)$/) && 
      !filePath.includes('node_modules') &&
      !filePath.includes('.git')
    );
    
    const chunker = new SmartChunker();
    let allChunks = [];
    
    for (const filePath of codeFiles.slice(0, 20)) { // More files for better testing
      try {
        const content = await gitScanner.readFile(filePath);
        const fileChunks = await chunker.chunkFile(filePath, content);
        allChunks.push(...fileChunks);
        
        if (allChunks.length >= 200) break; // Larger test set
      } catch (error) {
        console.warn(`Failed to chunk ${filePath}:`, error.message);
      }
    }
    
    const testChunks = allChunks.slice(0, 200);
    console.log(`📦 Using ${testChunks.length} chunks for precise benchmarking`);
    console.log('');

    // Test configurations
    const testConfigs = [
      {
        name: 'ProcessPool (10 workers)',
        config: { strategy: 'process-pool', processCount: 10 },
        description: 'True parallel processing with external processes'
      },
      {
        name: 'ProcessPool (5 workers)',
        config: { strategy: 'process-pool', processCount: 5 },
        description: 'Medium parallelism with external processes'
      },
      {
        name: 'ProcessPool (2 workers)',
        config: { strategy: 'process-pool', processCount: 2 },
        description: 'Low parallelism with external processes'
      },
      {
        name: 'Optimized Concurrent',
        config: { strategy: 'concurrent' }, // Uses auto-tuning
        description: 'Promise.all with shared model and auto-tuning'
      },
      {
        name: 'Manual Concurrent (5)',
        config: { strategy: 'concurrent', concurrency: 5 },
        description: 'Promise.all with manual concurrency=5'
      }
    ];

    const results = [];
    
    for (const testConfig of testConfigs) {
      console.log(`🧪 Testing: ${testConfig.name}`);
      console.log(`📝 ${testConfig.description}`);
      console.log('-'.repeat(60));
      
      const strategyManager = new EmbeddingStrategyManager();
      
      // Precise wall-clock timing
      const testStartTime = process.hrtime.bigint();
      
      try {
        const result = await strategyManager.generateEmbeddings(testChunks, testConfig.config);
        
        const testEndTime = process.hrtime.bigint();
        const testWallClockMs = Number(testEndTime - testStartTime) / 1_000_000;
        
        const benchmarkResult = {
          name: testConfig.name,
          strategy: result.strategy,
          
          // Timing metrics
          reportedTime: result.performance.totalTime,
          wallClockTime: testWallClockMs,
          
          // Throughput metrics  
          reportedThroughput: result.performance.chunksPerSecond,
          wallClockThroughput: testChunks.length / (testWallClockMs / 1000),
          
          // System metrics
          peakMemory: result.performance.peakMemoryMB,
          batches: result.performance.totalBatches,
          avgBatchTime: result.performance.averageBatchTime,
          
          // Validation
          chunksProcessed: result.chunks.length,
          success: result.chunks.length === testChunks.length
        };
        
        results.push(benchmarkResult);
        
        console.log('');
        console.log('📊 **PRECISE RESULTS**:');
        console.log(`  ⏱️  Wall-clock duration: ${benchmarkResult.wallClockTime.toFixed(0)}ms`);
        console.log(`  ⏱️  Reported duration: ${benchmarkResult.reportedTime}ms`);
        console.log(`  📈 Wall-clock throughput: ${benchmarkResult.wallClockThroughput.toFixed(2)} chunks/second`);
        console.log(`  📈 Reported throughput: ${benchmarkResult.reportedThroughput.toFixed(2)} chunks/second`);
        console.log(`  💾 Peak memory: ${benchmarkResult.peakMemory.toFixed(1)}MB`);
        console.log(`  📦 Batches: ${benchmarkResult.batches}`);
        console.log(`  ✅ Chunks processed: ${benchmarkResult.chunksProcessed}/${testChunks.length}`);
        
        const timingDifference = Math.abs(benchmarkResult.wallClockTime - benchmarkResult.reportedTime);
        if (timingDifference > 1000) { // More than 1 second difference
          console.log(`  ⚠️  Timing discrepancy: ${timingDifference.toFixed(0)}ms difference`);
        }
        
        console.log('');
        
        // Cleanup
        await strategyManager.cleanup();
        
        // Wait between tests to ensure clean state
        console.log('⏳ Cooling down between tests...');
        await new Promise(resolve => setTimeout(resolve, 5000));
        
      } catch (error) {
        console.error(`  ❌ Test failed for ${testConfig.name}:`, error.message);
        console.log('');
      }
    }
    
    if (results.length === 0) {
      console.log('❌ No successful tests to analyze');
      return false;
    }
    
    // Comprehensive Analysis
    console.log('📊 COMPREHENSIVE BENCHMARK ANALYSIS');
    console.log('='.repeat(70));
    console.log('');
    
    // Sort by wall-clock throughput (most accurate metric)
    results.sort((a, b) => b.wallClockThroughput - a.wallClockThroughput);
    
    console.log('🏆 **PERFORMANCE RANKING** (by wall-clock throughput):');
    results.forEach((result, index) => {
      const medal = index === 0 ? '🥇' : index === 1 ? '🥈' : index === 2 ? '🥉' : `${index + 1}.`;
      console.log(`  ${medal} ${result.name}: ${result.wallClockThroughput.toFixed(2)} chunks/s (${result.wallClockTime.toFixed(0)}ms)`);
    });
    console.log('');
    
    // Detailed comparison
    const best = results[0];
    console.log('🔍 **DETAILED ANALYSIS**:');
    console.log('');
    
    console.log(`**Best Performer: ${best.name}**`);
    console.log(`  • Wall-clock throughput: ${best.wallClockThroughput.toFixed(2)} chunks/second`);
    console.log(`  • Wall-clock duration: ${best.wallClockTime.toFixed(0)}ms (${(best.wallClockTime/1000).toFixed(1)}s)`);
    console.log(`  • Memory efficiency: ${best.peakMemory.toFixed(1)}MB peak`);
    console.log(`  • Processing batches: ${best.batches}`);
    console.log('');
    
    // ProcessPool analysis (if any ProcessPool results exist)
    const processPoolResults = results.filter(r => r.strategy === 'process-pool');
    if (processPoolResults.length > 0) {
      console.log('🏭 **PROCESSPOOL SCALABILITY**:');
      processPoolResults.forEach(result => {
        // Extract worker count from name
        const workerMatch = result.name.match(/(\d+) workers?/);
        const workerCount = workerMatch ? parseInt(workerMatch[1]) : 'unknown';
        const efficiencyPerWorker = result.wallClockThroughput / (typeof workerCount === 'number' ? workerCount : 1);
        
        console.log(`  • ${workerCount} workers: ${result.wallClockThroughput.toFixed(2)} chunks/s total`);
        if (typeof workerCount === 'number') {
          console.log(`    - Per-worker: ${efficiencyPerWorker.toFixed(2)} chunks/s/worker`);
          console.log(`    - Parallel efficiency: ${((result.wallClockThroughput / workerCount) / (processPoolResults[processPoolResults.length-1].wallClockThroughput / 2) * 100).toFixed(1)}%`);
        }
      });
      console.log('');
    }
    
    // Concurrent analysis
    const concurrentResults = results.filter(r => r.strategy === 'concurrent');
    if (concurrentResults.length > 0) {
      console.log('🔄 **CONCURRENT STRATEGY ANALYSIS**:');
      concurrentResults.forEach(result => {
        const timingAccuracy = Math.abs(result.wallClockTime - result.reportedTime) / result.wallClockTime * 100;
        console.log(`  • ${result.name}:`);
        console.log(`    - Throughput: ${result.wallClockThroughput.toFixed(2)} chunks/s`);
        console.log(`    - Timing accuracy: ${(100 - timingAccuracy).toFixed(1)}% (${timingAccuracy.toFixed(1)}% error)`);
        console.log(`    - Memory usage: ${result.peakMemory.toFixed(1)}MB`);
      });
      console.log('');
    }
    
    // Strategy comparison
    console.log('⚔️  **STRATEGY COMPARISON**:');
    const bestProcessPool = processPoolResults[0];
    const bestConcurrent = concurrentResults[0];
    
    if (bestProcessPool && bestConcurrent) {
      const speedRatio = bestProcessPool.wallClockThroughput / bestConcurrent.wallClockThroughput;
      const memoryRatio = bestConcurrent.peakMemory / bestProcessPool.peakMemory;
      
      console.log(`  ProcessPool vs Concurrent:`);
      console.log(`    • Speed: ProcessPool is ${speedRatio.toFixed(2)}x ${speedRatio > 1 ? 'faster' : 'slower'}`);
      console.log(`    • Memory: Concurrent uses ${memoryRatio.toFixed(2)}x ${memoryRatio > 1 ? 'more' : 'less'} memory`);
      console.log(`    • Complexity: ProcessPool higher (external processes)`);
      console.log(`    • Reliability: ProcessPool higher (process isolation)`);
    }
    
    console.log('');
    console.log('💡 **KEY INSIGHTS**:');
    console.log('');
    
    // Performance insights
    if (results[0].wallClockThroughput > 10) {
      console.log('✅ Excellent performance: >10 chunks/second achieved');
    } else if (results[0].wallClockThroughput > 5) {
      console.log('✅ Good performance: >5 chunks/second achieved');
    } else {
      console.log('⚠️  Performance below target: <5 chunks/second');
    }
    
    // Timing accuracy
    const avgTimingError = results.reduce((sum, r) => 
      sum + Math.abs(r.wallClockTime - r.reportedTime) / r.wallClockTime, 0) / results.length * 100;
    
    if (avgTimingError < 5) {
      console.log('✅ Accurate timing: <5% average error in reported metrics');
    } else {
      console.log(`⚠️  Timing inaccuracy: ${avgTimingError.toFixed(1)}% average error - use wall-clock metrics`);
    }
    
    // Memory efficiency
    const avgMemoryUsage = results.reduce((sum, r) => sum + r.peakMemory, 0) / results.length;
    if (avgMemoryUsage < 50) {
      console.log(`✅ Memory efficient: ${avgMemoryUsage.toFixed(1)}MB average usage`);
    } else {
      console.log(`⚠️  High memory usage: ${avgMemoryUsage.toFixed(1)}MB average`);
    }
    
    console.log('');
    console.log(`🎯 **RECOMMENDATION**: Use ${best.name} for optimal performance`);
    console.log(`   Wall-clock throughput: ${best.wallClockThroughput.toFixed(2)} chunks/second`);
    
    return best.wallClockThroughput > 5.0;
    
  } catch (error) {
    console.error('❌ Benchmark failed:', error.message);
    console.error('Stack trace:', error.stack);
    return false;
  }
}

// Run the benchmarks
if (require.main === module) {
  runPreciseBenchmarks()
    .then(success => {
      process.exit(success ? 0 : 1);
    })
    .catch(error => {
      console.error('Fatal error:', error);
      process.exit(1);
    });
}

module.exports = { runPreciseBenchmarks };