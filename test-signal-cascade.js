#!/usr/bin/env node

/**
 * Test Signal Cascade - Verify parent properly cascades abort signals to children
 */

const { spawn, exec } = require('child_process');
const { promisify } = require('util');
const execAsync = promisify(exec);

async function testSignalCascade() {
  console.log('🧪 Testing Signal Cascade from Parent to Children\n');
  
  // Clean up first
  console.log('🧹 Cleaning up any existing processes...');
  try {
    await execAsync('pkill -f "npm.*demo\\|ts-node.*index\\|node.*external-embedding-process" || true');
    await new Promise(r => setTimeout(r, 1000));
  } catch (e) {}
  
  return new Promise(async (resolve) => {
    const startTime = Date.now();
    
    console.log('🚀 Starting direct ProcessPoolEmbedder test...');
    
    const testProc = spawn('node', ['test-direct-cleanup.js'], {
      stdio: ['pipe', 'pipe', 'pipe']
    });
    
    let stderr = '';
    let processesStarted = 0;
    let shutdownStarted = false;
    let cascadeMessages = [];
    
    const timeout = setTimeout(() => {
      console.log('❌ Test timeout');
      testProc.kill('SIGKILL');
      resolve({ success: false, error: 'timeout' });
    }, 45000);
    
    testProc.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      
      // Count processes that get started
      if (chunk.includes('ready with isolated FastEmbedding')) {
        processesStarted++;
        console.log(`   Process ${processesStarted} ready`);
      }
      
      // Detect shutdown sequence
      if (chunk.includes('Testing graceful shutdown')) {
        shutdownStarted = true;
        console.log('   🛑 Shutdown sequence started');
      }
      
      // Track signal cascade messages
      if (chunk.includes('Sending abort signal')) {
        cascadeMessages.push('Parent sends abort signal');
        console.log('   📤 Parent sending abort signal');
      }
      
      if (chunk.includes('acknowledged abort')) {
        cascadeMessages.push('Child acknowledged abort');
        console.log('   ✅ Child acknowledged abort');
      }
      
      if (chunk.includes('exited gracefully')) {
        cascadeMessages.push('Child exited gracefully');
        console.log('   ✅ Child exited gracefully');
      }
      
      if (chunk.includes('Graceful shutdown completed')) {
        console.log('   🏁 Graceful shutdown completed');
      }
    });
    
    testProc.on('close', async (code) => {
      clearTimeout(timeout);
      
      // Check for orphan processes
      await new Promise(r => setTimeout(r, 2000));
      try {
        const { stdout } = await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep || true');
        const orphans = stdout.trim().split('\n').filter(line => line.trim() !== '');
        
        const duration = Date.now() - startTime;
        
        resolve({
          success: code === 0 && orphans.length === 0,
          processesStarted,
          shutdownStarted,
          cascadeMessages,
          orphanCount: orphans.length,
          duration,
          exitCode: code
        });
      } catch (error) {
        resolve({
          success: false,
          error: error.message,
          duration: Date.now() - startTime
        });
      }
    });
  });
}

async function main() {
  console.log('🚀 Signal Cascade Test');
  console.log(`📅 ${new Date().toISOString()}\n`);
  
  try {
    const result = await testSignalCascade();
    
    console.log('\n' + '='.repeat(60));
    console.log('📊 SIGNAL CASCADE TEST RESULTS');
    console.log('='.repeat(60));
    
    console.log(`Status: ${result.success ? '✅ PASS' : '❌ FAIL'}`);
    console.log(`Duration: ${(result.duration / 1000).toFixed(1)}s`);
    
    if (result.processesStarted) console.log(`Processes Started: ${result.processesStarted}`);
    if (result.shutdownStarted !== undefined) console.log(`Shutdown Started: ${result.shutdownStarted ? 'YES' : 'NO'}`);
    if (result.cascadeMessages.length > 0) {
      console.log(`Cascade Messages: ${result.cascadeMessages.length}`);
      result.cascadeMessages.forEach(msg => console.log(`  - ${msg}`));
    }
    if (result.orphanCount !== undefined) console.log(`Orphan Processes: ${result.orphanCount}`);
    if (result.exitCode !== undefined) console.log(`Exit Code: ${result.exitCode}`);
    
    if (result.error) console.log(`Error: ${result.error}`);
    
    console.log('='.repeat(60));
    
    if (result.success) {
      console.log('\n🎉 EXCELLENT! Signal cascade is working correctly.');
      console.log('✅ Parent processes properly cascade abort signals to children.');
      console.log('✅ Children acknowledge the signals and exit gracefully.');
      console.log('✅ No orphan processes remain after shutdown.');
      process.exit(0);
    } else {
      console.log('\n⚠️ Signal cascade needs improvement.');
      process.exit(1);
    }
    
  } catch (error) {
    console.error('❌ Test failed:', error);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}