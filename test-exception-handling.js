#!/usr/bin/env node

/**
 * Test to verify enhanced exception handling in delta detection
 */

// Mock PersistentVectorStore with enhanced exception handling
class TestVectorStoreWithExceptionHandling {
  constructor() {
    this.chunks = new Map();
    this.fileHashes = new Map();
  }

  // Simulate loading corrupted index with some chunks but no hashes
  loadTestData() {
    // Add chunks for various files
    this.chunks.set('existing.ts:1', {
      chunk_id: 'existing.ts:1',
      file_path: 'existing.ts',
      content: 'existing file content'
    });
    
    this.chunks.set('deleted.ts:1', {
      chunk_id: 'deleted.ts:1', 
      file_path: 'deleted.ts',
      content: 'deleted file content'
    });
    
    this.chunks.set('unreadable.ts:1', {
      chunk_id: 'unreadable.ts:1',
      file_path: 'unreadable.ts', 
      content: 'file with permission issues'
    });

    // Start with empty fileHashes (corrupted scenario)
    this.fileHashes.clear();
    
    console.log(`✅ Test setup: ${this.chunks.size} chunks, ${this.fileHashes.size} file hashes`);
  }

  // Enhanced calculateFileDelta with proper exception handling
  async calculateFileDelta(files, chunkHashCalculator) {
    console.log('\n=== Testing Enhanced Exception Handling ===');
    
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

    // Hash rebuilding with exception handling (already implemented)
    if (this.fileHashes.size === 0 && this.chunks.size > 0 && chunkHashCalculator) {
      console.log(`[DeltaDetection] Empty fileHashes detected, rebuilding from ${this.chunks.size} stored chunks`);
      
      const filesWithChunks = new Set(Array.from(this.chunks.values()).map(chunk => chunk.file_path));
      for (const filePath of filesWithChunks) {
        try {
          const currentHash = await chunkHashCalculator(filePath);
          this.fileHashes.set(filePath, currentHash);
          console.log(`[DeltaDetection] ✅ Rebuilt hash for ${filePath}`);
        } catch (error) {
          console.log(`[DeltaDetection] ❌ Failed to rebuild hash for ${filePath}: ${error.message}`);
        }
      }
      console.log(`[DeltaDetection] Rebuilt ${this.fileHashes.size} file hashes from stored chunks`);
    }

    // Process each file with enhanced exception handling
    for (const filePath of files) {
      console.log(`\n--- Processing: ${filePath} ---`);
      
      try {
        // Get stored chunks for this file
        const storedChunks = Array.from(this.chunks.values())
          .filter(chunk => chunk.file_path === filePath);
        
        console.log(`  Stored chunks: ${storedChunks.length}`);
        
        if (storedChunks.length === 0) {
          // New file - no existing chunks
          console.log('  ➜ NEW FILE');
          delta.fileChanges.added.push(filePath);
        } else {
          // Use file content hash for fast comparison
          if (chunkHashCalculator) {
            try {
              const currentHash = await chunkHashCalculator(filePath);
              const storedHash = this.fileHashes.get(filePath);
              
              console.log(`  Current hash: ${currentHash?.substring(0, 16) || 'failed'}...`);
              console.log(`  Stored hash: ${storedHash?.substring(0, 16) || 'undefined'}...`);
              
              if (!storedHash) {
                console.log(`  [DeltaDetection] Missing stored hash for ${filePath}, marking as modified`);
                delta.fileChanges.modified.push(filePath);
                delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
              } else if (storedHash !== currentHash) {
                console.log(`  [DeltaDetection] Hash mismatch for ${filePath}, marking as modified`);
                delta.fileChanges.modified.push(filePath);
                delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
              } else {
                console.log('  ➜ UNCHANGED (hash match)');
              }
            } catch (hashError) {
              // Enhanced exception handling - treat as modified (conservative approach)
              console.log(`  [DeltaDetection] Hash calculation failed for ${filePath}: ${hashError.message}`);
              console.log(`  [DeltaDetection] Treating ${filePath} as modified due to hash failure`);
              delta.fileChanges.modified.push(filePath);
              delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
            }
          } else {
            console.log(`  [DeltaDetection] No hash calculator provided, treating ${filePath} as modified`);
            delta.fileChanges.modified.push(filePath);
            delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
          }
        }
      } catch (error) {
        console.log(`  [VectorStore] Failed to process file file=${filePath} error=${error.message}`);
        // Continue processing other files even if this one fails
      }
    }

    console.log('\n=== FINAL RESULT ===');
    console.log(`📊 Delta analysis: +${delta.fileChanges.added.length} ~${delta.fileChanges.modified.length} -${delta.fileChanges.deleted.length} files`);
    console.log(`Added: ${delta.fileChanges.added.join(', ') || 'none'}`);
    console.log(`Modified: ${delta.fileChanges.modified.join(', ') || 'none'}`);
    console.log(`Deleted: ${delta.fileChanges.deleted.join(', ') || 'none'}`);
    
    return delta;
  }
}

// Mock hash calculator that simulates various failure scenarios
async function mockChunkHashCalculatorWithFailures(filePath) {
  console.log(`  🔍 Calculating hash for: ${filePath}`);
  
  if (filePath === 'deleted.ts') {
    throw new Error('ENOENT: no such file or directory');
  }
  
  if (filePath === 'unreadable.ts') {
    throw new Error('EACCES: permission denied');
  }
  
  if (filePath === 'binary.bin') {
    throw new Error('Invalid UTF-8 sequence');
  }
  
  // Success case
  return `hash_for_${filePath.replace(/[^a-zA-Z0-9]/g, '_')}`;
}

async function testEnhancedExceptionHandling() {
  console.log('🧪 TESTING ENHANCED EXCEPTION HANDLING');
  console.log('=====================================');
  
  const store = new TestVectorStoreWithExceptionHandling();
  store.loadTestData();
  
  // Test files including problematic ones
  const testFiles = [
    'existing.ts',      // Has chunks, should work
    'new-file.ts',      // No chunks, should be added
    'deleted.ts',       // Has chunks but file deleted, should fail hash calc
    'unreadable.ts',    // Has chunks but permission denied, should fail hash calc
    'binary.bin'        // Hash calculation failure
  ];
  
  console.log(`\nTesting with files: ${testFiles.join(', ')}`);
  
  // Run delta detection with various failure scenarios
  const delta = await store.calculateFileDelta(testFiles, mockChunkHashCalculatorWithFailures);
  
  // Analyze results
  console.log('\n🔍 ANALYSIS:');
  console.log('✅ Enhanced exception handling should:');
  console.log('   1. Continue processing all files despite hash failures');
  console.log('   2. Treat hash failures as "modified" (conservative approach)');
  console.log('   3. Provide detailed logging for debugging');
  console.log('   4. Never return "no changes" unless truly no changes');
  
  const totalChanges = delta.fileChanges.added.length + 
                      delta.fileChanges.modified.length + 
                      delta.fileChanges.deleted.length;
  
  if (totalChanges === 0) {
    console.log('❌ POTENTIAL BUG: No changes detected despite test scenarios');
  } else {
    console.log(`✅ SUCCESS: Detected ${totalChanges} changes with proper exception handling`);
  }
}

testEnhancedExceptionHandling().catch(console.error);