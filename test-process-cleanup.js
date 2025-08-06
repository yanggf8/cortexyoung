#!/usr/bin/env node

/**
 * Test Process Pool Cleanup - Force creation of ProcessPoolEmbedder and test cleanup
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

async function testProcessCleanup() {
  console.log('🧪 Testing Process Pool Cleanup with Forced Embedding Generation\n');
  
  const testStart = Date.now();
  
  // Check initial state
  console.log('📋 1. Checking for existing orphan processes...');
  const initialProcesses = await getNodeProcesses();
  if (initialProcesses.length > 0) {
    console.log(`⚠️  Found ${initialProcesses.length} existing external-embedding-process(es)`);
    console.log('   Killing them...');
    try {
      await execAsync('ps aux | grep "node.*external-embedding-process" | grep -v grep | awk \'{print $2}\' | xargs -r kill -9');
      await new Promise(resolve => setTimeout(resolve, 1000));
    } catch (e) {
      console.log('   Failed to kill some processes');
    }
  }
  console.log('✅ No existing orphan processes found');
  
  // Start the demo with force reindex to ensure embedding generation happens  
  console.log('\n🚀 2. Starting demo with force reindex (--reindex)...');
  const demoProc = spawn('npm', ['run', 'demo:reindex'], {
    stdio: ['pipe', 'pipe', 'pipe']
  });
  
  let stderr = '';
  let processesStarted = 0;
  let processesReady = 0;
  let embeddingStarted = false;
  
  return new Promise(async (resolve) => {
    const timeout = setTimeout(() => {
      console.log('❌ Test timeout - killing demo process');
      demoProc.kill('SIGKILL');
      resolve({ success: false, error: 'Timeout waiting for embedding to start' });
    }, 90000); // 90 seconds timeout
    
    demoProc.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      
      // Show relevant progress
      if (chunk.includes('Step') && chunk.includes('/10')) {
        const stepMatch = chunk.match(/Step (\d+)\/10/);
        if (stepMatch) {
          console.log(`   Progress: Step ${stepMatch[1]}/10`);
        }
      }
      
      // Count processes being spawned
      if (chunk.includes('Process') && chunk.includes('spawned')) {
        processesStarted++;
      }
      
      // Count processes that are ready
      if (chunk.includes('ready with isolated FastEmbedding')) {
        processesReady++;
        console.log(`   Process ${processesReady} ready...`);
      }
      
      // Check for embedding generation start
      if (chunk.includes('Processing') && chunk.includes('chunks with process pool')) {
        embeddingStarted = true;
        console.log('   🎯 Embedding generation started!');
      }
      
      // Once embedding has started and we have processes ready, interrupt
      if (embeddingStarted && processesReady >= 1) {
        setTimeout(async () => {
          console.log(`\n🛑 3. Sending SIGINT to main process (PID: ${demoProc.pid})...`);
          console.log('   (This should trigger our cleanup handlers)');
          
          demoProc.kill('SIGINT');
          
          // Wait a bit for cleanup to complete
          setTimeout(async () => {
            clearTimeout(timeout);
            
            console.log('\n🔍 4. Checking for orphan processes after cleanup...');
            const orphanProcesses = await getNodeProcesses();
            
            const testTime = Date.now() - testStart;
            
            if (orphanProcesses.length === 0) {
              console.log('✅ SUCCESS: No orphan processes found after cleanup!');
              resolve({
                success: true,
                processesStarted,
                processesReady,
                embeddingStarted,
                orphanProcesses: orphanProcesses.length,
                testTime,
                cleanupWorking: true
              });
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
              
              resolve({
                success: false,
                processesStarted,
                processesReady,
                embeddingStarted,
                orphanProcesses: orphanProcesses.length,
                testTime,
                cleanupWorking: false
              });
            }
          }, 5000); // Wait 5 seconds for cleanup
        }, 2000); // Wait 2 seconds after embedding starts
      }
    });
    
    demoProc.on('close', (code) => {
      // This will be called when SIGINT cleanup completes
      if (!embeddingStarted) {
        clearTimeout(timeout);
        resolve({ 
          success: false, 
          error: `Demo process closed early (code: ${code}) before embedding started` 
        });
      }
    });
  });
}

async function main() {
  console.log('🚀 Process Pool Cleanup Test');
  console.log(`📅 ${new Date().toISOString()}\n`);
  
  try {
    const result = await testProcessCleanup();
    
    console.log('\n' + '='.repeat(60));
    console.log('📊 TEST RESULTS');
    console.log('='.repeat(60));
    
    console.log(`Status: ${result.success ? '✅ PASS' : '❌ FAIL'}`);
    if (result.testTime) console.log(`Test Duration: ${(result.testTime / 1000).toFixed(1)}s`);
    
    if (result.processesStarted) console.log(`Processes Started: ${result.processesStarted}`);
    if (result.processesReady) console.log(`Processes Ready: ${result.processesReady}`);
    if (result.embeddingStarted !== undefined) console.log(`Embedding Started: ${result.embeddingStarted ? 'YES' : 'NO'}`);
    if (result.orphanProcesses !== undefined) console.log(`Orphan Processes: ${result.orphanProcesses}`);
    if (result.cleanupWorking !== undefined) console.log(`Cleanup Working: ${result.cleanupWorking ? 'YES' : 'NO'}`);
    
    if (result.error) {
      console.log(`Error: ${result.error}`);
    }
    
    console.log('='.repeat(60));
    
    if (result.success) {
      console.log('\n🎉 EXCELLENT! Process pool cleanup is working correctly.');
      console.log('✅ ProcessPoolEmbedder was created and child processes were properly cleaned up.');
      process.exit(0);
    } else {
      console.log('\n⚠️ Process pool cleanup has issues.');
      if (result.orphanProcesses > 0) {
        console.log('❌ Orphan processes were found - cleanup implementation needs fixes.');
      } else if (!result.embeddingStarted) {
        console.log('⚠️ Test ended before embedding generation could start.');
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