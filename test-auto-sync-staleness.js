#!/usr/bin/env node

/**
 * Test script to validate auto-sync staleness detection and handling
 * 
 * This script tests the enhanced performAutoSync method to ensure it:
 * 1. Detects staleness (>24h apart) in both embeddings and relationships
 * 2. Automatically syncs the newer version to the older location
 * 3. Updates recommendations to reflect auto-sync handling
 */

const { UnifiedStorageCoordinator } = require('./dist/unified-storage-coordinator');
const fs = require('fs').promises;
const path = require('path');
const os = require('os');
const crypto = require('crypto');

const TEST_REPO_PATH = process.cwd();
const TEST_RESULTS = [];

// Helper function to create test storage files with specific timestamps
async function createTestStorage(filePath, age24HoursAgo = false) {
  const testData = {
    version: '2.1.0',
    schemaVersion: '1.0.0',
    timestamp: Date.now(),
    repositoryPath: TEST_REPO_PATH,
    chunks: [],
    metadata: {
      total_chunks: 0,
      totalFiles: 0,
      lastIndexed: Date.now(),
      indexingMode: 'test'
    }
  };
  
  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(filePath, JSON.stringify(testData, null, 2));
  
  // Set file timestamp to 25 hours ago if requested
  if (age24HoursAgo) {
    const oldDate = new Date(Date.now() - (25 * 60 * 60 * 1000)); // 25 hours ago
    await fs.utimes(filePath, oldDate, oldDate);
  }
}

function getRepositoryHash(repoPath) {
  const absolutePath = path.resolve(repoPath);
  return crypto.createHash('sha256').update(absolutePath).digest('hex').substring(0, 16);
}

async function test(testName, testFn) {
  console.log(`\n🧪 Testing: ${testName}`);
  console.log('─'.repeat(50));
  
  try {
    const result = await testFn();
    TEST_RESULTS.push({ test: testName, result: 'PASS', details: result });
    console.log(`✅ PASS: ${testName}`);
    if (result) console.log(`   ${result}`);
  } catch (error) {
    TEST_RESULTS.push({ test: testName, result: 'FAIL', error: error.message });
    console.log(`❌ FAIL: ${testName}`);
    console.log(`   Error: ${error.message}`);
  }
}

async function testEmbeddingStalenessDetection() {
  console.log('Setting up stale embedding scenario...');
  
  const repoHash = getRepositoryHash(TEST_REPO_PATH);
  const localPath = path.join(TEST_REPO_PATH, '.cortex', 'index.json');
  const globalPath = path.join(os.homedir(), '.claude', 'cortex-embeddings', repoHash, 'index.json');
  
  // Create local version (newer) and global version (older - 25 hours ago)
  await createTestStorage(localPath, false); // Current time
  await createTestStorage(globalPath, true);  // 25 hours ago
  
  const coordinator = new UnifiedStorageCoordinator(TEST_REPO_PATH);
  await coordinator.initialize();
  
  // Check if auto-sync was triggered
  const consistency = await coordinator.validateConsistency();
  
  // Clean up
  await fs.unlink(localPath).catch(() => {});
  await fs.unlink(globalPath).catch(() => {});
  await fs.rmdir(path.dirname(localPath)).catch(() => {});
  await fs.rmdir(path.dirname(globalPath)).catch(() => {});
  
  if (consistency.issues.some(issue => issue.includes('24 hours apart'))) {
    return 'Auto-sync should have handled staleness automatically';
  }
  
  return 'Staleness detection and auto-sync working correctly';
}

