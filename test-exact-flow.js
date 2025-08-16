#!/usr/bin/env node

/**
 * Test the EXACT incremental indexing flow to find where "no changes" comes from
 */

const fs = require('fs');
const crypto = require('crypto');
const path = require('path');

// Simulate the exact chunkHashCalculator used in the real code
async function chunkHashCalculator(filePath) {
  try {
    const fullPath = path.resolve(filePath);
    const content = fs.readFileSync(fullPath, 'utf-8');
    return crypto.createHash('sha256').update(content).digest('hex');
  } catch (error) {
    throw new Error(`Failed to read ${filePath}: ${error.message}`);
  }
}

// Get the first few files from the repo for testing
function getTestFiles() {
  const files = [
    'README.md',
    'package.json', 
    'CLAUDE.md',
    'ROADMAP.md',
    'src/server.ts'
  ].filter(f => fs.existsSync(f));
  
  return files;
}

async function testExactFlow() {
  console.log('🔬 TESTING EXACT INCREMENTAL FLOW');
  console.log('================================');
  
  // Load the real index
  const indexPath = '/home/yanggf/a/cortexyoung/.cortex/index.json';
  
  if (!fs.existsSync(indexPath)) {
    console.log('❌ No index file to test with');
    return;
  }
  
  const indexData = JSON.parse(fs.readFileSync(indexPath, 'utf-8'));
  console.log(`✅ Loaded real index with ${indexData.chunks?.length || 0} chunks`);
  
  // Simulate the vector store state after loading
  const chunks = new Map();
  const fileHashes = new Map();
  
  // Load chunks
  if (indexData.chunks) {
    for (const chunk of indexData.chunks) {
      chunks.set(chunk.chunk_id, chunk);
    }
  }
  
  // Load file hashes  
  if (indexData.fileHashes) {
    for (const [filePath, hash] of Object.entries(indexData.fileHashes)) {
      fileHashes.set(filePath, hash);
    }
  }
  
  console.log(`📊 Simulated state: ${chunks.size} chunks, ${fileHashes.size} file hashes`);
  
  // Get test files
  const testFiles = getTestFiles();
  console.log(`📁 Testing with files: ${testFiles.join(', ')}`);
  
  // Now run the EXACT calculateFileDelta logic
  console.log('\n🧪 Running EXACT calculateFileDelta logic...');
  
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

  // Check each file for changes (EXACT COPY of the logic)
  for (const filePath of testFiles) {
    console.log(`\n--- Processing: ${filePath} ---`);
    
    try {
      // Get stored chunks for this file
      const storedChunks = Array.from(chunks.values())
        .filter(chunk => chunk.file_path === filePath);
      
      console.log(`  Stored chunks: ${storedChunks.length}`);
      
      if (storedChunks.length === 0) {
        // New file - no existing chunks
        console.log('  ➜ NEW FILE');
        delta.fileChanges.added.push(filePath);
      } else {
        // Use file content hash for fast comparison
        try {
          const currentHash = await chunkHashCalculator(filePath);
          const storedHash = fileHashes.get(filePath);
          
          console.log(`  Current hash: ${currentHash.substring(0, 16)}...`);
          console.log(`  Stored hash:  ${storedHash ? storedHash.substring(0, 16) + '...' : 'undefined'}`);
          
          if (!storedHash) {
            // No stored hash (old data format) - treat as modified to rebuild file hash
            console.log('  ➜ MODIFIED (no stored hash)');
            delta.fileChanges.modified.push(filePath);
            delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
          } else if (storedHash !== currentHash) {
            // File changed at content level
            console.log('  ➜ MODIFIED (hash changed)');
            delta.fileChanges.modified.push(filePath);
            delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
          } else {
            console.log('  ➜ UNCHANGED (hash match)');
          }
        } catch (hashError) {
          console.log(`  ❌ Hash calculation failed: ${hashError.message}`);
          // If we can't hash it, treat as modified
          console.log('  ➜ MODIFIED (hash error)');
          delta.fileChanges.modified.push(filePath);
          delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
        }
      }
    } catch (error) {
      console.log(`  ❌ Error processing ${filePath}: ${error.message}`);
    }
  }

  // Check for deleted files
  console.log('\n--- Checking for deleted files ---');
  const filesWithChunks = new Set(Array.from(chunks.values()).map(chunk => chunk.file_path));
  let deletedCount = 0;
  
  for (const filePath of filesWithChunks) {
    if (!testFiles.includes(filePath)) {
      // Only count it as deleted if it's a "real" file (not a .nx cache file)
      if (!filePath.startsWith('.nx/') && !filePath.startsWith('packages/') && !filePath.startsWith('apps/')) {
        console.log(`  ➜ DELETED: ${filePath}`);
        delta.fileChanges.deleted.push(filePath);
        deletedCount++;
        
        if (deletedCount >= 5) break; // Limit output
      }
    }
  }
  
  console.log('\n📊 FINAL RESULT:');
  console.log(`📊 Delta analysis: +${delta.fileChanges.added.length} ~${delta.fileChanges.modified.length} -${delta.fileChanges.deleted.length} files`);
  
  if (delta.fileChanges.added.length === 0 && 
      delta.fileChanges.modified.length === 0 && 
      delta.fileChanges.deleted.length === 0) {
    console.log('\n🚨 BUG REPRODUCED: No changes detected!');
    console.log('   This confirms the exact scenario');
  } else {
    console.log('\n✅ Changes detected correctly');
    console.log(`   Added: ${delta.fileChanges.added.join(', ') || 'none'}`);
    console.log(`   Modified: ${delta.fileChanges.modified.join(', ') || 'none'}`); 
    console.log(`   Deleted: ${delta.fileChanges.deleted.join(', ') || 'none'}`);
  }
}

testExactFlow().catch(console.error);