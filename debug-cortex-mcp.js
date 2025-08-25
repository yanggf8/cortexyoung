#!/usr/bin/env node

/**
 * Cortex MCP Debug Script
 * Helps diagnose connection issues and capture logs
 */

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

console.log('🔍 Cortex MCP Debug Script');
console.log('=============================');

// Check if cortex-mcp exists
const cortexMcpPath = path.join(__dirname, 'cortex-mcp.js');
if (!fs.existsSync(cortexMcpPath)) {
  console.error('❌ cortex-mcp.js not found');
  process.exit(1);
}

console.log('✅ cortex-mcp.js found');

// Check if dist/stdio-server.js exists
const stdioServerPath = path.join(__dirname, 'dist', 'stdio-server.js');
if (!fs.existsSync(stdioServerPath)) {
  console.error('❌ dist/stdio-server.js not found');
  process.exit(1);
}

console.log('✅ dist/stdio-server.js found');

// Test direct server startup
console.log('\n📡 Testing direct server startup...');
const testServer = spawn('node', [stdioServerPath, process.cwd()], {
  env: {
    ...process.env,
    MCP_MULTI_INSTANCE: 'true',
    CORTEX_SKIP_CLEANUP: 'true',
    CORTEX_DEBUG: 'true'
  }
});

let startupOutput = '';
let errorOutput = '';

testServer.stdout.on('data', (data) => {
  const output = data.toString();
  startupOutput += output;
  console.log('STDOUT:', output.trim());
});

testServer.stderr.on('data', (data) => {
  const output = data.toString();
  errorOutput += output;
  console.log('STDERR:', output.trim());
});

testServer.on('error', (error) => {
  console.error('❌ Server startup error:', error.message);
});

// Kill the test server after 5 seconds
setTimeout(() => {
  testServer.kill('SIGTERM');
  
  console.log('\n📊 Diagnostic Summary:');
  console.log('========================');
  
  if (startupOutput.includes('Connected and Ready')) {
    console.log('✅ Server starts successfully');
  } else {
    console.log('⚠️  Server may have startup issues');
  }
  
  if (errorOutput.includes('Error') || errorOutput.includes('Failed')) {
    console.log('⚠️  Errors detected in stderr output');
  }
  
  console.log('\n📝 Startup Output Length:', startupOutput.length, 'characters');
  console.log('📝 Error Output Length:', errorOutput.length, 'characters');
  
  // Check for common issues
  if (errorOutput.includes('ECONNREFUSED')) {
    console.log('🔍 Issue: Connection refused (likely centralized server not running)');
  }
  
  if (errorOutput.includes('EADDRINUSE')) {
    console.log('🔍 Issue: Port already in use');
  }
  
  if (errorOutput.includes('permission denied') || errorOutput.includes('EACCES')) {
    console.log('🔍 Issue: Permission denied');
  }
  
  // Process information
  console.log('\n🔧 Process Information:');
  console.log('Node.js version:', process.version);
  console.log('Platform:', process.platform);
  console.log('Architecture:', process.arch);
  console.log('Working directory:', process.cwd());
  
  console.log('\n💡 Troubleshooting Tips:');
  console.log('- If you see "Connection refused", this is expected (fallback mode)');
  console.log('- Server should show "Connected and Ready" message');
  console.log('- Multiple Claude Code instances should work with MCP_MULTI_INSTANCE=true');
  
  process.exit(0);
}, 5000);