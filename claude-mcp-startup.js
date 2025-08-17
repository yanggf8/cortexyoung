#!/usr/bin/env node

const { spawn } = require('child_process');
const path = require('path');
const http = require('http');

// Configuration
const REPO_PATH = '/home/yanggf/a/cortexyoung';
const SERVER_PORT = 8765;
const MCP_ENDPOINT = '/mcp';
const MAX_STARTUP_TIME = 60000; // 60 seconds

let serverProcess = null;

// Function to check if server is responsive
function checkServerHealth() {
  return new Promise((resolve) => {
    const req = http.request({
      hostname: 'localhost',
      port: SERVER_PORT,
      path: '/health',
      method: 'GET',
      timeout: 2000
    }, (res) => {
      resolve(res.statusCode === 200);
    });
    
    req.on('error', () => resolve(false));
    req.on('timeout', () => {
      req.destroy();
      resolve(false);
    });
    
    req.end();
  });
}

// Function to start the server
async function startServer() {
  console.log('🚀 Starting Cortex MCP Server...');
  
  // Start the server process
  serverProcess = spawn('npm', ['run', 'server'], {
    cwd: REPO_PATH,
    stdio: ['pipe', 'pipe', 'pipe'],
    env: {
      ...process.env,
      PORT: SERVER_PORT.toString()
    }
  });
  
  // Handle server output
  serverProcess.stdout.on('data', (data) => {
    const output = data.toString();
    if (output.includes('CORTEX MCP SERVER READY')) {
      console.log('✅ Server ready');
    }
  });
  
  serverProcess.stderr.on('data', (data) => {
    const output = data.toString();
    // Only log important errors, not verbose startup logs
    if (output.includes('ERROR') || output.includes('FATAL')) {
      console.error('❌ Server error:', output.trim());
    }
  });
  
  // Wait for server to be ready
  const startTime = Date.now();
  while (Date.now() - startTime < MAX_STARTUP_TIME) {
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    if (await checkServerHealth()) {
      console.log('🎉 Cortex MCP Server is ready and healthy!');
      return true;
    }
  }
  
  console.error('❌ Server failed to start within timeout period');
  return false;
}

// Function to handle graceful shutdown
function shutdown() {
  console.log('🛑 Shutting down Cortex MCP Server...');
  
  if (serverProcess) {
    serverProcess.kill('SIGTERM');
    
    setTimeout(() => {
      if (serverProcess && !serverProcess.killed) {
        console.log('⚡ Force killing server process...');
        serverProcess.kill('SIGKILL');
      }
    }, 5000);
  }
  
  process.exit(0);
}

// Handle process signals
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
process.on('exit', shutdown);

// Main execution
async function main() {
  // Check if server is already running
  if (await checkServerHealth()) {
    console.log('✅ Cortex MCP Server is already running');
    return;
  }
  
  // Start the server
  const success = await startServer();
  
  if (!success) {
    console.error('❌ Failed to start Cortex MCP Server');
    process.exit(1);
  }
  
  // Keep the process alive
  console.log('📡 MCP Server running at http://localhost:' + SERVER_PORT + MCP_ENDPOINT);
  console.log('🔧 Press Ctrl+C to stop');
  
  // Keep alive
  setInterval(async () => {
    if (!(await checkServerHealth())) {
      console.error('❌ Server health check failed, restarting...');
      await startServer();
    }
  }, 30000); // Check every 30 seconds
}

// Run if this file is executed directly
if (require.main === module) {
  main().catch(console.error);
}