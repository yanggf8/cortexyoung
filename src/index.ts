#!/usr/bin/env node

import { CodebaseIndexer } from './indexer';
import * as path from 'path';

async function main() {
  const repoPath = process.argv[2] || process.cwd();
  console.log(`🚀 Starting Cortex indexing for: ${repoPath}`);
  
  const indexer = new CodebaseIndexer(repoPath);
  
  try {
    const response = await indexer.indexRepository({
      repository_path: repoPath,
      mode: 'full'
    });
    
    console.log(`✅ Indexing complete!`);
    console.log(`📊 Results: ${response.chunks_processed} chunks`);
    console.log(`⚡ Processing time: ${response.time_taken_ms}ms`);
  } catch (error) {
    console.error('❌ Indexing failed:', error);
    process.exit(1);
  }
}

if (require.main === module) {
  main().catch(console.error);
}

export * from './types';
export * from './indexer';
export * from './searcher';
export * from './server';