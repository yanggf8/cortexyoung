#!/usr/bin/env node

const http = require('http');

const mcpRequest = {
  jsonrpc: '2.0',
  id: 1,
  method: 'tools/list',
  params: {}
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

console.log('🧪 Testing MCP connection to Cortex server...');
console.log(`📡 Endpoint: http://localhost:8765`);
console.log(`📋 Request: ${postData}`);
console.log('');

const req = http.request(options, (res) => {
  console.log(`📊 Status Code: ${res.statusCode}`);
  console.log(`📝 Headers:`, res.headers);
  console.log('');
  
  let data = '';
  res.on('data', (chunk) => {
    data += chunk;
  });
  
  res.on('end', () => {
    console.log('📋 Response:');
    try {
      const response = JSON.parse(data);
      console.log(JSON.stringify(response, null, 2));
      
      if (response.result && response.result.tools) {
        console.log('');
        console.log(`✅ SUCCESS! Found ${response.result.tools.length} MCP tools:`);
        response.result.tools.forEach((tool, index) => {
          console.log(`   ${index + 1}. ${tool.name} - ${tool.description.substring(0, 60)}...`);
        });
      }
    } catch (e) {
      console.log('📄 Raw response:', data);
    }
  });
});

req.on('error', (e) => {
  console.error(`❌ Request failed: ${e.message}`);
});

req.write(postData);
req.end();