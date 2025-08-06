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

// Function to check server health via HTTP
async function checkServerHealthHTTP(port) {
    const http = require('http');
    
    return new Promise((resolve) => {
        const timeout = setTimeout(() => {
            resolve({
                responding: false,
                method: 'HTTP',
                error: 'HTTP timeout after 3 seconds'
            });
        }, 3000);
        
        const options = {
            hostname: 'localhost',
            port: port,
            path: '/health',
            method: 'GET',
            timeout: 2500
        };
        
        const req = http.request(options, (res) => {
            clearTimeout(timeout);
            
            if (res.statusCode === 200) {
                let data = '';
                res.on('data', chunk => data += chunk);
                res.on('end', () => {
                    try {
                        const healthData = JSON.parse(data);
                        resolve({
                            responding: true,
                            method: 'HTTP',
                            data: healthData
                        });
                    } catch (error) {
                        resolve({
                            responding: true,
                            method: 'HTTP',
                            data: { status: 'healthy', raw: data }
                        });
                    }
                });
            } else {
                resolve({
                    responding: false,
                    method: 'HTTP',
                    error: `HTTP ${res.statusCode}`
                });
            }
        });
        
        req.on('error', (error) => {
            clearTimeout(timeout);
            resolve({
                responding: false,
                method: 'HTTP',
                error: error.message
            });
        });
        
        req.on('timeout', () => {
            clearTimeout(timeout);
            req.destroy();
            resolve({
                responding: false,
                method: 'HTTP',
                error: 'Request timeout'
            });
        });
        
        req.end();
    });
}

// Function to get startup progress via HTTP
async function getStartupProgressHTTP(port) {
    const http = require('http');
    
    return new Promise((resolve) => {
        const timeout = setTimeout(() => resolve(null), 2000);
        
        const options = {
            hostname: 'localhost',
            port: port,
            path: '/progress',
            method: 'GET',
            timeout: 1500
        };
        
        const req = http.request(options, (res) => {
            clearTimeout(timeout);
            
            if (res.statusCode === 200) {
                let data = '';
                res.on('data', chunk => data += chunk);
                res.on('end', () => {
                    try {
                        resolve(JSON.parse(data));
                    } catch (error) {
                        resolve(null);
                    }
                });
            } else {
                resolve(null);
            }
        });
        
        req.on('error', () => {
            clearTimeout(timeout);
            resolve(null);
        });
        
        req.on('timeout', () => {
            clearTimeout(timeout);
            req.destroy();
            resolve(null);
        });
        
        req.end();
    });
}

