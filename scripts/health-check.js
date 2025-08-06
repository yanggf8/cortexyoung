#!/usr/bin/env node

const { exec } = require('child_process');
const util = require('util');
const execPromise = util.promisify(exec);
const fs = require('fs');

console.log('🩺 Cortex System Health Check');
console.log('=============================');

let issues = 0;
let warnings = 0;

function printStatus(status, message) {
    const colors = {
        OK: '\x1b[32m',      // Green
        WARNING: '\x1b[33m',  // Yellow  
        ERROR: '\x1b[31m',    // Red
        INFO: '\x1b[34m',     // Blue
        RESET: '\x1b[0m'      // Reset
    };
    
    switch (status) {
        case 'OK':
            console.log(`   ✅ ${colors.OK}${message}${colors.RESET}`);
            break;
        case 'WARNING':
            console.log(`   ⚠️  ${colors.WARNING}${message}${colors.RESET}`);
            warnings++;
            break;
        case 'ERROR':
            console.log(`   ❌ ${colors.ERROR}${message}${colors.RESET}`);
            issues++;
            break;
        case 'INFO':
            console.log(`   ℹ️  ${colors.INFO}${message}${colors.RESET}`);
            break;
    }
}

async function findProcesses(pattern) {
    try {
        const { stdout } = await execPromise(`pgrep -f "${pattern}"`);
        const pids = stdout.trim().split('\n').filter(pid => pid && pid !== '');
        return pids;
    } catch (error) {
        return [];
    }
}

// Function to check server health endpoint
function checkServerHealth(port) {
    return new Promise((resolve) => {
        const http = require('http');
        const request = http.get(`http://localhost:${port}/health`, (response) => {
            let data = '';
            response.on('data', chunk => data += chunk);
            response.on('end', () => {
                resolve({
                    responding: true,
                    statusCode: response.statusCode,
                    data: data
                });
            });
        });
        
        request.on('error', (error) => {
            resolve({
                responding: false,
                error: error.message
            });
        });
        
        request.setTimeout(3000, () => {
            request.destroy();
            resolve({
                responding: false,
                error: 'Request timeout after 3 seconds'
            });
        });
    });
}

async function checkProcessHealth() {
    console.log('\n🔍 1. Server Status & Process Health Check');
    console.log('------------------------------------------');
    
    const PORT = process.env.PORT || 8765;
    
    // Check different types of processes
    const serverProcesses = await findProcesses('npm.*server|ts-node.*server|node.*server\\.js');
    const demoProcesses = await findProcesses('npm.*demo|ts-node.*index');
    const embeddingProcesses = await findProcesses('node.*external-embedding-process');
    
    // Determine server status
    if (serverProcesses.length > 0) {
        printStatus('INFO', `Server processes detected: ${serverProcesses.length} running`);
        
        // Check if server is responding
        const healthCheck = await checkServerHealth(PORT);
        
        if (healthCheck.responding) {
            printStatus('OK', `✅ SERVER RUNNING - Responding on port ${PORT}`);
            
            try {
                const healthData = JSON.parse(healthCheck.data);
                if (healthData.status === 'healthy') {
                    printStatus('OK', 'Server reports healthy status');
                    if (healthData.startup) {
                        printStatus('INFO', `Startup: ${healthData.startup.stage} (${healthData.startup.progress}% complete)`);
                    }
                } else {
                    printStatus('WARNING', `Server reports status: ${healthData.status}`);
                }
            } catch (error) {
                printStatus('INFO', 'Server responding but health data not parseable');
            }
        } else {
            printStatus('WARNING', `⚠️ SERVER STARTING - Process running but not responding on port ${PORT}`);
            printStatus('INFO', `Error: ${healthCheck.error}`);
            printStatus('INFO', 'Server may still be initializing - wait a few moments and check again');
        }
    } else {
        printStatus('INFO', `❌ SERVER NOT RUNNING - No server processes detected`);
        printStatus('INFO', 'Use: npm run startup to start the server');
    }
    
    // Check other processes
    if (demoProcesses.length > 0) {
        printStatus('INFO', `Demo/indexing processes running: ${demoProcesses.length}`);
    } else {
        printStatus('OK', 'No demo/indexing processes running');
    }
    
    if (embeddingProcesses.length > 0) {
        printStatus('WARNING', `Embedding worker processes still running: ${embeddingProcesses.length}`);
        printStatus('INFO', 'These may indicate incomplete shutdown or active processing');
        console.log('         Clean up with: npm run shutdown');
    } else {
        printStatus('OK', 'No orphaned embedding worker processes');
    }
}

