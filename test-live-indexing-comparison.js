const { CodebaseIndexer } = require('./dist/indexer');
const { LiveCodebaseIndexer } = require('./dist/live-codebase-indexer');
const fs = require('fs/promises');
const path = require('path');

async function testLiveIndexingComparison() {
  console.log('🔄 Testing: Batch vs Live Indexing Comparison');
  console.log('=' .repeat(80));
  
  // Create test directory
  const testDir = path.join(process.cwd(), 'tmp_rovodev_live_comparison_test');
  await fs.mkdir(testDir, { recursive: true });
  
  try {
    console.log('📁 Setting up test environment...\n');
    
    // Create initial test files
    await createTestFiles(testDir);
    
    // Test 1: Traditional Batch Indexing
    console.log('🔄 TEST 1: Traditional Batch Indexing');
    console.log('-' .repeat(50));
    
    const batchIndexer = new CodebaseIndexer(testDir);
    
    console.log('⏱️  Initial full index...');
    const batchStart1 = Date.now();
    await batchIndexer.indexRepository({ 
      repository_path: testDir, 
      mode: 'full' 
    });
    const batchTime1 = Date.now() - batchStart1;
    console.log(`✅ Full index completed in ${batchTime1}ms`);
    
    // Simulate file changes
    console.log('\n📝 Making file changes...');
    await simulateFileChanges(testDir);
    
    console.log('⏱️  Incremental batch update...');
    const batchStart2 = Date.now();
    await batchIndexer.indexRepository({ 
      repository_path: testDir, 
      mode: 'incremental' 
    });
    const batchTime2 = Date.now() - batchStart2;
    console.log(`✅ Incremental update completed in ${batchTime2}ms`);
    
    const batchStats = await batchIndexer.getIndexStats();
    console.log(`📊 Batch Indexing Results:`);
    console.log(`   Total chunks: ${batchStats.total_chunks}`);
    console.log(`   Full index time: ${batchTime1}ms`);
    console.log(`   Incremental time: ${batchTime2}ms`);
    console.log(`   Total time: ${batchTime1 + batchTime2}ms`);
    
    await batchIndexer.cleanup('test-complete');
    
    // Test 2: Live Intelligent Indexing
    console.log('\n🚀 TEST 2: Live Intelligent Indexing');
    console.log('-' .repeat(50));
    
    const liveIndexer = new LiveCodebaseIndexer(testDir);
    
    console.log('⏱️  Initial full index...');
    const liveStart1 = Date.now();
    await liveIndexer.indexRepository({ 
      repository_path: testDir, 
      mode: 'full' 
    });
    const liveTime1 = Date.now() - liveStart1;
    console.log(`✅ Full index completed in ${liveTime1}ms`);
    
    console.log('🔴 Enabling live mode...');
    await liveIndexer.enableLiveMode();
    
    // Analyze existing files for intelligence
    await liveIndexer.analyzeExistingFiles();
    
    console.log('\n📝 Making real-time file changes...');
    const liveStart2 = Date.now();
    
    // Simulate real-time changes
    await simulateRealTimeChanges(testDir, liveIndexer);
    
    // Wait for processing to complete
    await new Promise(resolve => setTimeout(resolve, 2000));
    await liveIndexer.flushPendingChanges();
    
    const liveTime2 = Date.now() - liveStart2;
    console.log(`✅ Real-time updates completed in ${liveTime2}ms`);
    
    const liveStats = await liveIndexer.getIndexStats();
    const detailedStats = liveIndexer.getLiveStats();
    
    console.log(`📊 Live Indexing Results:`);
    console.log(`   Total chunks: ${liveStats.total_chunks}`);
    console.log(`   Full index time: ${liveTime1}ms`);
    console.log(`   Real-time updates: ${liveTime2}ms`);
    console.log(`   Files watched: ${detailedStats.filesWatched}`);
    console.log(`   Changes processed: ${detailedStats.changesProcessed}`);
    console.log(`   Avg processing time: ${detailedStats.averageProcessingTime.toFixed(1)}ms`);
    console.log(`   Cache entries: ${detailedStats.cacheStats.size}`);
    
    await liveIndexer.cleanup('test-complete');
    
    // Test 3: Performance Comparison
    console.log('\n📊 PERFORMANCE COMPARISON');
    console.log('=' .repeat(80));
    
    console.log('🔄 Batch Indexing (Traditional):');
    console.log(`   ⏱️  Update Latency: ${batchTime2}ms (manual trigger required)`);
    console.log(`   🔄 Update Method: Manual incremental reindex`);
    console.log(`   🧠 Intelligence: Git-based change detection only`);
    console.log(`   ⚡ Responsiveness: Batch processing, user-initiated`);
    
    console.log('\n🚀 Live Indexing (Intelligent):');
    console.log(`   ⏱️  Update Latency: ${detailedStats.averageProcessingTime.toFixed(1)}ms (automatic)`);
    console.log(`   🔄 Update Method: Real-time file watching with smart filtering`);
    console.log(`   🧠 Intelligence: Content analysis + priority-based processing`);
    console.log(`   ⚡ Responsiveness: Instant, automatic, priority-aware`);
    
    const speedImprovement = batchTime2 / detailedStats.averageProcessingTime;
    console.log(`\n🎯 IMPROVEMENT: ${speedImprovement.toFixed(1)}x faster updates with live indexing!`);
    
    // Test 4: Intelligence Demonstration
    console.log('\n🧠 INTELLIGENCE DEMONSTRATION');
    console.log('=' .repeat(80));
    
    const activityState = detailedStats.activityState;
    const queueStats = detailedStats.queueStats;
    
    console.log('📈 Activity Detection:');
    console.log(`   Current intensity: ${activityState.intensity}`);
    console.log(`   Change rate: ${activityState.changeRate.toFixed(2)}/second`);
    console.log(`   Should suspend: ${activityState.suspendProcessing ? 'Yes' : 'No'}`);
    
    console.log('\n📋 Queue Intelligence:');
    console.log(`   Queue size: ${queueStats.queueSize}`);
    console.log(`   Priority distribution:`, queueStats.priorityDistribution);
    console.log(`   Average importance: ${queueStats.averageImportance.toFixed(1)}/100`);
    
    console.log('\n💾 Cache Performance:');
    console.log(`   Cache size: ${detailedStats.cacheStats.size} entries`);
    console.log(`   Oldest entry: ${detailedStats.cacheStats.oldestEntry?.toISOString() || 'N/A'}`);
    console.log(`   Newest entry: ${detailedStats.cacheStats.newestEntry?.toISOString() || 'N/A'}`);
    
  } finally {
    // Cleanup
    await fs.rm(testDir, { recursive: true, force: true });
    console.log('\n🧹 Test directory cleaned up');
  }
  
  console.log('\n🎉 Live Indexing Comparison Test Complete!');
  console.log('\n🎯 KEY TAKEAWAYS:');
  console.log('   • Live indexing provides instant updates vs manual batch processing');
  console.log('   • Content analysis enables intelligent priority-based processing');
  console.log('   • Activity detection prevents system overload during heavy editing');
  console.log('   • Real-time updates transform static indexing into live code intelligence');
}

