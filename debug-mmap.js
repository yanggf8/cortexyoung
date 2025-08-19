#!/usr/bin/env node

/**
 * Debug script for memory-mapped cache
 */

const { MemoryMappedCache } = require('./dist/memory-mapped-cache.js');
const crypto = require('crypto');

async function debugTest() {
  console.log('🔍 Debugging Memory-Mapped Cache\n');
  
  const cache = MemoryMappedCache.getInstance('./.cortex/debug-mmap', 10, 384);
  await cache.initialize();
  
  // Create simple test data
  const testText = "simple test";
  const contentHash = crypto.createHash('sha256').update(testText).digest('hex');
  const embedding = new Float32Array(384);
  for (let i = 0; i < 384; i++) {
    embedding[i] = i / 384; // Simple pattern: 0, 0.0026, 0.0052, ...
  }
  
  console.log('📊 Initial stats:', cache.getStats());
  console.log('🔑 Content hash:', contentHash);
  console.log('📏 Embedding length:', embedding.length);
  console.log('💻 First few values:', Array.from(embedding.slice(0, 5)));
  
  // Test cache miss first
  console.log('\n1️⃣ Testing cache miss...');
  const miss = cache.get(contentHash);
  console.log('   Result:', miss);
  console.log('   Stats after miss:', cache.getStats());
  
  // Test cache set
  console.log('\n2️⃣ Testing cache set...');
  const setResult = cache.set(contentHash, embedding);
  console.log('   Set result:', setResult);
  console.log('   Stats after set:', cache.getStats());
  
  // Test cache hit
  console.log('\n3️⃣ Testing cache hit...');
  const hit = cache.get(contentHash);
  console.log('   Hit result:', hit ? 'Found' : 'Not found');
  if (hit) {
    console.log('   Hit length:', hit.length);
    console.log('   First few values:', Array.from(hit.slice(0, 5)));
    
    // Check if data matches
    let matches = true;
    for (let i = 0; i < Math.min(10, embedding.length); i++) {
      if (Math.abs(hit[i] - embedding[i]) > 0.0001) {
        matches = false;
        console.log(`   ❌ Mismatch at index ${i}: expected ${embedding[i]}, got ${hit[i]}`);
        break;
      }
    }
    console.log('   Data matches:', matches);
  }
  
  console.log('\n📊 Final stats:', cache.getStats());
  
  // Test another hash to see if it's a hash issue
  console.log('\n4️⃣ Testing with different hash...');
  const testText2 = "another test";
  const contentHash2 = crypto.createHash('sha256').update(testText2).digest('hex');
  const embedding2 = new Float32Array(384);
  embedding2.fill(0.5);
  
  console.log('🔑 Content hash 2:', contentHash2);
  cache.set(contentHash2, embedding2);
  const hit2 = cache.get(contentHash2);
  console.log('   Second hash hit:', hit2 ? 'Found' : 'Not found');
  
  await cache.close();
}

debugTest().catch(console.error);