async function checkStorageHealth() {
    console.log('\n🗄️  2. Storage Health Check');
    console.log('-------------------------');
    
    try {
        const { stdout } = await execPromise('npm run --silent storage:status');
        
        // Parse storage status
        if (stdout.includes('Synchronized: ✅')) {
            printStatus('OK', 'Storage layers synchronized');
        } else if (stdout.includes('Synchronized: ❌')) {
            printStatus('WARNING', 'Storage layers not synchronized (auto-sync will handle)');
        }
        
        // Check embeddings
        if (stdout.includes('Embeddings: Local ✅') && stdout.includes('Global ✅')) {
            printStatus('OK', 'Embeddings available in both storages');
        } else if (stdout.includes('Embeddings:') && stdout.includes('✅')) {
            printStatus('WARNING', 'Embeddings available in one storage location');
        } else {
            printStatus('ERROR', 'No embeddings found');
        }
        
        // Check relationships  
        if (stdout.includes('Relationships: Local ✅') && stdout.includes('Global ✅')) {
            printStatus('OK', 'Relationships available in both storages');
        } else if (stdout.includes('Relationships:') && stdout.includes('✅')) {
            printStatus('WARNING', 'Relationships available in one storage location');
        } else {
            printStatus('WARNING', 'Relationships missing (will be rebuilt on startup)');
        }
        
        // Check chunks count
        const chunkMatch = stdout.match(/(\d+) chunks/);
        if (chunkMatch) {
            const chunks = parseInt(chunkMatch[1]);
            if (chunks > 0) {
                printStatus('OK', `Found ${chunks} indexed code chunks`);
            } else {
                printStatus('WARNING', 'No indexed code chunks found');
            }
        }
        
    } catch (error) {
        printStatus('ERROR', 'Storage health check failed');
    }
}


async function checkSystemResources() {
    console.log('\n🔧 3. System Resources');
    console.log('---------------------');
    
    // Check memory (Linux)
    try {
        const { stdout } = await execPromise('free -m');
        const memoryLine = stdout.split('\n')[1];
        const [, total, used, , , available] = memoryLine.split(/\s+/);
        const memoryUsagePercent = Math.round((used / total) * 100);
        
        if (memoryUsagePercent < 80) {
            printStatus('OK', `Memory usage: ${memoryUsagePercent}% (${available}MB available)`);
        } else {
            printStatus('WARNING', `High memory usage: ${memoryUsagePercent}% (${available}MB available)`);
        }
    } catch (error) {
        // Try macOS
        try {
            await execPromise('vm_stat');
            printStatus('INFO', 'Memory check available (macOS detected)');
        } catch (error2) {
            printStatus('INFO', 'Memory check not available on this system');
        }
    }
    
    // Check disk space
    try {
        const { stdout } = await execPromise('df -h .');
        const lines = stdout.trim().split('\n');
        const diskLine = lines[lines.length - 1];
        const diskUsage = diskLine.match(/(\d+)%/);
        
        if (diskUsage) {
            const usage = parseInt(diskUsage[1]);
            if (usage < 90) {
                printStatus('OK', `Disk space usage: ${usage}%`);
            } else {
                printStatus('WARNING', `High disk usage: ${usage}%`);
            }
        }
    } catch (error) {
        printStatus('INFO', 'Disk space check not available');
    }
    
    // Check Node.js version
    try {
        const { stdout } = await execPromise('node --version');
        printStatus('OK', `Node.js version: ${stdout.trim()}`);
    } catch (error) {
        printStatus('ERROR', 'Node.js not found');
    }
    
    // Check npm version
    try {
        const { stdout } = await execPromise('npm --version');
        printStatus('OK', `npm version: ${stdout.trim()}`);
    } catch (error) {
        printStatus('ERROR', 'npm not found');
    }
}

