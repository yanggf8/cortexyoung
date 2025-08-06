#!/usr/bin/env node

const { exec, spawn } = require('child_process');
const util = require('util');
const execPromise = util.promisify(exec);

console.log('🛑 Cortex Shutdown Script');
console.log('========================');

// Function to find processes by pattern
async function findProcesses(pattern, description) {
    try {
        const { stdout } = await execPromise(`pgrep -f "${pattern}"`);
        const pids = stdout.trim().split('\n').filter(pid => pid);
        if (pids.length > 0 && pids[0] !== '') {
            console.log(`   Found ${pids.length} ${description} processes: ${pids.join(', ')}`);
            return pids;
        }
    } catch (error) {
        // pgrep returns non-zero exit code when no matches found
    }
    console.log(`   No ${description} processes found`);
    return [];
}

// Function to kill processes gracefully then forcefully
async function killProcesses(pattern, description) {
    console.log(`🔄 Stopping ${description}...`);
    
    const pids = await findProcesses(pattern, description);
    if (pids.length === 0) {
        console.log(`   ✅ No ${description} processes running`);
        return;
    }

    try {
        // First attempt: graceful shutdown (SIGTERM)
        console.log('   🤝 Sending SIGTERM (graceful shutdown)...');
        await execPromise(`pkill -TERM -f "${pattern}"`);
        
        // Wait 3 seconds for graceful shutdown
        await new Promise(resolve => setTimeout(resolve, 3000));
        
        // Check if processes still running
        const remaining = await findProcesses(pattern, description);
        if (remaining.length === 0) {
            console.log(`   ✅ ${description} stopped gracefully`);
            return;
        }
        
        console.log('   ⏱️  Some processes still running, waiting 2 more seconds...');
        await new Promise(resolve => setTimeout(resolve, 2000));
        
        // Second attempt: force kill (SIGKILL)
        const stillRunning = await findProcesses(pattern, description);
        if (stillRunning.length > 0) {
            console.log(`   💥 Force killing remaining processes: ${stillRunning.join(', ')}`);
            await execPromise(`pkill -KILL -f "${pattern}"`);
            await new Promise(resolve => setTimeout(resolve, 1000));
        }
        
        // Final check
        const finalCheck = await findProcesses(pattern, description);
        if (finalCheck.length === 0) {
            console.log(`   ✅ All ${description} processes stopped`);
        } else {
            console.log(`   ⚠️  Warning: Some processes may still be running: ${finalCheck.join(', ')}`);
        }
        
    } catch (error) {
        console.log(`   ⚠️  Error stopping ${description}: ${error.message}`);
    }
}

async function main() {
    console.log('\n🔍 Scanning for Cortex processes...\n');
    
    // Check what's running before shutdown
    console.log('📊 Current process status:');
    await findProcesses('npm.*(server|demo)', 'npm server/demo');
    await findProcesses('ts-node.*(server|index)', 'ts-node server/indexer');
    await findProcesses('node.*server\\.js', 'compiled server');
    await findProcesses('node.*external-embedding-process', 'embedding workers');
    await findProcesses('cortex', 'cortex-related');
    
    console.log('\n🛑 Initiating shutdown sequence...\n');
    
    // 1. Stop main server processes
    await killProcesses('npm.*server', 'npm server');
    await killProcesses('ts-node.*server', 'ts-node server');
    await killProcesses('node.*server\\.js', 'compiled server');
    
    // 2. Stop demo/indexing processes
    await killProcesses('npm.*demo', 'npm demo');
    await killProcesses('ts-node.*index', 'ts-node indexer');
    
    // 3. Stop embedding worker processes
    await killProcesses('node.*external-embedding-process', 'embedding workers');
    
    // 4. Stop any remaining cortex processes
    await killProcesses('cortex', 'cortex-related');
    
    console.log('\n🔍 Post-shutdown verification...\n');
    
    // Verify shutdown
    const patterns = [
        'npm.*server',
        'ts-node.*server', 
        'node.*server\\.js',
        'npm.*demo',
        'ts-node.*index',
        'node.*external-embedding-process'
    ];
    
    let allClear = true;
    for (const pattern of patterns) {
        const remaining = await findProcesses(pattern, `pattern '${pattern}'`);
        if (remaining.length > 0) {
            console.log(`⚠️  Warning: Processes still running for pattern '${pattern}': ${remaining.join(', ')}`);
            allClear = false;
        }
    }
    
    if (allClear) {
        console.log('✅ All Cortex processes successfully stopped');
        console.log('\n💡 System is clean and ready for restart');
        console.log('   Use: npm run startup or npm run server');
    } else {
        console.log('⚠️  Some processes may still be running');
        console.log('   You may need to restart your terminal or reboot if issues persist');
    }
    
    console.log('\n🎯 Shutdown complete!');
}

main().catch(error => {
    console.error('❌ Shutdown script failed:', error.message);
    process.exit(1);
});