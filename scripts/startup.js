#!/usr/bin/env node

const { spawn, exec } = require('child_process');
const util = require('util');
const execPromise = util.promisify(exec);
const fs = require('fs');

console.log('🚀 Cortex Startup Script');
console.log('========================');

// Parse command line arguments
const args = process.argv.slice(2);
let mode = 'server';
let port = process.env.PORT || 8765;
let rebuild = false;
let healthCheck = true;
let background = false;

// Parse arguments
for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg.startsWith('--mode=')) {
        mode = arg.split('=')[1];
    } else if (arg.startsWith('--port=')) {
        port = arg.split('=')[1];
    } else if (arg === '--rebuild') {
        rebuild = true;
    } else if (arg === '--no-health-check') {
        healthCheck = false;
    } else if (arg === '--background') {
        background = true;
    } else if (arg === '--help') {
        console.log('\nUsage: npm run startup [-- OPTIONS]');
        console.log('       node scripts/startup.js [OPTIONS]');
        console.log('\nOptions:');
        console.log('  --mode=MODE           Startup mode (server, demo, build)');
        console.log('  --port=PORT           Server port (default: 8765)');
        console.log('  --rebuild            Force full rebuild');
        console.log('  --no-health-check    Skip pre-startup health check');
        console.log('  --background         Run in background (server modes only)');
        console.log('  --help               Show this help message');
        console.log('\nModes:');
        console.log('  server               Start development server (default)');
        console.log('  demo                 Run indexing demo');
        console.log('  build                Build and start production server');
        console.log('\nExamples:');
        console.log('  npm run startup                              # Default server start');
        console.log('  npm run startup -- --mode=demo              # Run indexing demo');
        console.log('  npm run startup -- --rebuild                # Force rebuild server');
        console.log('  npm run startup -- --port=9000              # Custom port');
        console.log('  npm run startup -- --background             # Run in background');
        process.exit(0);
    }
}

// Function to find processes
async function findProcesses(pattern) {
    try {
        const { stdout } = await execPromise(`pgrep -f "${pattern}"`);
        const pids = stdout.trim().split('\n').filter(pid => pid && pid !== '');
        return pids;
    } catch (error) {
        return [];
    }
}

// Function to check port
function checkPort(port) {
    return new Promise((resolve) => {
        const server = require('net').createServer();
        server.listen(port, () => {
            server.once('close', () => resolve({ available: true }));
            server.close();
        });
        server.on('error', () => resolve({ available: false }));
    });
}

async function preStartupHealthCheck() {
    if (!healthCheck) return;
    
    console.log('\n🩺 Pre-startup health check...');
    
    // Check for existing processes
    const existingServer = await findProcesses('npm.*server|ts-node.*server|node.*server\\.js');
    if (existingServer.length > 0) {
        console.log(`⚠️  Found existing server processes: ${existingServer.join(', ')}`);
        console.log('🛑 Running cleanup first...');
        
        // Run shutdown script
        try {
            await execPromise('node scripts/shutdown.js');
            console.log('✅ Cleanup completed');
            await new Promise(resolve => setTimeout(resolve, 3000));
        } catch (error) {
            console.log('⚠️  Cleanup had some issues, continuing anyway...');
        }
    } else {
        console.log('✅ No conflicting processes found');
    }
    
    // Check port availability
    const portCheck = await checkPort(port);
    if (!portCheck.available) {
        console.log(`⚠️  Port ${port} is in use`);
        console.log('🔧 Attempting to free port...');
        try {
            await execPromise(`lsof -ti:${port} | xargs -r kill -TERM`);
            await new Promise(resolve => setTimeout(resolve, 2000));
        } catch (error) {
            console.log('⚠️  Could not free port automatically');
        }
    } else {
        console.log(`✅ Port ${port} is available`);
    }
    
    // Quick storage health check
    console.log('📊 Storage health check...');
    try {
        await execPromise('npm run --silent storage:status');
        console.log('✅ Storage system healthy');
    } catch (error) {
        console.log('⚠️  Storage check completed with warnings');
    }
}

