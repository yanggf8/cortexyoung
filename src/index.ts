#!/usr/bin/env node

import { CodebaseIndexer } from './indexer';
import { StartupStageTracker } from './startup-stages';
import * as path from 'path';

async function main() {
  // Parse command line arguments
  const args = process.argv.slice(2);
  const repoPath = args.find(arg => !arg.startsWith('--')) || process.cwd();
  const forceReindex = args.includes('--reindex') || args.includes('--force-rebuild');
  const forceFullMode = args.includes('--full');
  
  console.log(`🚀 Starting Cortex indexing for: ${repoPath}`);
  if (forceReindex) console.log('🔄 Force rebuild requested (--reindex)');
  if (forceFullMode) console.log('🔄 Full mode requested (--full)');
  
  // Initialize stage tracker for progress monitoring
  const stageTracker = new StartupStageTracker();
  const indexer = new CodebaseIndexer(repoPath, stageTracker);
  
  try {
    stageTracker.startStage('cache_check', 'Checking for existing embeddings cache');
    
    // Check if index exists to determine mode
    const vectorStore = (indexer as any).vectorStore;
    await vectorStore.initialize();
    const hasExistingIndex = await vectorStore.indexExists();
    
    // Determine indexing mode
    let mode: 'full' | 'incremental' | 'reindex';
    if (forceReindex) {
      mode = 'reindex';
    } else if (forceFullMode) {
      mode = 'full';
    } else {
      mode = hasExistingIndex ? 'incremental' : 'full';
    }
    
    console.log(`🔍 Index mode: ${mode} (existing index: ${hasExistingIndex ? 'found' : 'not found'})`);
    
    stageTracker.completeStage('cache_check', `Mode selected: ${mode}`);
    
    const response = await indexer.indexRepository({
      repository_path: repoPath,
      mode,
      force_rebuild: forceReindex
    });
    
    stageTracker.completeStage('mcp_ready', 'Indexing completed successfully');
    
    console.log(`✅ Indexing complete!`);
    console.log(`📊 Results: ${response.chunks_processed} chunks`);
    console.log(`⚡ Processing time: ${response.time_taken_ms}ms`);
    console.log(`🎯 ${stageTracker.getProgressSummary()}`);
  } catch (error) {
    stageTracker.failStage(stageTracker.getCurrentStage()?.id || 'unknown', error instanceof Error ? error.message : String(error));
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