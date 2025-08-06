#!/usr/bin/env node

/**
 * Test Orphan Prevention - Verify no child processes remain after main process exits
 * This test spawns the demo process, waits for processes to start, kills it, then checks for orphans
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

async function testOrphanPrevention() {
  console.log('🧪 Testing Orphan Process Prevention\n');
  
  const testStart = Date.now();
  
  // Check initial state
  console.log('📋 1. Checking for existing orphan processes...');
  const initialProcesses = await getNodeProcesses();
  if (initialProcesses.length > 0) {
    console.log(`⚠️  Found ${initialProcesses.length} existing external-embedding-process(es):`);
    initialProcesses.forEach((proc, i) => console.log(`   ${i + 1}. ${proc}`));
    console.log('   Please kill these processes manually before running the test.');
    process.exit(1);
  }
  console.log('✅ No existing orphan processes found');
  
  // Start the demo process
  console.log('\n🚀 2. Starting demo process...');
  const demoProc = spawn('npm', ['run', 'demo'], {
    stdio: ['pipe', 'pipe', 'pipe']
  });
  
  let stderr = '';
  let processesStarted = 0;
  let processesReady = 0;
  
  return new Promise(async (resolve) => {
    const timeout = setTimeout(() => {
      console.log('❌ Test timeout - killing demo process');
      demoProc.kill('SIGKILL');
      resolve({ success: false, error: 'Timeout waiting for processes to start' });
    }, 60000);
    
    demoProc.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      
      // Count processes being spawned
      if (chunk.includes('🔧 Process') && chunk.includes('spawned with PID')) {
        processesStarted++;
      }
      
      // Count processes that are ready
      if (chunk.includes('ready with isolated FastEmbedding')) {
        processesReady++;
        console.log(`   Process ${processesReady} ready...`);
      }
      
      // Once we have some processes ready, interrupt with SIGINT
      if (processesReady >= 3) {
        setTimeout(async () => {
          console.log(`\n🛑 3. Sending SIGINT to main process (PID: ${demoProc.pid})...`);
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
                orphanProcesses: orphanProcesses.length,
                testTime,
                cleanupWorking: true
              });
            } else {
              console.log(`❌ FAILURE: Found ${orphanProcesses.length} orphan process(es):`);
              orphanProcesses.forEach((proc, i) => console.log(`   ${i + 1}. ${proc}`));
              
              // Kill the orphans for cleanup
              console.log('🧹 Killing orphan processes...');
              for (const proc of orphanProcesses) {
                try {
                  const pidMatch = proc.match(/\s+(\d+)\s+/);
                  if (pidMatch) {
                    const pid = parseInt(pidMatch[1]);
                    process.kill(pid, 'SIGKILL');
                    console.log(`   Killed PID ${pid}`);
                  }
                } catch (error) {
                  console.log(`   Failed to kill process: ${error.message}`);
                }
              }
              
              resolve({
                success: false,
                processesStarted,
                processesReady,
                orphanProcesses: orphanProcesses.length,
                testTime,
                cleanupWorking: false
              });
            }
          }, 3000); // Wait 3 seconds for cleanup
        }, 1000); // Wait 1 second after processes are ready
      }
    });
    
    demoProc.on('close', (code) => {
      // This will be called when SIGINT cleanup completes
      if (processesReady < 3) {
        clearTimeout(timeout);
        resolve({ 
          success: false, 
          error: `Demo process closed early (code: ${code}) with only ${processesReady} processes ready` 
        });
      }
    });
  });
}

async function main() {
  console.log('🚀 Orphan Process Prevention Test');
  console.log(`📅 ${new Date().toISOString()}\n`);
  
  try {
    const result = await testOrphanPrevention();
    
    console.log('\n' + '='.repeat(60));
    console.log('📊 TEST RESULTS');
    console.log('='.repeat(60));
    
    console.log(`Status: ${result.success ? '✅ PASS' : '❌ FAIL'}`);
    console.log(`Test Duration: ${(result.testTime / 1000).toFixed(1)}s`);
    
    if (result.processesStarted) console.log(`Processes Started: ${result.processesStarted}`);
    if (result.processesReady) console.log(`Processes Ready: ${result.processesReady}`);
    if (result.orphanProcesses !== undefined) console.log(`Orphan Processes: ${result.orphanProcesses}`);
    if (result.cleanupWorking !== undefined) console.log(`Cleanup Working: ${result.cleanupWorking ? 'YES' : 'NO'}`);
    
    if (result.error) {
      console.log(`Error: ${result.error}`);
    }
    
    console.log('='.repeat(60));
    
    if (result.success) {
      console.log('\n🎉 EXCELLENT! Process pool cleanup is working correctly.');
      console.log('✅ No orphan processes remain after main process exits.');
      process.exit(0);
    } else {
      console.log('\n⚠️ Process pool cleanup needs investigation.');
      if (result.orphanProcesses > 0) {
        console.log('❌ Orphan processes were found - cleanup implementation may have issues.');
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