async function main() {
    // Get server status first for summary
    const PORT = process.env.PORT || 8765;
    const serverProcesses = await findProcesses('npm.*server|ts-node.*server|node.*server\\.js');
    const healthCheck = serverProcesses.length > 0 ? await checkServerHealth(PORT) : { responding: false };
    
    await checkProcessHealth();
    await checkStorageHealth();
    await checkSystemResources();
    
    console.log('\n📊 Health Check Summary');
    console.log('======================');
    
    // Server status summary
    if (serverProcesses.length > 0 && healthCheck.responding) {
        printStatus('OK', '🚀 SERVER STATUS: RUNNING & HEALTHY');
    } else if (serverProcesses.length > 0) {
        printStatus('WARNING', '⚠️ SERVER STATUS: STARTING (not yet responding)');
    } else {
        printStatus('INFO', '💤 SERVER STATUS: NOT RUNNING');
    }
    
    // Overall health summary
    if (issues === 0 && warnings === 0) {
        printStatus('OK', 'Overall system health: EXCELLENT');
    } else if (issues === 0) {
        printStatus('WARNING', `Overall system health: GOOD with ${warnings} minor warnings`);
        console.log('\n💡 Recommendations:');
        console.log('   • Warnings are usually minor and often resolve automatically');
        console.log('   • Consider running: npm run storage:sync if storage issues persist');
        console.log('   • Run: npm run shutdown followed by npm run startup for a clean restart');
    } else {
        printStatus('ERROR', `Overall system health: NEEDS ATTENTION (${issues} critical issues, ${warnings} warnings)`);
        console.log('\n🔧 Troubleshooting:');
        console.log('   • Run: npm run shutdown to clean up processes');
        console.log('   • Try: npm run cache:clear-all for storage issues');
        console.log('   • Check logs in: logs/cortex-server.log');
        console.log('   • Restart with: npm run startup -- --rebuild');
    }
    
    // Quick actions based on server status
    console.log('\n🎯 Quick Actions:');
    if (serverProcesses.length === 0) {
        console.log('   • Start server: npm run startup');
        console.log('   • Background mode: npm run startup -- --background');
    } else if (!healthCheck.responding) {
        console.log('   • Wait for server to finish starting up');
        console.log('   • Check again in 30 seconds: npm run health');
    } else {
        console.log('   • Server is ready to use!');
        console.log('   • Check detailed status: npm run status');
    }
    
    console.log('\n📝 Log files available:');
    if (fs.existsSync('/tmp/cortex_storage_health.log')) {
        printStatus('INFO', 'Storage status: /tmp/cortex_storage_health.log');
    }
    if (fs.existsSync('/tmp/cortex_perf_health.log')) {
        printStatus('INFO', 'Performance results: /tmp/cortex_perf_health.log');
    }
    if (fs.existsSync('logs/cortex-server.log')) {
        printStatus('INFO', 'Server logs: logs/cortex-server.log');
    }
    
    console.log('\n🎯 Health check complete!');
    
    // Exit with appropriate code
    if (issues > 0) {
        process.exit(1);
    } else if (warnings > 0) {
        process.exit(2);
    } else {
        process.exit(0);
    }
}

main().catch(error => {
    console.error('❌ Health check script failed:', error.message);
    process.exit(3);
});