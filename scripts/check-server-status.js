#!/usr/bin/env node

const http = require('http');
const { exec } = require('child_process');
const util = require('util');
const execPromise = util.promisify(exec);

const PORT = process.env.PORT || 8765;

console.log('📊 Cortex Server Status Check');
console.log('============================');

// Function to check if server is responding
function checkServerHealth(port) {
    return new Promise((resolve) => {
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
        
        request.setTimeout(5000, () => {
            request.destroy();
            resolve({
                responding: false,
                error: 'Request timeout after 5 seconds'
            });
        });
    });
}

// Function to check for running processes
async function checkProcesses() {
    const processes = {};
    
    const patterns = [
        { pattern: 'npm.*server', name: 'npm_server' },
        { pattern: 'ts-node.*server', name: 'ts_node_server' },
        { pattern: 'node.*server\\.js', name: 'compiled_server' },
        { pattern: 'npm.*demo', name: 'npm_demo' },
        { pattern: 'ts-node.*index', name: 'ts_node_indexer' },
        { pattern: 'node.*external-embedding-process', name: 'embedding_workers' }
    ];
    
    for (const { pattern, name } of patterns) {
        try {
            const { stdout } = await execPromise(`pgrep -f "${pattern}"`);
            const pids = stdout.trim().split('\n').filter(pid => pid && pid !== '');
            processes[name] = pids.length > 0 ? pids : [];
        } catch (error) {
            processes[name] = [];
        }
    }
    
    return processes;
}

// Function to check port availability
function checkPort(port) {
    return new Promise((resolve) => {
        const server = require('net').createServer();
        
        server.listen(port, () => {
            server.once('close', () => {
                resolve({ available: true });
            });
            server.close();
        });
        
        server.on('error', (error) => {
            resolve({ 
                available: false, 
                error: error.code === 'EADDRINUSE' ? 'Port in use' : error.message 
            });
        });
    });
}

async function main() {
    console.log(`🔍 Checking server on port ${PORT}...\n`);
    
    // 1. Check server health endpoint
    console.log('🌐 Server Health Check:');
    const healthCheck = await checkServerHealth(PORT);
    
    if (healthCheck.responding) {
        console.log(`   ✅ Server is responding (HTTP ${healthCheck.statusCode})`);
        
        try {
            const healthData = JSON.parse(healthCheck.data);
            if (healthData.status === 'healthy') {
                console.log('   ✅ Server reports healthy status');
                if (healthData.startup) {
                    console.log(`   📊 Startup: ${healthData.startup.stage} (${healthData.startup.progress}% complete)`);
                }
            } else {
                console.log(`   ⚠️  Server reports status: ${healthData.status}`);
            }
        } catch (error) {
            console.log('   ℹ️  Server responding but health data not parseable');
        }
    } else {
        console.log('   ❌ Server not responding');
        console.log(`   💬 Error: ${healthCheck.error}`);
    }
    
    // 2. Check running processes
    console.log('\n🔍 Process Check:');
    const processes = await checkProcesses();
    
    let totalProcesses = 0;
    for (const [name, pids] of Object.entries(processes)) {
        if (pids.length > 0) {
            console.log(`   ✅ ${name.replace(/_/g, ' ')}: ${pids.length} running (PIDs: ${pids.join(', ')})`);
            totalProcesses += pids.length;
        } else {
            console.log(`   ⚪ ${name.replace(/_/g, ' ')}: not running`);
        }
    }
    
    if (totalProcesses === 0) {
        console.log('   ℹ️  No Cortex processes currently running');
    }
    
    // 3. Check port availability
    console.log('\n🔌 Port Check:');
    const portCheck = await checkPort(PORT);
    
    if (portCheck.available) {
        console.log(`   ✅ Port ${PORT} is available`);
    } else {
        console.log(`   ⚠️  Port ${PORT} is not available (${portCheck.error})`);
    }
    
    // 4. Quick storage check
    console.log('\n🗄️  Storage Quick Check:');
    try {
        const { stdout } = await execPromise('npm run --silent storage:stats');
        
        // Parse basic info from storage stats
        const chunkMatch = stdout.match(/Chunks: (\d+)/);
        const relationshipMatch = stdout.match(/Relationships: (\d+)/);
        
        if (chunkMatch) {
            const chunks = parseInt(chunkMatch[1]);
            console.log(`   📊 Embeddings: ${chunks} chunks indexed`);
        }
        
        if (relationshipMatch) {
            const relationships = parseInt(relationshipMatch[1]);
            console.log(`   🔗 Relationships: ${relationships} relationships`);
            if (relationships === 0) {
                console.log('   ⚠️  No relationships found (may need rebuild)');
            }
        }
        
        console.log('   ✅ Storage system accessible');
        
    } catch (error) {
        console.log('   ⚠️  Storage check failed - may need initialization');
    }
    
    // 5. Summary and recommendations
    console.log('\n📋 Summary:');
    
    if (healthCheck.responding && totalProcesses > 0) {
        console.log('✅ Status: SERVER RUNNING');
        console.log('💡 Server is operational and responding to requests');
    } else if (totalProcesses > 0) {
        console.log('⚠️  Status: SERVER STARTING');
        console.log('💡 Processes are running but server may still be initializing');
        console.log('   Wait a few moments and check again');
    } else {
        console.log('❌ Status: SERVER NOT RUNNING');
        console.log('💡 Recommendations:');
        console.log('   • Start server: npm run server');
        console.log('   • Or use startup script: npm run startup');
        console.log('   • For background start: nohup npm run server > logs/server.log 2>&1 &');
    }
    
    // Exit codes for scripting
    if (healthCheck.responding) {
        process.exit(0); // Server healthy
    } else if (totalProcesses > 0) {
        process.exit(1); // Server starting
    } else {
        process.exit(2); // Server not running
    }
}

main().catch(error => {
    console.error('❌ Status check failed:', error.message);
    process.exit(3);
});