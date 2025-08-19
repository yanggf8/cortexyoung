#!/usr/bin/env node

/**
 * Debug script for memory-mapped cache hash storage
 */

const { MemoryMappedCache } = require('./dist/memory-mapped-cache.js');
const crypto = require('crypto');

async function debugHashStorage() {
  console.log('🔍 Debugging Hash Storage\n');
  
  const cache = MemoryMappedCache.getInstance('./.cortex/debug-mmap2', 10, 384);
  await cache.initialize();
  
  // Create test data
  const testText = "test123";
  const contentHash = crypto.createHash('sha256').update(testText).digest('hex');
  const embedding = new Float32Array(384);
  embedding.fill(0.5);
  
  console.log('🔑 Content hash:', contentHash);
  console.log('📏 Hash length:', contentHash.length);
  
  // Store in cache
  console.log('\n💾 Storing in cache...');
  const setResult = cache.set(contentHash, embedding);
  console.log('   Set result:', setResult);
  
  // Check current entries count
  const stats = cache.getStats();
  console.log('   Current entries:', stats.size);
  
  // Now try to retrieve
  console.log('\n🔍 Retrieving from cache...');
  const result = cache.get(contentHash);
  console.log('   Get result:', result ? 'Found' : 'Not found');
  
  // Let's also test what the internal findEntry method would return
  // We need to access private methods, so let's create our own simple test
  console.log('\n🧪 Manual hash test...');
  
  // Create a second cache instance to test shared memory
  const cache2 = MemoryMappedCache.getInstance('./.cortex/debug-mmap2', 10, 384);
  // Note: getInstance returns the same instance, so this is the same cache
  const result2 = cache2.get(contentHash);
  console.log('   Second instance result:', result2 ? 'Found' : 'Not found');
  
  await cache.close();
}

debugHashStorage().catch(console.error);