#!/usr/bin/env node

/**
 * Test the exact scenario from the logs:
 * - 3,364 chunks loaded from corrupted index
 * - 110 files scanned 
 * - Result: "📊 Delta analysis: +0 ~0 -0 files"
 */

const fs = require('fs');
const path = require('path');

// Get actual files in the repo to test with real data
function getCurrentFiles() {
  const gitFiles = [];
  
  function scanDirectory(dir, depth = 0) {
    if (depth > 3) return; // Limit depth
    
    try {
      const items = fs.readdirSync(dir);
      for (const item of items) {
        if (item.startsWith('.') && item !== '.claude') continue; // Skip hidden except .claude
        
        const fullPath = path.join(dir, item);
        const stat = fs.statSync(fullPath);
        
        if (stat.isDirectory()) {
          if (item !== 'node_modules' && item !== 'dist' && item !== '.git') {
            scanDirectory(fullPath, depth + 1);
          }
        } else if (stat.isFile()) {
          // Get relative path
          const relativePath = path.relative(process.cwd(), fullPath);
          if (relativePath.match(/\.(ts|js|json|md)$/)) {
            gitFiles.push(relativePath);
          }
        }
      }
    } catch (error) {
      // Skip directories we can't read
    }
  }
  
  scanDirectory(process.cwd());
  return gitFiles.slice(0, 20); // Limit to first 20 files for testing
}

// Check what's actually in the corrupted index
function analyzeCorruptedIndex() {
  const indexPath = '/home/yanggf/a/cortexyoung/.cortex/index.json';
  
  if (!fs.existsSync(indexPath)) {
    console.log('❌ No local index file found');
    return null;
  }
  
  try {
    const indexData = JSON.parse(fs.readFileSync(indexPath, 'utf-8'));
    
    console.log('\n=== REAL INDEX ANALYSIS ===');
    console.log(`Index file: ${indexPath}`);
    console.log(`Total chunks: ${indexData.chunks?.length || 0}`);
    console.log(`Has fileHashes: ${!!indexData.fileHashes}`);
    console.log(`FileHashes count: ${indexData.fileHashes ? Object.keys(indexData.fileHashes).length : 0}`);
    
    if (indexData.chunks && indexData.chunks.length > 0) {
      const uniqueFiles = new Set(indexData.chunks.map(chunk => chunk.file_path));
      console.log(`Unique files in chunks: ${uniqueFiles.size}`);
      console.log(`Sample files:`, Array.from(uniqueFiles).slice(0, 5));
    }
    
    return indexData;
  } catch (error) {
    console.log(`❌ Failed to parse index: ${error.message}`);
    return null;
  }
}

// Simulate the exact incremental indexing flow
async function simulateIncrementalFlow() {
  console.log('🔍 SIMULATING REAL INCREMENTAL INDEXING FLOW');
  console.log('============================================');
  
  // Analyze real corrupted index
  const indexData = analyzeCorruptedIndex();
  if (!indexData) {
    console.log('⚠️  Cannot test with real index, using mock data');
    return;
  }
  
  // Get current files (simulate the 110 files scan)
  const currentFiles = getCurrentFiles();
  console.log(`\n📁 Current files scanned: ${currentFiles.length}`);
  console.log('Sample files:', currentFiles.slice(0, 5));
  
  // Simulate the conditions that led to "no changes"
  console.log('\n🧪 Testing Incremental Mode Detection...');
  
  // Simulate hasValidIndex check (this should return true since we have chunks)
  const hasChunks = indexData.chunks && indexData.chunks.length > 0;
  const hasFileHashes = indexData.fileHashes && Object.keys(indexData.fileHashes).length > 0;
  
  console.log(`✅ Has chunks: ${hasChunks} (${indexData.chunks?.length || 0} chunks)`);
  console.log(`❌ Has fileHashes: ${hasFileHashes} (${hasFileHashes ? Object.keys(indexData.fileHashes).length : 0} hashes)`);
  
  if (hasChunks) {
    console.log('✅ hasValidIndex() would return TRUE');
    console.log('✅ System would choose INCREMENTAL mode');
    
    // Now test the delta detection with real data
    console.log('\n🔍 Testing Delta Detection with Real Data...');
    
    // Build map of files with chunks
    const filesWithChunks = new Map();
    if (indexData.chunks) {
      for (const chunk of indexData.chunks) {
        if (!filesWithChunks.has(chunk.file_path)) {
          filesWithChunks.set(chunk.file_path, []);
        }
        filesWithChunks.get(chunk.file_path).push(chunk);
      }
    }
    
    console.log(`Files with stored chunks: ${filesWithChunks.size}`);
    
    // Test delta detection for each current file
    let addedCount = 0;
    let modifiedCount = 0;
    let errorCount = 0;
    
    console.log('\n--- Testing each current file ---');
    for (const filePath of currentFiles.slice(0, 5)) { // Test first 5 files
      const storedChunks = filesWithChunks.get(filePath) || [];
      
      if (storedChunks.length === 0) {
        console.log(`${filePath}: NEW (no stored chunks)`);
        addedCount++;
      } else {
        // Check if file actually exists and is readable
        const fullPath = path.resolve(filePath);
        try {
          if (fs.existsSync(fullPath)) {
            const storedHash = hasFileHashes ? indexData.fileHashes[filePath] : undefined;
            if (!storedHash) {
              console.log(`${filePath}: MODIFIED (${storedChunks.length} chunks, no stored hash)`);
              modifiedCount++;
            } else {
              console.log(`${filePath}: UNCHANGED? (has stored hash)`);
            }
          } else {
            console.log(`${filePath}: FILE NOT FOUND`);
            errorCount++;
          }
        } catch (error) {
          console.log(`${filePath}: ERROR - ${error.message}`);
          errorCount++;
        }
      }
    }
    
    console.log('\n📊 PREDICTED Delta Analysis:');
    console.log(`+${addedCount} ~${modifiedCount} -0 files (from ${currentFiles.slice(0, 5).length} tested)`);
    console.log(`Errors: ${errorCount}`);
    
    if (addedCount === 0 && modifiedCount === 0) {
      console.log('\n🚨 BUG CONFIRMED: Logic should detect changes but predicts none!');
      console.log('   Possible causes:');
      console.log('   1. Exception during delta calculation');
      console.log('   2. Early return preventing loop execution');
      console.log('   3. File path mismatch between stored chunks and current files');
      console.log('   4. Hash calculator not provided to calculateFileDelta');
    } else {
      console.log('\n✅ Logic should work correctly - bug must be elsewhere');
    }
  } else {
    console.log('❌ hasValidIndex() would return FALSE');
    console.log('❌ System would choose FULL mode, not incremental');
  }
}

// Run the real scenario test
simulateIncrementalFlow().catch(console.error);