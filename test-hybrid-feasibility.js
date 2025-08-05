#!/usr/bin/env node

/**
 * Hybrid Strategy Feasibility Test
 * 
 * Tests the hybrid 2-process SharedArrayBuffer approach on ~100 chunks
 * Success criteria: > 5.1 chunks/second (must beat concurrent strategy)
 * Failure criteria: < 4.0 chunks/second (abandon approach)
 */

const { EmbeddingStrategyManager } = require('./dist/embedding-strategy');
const { GitScanner } = require('./dist/git-scanner');
const { SmartChunker } = require('./dist/chunker');

async function runHybridFeasibilityTest() {
  console.log('🧪 Hybrid Strategy Feasibility Test');
  console.log('=' .repeat(50));
  console.log('📊 Target: >5.1 chunks/second (beat concurrent)');
  console.log('🚨 Failure threshold: <4.0 chunks/second');
  console.log('');

  try {
    // Scan repository for test files
    console.log('🔍 Scanning repository for test files...');
    const gitScanner = new GitScanner('.');
    const scanResult = await gitScanner.scanRepository('full');
    const files = scanResult.files;
    
    // Filter for JavaScript/TypeScript files
    const codeFiles = files.filter(filePath => 
      filePath.match(/\.(js|ts|jsx|tsx)$/) && 
      !filePath.includes('node_modules') &&
      !filePath.includes('.git')
    );
    
    console.log(`📁 Found ${codeFiles.length} code files`);
    
    // Chunk the files to get ~100 chunks
    console.log('✂️  Chunking files to get ~100 test chunks...');
    const chunker = new SmartChunker();
    let allChunks = [];
    
    for (const filePath of codeFiles.slice(0, 15)) { // Limit files to get ~100 chunks
      try {
        const content = await gitScanner.readFile(filePath);
        const fileChunks = await chunker.chunkFile(filePath, content);
        allChunks.push(...fileChunks);
        
        // Stop when we have enough chunks
        if (allChunks.length >= 100) {
          break;
        }
      } catch (error) {
        console.warn(`Failed to chunk ${filePath}:`, error.message);
      }
    }
    
    // Take exactly 100 chunks for consistent testing
    const testChunks = allChunks.slice(0, 100);
    console.log(`📦 Using ${testChunks.length} chunks for hybrid feasibility test`);
    
    if (testChunks.length < 50) {
      throw new Error('Insufficient chunks for meaningful test');
    }
    
    // Initialize strategy manager
    const strategyManager = new EmbeddingStrategyManager();
    
    console.log('');
    console.log('🚀 Testing Hybrid Strategy...');
    console.log('-'.repeat(30));
    
    const startTime = Date.now();
    
    // Test hybrid strategy
    const hybridResult = await strategyManager.generateEmbeddings(testChunks, {
      strategy: 'hybrid'
    });
    
    const totalTime = Date.now() - startTime;
    const chunksPerSecond = hybridResult.performance.chunksPerSecond;
    
    console.log('');
    console.log('📊 Hybrid Strategy Results:');
    console.log('=' .repeat(40));
    console.log(`⏱️  Total time: ${totalTime}ms`);
    console.log(`📈 Throughput: ${chunksPerSecond.toFixed(2)} chunks/second`);
    console.log(`💾 Peak memory: ${hybridResult.performance.peakMemoryMB.toFixed(1)}MB`);
    console.log(`🎯 Chunks processed: ${hybridResult.chunks.length}`);
    console.log(`📦 Total batches: ${hybridResult.performance.totalBatches}`);
    console.log(`⏰ Average batch time: ${hybridResult.performance.averageBatchTime}ms`);
    
    // Performance validation
    console.log('');
    console.log('🎯 Performance Validation:');
    console.log('-'.repeat(25));
    
    if (chunksPerSecond > 5.1) {
      console.log(`✅ SUCCESS: ${chunksPerSecond.toFixed(2)} > 5.1 chunks/second`);
      console.log(`🎉 Hybrid strategy BEATS concurrent strategy target!`);
      console.log(`📝 Recommendation: Proceed with full comparison testing`);
      
      // Calculate projected performance for larger datasets
      const timeFor1000Chunks = (1000 / chunksPerSecond);
      const timeFor1857Chunks = (1857 / chunksPerSecond); // Full corpus size
      
      console.log('');
      console.log('📊 Projected Performance:');
      console.log(`  • 1,000 chunks: ${timeFor1000Chunks.toFixed(1)}s`);
      console.log(`  • 1,857 chunks (full corpus): ${timeFor1857Chunks.toFixed(1)}s`);
      
      return true; // Success
      
    } else if (chunksPerSecond >= 4.0) {
      console.log(`⚠️  MARGINAL: ${chunksPerSecond.toFixed(2)} chunks/second (4.0-5.1 range)`);
      console.log(`🤔 Close to target but doesn't beat concurrent strategy`);
      console.log(`📝 Recommendation: Consider optimizations or abandon for now`);
      
      return false; // Marginal failure
      
    } else {
      console.log(`❌ FAILURE: ${chunksPerSecond.toFixed(2)} < 4.0 chunks/second`);
      console.log(`💔 Hybrid strategy underperforms - abandon approach`);
      console.log(`📝 Recommendation: Focus on existing strategies`);
      
      return false; // Clear failure
    }
    
  } catch (error) {
    console.error('❌ Feasibility test failed:', error.message);
    console.error('Stack trace:', error.stack);
    return false;
  }
}

// Run the test
if (require.main === module) {
  runHybridFeasibilityTest()
    .then(success => {
      process.exit(success ? 0 : 1);
    })
    .catch(error => {
      console.error('Fatal error:', error);
      process.exit(1);
    });
}

module.exports = { runHybridFeasibilityTest };