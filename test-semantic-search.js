#!/usr/bin/env node

const http = require('http');

const mcpRequest = {
  jsonrpc: '2.0',
  id: 2,
  method: 'tools/call',
  params: {
    name: 'semantic_search',
    arguments: {
      query: 'semantic search indexer',
      max_chunks: 5
    }
  }
};

const postData = JSON.stringify(mcpRequest);

const options = {
  hostname: 'localhost',
  port: 8765,
  path: '/mcp',
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(postData)
  }
};

console.log('🔍 Testing semantic search functionality...');
console.log(`📋 Query: "${mcpRequest.params.arguments.query}"`);
console.log('');

const req = http.request(options, (res) => {
  console.log(`📊 Status Code: ${res.statusCode}`);
  
  let data = '';
  res.on('data', (chunk) => {
    data += chunk;
  });
  
  res.on('end', () => {
    try {
      const response = JSON.parse(data);
      
      if (response.result && response.result.content) {
        const result = response.result.content;
        console.log(`✅ SUCCESS! Found ${result.chunks?.length || 0} relevant chunks`);
        console.log(`🎯 Context optimization hints:`, result.context_optimization);
        console.log('');
        console.log('📄 Sample chunks:');
        
        if (result.chunks) {
          result.chunks.slice(0, 2).forEach((chunk, index) => {
            console.log(`   ${index + 1}. ${chunk.file_path}:${chunk.start_line}`);
            console.log(`      ${chunk.content.substring(0, 100)}...`);
          });
        }
      } else if (response.error) {
        console.log('❌ Error:', response.error);
      } else {
        console.log('📋 Response:', JSON.stringify(response, null, 2));
      }
    } catch (e) {
      console.log('❌ Parse error:', e.message);
      console.log('📄 Raw response:', data);
    }
  });
});

req.on('error', (e) => {
  console.error(`❌ Request failed: ${e.message}`);
});

req.write(postData);
req.end();