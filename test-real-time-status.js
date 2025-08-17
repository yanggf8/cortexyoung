#!/usr/bin/env node

const http = require('http');

const mcpRequest = {
  jsonrpc: '2.0',
  id: 3,
  method: 'tools/call',
  params: {
    name: 'real_time_status',
    arguments: {}
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

console.log('📊 Testing real-time status functionality...');
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
      
      if (response.result && response.result.content && response.result.content[0]) {
        // Parse the JSON text from the content
        const statusText = response.result.content[0].text;
        const status = JSON.parse(statusText);
        console.log('✅ Real-time Status:');
        console.log(`   📡 Real-time enabled: ${status.realTimeEnabled}`);
        console.log(`   🔄 File watching active: ${status.fileWatchingActive}`);
        console.log(`   📊 Context freshness: ${status.contextFreshness}`);
        console.log(`   ⏱️  Last update: ${status.lastUpdate}`);
        console.log(`   🧽 Pending updates: ${status.pendingUpdates}`);
        console.log(`   🏁 Status: ${status.status}`);
        console.log('');
        console.log('🔧 System Info:');
        console.log(`   📡 Real-time supported: ${status.systemInfo?.realTimeUpdatesSupported}`);
        console.log(`   👀 File watcher: ${status.systemInfo?.fileWatcherType}`);
        console.log(`   🧠 Semantic filtering: ${status.systemInfo?.semanticFilteringEnabled}`);
      } else if (response.error) {
        console.log('❌ Error:', response.error);
      } else {
        console.log('📋 Full Response:', JSON.stringify(response, null, 2));
      }
      
      // Always show the raw response for debugging
      console.log('');
      console.log('🔍 Debug - Full Response Structure:');
      console.log(JSON.stringify(response, null, 2));
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