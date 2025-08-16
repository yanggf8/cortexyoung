#!/usr/bin/env node

/**
 * Test to verify the exact bug in delta detection logic
 * This test will simulate the exact conditions that caused "no changes" detection
 */

const fs = require('fs').promises;
const path = require('path');

// Mock the PersistentVectorStore delta detection logic
class TestVectorStore {
  constructor() {
    this.chunks = new Map();
    this.fileHashes = new Map();
  }

  // Simulate loading chunks from corrupted index (without fileHashes)
  loadCorruptedIndex() {
    console.log('\n=== Simulating Corrupted Index Load ===');
    
    // Add some sample chunks (simulating 3364 chunks loaded)
    this.chunks.set('src/server.ts:1', {
      chunk_id: 'src/server.ts:1',
      file_path: 'src/server.ts',
      content: 'import express from "express";'
    });
    
    this.chunks.set('src/indexer.ts:1', {
      chunk_id: 'src/indexer.ts:1', 
      file_path: 'src/indexer.ts',
      content: 'export class CodebaseIndexer {'
    });
    
    this.chunks.set('old-file.ts:1', {
      chunk_id: 'old-file.ts:1',
      file_path: 'old-file.ts', 
      content: 'this file was deleted'
    });

    // Simulate corrupted index: fileHashes is EMPTY (missing from JSON)
    this.fileHashes.clear();
    
    console.log(`✅ Loaded ${this.chunks.size} chunks`);
    console.log(`❌ fileHashes size: ${this.fileHashes.size} (corrupted - should be empty)`);
  }

  // Test the exact calculateFileDelta logic 
  async calculateFileDelta(files, chunkHashCalculator) {
    console.log('\n=== Testing Delta Detection Logic ===');
    console.log(`Input files: ${files.length}`);
    console.log(`Stored chunks: ${this.chunks.size}`);
    console.log(`FileHashes size: ${this.fileHashes.size}`);

    const delta = {
      added: [],
      updated: [],
      removed: [],
      fileChanges: {
        added: [],
        modified: [],
        deleted: []
      }
    };

    // Test each file for changes (exact logic from PersistentVectorStore)
    for (const filePath of files) {
      console.log(`\n--- Processing file: ${filePath} ---`);
      
      try {
        // Get stored chunks for this file (exact same logic)
        const storedChunks = Array.from(this.chunks.values())
          .filter(chunk => chunk.file_path === filePath);
        
        console.log(`  Stored chunks for ${filePath}: ${storedChunks.length}`);
        
        if (storedChunks.length === 0) {
          // New file - no existing chunks
          console.log(`  ➜ DECISION: New file (no stored chunks)`);
          delta.fileChanges.added.push(filePath);
        } else {
          // Use file content hash for fast comparison  
          if (chunkHashCalculator) {
            const currentHash = await chunkHashCalculator(filePath);
            const storedHash = this.fileHashes.get(filePath);
            
            console.log(`  Current hash: ${currentHash}`);
            console.log(`  Stored hash: ${storedHash || 'undefined'}`);
            
            if (!storedHash) {
              // No stored hash (old data format) - treat as modified to rebuild file hash
              console.log(`  ➜ DECISION: Modified (missing stored hash)`);
              delta.fileChanges.modified.push(filePath);
              delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
            } else if (storedHash !== currentHash) {
              // File changed at content level
              console.log(`  ➜ DECISION: Modified (hash mismatch)`);
              delta.fileChanges.modified.push(filePath);
              delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
            } else {
              console.log(`  ➜ DECISION: Unchanged (hash match)`);
            }
            // If hashes match, file is unchanged - no action needed
          } else {
            // Fallback: treat as modified if no calculator provided
            console.log(`  ➜ DECISION: Modified (no hash calculator)`);
            delta.fileChanges.modified.push(filePath);
            delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
          }
        }
      } catch (error) {
        console.log(`  ❌ ERROR processing ${filePath}: ${error.message}`);
      }
    }

    // Check for deleted files - files that have chunks but aren't in current file list
    console.log('\n--- Checking for deleted files ---');
    const filesWithChunks = new Set(Array.from(this.chunks.values()).map(chunk => chunk.file_path));
    console.log(`Files with chunks: ${Array.from(filesWithChunks).join(', ')}`);
    
    for (const filePath of filesWithChunks) {
      if (!files.includes(filePath)) {
        console.log(`  ➜ DELETED: ${filePath}`);
        delta.fileChanges.deleted.push(filePath);
        
        // Remove chunks for deleted files
        const deletedChunks = Array.from(this.chunks.values())
          .filter(chunk => chunk.file_path === filePath);
        delta.removed.push(...deletedChunks.map(chunk => chunk.chunk_id));
      }
    }

    console.log('\n=== FINAL DELTA RESULT ===');
    console.log(`📊 Delta analysis: +${delta.fileChanges.added.length} ~${delta.fileChanges.modified.length} -${delta.fileChanges.deleted.length} files`);
    console.log(`Added files: ${delta.fileChanges.added.join(', ') || 'none'}`);
    console.log(`Modified files: ${delta.fileChanges.modified.join(', ') || 'none'}`);
    console.log(`Deleted files: ${delta.fileChanges.deleted.join(', ') || 'none'}`);
    
    return delta;
  }
}

// Mock chunk hash calculator (simulates file content hashing)
async function mockChunkHashCalculator(filePath) {
  // Simulate different hashes for different files
  const mockHashes = {
    'src/server.ts': 'hash123abc',
    'src/indexer.ts': 'hash456def', 
    'README.md': 'hash789ghi',
    'package.json': 'hashxyzuvw'
  };
  
  return mockHashes[filePath] || `hash_${filePath.replace(/[^a-zA-Z0-9]/g, '_')}`;
}

// Main test function
async function runDeltaDetectionTest() {
  console.log('🧪 DELTA DETECTION BUG TEST');
  console.log('===========================');
  
  const store = new TestVectorStore();
  
  // Simulate the exact conditions that caused the bug
  store.loadCorruptedIndex();
  
  // Simulate current files (like the 110 files detected)
  const currentFiles = [
    'src/server.ts',      // Has stored chunks
    'src/indexer.ts',     // Has stored chunks  
    'README.md',          // No stored chunks (new file)
    'package.json'        // No stored chunks (new file)
  ];
  
  console.log(`\nTesting with ${currentFiles.length} current files:`, currentFiles);
  
  // Test the delta detection logic
  const delta = await store.calculateFileDelta(currentFiles, mockChunkHashCalculator);
  
  // Analyze results
  console.log('\n🔍 ANALYSIS:');
  if (delta.fileChanges.added.length === 0 && 
      delta.fileChanges.modified.length === 0 && 
      delta.fileChanges.deleted.length === 0) {
    console.log('❌ BUG REPRODUCED: No changes detected despite corrupted index!');
  } else {
    console.log('✅ Logic working: Changes properly detected');
  }
  
  console.log('\n📋 Expected behavior:');
  console.log('- Files with stored chunks but no hash → Modified');
  console.log('- Files without stored chunks → Added');  
  console.log('- Stored chunks for missing files → Deleted');
}

// Run the test
runDeltaDetectionTest().catch(console.error);