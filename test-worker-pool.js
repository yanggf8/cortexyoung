const { WorkerPoolEmbedder } = require('./dist/worker-pool-embedder');

async function testWorkerPool() {
  console.log('🧪 WORKER POOL EMBEDDER TEST');
  console.log('='.repeat(50));

  const embedder = new WorkerPoolEmbedder();
  
  try {
    // Create test chunks
    const testChunks = [
      {
        id: 'chunk1',
        file_path: 'src/test.ts',
        symbol_name: 'testFunction',
        chunk_type: 'function',
        content: 'function testFunction() { return "hello"; }',
        relationships: { imports: ['fs', 'path'], exports: [] },
        language_metadata: { language: 'typescript' },
        start_line: 1,
        end_line: 3
      },
      {
        id: 'chunk2',
        file_path: 'src/utils.ts',
        symbol_name: 'utilityClass',
        chunk_type: 'class',
        content: 'class UtilityClass { constructor() {} }',
        relationships: { imports: ['lodash'], exports: ['UtilityClass'] },
        language_metadata: { language: 'typescript' },
        start_line: 5,
        end_line: 7
      },
      {
        id: 'chunk3',
        file_path: 'src/constants.ts',
        symbol_name: 'API_URL',
        chunk_type: 'variable',
        content: 'const API_URL = "https://api.example.com";',
        relationships: { imports: [], exports: ['API_URL'] },
        language_metadata: { language: 'typescript' },
        start_line: 1,
        end_line: 1
      }
    ];

    console.log(`\n📊 Testing with ${testChunks.length} chunks...`);
    
    // Test pool status before initialization
    console.log('\n🔍 Pool status before initialization:');
    console.log(embedder.getPoolStatus());
    
    // Process embeddings
    const startTime = Date.now();
    const embeddedChunks = await embedder.processAllEmbeddings(testChunks);
    const duration = Date.now() - startTime;
    
    console.log(`\n✅ Processing completed in ${duration}ms`);
    
    // Validate results
    console.log('\n🔍 Results validation:');
    console.log(`Input chunks: ${testChunks.length}`);
    console.log(`Output chunks: ${embeddedChunks.length}`);
    console.log(`All have embeddings: ${embeddedChunks.every(chunk => chunk.embedding && chunk.embedding.length === 384)}`);
    console.log(`Order preserved: ${embeddedChunks.every((chunk, i) => chunk.id === testChunks[i].id)}`);
    console.log(`Timestamps added: ${embeddedChunks.every(chunk => chunk.indexed_at)}`);
    
    // Show embedding details
    console.log('\n📋 Embedding details:');
    embeddedChunks.forEach((chunk, i) => {
      console.log(`  Chunk ${i}: ${chunk.id} -> ${chunk.embedding.length}D vector (${chunk.indexed_at})`);
    });
    
    // Test pool status after processing
    console.log('\n🔍 Pool status after processing:');
    console.log(embedder.getPoolStatus());
    
    // Test optimized embedding text generation
    console.log('\n📝 Optimized embedding text examples:');
    testChunks.forEach((chunk, i) => {
      // Simulate the optimized text generation
      const parts = [];
      if (chunk.symbol_name) parts.push(chunk.symbol_name);
      parts.push(chunk.chunk_type);
      parts.push(chunk.content);
      if (chunk.relationships.imports.length > 0) {
        parts.push(chunk.relationships.imports.slice(0, 3).join(' '));
      }
      const optimizedText = parts.join(' ');
      
      console.log(`  Chunk ${i}: "${optimizedText.substring(0, 100)}${optimizedText.length > 100 ? '...' : ''}"`);
    });
    
    console.log('\n🎉 Worker pool test completed successfully!');
    
  } catch (error) {
    console.error('\n❌ Worker pool test failed:', error);
  } finally {
    // Clean shutdown
    await embedder.shutdown();
    console.log('\n🛑 Worker pool shut down');
  }
}

// Performance comparison test
async function performanceComparison() {
  console.log('\n🏁 PERFORMANCE COMPARISON');
  console.log('='.repeat(50));
  
  // Create larger test dataset
  const largeTestChunks = [];
  for (let i = 0; i < 50; i++) {
    largeTestChunks.push({
      id: `chunk${i}`,
      file_path: `src/file${i}.ts`,
      symbol_name: `function${i}`,
      chunk_type: 'function',
      content: `function function${i}() { return ${i}; }`.repeat(10), // Longer content
      relationships: { imports: ['fs', 'path', 'util'], exports: [] },
      language_metadata: { language: 'typescript' },
      start_line: 1,
      end_line: 3
    });
  }
  
  console.log(`📊 Performance test with ${largeTestChunks.length} chunks...`);
  
  const embedder = new WorkerPoolEmbedder();
  
  try {
    const startTime = Date.now();
    const results = await embedder.processAllEmbeddings(largeTestChunks);
    const duration = Date.now() - startTime;
    
    const chunksPerSecond = Math.round((largeTestChunks.length / duration) * 1000);
    
    console.log(`\n⚡ Performance Results:`);
    console.log(`  Total time: ${duration}ms`);
    console.log(`  Chunks per second: ${chunksPerSecond}`);
    console.log(`  Average per chunk: ${Math.round(duration / largeTestChunks.length)}ms`);
    console.log(`  All embeddings valid: ${results.every(r => r.embedding && r.embedding.length === 384)}`);
    
  } catch (error) {
    console.error('❌ Performance test failed:', error);
  } finally {
    await embedder.shutdown();
  }
}

// Run tests
async function runAllTests() {
  try {
    await testWorkerPool();
    await performanceComparison();
  } catch (error) {
    console.error('❌ Test suite failed:', error);
    process.exit(1);
  }
}

if (require.main === module) {
  runAllTests();
}

module.exports = { testWorkerPool, performanceComparison };
