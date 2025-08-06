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

async function checkProcessHealth() {
    console.log('\n🔍 1. Process Health Check');
    console.log('-------------------------');
    
    const serverProcesses = await findProcesses('npm.*server|ts-node.*server|node.*server\\.js');
    const demoProcesses = await findProcesses('npm.*demo|ts-node.*index');
    const embeddingProcesses = await findProcesses('node.*external-embedding-process');
    
    if (serverProcesses.length > 0) {
        printStatus('INFO', `Server processes running: ${serverProcesses.length}`);
        
        // Check if server is responding
        try {
            const { stdout } = await execPromise('curl -s http://localhost:8765/health');
            printStatus('OK', 'Server responding on port 8765');
        } catch (error) {
            printStatus('WARNING', 'Server process running but not responding on port 8765');
        }
    } else {
        printStatus('INFO', 'No server processes running');
    }
    
    if (demoProcesses.length > 0) {
        printStatus('INFO', `Demo/indexing processes running: ${demoProcesses.length}`);
    } else {
        printStatus('OK', 'No demo processes running');
    }
    
    if (embeddingProcesses.length > 0) {
        printStatus('WARNING', `Embedding worker processes still running: ${embeddingProcesses.length} (may indicate incomplete shutdown)`);
        console.log('         Run: npm run shutdown to clean up');
    } else {
        printStatus('OK', 'No orphaned embedding processes');
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

async function checkPerformanceHealth() {
    console.log('\n⚡ 3. Performance Health Check');
    console.log('----------------------------');
    
    try {
        const { stdout } = await execPromise('npm run --silent validate:performance');
        
        // Check test results
        if (stdout.includes('Tests Passed: 4/4')) {
            printStatus('OK', 'All performance tests passed');
        } else {
            printStatus('WARNING', 'Some performance tests failed');
        }
        
        // Check storage performance
        const storageMatch = stdout.match(/Status Check: ([0-9.]+)ms/);
        if (storageMatch) {
            const storageTime = parseFloat(storageMatch[1]);
            if (storageTime < 10) {
                printStatus('OK', `Storage operations fast (${storageTime} ms)`);
            } else {
                printStatus('WARNING', `Storage operations slower than expected (${storageTime} ms)`);
            }
        }
        
        // Check cache loading
        const cacheMatch = stdout.match(/Cache Detection: ([0-9]+)ms/);
        if (cacheMatch) {
            const cacheTime = parseInt(cacheMatch[1]);
            if (cacheTime < 5000) {
                printStatus('OK', `Cache loading efficient (${cacheTime} ms)`);
            } else {
                printStatus('WARNING', `Cache loading slower than expected (${cacheTime} ms)`);
            }
        }
        
    } catch (error) {
        printStatus('WARNING', 'Performance validation completed with warnings');
    }
}

async function checkSystemResources() {
    console.log('\n🔧 4. System Resources');
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
    await checkProcessHealth();
    await checkStorageHealth();
    await checkPerformanceHealth();
    await checkSystemResources();
    
    console.log('\n📊 Health Check Summary');
    console.log('======================');
    
    if (issues === 0 && warnings === 0) {
        printStatus('OK', 'System is healthy! All checks passed.');
    } else if (issues === 0) {
        printStatus('WARNING', `System is mostly healthy with ${warnings} warnings`);
        console.log('\n💡 Recommendations:');
        console.log('   • Warnings are usually minor and often resolve automatically');
        console.log('   • Consider running: npm run storage:sync if storage issues persist');
        console.log('   • Run: npm run shutdown followed by npm run startup for a clean restart');
    } else {
        printStatus('ERROR', `System has ${issues} critical issues and ${warnings} warnings`);
        console.log('\n🔧 Troubleshooting:');
        console.log('   • Run: npm run shutdown to clean up processes');
        console.log('   • Try: npm run cache:clear-all for storage issues');
        console.log('   • Check logs in: logs/cortex-server.log');
        console.log('   • Restart with: npm run startup -- --rebuild');
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