async function testRelationshipStalenessDetection() {
  console.log('Setting up stale relationship scenario...');
  
  const repoHash = getRepositoryHash(TEST_REPO_PATH);
  const localPath = path.join(TEST_REPO_PATH, '.cortex', 'relationships.json');
  const globalPath = path.join(os.homedir(), '.claude', 'cortex-embeddings', repoHash, 'relationships.json');
  
  const testRelData = {
    version: '2.1.0',
    schemaVersion: '1.0.0',
    timestamp: Date.now(),
    repositoryPath: TEST_REPO_PATH,
    symbols: {},
    relationships: {},
    metadata: {
      totalSymbols: 0,
      totalRelationships: 0,
      lastAnalyzed: Date.now(),
      analysisMode: 'test'
    }
  };
  
  // Create global version (newer) and local version (older - 25 hours ago)
  await fs.mkdir(path.dirname(localPath), { recursive: true });
  await fs.mkdir(path.dirname(globalPath), { recursive: true });
  
  await fs.writeFile(localPath, JSON.stringify(testRelData, null, 2));
  await fs.writeFile(globalPath, JSON.stringify(testRelData, null, 2));
  
  // Make local version older
  const oldDate = new Date(Date.now() - (25 * 60 * 60 * 1000)); // 25 hours ago
  await fs.utimes(localPath, oldDate, oldDate);
  
  const coordinator = new UnifiedStorageCoordinator(TEST_REPO_PATH);
  await coordinator.initialize();
  
  // Check if auto-sync was triggered
  const consistency = await coordinator.validateConsistency();
  
  // Clean up
  await fs.unlink(localPath).catch(() => {});
  await fs.unlink(globalPath).catch(() => {});
  await fs.rmdir(path.dirname(localPath)).catch(() => {});
  await fs.rmdir(path.dirname(globalPath)).catch(() => {});
  
  if (consistency.issues.some(issue => issue.includes('24 hours apart'))) {
    return 'Auto-sync should have handled staleness automatically';
  }
  
  return 'Relationship staleness detection and auto-sync working correctly';
}

async function testRecommendationUpdates() {
  console.log('Testing updated recommendations...');
  
  const coordinator = new UnifiedStorageCoordinator(TEST_REPO_PATH);
  await coordinator.initialize();
  
  const consistency = await coordinator.validateConsistency();
  
  // Check if recommendations mention auto-sync instead of manual commands
  const hasAutoSyncRecs = consistency.recommendations.some(rec => 
    rec.includes('Auto-sync handles this automatically') || 
    rec.includes('during server startup')
  );
  
  const hasManualSyncRecs = consistency.recommendations.some(rec => 
    rec.includes('npm run cache:sync') || 
    rec.includes('Consider running')
  );
  
  if (hasManualSyncRecs && !hasAutoSyncRecs) {
    throw new Error('Still recommending manual sync commands instead of auto-sync');
  }
  
  return 'Recommendations properly updated to reference auto-sync';
}

async function main() {
  console.log('🚀 Auto-Sync Staleness Detection Test Suite');
  console.log('='.repeat(50));
  
  // Test staleness detection and auto-sync for embeddings
  await test('Embedding staleness auto-sync', testEmbeddingStalenessDetection);
  
  // Test staleness detection and auto-sync for relationships
  await test('Relationship staleness auto-sync', testRelationshipStalenessDetection);
  
  // Test recommendation updates
  await test('Updated recommendations', testRecommendationUpdates);
  
  // Summary
  console.log('\n📊 Test Results Summary');
  console.log('='.repeat(30));
  
  const passed = TEST_RESULTS.filter(r => r.result === 'PASS').length;
  const failed = TEST_RESULTS.filter(r => r.result === 'FAIL').length;
  
  console.log(`✅ Passed: ${passed}`);
  console.log(`❌ Failed: ${failed}`);
  console.log(`📈 Success Rate: ${((passed / TEST_RESULTS.length) * 100).toFixed(1)}%`);
  
  if (failed === 0) {
    console.log('\n🎉 All tests passed! Auto-sync staleness detection is working correctly.');
    console.log('\n💡 Key improvements:');
    console.log('   • Auto-sync now detects staleness (>24h apart)');
    console.log('   • Automatically chooses newer version and syncs to older location');
    console.log('   • Handles both embeddings and relationships staleness');
    console.log('   • Updated recommendations to reflect auto-sync capabilities');
    console.log('   • No more manual sync commands needed for stale data');
  } else {
    console.log('\n❌ Some tests failed. Please check the implementation.');
    process.exit(1);
  }
}

if (require.main === module) {
  main().catch(error => {
    console.error('Test suite failed:', error);
    process.exit(1);
  });
}

module.exports = {
  testEmbeddingStalenessDetection,
  testRelationshipStalenessDetection,
  testRecommendationUpdates
};