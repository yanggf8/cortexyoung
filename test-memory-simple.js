#!/usr/bin/env node

// Simple test to verify child process memory reporting works
const { spawn } = require('child_process');
const path = require('path');

async function testChildMemory() {
  console.log('🧪 Testing child process memory reporting...');
  
  // Spawn a single child process
  const processScript = path.join(__dirname, 'dist', 'external-embedding-process.js');
  const childProcess = spawn('node', [processScript], {
    stdio: ['pipe', 'pipe', 'pipe']
  });
  
  let messageBuffer = '';
  
  // Handle stdout
  childProcess.stdout.on('data', (data) => {
    messageBuffer += data.toString();
    
    let lines = messageBuffer.split('\n');
    messageBuffer = lines.pop() || '';
    
    for (const line of lines) {
      if (line.trim()) {
        try {
          const message = JSON.parse(line);
          console.log('📨 Received:', message);
          
          if (message.type === 'memory_response') {
            const memoryMB = Math.round(message.memoryUsage.heapUsed / (1024 * 1024));
            console.log(`✅ Child process memory: ${memoryMB}MB`);
            cleanup();
            return;
          }
        } catch (error) {
          console.warn('⚠️ Failed to parse:', line);
        }
      }
    }
  });
  
  // Handle stderr
  childProcess.stderr.on('data', (data) => {
    console.log('📢 Child stderr:', data.toString().trim());
  });
  
  function cleanup() {
    console.log('🧹 Cleaning up...');
    childProcess.kill();
  }
  
  // Initialize the process
  console.log('🚀 Initializing child process...');
  childProcess.stdin.write(JSON.stringify({
    type: 'init',
    data: { processId: 0 }
  }) + '\n');
  
  // Wait for initialization
  setTimeout(() => {
    console.log('🔍 Querying memory...');
    childProcess.stdin.write(JSON.stringify({
      type: 'query_memory',
      requestId: 'test-memory-123'
    }) + '\n');
  }, 5000);
  
  // Timeout after 10 seconds
  setTimeout(() => {
    console.log('⏰ Test timeout');
    cleanup();
  }, 10000);
}

testChildMemory().catch(console.error);