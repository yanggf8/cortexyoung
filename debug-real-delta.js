#!/usr/bin/env node

/**
 * Debug script to test the actual delta detection with real vector store data
 */

const { GitScanner } = require('./dist/git-scanner');
const { PersistentVectorStore } = require('./dist/persistent-vector-store');
const path = require('path');

async function debugRealDelta() {
  console.log('🔍 Debugging Real Delta Detection');
  console.log('='.repeat(50));
  
  const repoPath = process.cwd();
  const gitScanner = new GitScanner(repoPath);
  const vectorStore = new PersistentVectorStore(repoPath);
  
  console.log(`📂 Repository path: ${repoPath}`);
  console.log();
  
  // Step 1: Load existing vector store
  console.log('📋 Step 1: Loading existing vector store');
  try {
    await vectorStore.loadPersistedIndex();
    const chunkCount = vectorStore.getChunkCount();
    console.log(`   ✅ Loaded ${chunkCount} chunks from storage`);
    
    // Check if there are chunks for src/indexer.ts
    const indexerChunks = Array.from(vectorStore.chunks.values()).filter(chunk => 
      chunk.file_path === 'src/indexer.ts'
    );
    console.log(`   📄 Chunks for src/indexer.ts: ${indexerChunks.length}`);
    
    if (indexerChunks.length > 0) {
      console.log(`   📄 First chunk ID: ${indexerChunks[0].chunk_id}`);
      console.log(`   📄 File path in chunk: "${indexerChunks[0].file_path}"`);
    }
    
  } catch (error) {
    console.log(`   ❌ Failed to load vector store: ${error.message}`);
    return;
  }
  
  console.log();
  
  // Step 2: Get current files from GitScanner
  console.log('📋 Step 2: Getting files from GitScanner');
  const scanResult = await gitScanner.scanRepository('full');
  const filesArray = scanResult.files;
  
  console.log(`   📁 Total files: ${filesArray.length}`);
  const indexerInFiles = filesArray.includes('src/indexer.ts');
  console.log(`   📄 src/indexer.ts in files array: ${indexerInFiles}`);
  
  console.log();
  
  // Step 3: Test the exact delta detection logic
  console.log('📋 Step 3: Testing delta detection logic');
  
  // Get all files that have chunks
  const filesWithChunks = new Set(Array.from(vectorStore.chunks.values()).map(chunk => chunk.file_path));
  console.log(`   📦 Files with chunks: ${filesWithChunks.size}`);
  console.log(`   📄 src/indexer.ts has chunks: ${filesWithChunks.has('src/indexer.ts')}`);
  
  // Test the problematic logic
  const wouldBeDeleted = [];
  const wouldBeModified = [];
  
  for (const filePath of filesWithChunks) {
    if (!filesArray.includes(filePath)) {
      wouldBeDeleted.push(filePath);
    } else {
      wouldBeModified.push(filePath);
    }
  }
  
  console.log();
  console.log('🎯 DELTA ANALYSIS RESULTS:');
  console.log(`   Files that would be marked as DELETED: ${wouldBeDeleted.length}`);
  if (wouldBeDeleted.length > 0) {
    console.log(`   Deleted files: ${wouldBeDeleted.slice(0, 10).join(', ')}`);
  }
  if (wouldBeDeleted.includes('src/indexer.ts')) {
    console.log(`   ❌ BUG CONFIRMED: src/indexer.ts would be marked as DELETED`);
    console.log(`      - Has chunks: ${filesWithChunks.has('src/indexer.ts')}`);
    console.log(`      - In files array: ${filesArray.includes('src/indexer.ts')}`);
    console.log(`      - Files array type: ${typeof filesArray}`);
    console.log(`      - Files array is array: ${Array.isArray(filesArray)}`);
    
    // Debug the includes check
    console.log();
    console.log('🔍 DETAILED INCLUDES CHECK:');
    console.log(`   filesArray.includes('src/indexer.ts'): ${filesArray.includes('src/indexer.ts')}`);
    console.log(`   filesArray.indexOf('src/indexer.ts'): ${filesArray.indexOf('src/indexer.ts')}`);
    console.log(`   Finding by filter: ${filesArray.filter(f => f === 'src/indexer.ts').length}`);
    console.log(`   Finding by find: ${!!filesArray.find(f => f === 'src/indexer.ts')}`);
    
    // Check for case sensitivity or whitespace issues
    const exactMatches = filesArray.filter(f => f.includes('indexer'));
    console.log(`   Files containing 'indexer': ${exactMatches}`);
    
  } else {
    console.log(`   ✅ src/indexer.ts would NOT be marked as deleted`);
  }
  
  console.log();
  
  // Step 4: Run actual calculateFileDelta
  console.log('📋 Step 4: Running actual calculateFileDelta');
  try {
    const crypto = require('crypto');
    const chunkHashCalculator = async (filePath) => {
      const content = await gitScanner.readFile(filePath);
      return crypto.createHash('sha256').update(content).digest('hex');
    };
    
    const delta = await vectorStore.calculateFileDelta(filesArray, chunkHashCalculator);
    
    console.log(`   📊 Delta results:`);
    console.log(`      Added: ${delta.fileChanges.added.length}`);
    console.log(`      Modified: ${delta.fileChanges.modified.length}`);
    console.log(`      Deleted: ${delta.fileChanges.deleted.length}`);
    
    if (delta.fileChanges.deleted.length > 0) {
      console.log(`      Deleted files: ${delta.fileChanges.deleted.slice(0, 10).join(', ')}`);
    }
    
    if (delta.fileChanges.deleted.includes('src/indexer.ts')) {
      console.log(`   ❌ CONFIRMED: src/indexer.ts marked as DELETED in actual delta`);
    } else if (delta.fileChanges.modified.includes('src/indexer.ts')) {
      console.log(`   ✅ src/indexer.ts correctly marked as MODIFIED`);
    } else if (delta.fileChanges.added.includes('src/indexer.ts')) {
      console.log(`   ⚠️  src/indexer.ts marked as ADDED (unexpected)`);
    } else {
      console.log(`   ℹ️  src/indexer.ts not in any delta category (unchanged)`);
    }
    
  } catch (error) {
    console.log(`   ❌ Error in calculateFileDelta: ${error.message}`);
  }
}

debugRealDelta().catch(console.error);