function determineCommand(mode, rebuild) {
    switch (mode) {
        case 'server':
            return rebuild ? 'npm run server:rebuild' : 'npm run server';
        case 'demo':
            return rebuild ? 'npm run demo:reindex' : 'npm run demo';
        case 'build':
            return 'npm run build && npm start';
        default:
            throw new Error(`Unknown mode: ${mode}`);
    }
}

function startProcess(command, background, port) {
    // Set environment variables
    const env = { 
        ...process.env, 
        PORT: port,
        LOG_FILE: 'logs/cortex-server.log'
    };
    
    // Create logs directory
    if (!fs.existsSync('logs')) {
        fs.mkdirSync('logs', { recursive: true });
    }
    
    const [cmd, ...args] = command.split(' ');
    
    if (background && (mode === 'server' || mode === 'build')) {
        console.log(`🎯 Starting server in background...`);
        console.log(`💻 Command: ${command}`);
        console.log(`🌐 Port: ${port}`);
        console.log(`📝 Logs: logs/cortex-server.log`);
        console.log(`🛑 Stop with: npm run shutdown`);
        
        // Create background process
        const child = spawn(cmd, args, {
            env,
            detached: true,
            stdio: ['ignore', fs.openSync('logs/cortex-server.log', 'a'), fs.openSync('logs/cortex-server.log', 'a')]
        });
        
        child.unref();
        
        console.log(`✅ Server started in background (PID: ${child.pid})`);
        console.log(`📊 Check status: npm run status`);
        console.log(`🩺 Check health: curl http://localhost:${port}/health`);
        
        return;
    } else {
        console.log(`🎯 Starting ${mode}...`);
        console.log(`💻 Command: ${command}`);
        if (mode === 'server' || mode === 'build') {
            console.log(`🌐 Port: ${port}`);
            console.log(`   Health endpoint: http://localhost:${port}/health`);
            console.log(`   Status endpoint: http://localhost:${port}/status`);
        }
        console.log(`📝 Logs: logs/cortex-server.log`);
        console.log('');
        console.log('⏹️  Press Ctrl+C to stop gracefully');
        console.log('⚠️  Or use: npm run shutdown for complete cleanup');
        console.log('');
        
        // Start interactive process
        const child = spawn(cmd, args, {
            env,
            stdio: 'inherit'
        });
        
        // Handle process exit
        child.on('exit', (code, signal) => {
            console.log('');
            if (code === 0) {
                console.log('✅ Process completed successfully');
            } else if (signal === 'SIGINT') {
                console.log('⏹️  Process stopped by user (Ctrl+C)');
            } else {
                console.log(`⚠️  Process exited with code: ${code}, signal: ${signal}`);
            }
            console.log('🎯 Startup script complete!');
        });
        
        // Handle script termination
        process.on('SIGINT', () => {
            console.log('\n🛑 Received Ctrl+C, stopping...');
            child.kill('SIGINT');
        });
        
        process.on('SIGTERM', () => {
            console.log('\n🛑 Received termination signal, stopping...');
            child.kill('SIGTERM');
        });
    }
}

async function main() {
    try {
        await preStartupHealthCheck();
        
        console.log(`\n🚀 Starting Cortex in '${mode}' mode...`);
        
        // Special handling for build mode
        if (mode === 'build') {
            console.log('🔧 Building project...');
            try {
                await execPromise('npm run build');
                console.log('✅ Build successful');
            } catch (error) {
                console.log('❌ Build failed');
                console.error(error.message);
                process.exit(1);
            }
        }
        
        const command = determineCommand(mode, rebuild);
        startProcess(command, background, port);
        
    } catch (error) {
        console.error('❌ Startup failed:', error.message);
        process.exit(1);
    }
}

main();