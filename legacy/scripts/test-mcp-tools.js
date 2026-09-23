#!/usr/bin/env node

/**
 * Test script to verify MCP tools work correctly
 * Usage: node scripts/test-mcp-tools.js [project-path]
 */

import { spawn } from 'child_process';
import { join } from 'path';

const projectPath = process.argv[2] || process.cwd();

console.log('🧪 Testing MCP Tools');
console.log('====================');
console.log(`Project Path: ${projectPath}\n`);

// Test MCP tools by sending JSON-RPC requests
const testTools = [
  {
    name: 'list_tools',
    request: {
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/list'
    }
  },
  {
    name: 'get_project_context',
    request: {
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/call',
      params: {
        name: 'get_project_context',
        arguments: {}
      }
    }
  },
  {
    name: 'get_implementation_patterns',
    request: {
      jsonrpc: '2.0',
      id: 3,
      method: 'tools/call',
      params: {
        name: 'get_implementation_patterns',
        arguments: {}
      }
    }
  },
  {
    name: 'get_critical_guardrails',
    request: {
      jsonrpc: '2.0',
      id: 4,
      method: 'tools/call',
      params: {
        name: 'get_critical_guardrails',
        arguments: {}
      }
    }
  }
];

async function testMCPTools() {
  console.log('🚀 Starting MCP Server for tool testing...\n');
  
  const serverPath = join(process.cwd(), 'dist', 'server.js');
  const server = spawn('node', [serverPath], {
    cwd: projectPath,
    stdio: ['pipe', 'pipe', 'pipe']
  });

  let responses = [];
  let responseCount = 0;

  server.stdout.on('data', (data) => {
    const lines = data.toString().split('\n').filter(line => line.trim());
    
    for (const line of lines) {
      try {
        const response = JSON.parse(line);
        if (response.jsonrpc && response.id) {
          responses[response.id] = response;
          responseCount++;
          
          console.log(`📨 Received response for ID ${response.id}`);
          
          if (responseCount >= testTools.length) {
            // All responses received
            displayResults();
            server.kill('SIGTERM');
          }
        }
      } catch (error) {
        // Non-JSON output (logs, etc.) - ignore
      }
    }
  });

  server.stderr.on('data', (data) => {
    // Server logs - can ignore for testing
  });

  server.on('close', (code) => {
    console.log(`\n✅ MCP Server test completed (exit code: ${code})`);
    process.exit(code || 0);
  });

  // Wait for server to initialize
  setTimeout(() => {
    console.log('📤 Sending tool requests...\n');
    
    // Send test requests
    for (const test of testTools) {
      const request = JSON.stringify(test.request) + '\n';
      server.stdin.write(request);
    }
  }, 2000);

  function displayResults() {
    console.log('\n📊 MCP Tool Test Results:');
    console.log('=========================\n');
    
    for (const test of testTools) {
      const response = responses[test.request.id];
      
      if (response && response.result) {
        console.log(`✅ ${test.name}: SUCCESS`);
        
        if (test.name === 'list_tools') {
          const tools = response.result.tools || [];
          console.log(`   Tools available: ${tools.length}`);
          tools.forEach(tool => console.log(`   - ${tool.name}: ${tool.description}`));
        } else if (response.result.content && response.result.content[0]) {
          const content = response.result.content[0].text;
          const preview = content.substring(0, 100) + (content.length > 100 ? '...' : '');
          console.log(`   Response: ${preview}`);
        }
      } else if (response && response.error) {
        console.log(`❌ ${test.name}: ERROR - ${response.error.message}`);
      } else {
        console.log(`⏱️ ${test.name}: NO RESPONSE`);
      }
      console.log();
    }
  }

  // Timeout after 10 seconds
  setTimeout(() => {
    console.log('⏰ Test timeout - stopping server');
    server.kill('SIGTERM');
  }, 10000);
}

testMCPTools().catch(error => {
  console.error('💥 Test failed:', error);
  process.exit(1);
});