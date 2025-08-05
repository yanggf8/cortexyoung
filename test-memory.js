#!/usr/bin/env node

// Quick test for memory monitoring functionality
const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder');

async function testMemoryMonitoring() {
  console.log('🧪 Testing child process memory monitoring...');
  
  const embedder = new ProcessPoolEmbedder();
  
  try {
    // Initialize the process pool
    await embedder.initialize();
    console.log('✅ Process pool initialized');
    
    // Wait a bit for processes to settle
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    // Test memory monitoring
    console.log('🔍 Testing getSystemWideMemoryStats...');
    const memStats = await embedder.getSystemWideMemoryStats();
    
    console.log('📊 Memory Stats Result:');
    console.log(`  Total: ${memStats.total}MB`);
    console.log(`  Main Process: ${memStats.main}MB`);
    console.log(`  Child Processes: [${memStats.children.join(', ')}]MB`);
    console.log(`  Total Child Memory: ${memStats.children.reduce((sum, mem) => sum + mem, 0)}MB`);
    console.log(`  Available: ${memStats.available}MB`);
    
    if (memStats.children.every(mem => mem === 0)) {
      console.error('❌ All child processes report 0MB memory - monitoring not working!');
    } else {
      console.log('✅ Child process memory monitoring is working!');
    }
    
  } catch (error) {
    console.error('❌ Test failed:', error);
  } finally {
    console.log('🛑 Shutting down...');
    await embedder.shutdown();
  }
}

testMemoryMonitoring().catch(console.error);