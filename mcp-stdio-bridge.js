#!/usr/bin/env node

const http = require('http');
const { spawn } = require('child_process');

// Configuration
const REPO_PATH = '/home/yanggf/a/cortexyoung';
const SERVER_PORT = 8765;
const MCP_ENDPOINT = '/mcp';

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

// Function to start the HTTP server
async function startHTTPServer() {
  // Check if already running
  if (await checkServerHealth()) {
    return true;
  }
  
  // Start the server process
  serverProcess = spawn('npm', ['run', 'server'], {
    cwd: REPO_PATH,
    stdio: ['pipe', 'pipe', 'pipe'],
    env: {
      ...process.env,
      PORT: SERVER_PORT.toString()
    }
  });
  
  // Wait for server to be ready
  const maxWait = 30000; // 30 seconds
  const startTime = Date.now();
  
  while (Date.now() - startTime < maxWait) {
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    if (await checkServerHealth()) {
      return true;
    }
  }
  
  return false;
}

// Function to forward MCP requests to HTTP server
function forwardMCPRequest(mcpRequest) {
  return new Promise((resolve, reject) => {
    const postData = JSON.stringify(mcpRequest);
    
    const options = {
      hostname: 'localhost',
      port: SERVER_PORT,
      path: MCP_ENDPOINT,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(postData)
      }
    };
    
    const req = http.request(options, (res) => {
      let data = '';
      res.on('data', (chunk) => {
        data += chunk;
      });
      
      res.on('end', () => {
        try {
          const response = JSON.parse(data);
          resolve(response);
        } catch (e) {
          reject(new Error(`Invalid JSON response: ${e.message}`));
        }
      });
    });
    
    req.on('error', (err) => {
      reject(new Error(`HTTP request failed: ${err.message}`));
    });
    
    req.write(postData);
    req.end();
  });
}

// Handle graceful shutdown
function shutdown() {
  if (serverProcess) {
    serverProcess.kill('SIGTERM');
    setTimeout(() => {
      if (serverProcess && !serverProcess.killed) {
        serverProcess.kill('SIGKILL');
      }
    }, 3000);
  }
}

process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
process.on('exit', shutdown);

// Main stdio-to-HTTP bridge
async function main() {
  // Start the HTTP server
  const serverReady = await startHTTPServer();
  
  if (!serverReady) {
    console.error('Failed to start HTTP server');
    process.exit(1);
  }
  
  // Set up stdio communication with Claude Code
  process.stdin.setEncoding('utf8');
  let buffer = '';
  
  process.stdin.on('data', async (chunk) => {
    buffer += chunk;
    
    // Process complete JSON messages (one per line)
    const lines = buffer.split('\n');
    buffer = lines.pop() || ''; // Keep incomplete line in buffer
    
    for (const line of lines) {
      if (line.trim()) {
        try {
          const mcpRequest = JSON.parse(line);
          const response = await forwardMCPRequest(mcpRequest);
          
          // Send response back to Claude Code via stdout
          process.stdout.write(JSON.stringify(response) + '\n');
        } catch (error) {
          // Send error response
          const errorResponse = {
            jsonrpc: '2.0',
            error: {
              code: -32603,
              message: 'Internal error',
              data: error.message
            },
            id: null
          };
          
          try {
            const mcpRequest = JSON.parse(line);
            errorResponse.id = mcpRequest.id;
          } catch (e) {
            // Could not parse request
          }
          
          process.stdout.write(JSON.stringify(errorResponse) + '\n');
        }
      }
    }
  });
  
  process.stdin.on('end', () => {
    shutdown();
    process.exit(0);
  });
}

// Run the bridge
main().catch((error) => {
  console.error('Bridge failed:', error);
  process.exit(1);
});