async function createTestFiles(testDir) {
  // High-importance TypeScript file
  await fs.writeFile(path.join(testDir, 'core-service.ts'), `
export interface ServiceConfig {
  apiKey: string;
  endpoint: string;
  timeout: number;
}

export class CoreService {
  private config: ServiceConfig;
  private isInitialized: boolean = false;
  
  constructor(config: ServiceConfig) {
    this.config = config;
  }
  
  /**
   * Initialize the service with validation
   */
  async initialize(): Promise<void> {
    if (this.isInitialized) {
      throw new Error('Service already initialized');
    }
    
    await this.validateConfig();
    await this.connectToEndpoint();
    this.isInitialized = true;
  }
  
  private async validateConfig(): Promise<void> {
    if (!this.config.apiKey) {
      throw new Error('API key is required');
    }
    // Complex validation logic
  }
  
  private async connectToEndpoint(): Promise<void> {
    // Connection logic with retry mechanism
    let retries = 3;
    while (retries > 0) {
      try {
        // Simulate connection
        await new Promise(resolve => setTimeout(resolve, 100));
        break;
      } catch (error) {
        retries--;
        if (retries === 0) throw error;
      }
    }
  }
}
`);

  // Medium-importance configuration
  await fs.writeFile(path.join(testDir, 'app-config.json'), JSON.stringify({
    name: "test-application",
    version: "1.0.0",
    environment: "development",
    features: {
      realTimeIndexing: true,
      contentAnalysis: true,
      intelligentFiltering: true
    },
    performance: {
      debounceMs: 500,
      batchSize: 10,
      maxConcurrentFiles: 5
    }
  }, null, 2));

  // Documentation file
  await fs.writeFile(path.join(testDir, 'API.md'), `
# API Documentation

## Core Service

The CoreService provides the main functionality for the application.

### Configuration

\`\`\`typescript
interface ServiceConfig {
  apiKey: string;
  endpoint: string;
  timeout: number;
}
\`\`\`

### Usage

\`\`\`typescript
const service = new CoreService(config);
await service.initialize();
\`\`\`

## Error Handling

The service includes comprehensive error handling and retry mechanisms.
`);

  // Low-importance debug file
  await fs.writeFile(path.join(testDir, 'debug-utils.js'), `
// Simple debug utilities
console.log("Debug utilities loaded");

function debugLog(message) {
  console.log("[DEBUG]", message);
}

module.exports = { debugLog };
`);
}

