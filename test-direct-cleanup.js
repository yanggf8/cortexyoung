#!/usr/bin/env node

/**
 * Direct ProcessPoolEmbedder Cleanup Test
 * Tests the ProcessPoolEmbedder directly without full indexing
 */

const { exec } = require('child_process');
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

async function testDirectCleanup() {
  console.log('🧪 Testing ProcessPoolEmbedder Direct Cleanup\n');
  
  const testStart = Date.now();
  
  try {
    // Clean up any existing orphan processes
    console.log('📋 1. Cleaning up any existing orphan processes...');
    try {
      await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep | awk \'{print $2}\' | xargs -r kill -9');
      await new Promise(resolve => setTimeout(resolve, 1000));
    } catch (e) {
      // Ignore errors - processes might not exist
    }
    
    const initialProcesses = await getNodeProcesses();
    if (initialProcesses.length > 0) {
      console.log(`⚠️  Still found ${initialProcesses.length} processes after cleanup attempt`);
      return { success: false, error: 'Could not clean initial processes' };
    }
    console.log('✅ No existing orphan processes found');
    
    // Import and test the ProcessPoolEmbedder directly
    console.log('\n🚀 2. Testing ProcessPoolEmbedder directly...');
    
    const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
    const embedder = new ProcessPoolEmbedder();
    
    console.log('✅ ProcessPoolEmbedder instance created');
    
    // Initialize the embedder (this will spawn child processes)
    console.log('\n🔧 3. Initializing ProcessPoolEmbedder...');
    await embedder.initialize();
    console.log('✅ ProcessPoolEmbedder initialized');
    
    // Check if child processes were spawned
    await new Promise(resolve => setTimeout(resolve, 2000)); // Wait a bit for processes to spawn
    const runningProcesses = await getNodeProcesses();
    
    if (runningProcesses.length === 0) {
      console.log('❌ No child processes found - initialization may have failed');
      return { success: false, error: 'No child processes were spawned' };
    }
    
    console.log(`✅ Found ${runningProcesses.length} child processes running`);
    runningProcesses.forEach((proc, i) => console.log(`   ${i + 1}. Process found`));
    
    // Now test the cleanup
    console.log('\n🛑 4. Testing graceful shutdown...');
    await embedder.shutdown('test');
    console.log('✅ Shutdown method completed');
    
    // Wait a bit for processes to actually exit
    await new Promise(resolve => setTimeout(resolve, 3000));
    
    // Check for orphan processes after cleanup
    console.log('\n🔍 5. Checking for orphan processes after cleanup...');
    const orphanProcesses = await getNodeProcesses();
    
    const testTime = Date.now() - testStart;
    
    if (orphanProcesses.length === 0) {
      console.log('✅ SUCCESS: No orphan processes found after cleanup!');
      return {
        success: true,
        initialProcesses: runningProcesses.length,
        orphanProcesses: orphanProcesses.length,
        testTime,
        cleanupWorking: true
      };
    } else {
      console.log(`❌ FAILURE: Found ${orphanProcesses.length} orphan process(es):`);
      orphanProcesses.forEach((proc, i) => console.log(`   ${i + 1}. ${proc}`));
      
      // Kill the orphans for cleanup
      console.log('🧹 Killing orphan processes...');
      try {
        await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep | awk \'{print $2}\' | xargs -r kill -9');
        console.log('   Orphan processes killed');
      } catch (e) {
        console.log('   Failed to kill some orphan processes');
      }
      
      return {
        success: false,
        initialProcesses: runningProcesses.length,
        orphanProcesses: orphanProcesses.length,
        testTime,
        cleanupWorking: false
      };
    }
    
  } catch (error) {
    console.error('❌ Test error:', error);
    return { success: false, error: error.message };
  }
}

async function main() {
  console.log('🚀 Direct ProcessPoolEmbedder Cleanup Test');
  console.log(`📅 ${new Date().toISOString()}\n`);
  
  try {
    const result = await testDirectCleanup();
    
    console.log('\n' + '='.repeat(60));
    console.log('📊 TEST RESULTS');
    console.log('='.repeat(60));
    
    console.log(`Status: ${result.success ? '✅ PASS' : '❌ FAIL'}`);
    if (result.testTime) console.log(`Test Duration: ${(result.testTime / 1000).toFixed(1)}s`);
    
    if (result.initialProcesses) console.log(`Initial Child Processes: ${result.initialProcesses}`);
    if (result.orphanProcesses !== undefined) console.log(`Orphan Processes: ${result.orphanProcesses}`);
    if (result.cleanupWorking !== undefined) console.log(`Cleanup Working: ${result.cleanupWorking ? 'YES' : 'NO'}`);
    
    if (result.error) {
      console.log(`Error: ${result.error}`);
    }
    
    console.log('='.repeat(60));
    
    if (result.success) {
      console.log('\n🎉 EXCELLENT! ProcessPoolEmbedder cleanup is working correctly.');
      console.log('✅ Child processes were properly cleaned up on shutdown.');
      process.exit(0);
    } else {
      console.log('\n⚠️ ProcessPoolEmbedder cleanup has issues.');
      if (result.orphanProcesses > 0) {
        console.log('❌ Orphan processes were found - cleanup implementation needs fixes.');
      }
      process.exit(1);
    }
    
  } catch (error) {
    console.error('❌ Test failed with error:', error);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}