#!/usr/bin/env node

// Simple test script for MCP server
const http = require('http');

async function testMCPServer(port = 8765) {
  console.log(`Testing MCP server on port ${port}...`);
  
  const baseUrl = `http://localhost:${port}`;
  
  // Test 1: Health check
  console.log('\n1. Testing health endpoint...');
  try {
    const healthResponse = await fetch(`${baseUrl}/health`);
    const healthData = await healthResponse.json();
    console.log('✅ Health check:', healthData);
  } catch (error) {
    console.log('❌ Health check failed:', error.message);
    return;
  }
  
  // Test 2: Initialize
  console.log('\n2. Testing MCP initialization...');
  try {
    const initResponse = await makeJsonRpcRequest(baseUrl, {
      jsonrpc: '2.0',
      method: 'initialize',
      params: {
        protocolVersion: '2025-01-07',
        capabilities: {},
        clientInfo: {
          name: 'test-client',
          version: '1.0.0'
        }
      },
      id: 1
    });
    console.log('✅ Initialize:', initResponse);
  } catch (error) {
    console.log('❌ Initialize failed:', error.message);
  }
  
  // Test 3: List tools
  console.log('\n3. Testing tools/list...');
  try {
    const toolsResponse = await makeJsonRpcRequest(baseUrl, {
      jsonrpc: '2.0',
      method: 'tools/list',
      params: {},
      id: 2
    });
    console.log('✅ Tools list:', JSON.stringify(toolsResponse, null, 2));
  } catch (error) {
    console.log('❌ Tools list failed:', error.message);
  }
  
  // Test 4: Call semantic_search tool
  console.log('\n4. Testing semantic_search tool...');
  try {
    const searchResponse = await makeJsonRpcRequest(baseUrl, {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'semantic_search',
        arguments: {
          query: 'test search',
          max_chunks: 5
        }
      },
      id: 3
    });
    console.log('✅ Semantic search:', JSON.stringify(searchResponse, null, 2));
  } catch (error) {
    console.log('❌ Semantic search failed:', error.message);
  }
  
  console.log('\nMCP server test completed!');
}

async function makeJsonRpcRequest(baseUrl, payload) {
  const response = await fetch(`${baseUrl}/mcp`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(payload)
  });
  
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${response.statusText}`);
  }
  
  return await response.json();
}

// Run the test
if (require.main === module) {
  const port = process.argv[2] ? parseInt(process.argv[2]) : 8765;
  testMCPServer(port).catch(console.error);
}

module.exports = { testMCPServer };