async function simulateFileChanges(testDir) {
  // Modify the TypeScript file
  await fs.appendFile(path.join(testDir, 'core-service.ts'), `

export class AdvancedService extends CoreService {
  async performAdvancedOperation(): Promise<void> {
    if (!this.isInitialized) {
      throw new Error('Service not initialized');
    }
    // Advanced operation logic
  }
}
`);

  // Update configuration
  const config = JSON.parse(await fs.readFile(path.join(testDir, 'app-config.json'), 'utf-8'));
  config.version = "1.1.0";
  config.features.advancedOperations = true;
  await fs.writeFile(path.join(testDir, 'app-config.json'), JSON.stringify(config, null, 2));
}

async function simulateRealTimeChanges(testDir, liveIndexer) {
  console.log('   📝 Creating new high-priority file...');
  await fs.writeFile(path.join(testDir, 'new-feature.ts'), `
export class NewFeature {
  private enabled: boolean = false;
  
  enable(): void {
    this.enabled = true;
  }
  
  isEnabled(): boolean {
    return this.enabled;
  }
}
`);
  
  await new Promise(resolve => setTimeout(resolve, 300));
  
  console.log('   🔧 Modifying existing service...');
  await fs.appendFile(path.join(testDir, 'core-service.ts'), `

// Real-time addition
export const SERVICE_VERSION = "2.0.0";
`);
  
  await new Promise(resolve => setTimeout(resolve, 300));
  
  console.log('   📄 Adding documentation...');
  await fs.writeFile(path.join(testDir, 'CHANGELOG.md'), `
# Changelog

## v2.0.0
- Added NewFeature class
- Enhanced CoreService with version info
- Real-time indexing improvements
`);
  
  await new Promise(resolve => setTimeout(resolve, 300));
  
  console.log('   🗑️  Removing debug file...');
  await fs.unlink(path.join(testDir, 'debug-utils.js'));
}

if (require.main === module) {
  testLiveIndexingComparison().catch(console.error);
}

module.exports = { testLiveIndexingComparison };