#!/usr/bin/env node

import { CodebaseIndexer } from '../../packages/core/src/index';
import { IndexRequest } from '../../packages/shared/src/index';

async function main() {
  console.log('🚀 Cortex V2.1 Demo - Semantic Code Indexing');
  console.log('================================================');

  // Use current repository as demo
  const repositoryPath = process.cwd();

  console.log('🔗 Initializing fastembed-js embedding model...');
  
  // Initialize indexer
  const indexer = new CodebaseIndexer(repositoryPath);
  
  // Check model info
  const modelInfo = await indexer['embedder'].getModelInfo();
  if (modelInfo.isLoaded) {
    console.log(`✅ ${modelInfo.name} model loaded (${modelInfo.dimension} dimensions)`);
  } else {
    console.log('⚠️  Embedding model failed to load, using mock embeddings');
  }

  try {
    // Clear any existing index
    console.log('\n🧹 Clearing existing index...');
    await indexer.clearIndex();

    // Index the repository
    const indexRequest: IndexRequest = {
      repository_path: repositoryPath,
      mode: 'full'
    };

    console.log('\n📊 Starting full repository indexing...');
    console.log(`Repository: ${repositoryPath}`);
    
    const result = await indexer.indexRepository(indexRequest);

    if (result.status === 'success') {
      console.log('\n✅ Indexing completed successfully!');
      console.log(`📈 Statistics:`);
      console.log(`   - Chunks processed: ${result.chunks_processed}`);
      console.log(`   - Time taken: ${result.time_taken_ms}ms`);
      console.log(`   - Average time per chunk: ${(result.time_taken_ms / result.chunks_processed).toFixed(2)}ms`);

      // Get final stats
      const stats = await indexer.getIndexStats();
      console.log(`   - Total chunks in index: ${stats.total_chunks}`);
    } else {
      console.error('\n❌ Indexing failed:');
      console.error(`   Error: ${result.error_message}`);
      process.exit(1);
    }

    console.log('\n🎉 Demo completed successfully!');
    
    // Show model info
    console.log(`\n📊 Model Information:`);
    console.log(`   - Model: ${modelInfo.name}`);
    console.log(`   - Dimensions: ${modelInfo.dimension}`);
    console.log(`   - Status: ${modelInfo.isLoaded ? 'Loaded' : 'Mock embeddings'}`);
    console.log('\nNext steps:');
    console.log('1. Start the MCP server: npm run server');
    console.log('2. Test semantic search queries');
    console.log('3. Integrate with Claude Code tools');

  } catch (error) {
    console.error('\n💥 Demo failed with error:', error);
    process.exit(1);
  }
}

// Handle process termination gracefully
process.on('SIGINT', () => {
  console.log('\n\n⏹️  Demo interrupted by user');
  process.exit(0);
});

process.on('uncaughtException', (error) => {
  console.error('\n💥 Uncaught exception:', error);
  process.exit(1);
});

// Run the demo
main().catch((error) => {
  console.error('\n💥 Unhandled error:', error);
  process.exit(1);
});