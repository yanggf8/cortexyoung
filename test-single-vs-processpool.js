#!/usr/bin/env node

/**
 * Single Process Concurrent vs ProcessPool Comparison
 * 
 * Direct wall-clock comparison between:
 * - Single-process concurrent (Promise.all with shared model)
 * - ProcessPool (multiple external processes)
 */

const { EmbeddingStrategyManager } = require('./dist/embedding-strategy');
const { GitScanner } = require('./dist/git-scanner');
const { SmartChunker } = require('./dist/chunker');

async function compareSingleVsProcessPool() {
  console.log('⚔️  Single Process Concurrent vs ProcessPool Comparison');
  console.log('=' .repeat(70));
  console.log('🎯 Goal: Direct wall-clock performance comparison');
  console.log('📊 Testing: Same dataset, precise timing');
  console.log('');

  try {
    // Prepare test data
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
    
    for (const filePath of codeFiles.slice(0, 20)) {
      try {
        const content = await gitScanner.readFile(filePath);
        const fileChunks = await chunker.chunkFile(filePath, content);
        allChunks.push(...fileChunks);
        
        if (allChunks.length >= 100) break; // Smaller test for faster comparison
      } catch (error) {
        console.warn(`Failed to chunk ${filePath}:`, error.message);
      }
    }
    
    const testChunks = allChunks.slice(0, 100);
    console.log(`📦 Using ${testChunks.length} chunks for comparison`);
    console.log('');

    // Test configurations - focused comparison
    const testConfigs = [
      {
        name: 'Single Process Concurrent (Auto)',
        config: { strategy: 'concurrent' },
        description: 'Promise.all with shared BGE model, auto-tuned concurrency'
      },
      {
        name: 'ProcessPool (2 workers)',
        config: { strategy: 'process-pool', processCount: 2 },
        description: 'External processes with isolated BGE models'
      },
      {
        name: 'ProcessPool (5 workers)',
        config: { strategy: 'process-pool', processCount: 5 },
        description: 'External processes with isolated BGE models'
      }
    ];

    const results = [];
    
    for (const testConfig of testConfigs) {
      console.log(`🧪 Testing: ${testConfig.name}`);
      console.log(`📝 ${testConfig.description}`);
      console.log('-'.repeat(60));
      
      // Force garbage collection before each test
      if (global.gc) {
        global.gc();
      }
      
      const strategyManager = new EmbeddingStrategyManager();
      
      // Precise wall-clock timing
      const testStartTime = process.hrtime.bigint();
      const startMemory = process.memoryUsage();
      
      try {
        const result = await strategyManager.generateEmbeddings(testChunks, testConfig.config);
        
        const testEndTime = process.hrtime.bigint();
        const endMemory = process.memoryUsage();
        const testWallClockMs = Number(testEndTime - testStartTime) / 1_000_000;
        
        const benchmarkResult = {
          name: testConfig.name,
          strategy: result.strategy,
          config: testConfig.config,
          
          // Timing metrics (wall-clock is ground truth)
          wallClockTime: testWallClockMs,
          reportedTime: result.performance.totalTime,
          timingAccuracy: Math.abs(testWallClockMs - result.performance.totalTime) / testWallClockMs * 100,
          
          // Throughput metrics  
          wallClockThroughput: testChunks.length / (testWallClockMs / 1000),
          reportedThroughput: result.performance.chunksPerSecond,
          
          // Resource metrics
          peakMemory: result.performance.peakMemoryMB,
          memoryDelta: (endMemory.heapUsed - startMemory.heapUsed) / 1024 / 1024,
          batches: result.performance.totalBatches,
          
          // Validation
          chunksProcessed: result.chunks.length,
          success: result.chunks.length === testChunks.length
        };
        
        results.push(benchmarkResult);
        
        console.log('');
        console.log('📊 **WALL-CLOCK RESULTS**:');
        console.log(`  ⏱️  Wall-clock duration: ${benchmarkResult.wallClockTime.toFixed(0)}ms (${(benchmarkResult.wallClockTime/1000).toFixed(1)}s)`);
        console.log(`  📈 Wall-clock throughput: ${benchmarkResult.wallClockThroughput.toFixed(2)} chunks/second`);
        console.log(`  💾 Memory delta: ${benchmarkResult.memoryDelta.toFixed(1)}MB`);
        console.log(`  📦 Batches: ${benchmarkResult.batches}`);
        console.log(`  ✅ Success: ${benchmarkResult.success}`);
        
        if (benchmarkResult.timingAccuracy > 5) {
          console.log(`  ⚠️  Timing discrepancy: ${benchmarkResult.timingAccuracy.toFixed(1)}% error`);
        } else {
          console.log(`  ✅ Timing accuracy: ${benchmarkResult.timingAccuracy.toFixed(1)}% error`);
        }
        
        console.log('');
        
        // Cleanup
        await strategyManager.cleanup();
        
        // Cooling period between tests
        if (testConfig.config.strategy === 'process-pool') {
          console.log('⏳ Extended cooling for ProcessPool cleanup...');
          await new Promise(resolve => setTimeout(resolve, 8000));
        } else {
          console.log('⏳ Cooling down...');
          await new Promise(resolve => setTimeout(resolve, 3000));
        }
        
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
    console.log('📊 SINGLE PROCESS vs PROCESSPOOL ANALYSIS');
    console.log('='.repeat(70));
    console.log('');
    
    // Sort by wall-clock throughput
    results.sort((a, b) => b.wallClockThroughput - a.wallClockThroughput);
    
    console.log('🏆 **PERFORMANCE RANKING** (by wall-clock throughput):');
    results.forEach((result, index) => {
      const medal = index === 0 ? '🥇' : index === 1 ? '🥈' : index === 2 ? '🥉' : `${index + 1}.`;
      const duration = (result.wallClockTime / 1000).toFixed(1);
      console.log(`  ${medal} ${result.name}: ${result.wallClockThroughput.toFixed(2)} chunks/s (${duration}s)`);
    });
    console.log('');
    
    // Strategy-specific analysis
    const concurrentResults = results.filter(r => r.strategy === 'concurrent');
    const processPoolResults = results.filter(r => r.strategy === 'process-pool');
    
    console.log('🔍 **STRATEGY ANALYSIS**:');
    console.log('');
    
    if (concurrentResults.length > 0) {
      const bestConcurrent = concurrentResults[0];
      console.log('**Single Process Concurrent (Best):**');
      console.log(`  Strategy: ${bestConcurrent.name}`);
      console.log(`  Throughput: ${bestConcurrent.wallClockThroughput.toFixed(2)} chunks/second`);
      console.log(`  Duration: ${(bestConcurrent.wallClockTime/1000).toFixed(1)}s`);
      console.log(`  Memory usage: ${bestConcurrent.memoryDelta.toFixed(1)}MB delta`);
      console.log(`  Architecture: Single Node.js process, shared BGE model`);
      console.log(`  Concurrency: Promise.all with ${bestConcurrent.config.concurrency || 'auto-tuned'} concurrent operations`);
      console.log('');
    }
    
    if (processPoolResults.length > 0) {
      const bestProcessPool = processPoolResults[0];
      console.log('**ProcessPool (Best):**');
      console.log(`  Strategy: ${bestProcessPool.name}`);
      console.log(`  Throughput: ${bestProcessPool.wallClockThroughput.toFixed(2)} chunks/second`);
      console.log(`  Duration: ${(bestProcessPool.wallClockTime/1000).toFixed(1)}s`);
      console.log(`  Memory usage: ${bestProcessPool.memoryDelta.toFixed(1)}MB delta`);
      console.log(`  Architecture: ${bestProcessPool.config.processCount} external Node.js processes`);
      console.log(`  Isolation: Complete process separation with individual BGE models`);
      console.log('');
    }
    
    // Head-to-head comparison
    if (concurrentResults.length > 0 && processPoolResults.length > 0) {
      const bestConcurrent = concurrentResults[0];
      const bestProcessPool = processPoolResults[0];
      
      console.log('⚔️  **HEAD-TO-HEAD COMPARISON**:');
      console.log('');
      
      const speedRatio = bestConcurrent.wallClockThroughput / bestProcessPool.wallClockThroughput;
      const timeRatio = bestProcessPool.wallClockTime / bestConcurrent.wallClockTime;
      const memoryRatio = bestProcessPool.memoryDelta / bestConcurrent.memoryDelta;
      
      console.log(`**Performance:**`);
      console.log(`  Single Process: ${bestConcurrent.wallClockThroughput.toFixed(2)} chunks/s`);
      console.log(`  ProcessPool: ${bestProcessPool.wallClockThroughput.toFixed(2)} chunks/s`);
      console.log(`  Winner: ${speedRatio > 1 ? 'Single Process' : 'ProcessPool'} (${Math.abs(speedRatio - 1) * 100 + 100}% ${speedRatio > 1 ? 'faster' : 'slower'})`);
      console.log('');
      
      console.log(`**Time to Complete:**`);
      console.log(`  Single Process: ${(bestConcurrent.wallClockTime/1000).toFixed(1)}s`);
      console.log(`  ProcessPool: ${(bestProcessPool.wallClockTime/1000).toFixed(1)}s`);
      console.log(`  Difference: ${Math.abs(bestProcessPool.wallClockTime - bestConcurrent.wallClockTime)/1000}s`);
      console.log('');
      
      console.log(`**Memory Efficiency:**`);
      console.log(`  Single Process: ${bestConcurrent.memoryDelta.toFixed(1)}MB delta`);
      console.log(`  ProcessPool: ${bestProcessPool.memoryDelta.toFixed(1)}MB delta`);
      console.log(`  Winner: ${memoryRatio < 1 ? 'ProcessPool' : 'Single Process'} (${Math.abs(memoryRatio - 1) * 100 + 100}% ${memoryRatio < 1 ? 'less' : 'more'} memory)`);
      console.log('');
    }
    
    // ProcessPool scalability analysis
    if (processPoolResults.length > 1) {
      console.log('📈 **PROCESSPOOL SCALABILITY**:');
      processPoolResults.forEach(result => {
        const workerCount = result.config.processCount;
        const efficiencyPerWorker = result.wallClockThroughput / workerCount;
        console.log(`  ${workerCount} workers: ${result.wallClockThroughput.toFixed(2)} chunks/s total (${efficiencyPerWorker.toFixed(2)} per worker)`);
      });
      
      // Calculate ideal worker count
      const efficiencies = processPoolResults.map(r => ({
        workers: r.config.processCount,
        throughput: r.wallClockThroughput,
        efficiency: r.wallClockThroughput / r.config.processCount
      }));
      
      const bestEfficiency = efficiencies.reduce((best, current) => 
        current.efficiency > best.efficiency ? current : best
      );
      
      console.log(`  Optimal configuration: ${bestEfficiency.workers} workers (${bestEfficiency.efficiency.toFixed(2)} chunks/s per worker)`);
      console.log('');
    }
    
    console.log('💡 **KEY INSIGHTS**:');
    console.log('');
    
    const winner = results[0];
    console.log(`✅ **Winner**: ${winner.name}`);
    console.log(`   Performance: ${winner.wallClockThroughput.toFixed(2)} chunks/second`);
    console.log(`   Time: ${(winner.wallClockTime/1000).toFixed(1)} seconds`);
    
    if (winner.strategy === 'concurrent') {
      console.log(`   Architecture: Single process wins due to shared model efficiency`);
      console.log(`   Benefit: No process startup overhead, lower memory usage`);
    } else {
      console.log(`   Architecture: ProcessPool wins due to true parallelism`);
      console.log(`   Benefit: Complete isolation, better CPU utilization`);
    }
    
    console.log('');
    console.log('📋 **RECOMMENDATIONS**:');
    
    if (concurrentResults[0] && processPoolResults[0]) {
      const concurrentBest = concurrentResults[0];
      const processPoolBest = processPoolResults[0];
      
      if (concurrentBest.wallClockThroughput > processPoolBest.wallClockThroughput) {
        console.log(`• Use Single Process Concurrent for ${testChunks.length}-chunk datasets`);
        console.log(`• ProcessPool overhead not justified for this dataset size`);
        console.log(`• Consider ProcessPool for larger datasets (>1000 chunks)`);
      } else {
        console.log(`• Use ProcessPool for ${testChunks.length}-chunk datasets`);
        console.log(`• True parallelism benefits outweigh startup overhead`);
        console.log(`• ProcessPool recommended for production workloads`);
      }
    }
    
    return true;
    
  } catch (error) {
    console.error('❌ Comparison failed:', error.message);
    console.error('Stack trace:', error.stack);
    return false;
  }
}

// Run the comparison
if (require.main === module) {
  compareSingleVsProcessPool()
    .then(success => {
      process.exit(success ? 0 : 1);
    })
    .catch(error => {
      console.error('Fatal error:', error);
      process.exit(1);
    });
}

module.exports = { compareSingleVsProcessPool };