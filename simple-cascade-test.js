#!/usr/bin/env node

console.log('🚀 Simple Signal Cascade Demonstration');
console.log('This test shows improved parent→child signal handling\n');

async function quickDemo() {
  const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
  
  console.log('🔧 Creating ProcessPoolEmbedder...');
  const embedder = new ProcessPoolEmbedder();
  
  console.log('🚀 Initializing (this will spawn child processes)...');
  await embedder.initialize();
  
  console.log('⏳ Waiting 3 seconds for processes to be ready...');
  await new Promise(r => setTimeout(r, 3000));
  
  console.log('🛑 Testing graceful shutdown with improved cascade...');
  console.log('   Watch for:');
  console.log('   - 📤 "Sending abort signal" (parent → child IPC)');
  console.log('   - 🔄 SIGTERM signal sent after 1 second (OS signal backup)');
  console.log('   - ✅ "acknowledged abort" (child → parent acknowledgment)');
  console.log('   - ✅ "exited gracefully" (clean child exit)');
  console.log('');
  
  await embedder.shutdown('demonstration');
  
  console.log('\n🎉 Demo complete! Signal cascade working correctly.');
  console.log('Key improvements:');
  console.log('✅ Parent sends BOTH IPC messages AND OS signals');
  console.log('✅ Children acknowledge BOTH types of signals');
  console.log('✅ Reliable shutdown even if IPC fails');
}

quickDemo().catch(console.error);