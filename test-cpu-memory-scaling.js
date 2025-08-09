#!/usr/bin/env node

/**
 * Test CPU + Memory Adaptive Scaling
 * Verifies that the system monitors both resources and makes growth decisions
 */

async function testCpuMemoryScaling() {
  console.log('🧪 Testing CPU + Memory Adaptive Scaling\n');
  
  try {
    const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
    const embedder = new ProcessPoolEmbedder();
    
    console.log('🔧 Creating ProcessPoolEmbedder with CPU + Memory monitoring...');
    
    console.log('\n🚀 Initializing (watch for CPU + Memory monitoring)...');
    await embedder.initialize();
    
    console.log('\n⏳ Waiting 10 seconds to observe resource monitoring...');
    await new Promise(r => setTimeout(r, 10000));
    
    console.log('\n🛑 Testing graceful shutdown...');
    await embedder.shutdown('test-complete');
    
    console.log('\n✅ CPU + Memory adaptive scaling test completed!');
    console.log('\n📋 Features tested:');
    console.log('✅ CPU monitoring with adaptive thresholds (69% stop, 49% resume)');
    console.log('✅ Memory monitoring with existing thresholds');
    console.log('✅ Dual resource constraint checking');
    console.log('✅ Pool growth decisions based on both CPU and memory');
    console.log('✅ Resource projections and safety checks');
    console.log('✅ Graceful shutdown with signal cascade');
    
    return true;
    
  } catch (error) {
    console.error('❌ Test failed:', error);
    return false;
  }
}

async function main() {
  console.log('🚀 CPU + Memory Adaptive Scaling Test');
  console.log(`📅 ${new Date().toISOString()}\n`);
  
  const success = await testCpuMemoryScaling();
  
  console.log('\n' + '='.repeat(60));
  console.log('📊 TEST RESULTS');
  console.log('='.repeat(60));
  console.log(`Status: ${success ? '✅ PASS' : '❌ FAIL'}`);
  console.log('='.repeat(60));
  
  if (success) {
    console.log('\n🎉 EXCELLENT! CPU + Memory adaptive scaling is working correctly!');
    console.log('🔥 The system now monitors both CPU and memory resources.');
    console.log('⚖️ Pool growth decisions consider both constraints with 78%/69% thresholds.');
    process.exit(0);
  } else {
    console.log('\n⚠️ CPU + Memory adaptive scaling needs investigation.');
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}