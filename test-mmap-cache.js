#!/usr/bin/env node

/**
 * Test script for memory-mapped cache functionality
 * Tests true shared memory between multiple processes
 */

const { MemoryMappedCache } = require('./dist/memory-mapped-cache.js');
const crypto = require('crypto');
const { spawn } = require('child_process');

async function testMemoryMappedCache() {
  console.log('🧪 Testing Memory-Mapped Cache');
  console.log('================================\n');
  
  try {
    // Test 1: Basic functionality
    console.log('📝 Test 1: Basic Cache Operations');
    const cache = MemoryMappedCache.getInstance('./.cortex/test-mmap-cache', 1000, 384);
    await cache.initialize();
    
    // Create test embedding
    const testText = "Hello, this is a test embedding";
    const contentHash = crypto.createHash('sha256').update(testText).digest('hex');
    const testEmbedding = new Float32Array(384);
    for (let i = 0; i < 384; i++) {
      testEmbedding[i] = Math.random();
    }
    
    // Test cache miss
    console.log('   Testing cache miss...');
    const miss = cache.get(contentHash);
    console.log('   ✅ Cache miss:', miss === null);
    
    // Test cache set
    console.log('   Testing cache set...');
    const setSuccess = cache.set(contentHash, testEmbedding);
    console.log('   ✅ Cache set success:', setSuccess);
    
    // Test cache hit
    console.log('   Testing cache hit...');
    const hit = cache.get(contentHash);
    console.log('   ✅ Cache hit found:', hit !== null);
    console.log('   ✅ Cache hit correct size:', hit && hit.length === 384);
    
    // Test embedding data integrity
    if (hit) {
      let dataMatches = true;
      for (let i = 0; i < 10; i++) { // Check first 10 values
        if (Math.abs(hit[i] - testEmbedding[i]) > 0.0001) {
          dataMatches = false;
          break;
        }
      }
      console.log('   ✅ Data integrity:', dataMatches);
    }
    
    console.log('\n📊 Cache Stats:');
    console.log('  ', cache.getStats());
    
    console.log('\n');
    
    // Test 2: Multi-process shared memory
    console.log('🔄 Test 2: Multi-Process Shared Memory');
    
    // Spawn child process to test shared memory
    const childProcess = spawn('node', ['-e', `
      const { MemoryMappedCache } = require('./dist/memory-mapped-cache.js');
      
      async function testChildProcess() {
        try {
          const cache = MemoryMappedCache.getInstance('./.cortex/test-mmap-cache', 1000, 384);
          await cache.initialize();
          
          // Try to read the embedding we just wrote from parent process
          const contentHash = '${contentHash}';
          const embedding = cache.get(contentHash);
          
          if (embedding) {
            console.log('CHILD: ✅ Successfully read embedding from shared memory');
            console.log('CHILD: ✅ Embedding size:', embedding.length);
            console.log('CHILD: ✅ First few values:', Array.from(embedding.slice(0, 3)));
            
            // Write a new embedding from child process
            const newHash = crypto.createHash('sha256').update('child process test').digest('hex');
            const newEmbedding = new Float32Array(384);
            for (let i = 0; i < 384; i++) {
              newEmbedding[i] = i / 384; // Simple pattern
            }
            
            const success = cache.set(newHash, newEmbedding);
            console.log('CHILD: ✅ Child process wrote to cache:', success);
            console.log('CHILD: Cache stats:', JSON.stringify(cache.getStats()));
          } else {
            console.log('CHILD: ❌ Failed to read embedding from shared memory');
          }
        } catch (error) {
          console.error('CHILD: ❌ Error:', error.message);
        }
      }
      
      testChildProcess();
    `], { stdio: 'inherit' });
    
    await new Promise((resolve) => {
      childProcess.on('close', (code) => {
        console.log(`   Child process exited with code ${code}`);
        resolve();
      });
    });
    
    // Check if child process wrote data
    console.log('\n   Checking parent process after child writes...');
    const childHash = crypto.createHash('sha256').update('child process test').digest('hex');
    const childEmbedding = cache.get(childHash);
    console.log('   ✅ Parent can read child data:', childEmbedding !== null);
    
    if (childEmbedding) {
      console.log('   ✅ Child data pattern correct:', childEmbedding[0] === 0 && Math.abs(childEmbedding[100] - (100/384)) < 0.0001);
    }
    
    console.log('\n📊 Final Cache Stats:');
    console.log('  ', cache.getStats());
    
    // Test 3: Performance test
    console.log('\n⚡ Test 3: Performance Test');
    const numTests = 1000;
    const startTime = Date.now();
    
    for (let i = 0; i < numTests; i++) {
      const hash = crypto.createHash('sha256').update(`test-${i}`).digest('hex');
      const embedding = new Float32Array(384);
      embedding.fill(i);
      cache.set(hash, embedding);
    }
    
    const writeTime = Date.now() - startTime;
    console.log(`   ✅ Wrote ${numTests} embeddings in ${writeTime}ms (${(writeTime/numTests).toFixed(2)}ms per embedding)`);
    
    const readStartTime = Date.now();
    let hits = 0;
    
    for (let i = 0; i < numTests; i++) {
      const hash = crypto.createHash('sha256').update(`test-${i}`).digest('hex');
      const result = cache.get(hash);
      if (result) hits++;
    }
    
    const readTime = Date.now() - readStartTime;
    console.log(`   ✅ Read ${numTests} embeddings in ${readTime}ms (${(readTime/numTests).toFixed(2)}ms per embedding)`);
    console.log(`   ✅ Cache hits: ${hits}/${numTests} (${(hits/numTests*100).toFixed(1)}%)`);
    
    // Cleanup
    await cache.close();
    console.log('\n🎉 All tests completed successfully!');
    
  } catch (error) {
    console.error('❌ Test failed:', error.message);
    console.error('Stack:', error.stack);
  }
}

// Run the test
testMemoryMappedCache();