// Function to show startup progress via HTTP
async function showStartupProgressHTTP(port) {
    console.log('\n   📋 Startup Progress:');
    
    // Try to get progress from the server via HTTP
    const progressData = await getStartupProgressHTTP(port);
    
    if (progressData && progressData.stages) {
        console.log('   🎯 Live Progress from Server (via HTTP):');
        const stages = progressData.stages;
        let stepNum = 1;
        
        for (const [stageName, stageData] of Object.entries(stages)) {
            const status = stageData.status;
            const duration = stageData.duration;
            
            if (status === 'completed') {
                console.log(`   ✅ [Step ${stepNum}/10] ${stageName} - completed (${duration}ms)`);
            } else if (status === 'in_progress') {
                const progress = stageData.progress || 0;
                console.log(`   🔄 [Step ${stepNum}/10] ${stageName} - ${progress}% complete`);
                if (stageData.details) {
                    console.log(`      Details: ${stageData.details}`);
                }
            } else {
                console.log(`   ⏳ [Step ${stepNum}/10] ${stageName} - pending`);
            }
            stepNum++;
        }
        
        if (progressData.eta) {
            console.log(`   ⏱️  Estimated time remaining: ${progressData.eta} seconds`);
        }
        
        if (progressData.currentStage) {
            console.log(`   📍 Current: ${progressData.currentStage} (${progressData.overallProgress || 0}% overall)`);
        }
        
        return;
    }
    
    // Fallback: Parse recent logs for startup information
    const logPaths = [
        'logs/cortex-server.log',
        'nohup.out',
        '/tmp/cortex-startup.log'
    ];
    
    let recentLogs = null;
    for (const logPath of logPaths) {
        try {
            if (require('fs').existsSync(logPath)) {
                const { stdout } = await execPromise(`tail -20 "${logPath}"`);
                recentLogs = stdout;
                console.log(`   📝 Recent logs from ${logPath}:`);
                break;
            }
        } catch (error) {
            continue;
        }
    }
    
    if (recentLogs) {
        // Parse for startup stages
        const lines = recentLogs.split('\n').filter(line => line.trim());
        const recentSteps = lines
            .filter(line => line.includes('[Step') || line.includes('✅') || line.includes('🚀'))
            .slice(-5); // Show last 5 startup-related lines
        
        if (recentSteps.length > 0) {
            recentSteps.forEach(step => {
                const cleanStep = step.replace(/\x1b\[[0-9;]*m/g, ''); // Remove ANSI colors
                console.log(`   ${cleanStep}`);
            });
        } else {
            // Show general recent activity
            const recentActivity = lines.slice(-3);
            recentActivity.forEach(line => {
                const cleanLine = line.replace(/\x1b\[[0-9;]*m/g, '');
                if (cleanLine.length > 100) {
                    console.log(`   ${cleanLine.substring(0, 97)}...`);
                } else {
                    console.log(`   ${cleanLine}`);
                }
            });
        }
    } else {
        console.log('   ⚠️  No accessible log files found');
        console.log('   💡 Server may be starting up - logs will appear shortly');
    }
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
        
        // Get the main server PID (usually ts-node process)
        const mainServerPids = await findProcesses('ts-node.*server|node.*server\\.js');
        const serverPid = mainServerPids.length > 0 ? mainServerPids[0] : null;
        
        if (serverPid) {
            // Try HTTP health check first
            const healthCheck = await checkServerHealthHTTP(PORT);
            
            if (healthCheck.responding) {
                printStatus('OK', `✅ SERVER RUNNING - Responding via ${healthCheck.method}`);
                
                const healthData = healthCheck.data;
                if (healthData) {
                    printStatus('OK', `Server status: ${healthData.status}`);
                    
                    // Handle different health data formats
                    if (healthData.uptime !== undefined) {
                        const memoryInfo = healthData.memoryUsage && healthData.memoryUsage.heapUsed 
                            ? `Memory: ${Math.round(healthData.memoryUsage.heapUsed / 1024 / 1024)}MB`
                            : 'Memory: N/A';
                        printStatus('INFO', `Uptime: ${Math.round(healthData.uptime)}s, ${memoryInfo}`);
                    }
                    
                    if (healthData.version) {
                        printStatus('INFO', `Server version: ${healthData.version}`);
                    }
                    
                    if (healthData.startup) {
                        const startup = healthData.startup;
                        printStatus('INFO', `Startup: ${startup.currentStage || 'Ready'} (${startup.overallProgress || 100}% complete)`);
                    }
                }
            } else {
                printStatus('WARNING', `⚠️ SERVER STARTING - Process running but not responding via HTTP`);
                printStatus('INFO', `HTTP Error: ${healthCheck.error}`);
                
                // Show startup progress if available
                await showStartupProgressHTTP(PORT);
            }
        } else {
            printStatus('WARNING', '⚠️ Server process detected but cannot identify main PID');
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
    const healthCheck = serverProcesses.length > 0 ? await checkServerHealthHTTP(PORT) : { responding: false };
    
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
        
        // Show additional startup info in summary via HTTP
        const progressData = await getStartupProgressHTTP(PORT);
        
        if (progressData) {
            if (progressData.currentStage) {
                printStatus('INFO', `Current Stage: ${progressData.currentStage} (${progressData.overallProgress || 0}% complete)`);
                if (progressData.eta) {
                    printStatus('INFO', `Estimated completion: ${progressData.eta} seconds`);
                }
            }
        }
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