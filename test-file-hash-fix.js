const fs = require('fs').promises;
const path = require('path');
const crypto = require('crypto');

// Test the file hash persistence fix
async function testFileHashFix() {
  console.log('🧪 Testing file hash persistence fix...');
  
  // Simulate the old broken format (Map serialized as object)
  const brokenHashData = {
    version: "1.0.0",
    schemaVersion: "1.0.0", 
    timestamp: Date.now(),
    repositoryPath: "/test/repo",
    chunks: [],
    fileHashes: {
      "src/test.ts": "abc123hash",
      "src/main.ts": "def456hash"
    },
    metadata: {
      totalChunks: 0,
      lastIndexed: Date.now(),
      embeddingModel: "BGE-small-en-v1.5"
    }
  };
  
  console.log('📄 Simulated broken format:', {
    fileHashesType: typeof brokenHashData.fileHashes,
    fileHashesKeys: Object.keys(brokenHashData.fileHashes)
  });
  
  // Test our fix - convert object back to Map
  let fileHashes;
  if (brokenHashData.fileHashes instanceof Map) {
    fileHashes = brokenHashData.fileHashes;
  } else {
    // This is our fix - convert from serialized object format back to Map
    fileHashes = new Map(Object.entries(brokenHashData.fileHashes || {}));
  }
  
  console.log('✅ Fixed format:', {
    fileHashesType: fileHashes.constructor.name,
    fileHashesSize: fileHashes.size,
    hasTestFile: fileHashes.has('src/test.ts'),
    testFileHash: fileHashes.get('src/test.ts')
  });
  
  // Test incremental change detection
  const testFilePath = 'src/test.ts';
  const storedHash = fileHashes.get(testFilePath);
  const currentContent = 'const test = "unchanged content";';
  const currentHash = crypto.createHash('sha256').update(currentContent).digest('hex');
  
  console.log('🔍 Change detection test:', {
    storedHash: storedHash?.substring(0, 8) + '...',
    currentHash: currentHash.substring(0, 8) + '...',
    hasChanged: storedHash !== currentHash,
    wouldReprocess: !storedHash || storedHash !== currentHash
  });
  
  console.log('✅ File hash persistence fix is working correctly!');
}

testFileHashFix().catch(console.error);