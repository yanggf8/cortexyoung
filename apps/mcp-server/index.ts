#!/usr/bin/env node

import { CodebaseIndexer } from '../../packages/core/src/index';
import { CortexMCPServer } from '../../packages/mcp-server/src/server';

async function startMCPServer() {
  console.log('🚀 Cortex V2.1 MCP Server');
  console.log('========================');

  // Use current repository as demo
  const repositoryPath = process.cwd();
  const port = parseInt(process.env['PORT'] || '8765');
  
  // Initialize indexer
  console.log('🔄 Initializing codebase indexer...');
  const indexer = new CodebaseIndexer(repositoryPath);
  
  // Check if we have an existing index, if not, create one
  let stats = await indexer.getIndexStats();
  if (stats.total_chunks === 0) {
    console.log('📊 No existing index found. Creating one...');
    
    const indexRequest = {
      repository_path: repositoryPath,
      mode: 'full' as const
    };
    
    console.log('🔄 Indexing repository with embeddings...');
    const indexResult = await indexer.indexRepository(indexRequest);
    
    if (indexResult.status === 'success') {
      console.log(`✅ Indexed ${indexResult.chunks_processed} chunks in ${indexResult.time_taken_ms}ms`);
      stats = await indexer.getIndexStats();
    } else {
      console.error('❌ Indexing failed:', indexResult.error_message);
      process.exit(1);
    }
  }
  
  console.log(`📈 Using index with ${stats.total_chunks} chunks`);
  
  // Create MCP server
  const mcpServer = new CortexMCPServer(indexer, indexer['searcher']);
  
  // Start server
  await mcpServer.start(port);
  
  console.log('\n🎯 Server ready for Claude Code integration!');
  console.log('\nAvailable tools:');
  const tools = mcpServer.getAvailableTools();
  tools.forEach(tool => {
    console.log(`  📋 ${tool.name}: ${tool.description}`);
  });
  
  console.log('\n📡 Test endpoints:');
  console.log(`  curl http://localhost:${port}/tools`);
  console.log(`  curl -X POST http://localhost:${port}/call -d '{"method":"semantic_search","params":{"query":"embedding functionality"},"id":1}'`);
  
  // Handle graceful shutdown
  const shutdown = async () => {
    console.log('\n🛑 Shutting down...');
    await mcpServer.stop();
    process.exit(0);
  };
  
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
  
  // Keep process alive
  await new Promise(() => {});
}

// Handle errors
process.on('uncaughtException', (error) => {
  console.error('\n💥 Uncaught exception:', error);
  process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
  console.error('\n💥 Unhandled rejection at:', promise, 'reason:', reason);
  process.exit(1);
});

// Start the server
startMCPServer().catch((error) => {
  console.error('\n💥 Failed to start MCP server:', error);
  process.exit(1);
});