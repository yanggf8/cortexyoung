#!/usr/bin/env node

/**
 * Final Comprehensive Cleanup Test
 * Test both direct ProcessPoolEmbedder cleanup and main process signal handling
 */

const { spawn, exec } = require('child_process');
const { promisify } = require('util');
const execAsync = promisify(exec);

async function getNodeProcesses() {
  try {
    const { stdout } = await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep || true');
    return stdout.trim().split('\n').filter(line => line.trim() !== '');
  } catch (error) {
    return [];
  }
}

async function testMainProcessSignalHandling() {
  console.log('🧪 Testing Main Process Signal Handling\n');
  
  return new Promise(async (resolve) => {
    // Clean up first
    try {
      await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep | awk \'{print $2}\' | xargs -r kill -9');
      await new Promise(r => setTimeout(r, 1000));
    } catch (e) {}
    
    const startTime = Date.now();
    
    // Start demo
    console.log('🚀 Starting demo process...');
    const demoProc = spawn('npm', ['run', 'demo'], {
      stdio: ['pipe', 'pipe', 'pipe']
    });
    
    let processFound = false;
    let cleanupMessageSeen = false;
    let stderr = '';
    
    const timeout = setTimeout(async () => {
      console.log('🛑 Sending SIGINT to demo process...');
      demoProc.kill('SIGINT');
      
      setTimeout(async () => {
        const orphans = await getNodeProcesses();
        const duration = Date.now() - startTime;
        
        resolve({
          testName: 'Main Process Signal Handling',
          success: orphans.length === 0 && cleanupMessageSeen,
          processFound,
          cleanupMessageSeen,
          orphanCount: orphans.length,
          duration
        });
      }, 3000);
    }, 8000); // Wait 8 seconds then interrupt
    
    demoProc.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      
      if (chunk.includes('Cleaning up process pool')) {
        cleanupMessageSeen = true;
        console.log('✅ Cleanup message detected');
      }
    });
    
    demoProc.on('close', (code) => {
      clearTimeout(timeout);
    });
  });
}

async function testDirectProcessPoolEmbedder() {
  console.log('🧪 Testing Direct ProcessPoolEmbedder\n');
  
  return new Promise(async (resolve) => {
    try {
      // Clean up first
      try {
        await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep | awk \'{print $2}\' | xargs -r kill -9');
        await new Promise(r => setTimeout(r, 1000));
      } catch (e) {}
      
      const startTime = Date.now();
      
      console.log('🔧 Creating ProcessPoolEmbedder...');
      const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
      const embedder = new ProcessPoolEmbedder();
      
      console.log('🚀 Initializing...');
      await embedder.initialize();
      
      await new Promise(r => setTimeout(r, 2000));
      const processesBeforeShutdown = await getNodeProcesses();
      
      console.log(`📊 Found ${processesBeforeShutdown.length} processes before shutdown`);
      
      console.log('🛑 Calling shutdown...');
      await embedder.shutdown('final-test');
      
      await new Promise(r => setTimeout(r, 3000));
      const orphans = await getNodeProcesses();
      const duration = Date.now() - startTime;
      
      resolve({
        testName: 'Direct ProcessPoolEmbedder',
        success: orphans.length === 0 && processesBeforeShutdown.length > 0,
        processesBeforeShutdown: processesBeforeShutdown.length,
        orphanCount: orphans.length,
        duration
      });
      
    } catch (error) {
      resolve({
        testName: 'Direct ProcessPoolEmbedder',
        success: false,
        error: error.message,
        duration: Date.now() - Date.now()
      });
    }
  });
}

async function main() {
  console.log('🚀 Final Comprehensive Cleanup Test');
  console.log(`📅 ${new Date().toISOString()}\n`);
  
  const results = [];
  
  try {
    // Test 1: Direct ProcessPoolEmbedder cleanup
    const directTest = await testDirectProcessPoolEmbedder();
    results.push(directTest);
    
    console.log('\n' + '-'.repeat(50) + '\n');
    
    // Test 2: Main process signal handling
    const signalTest = await testMainProcessSignalHandling();
    results.push(signalTest);
    
    // Print results
    console.log('\n' + '='.repeat(60));
    console.log('📊 FINAL TEST RESULTS');
    console.log('='.repeat(60));
    
    let allPassed = true;
    
    results.forEach(result => {
      const status = result.success ? '✅ PASS' : '❌ FAIL';
      const duration = (result.duration / 1000).toFixed(1);
      
      console.log(`\n${status} ${result.testName} (${duration}s)`);
      
      if (result.processesBeforeShutdown) {
        console.log(`   Processes Created: ${result.processesBeforeShutdown}`);
      }
      if (result.processFound !== undefined) {
        console.log(`   Process Activity: ${result.processFound ? 'YES' : 'NO'}`);
      }
      if (result.cleanupMessageSeen !== undefined) {
        console.log(`   Cleanup Message: ${result.cleanupMessageSeen ? 'YES' : 'NO'}`);
      }
      console.log(`   Orphan Processes: ${result.orphanCount}`);
      
      if (result.error) {
        console.log(`   Error: ${result.error}`);
      }
      
      if (!result.success) allPassed = false;
    });
    
    console.log(`\n📈 Overall: ${allPassed ? '🎉 ALL TESTS PASSED' : '⚠️ SOME TESTS FAILED'}`);
    console.log('='.repeat(60));
    
    if (allPassed) {
      console.log('\n🎊 EXCELLENT! Process pool cleanup is working perfectly!');
      console.log('✅ Both direct cleanup and signal handling work correctly.');
      console.log('✅ No orphan processes remain after any type of shutdown.');
      process.exit(0);
    } else {
      console.log('\n⚠️ Some cleanup functionality needs attention.');
      process.exit(1);
    }
    
  } catch (error) {
    console.error('❌ Test suite failed:', error);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}