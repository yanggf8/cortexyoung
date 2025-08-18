const { FileWatcher } = require('./dist/file-watcher');
const { ActivityDetector } = require('./dist/activity-detector');
const { ChangeProcessor } = require('./dist/change-processor');
const fs = require('fs/promises');
const path = require('path');

async function testIntelligentFileWatching() {
  console.log('🧪 Testing Intelligent File Watching System...');
  console.log('=' .repeat(80));
  
  // Create test directory
  const testDir = path.join(process.cwd(), 'tmp_rovodev_intelligent_watch_test');
  await fs.mkdir(testDir, { recursive: true });
  
  try {
    // Initialize components
    const activityDetector = new ActivityDetector();
    const changeProcessor = new ChangeProcessor(activityDetector, {
      debounceMs: 200,
      batchSize: 5,
      maxQueueSize: 100
    });
    
    const fileWatcher = new FileWatcher({
      repositoryPath: testDir,
      debounceMs: 200,
      ignorePatterns: ['**/node_modules/**'],
      enableContentAnalysis: true,
      analysisThreshold: 15 // Lower threshold for testing
    });
    
    // Set up event handlers
    let fileChangeCount = 0;
    let indexableChangeCount = 0;
    let batchCount = 0;
    const processedFiles = [];
    
    fileWatcher.on('fileChange', (event) => {
      fileChangeCount++;
      console.log(`📁 File change: ${event.type} ${event.relativePath}`);
      console.log(`   Priority: ${event.indexingPriority}, Should Index: ${event.shouldIndex ? '✅' : '❌'}`);
      console.log(`   Reason: ${event.filterReason}`);
      
      if (event.contentAnalysis) {
        console.log(`   Analysis: ${event.contentAnalysis.language}, importance: ${event.contentAnalysis.estimatedImportance}/100`);
      }
      
      changeProcessor.processChange(event);
    });
    
    fileWatcher.on('indexableFileChange', (event) => {
      indexableChangeCount++;
      console.log(`🎯 Indexable file: ${event.relativePath} (${event.indexingPriority})`);
    });
    
    process.on('cortex:fileChangeBatch', (batch) => {
      batchCount++;
      console.log(`\n🔄 Processing batch ${batchCount}:`);
      console.log(`   Files: ${batch.events.length}`);
      console.log(`   Highest Priority: ${batch.highestPriority}`);
      console.log(`   Total Importance: ${batch.totalImportance.toFixed(1)}`);
      
      batch.events.forEach(event => {
        processedFiles.push({
          path: event.relativePath,
          priority: event.indexingPriority,
          importance: event.contentAnalysis?.estimatedImportance || 0
        });
        console.log(`     - ${event.relativePath} (${event.indexingPriority}, ${event.contentAnalysis?.estimatedImportance || 0}/100)`);
      });
    });
    
    // Initialize and start watching
    await fileWatcher.initialize();
    await fileWatcher.start();
    
    console.log('\n✅ Intelligent file watcher started');
    console.log('📝 Creating test files with different characteristics...\n');
    
    // Test 1: High-value TypeScript file
    await fs.writeFile(path.join(testDir, 'main.ts'), `
import { Component } from './types';

export class MainApplication {
  private components: Component[] = [];
  
  /**
   * Initialize the application with components
   * @param components Array of components to register
   */
  async initialize(components: Component[]): Promise<void> {
    this.components = components;
    await this.startServices();
  }
  
  private async startServices(): Promise<void> {
    // Complex initialization logic
    for (const component of this.components) {
      await component.start();
    }
  }
}

export default MainApplication;
`);
    await new Promise(resolve => setTimeout(resolve, 100));
    
    // Test 2: Configuration file
    await fs.writeFile(path.join(testDir, 'config.json'), JSON.stringify({
      name: "test-app",
      version: "1.0.0",
      dependencies: {
        "typescript": "^5.0.0"
      },
      scripts: {
        "build": "tsc",
        "test": "jest"
      }
    }, null, 2));
    await new Promise(resolve => setTimeout(resolve, 100));
    
    // Test 3: Documentation file
    await fs.writeFile(path.join(testDir, 'README.md'), `
# Test Application

This is a comprehensive test application for demonstrating intelligent file watching.

## Features

- **Smart Analysis**: Automatically analyzes file content and importance
- **Priority Processing**: Processes high-importance files first
- **Activity Detection**: Adapts to development activity patterns

## Usage

\`\`\`typescript
import MainApplication from './main';

const app = new MainApplication();
await app.initialize(components);
\`\`\`

## Configuration

See \`config.json\` for configuration options.
`);
    await new Promise(resolve => setTimeout(resolve, 100));
    
    // Test 4: Simple debug file (should be low priority)
    await fs.writeFile(path.join(testDir, 'debug.js'), `
console.log("Debug output");
console.log("Testing something");
// Just a temporary debug file
`);
    await new Promise(resolve => setTimeout(resolve, 100));
    
    // Test 5: Test file
    await fs.writeFile(path.join(testDir, 'main.test.ts'), `
import { MainApplication } from './main';

describe('MainApplication', () => {
  it('should initialize correctly', async () => {
    const app = new MainApplication();
    await app.initialize([]);
    expect(app).toBeDefined();
  });
  
  it('should handle components', async () => {
    const app = new MainApplication();
    const mockComponent = { start: jest.fn() };
    await app.initialize([mockComponent]);
    expect(mockComponent.start).toHaveBeenCalled();
  });
});
`);
    await new Promise(resolve => setTimeout(resolve, 100));
    
    // Test 6: Rapid changes (should trigger activity detection)
    console.log('🔥 Testing rapid changes (activity detection)...');
    for (let i = 0; i < 15; i++) {
      await fs.writeFile(path.join(testDir, `rapid-${i}.js`), `console.log("Rapid change ${i}");`);
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    
    // Wait for processing
    console.log('\n⏳ Waiting for processing to complete...');
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    // Process any remaining changes
    await changeProcessor.processAllPending();
    
    // Test 7: Modify existing file
    console.log('\n📝 Testing file modification...');
    await fs.writeFile(path.join(testDir, 'main.ts'), `
import { Component } from './types';

export class MainApplication {
  private components: Component[] = [];
  private isInitialized: boolean = false;
  
  /**
   * Initialize the application with components
   * @param components Array of components to register
   */
  async initialize(components: Component[]): Promise<void> {
    if (this.isInitialized) {
      throw new Error('Application already initialized');
    }
    
    this.components = components;
    await this.startServices();
    this.isInitialized = true;
  }
  
  private async startServices(): Promise<void> {
    // Enhanced initialization logic with error handling
    for (const component of this.components) {
      try {
        await component.start();
      } catch (error) {
        console.error('Failed to start component:', error);
        throw error;
      }
    }
  }
  
  async shutdown(): Promise<void> {
    for (const component of this.components) {
      await component.stop?.();
    }
    this.isInitialized = false;
  }
}

export default MainApplication;
`);
    
    // Wait for final processing
    await new Promise(resolve => setTimeout(resolve, 1000));
    await changeProcessor.processAllPending();
    
    // Display results
    console.log('\n📊 Test Results:');
    console.log('=' .repeat(80));
    
    console.log(`📁 Total file changes detected: ${fileChangeCount}`);
    console.log(`🎯 Indexable changes: ${indexableChangeCount}`);
    console.log(`🔄 Processing batches: ${batchCount}`);
    
    // Activity stats
    const activityStats = activityDetector.getStats();
    console.log(`\n📈 Activity Statistics:`);
    console.log(`   Active files: ${activityStats.totalFiles}`);
    console.log(`   Total changes: ${activityStats.totalChanges}`);
    console.log(`   Average changes per file: ${activityStats.averageChangesPerFile.toFixed(2)}`);
    
    // Processing stats
    const processingStats = changeProcessor.getProcessingStats();
    console.log(`\n⚡ Processing Statistics:`);
    console.log(`   Total processed: ${processingStats.totalProcessed}`);
    console.log(`   Total skipped: ${processingStats.totalSkipped}`);
    console.log(`   Average processing time: ${processingStats.averageProcessingTime.toFixed(1)}ms`);
    console.log(`   Last batch size: ${processingStats.lastBatchSize}`);
    
    // Queue stats
    const queueStats = changeProcessor.getQueueStats();
    console.log(`\n📋 Queue Statistics:`);
    console.log(`   Queue size: ${queueStats.queueSize}`);
    console.log(`   Pending debounces: ${queueStats.pendingDebounces}`);
    console.log(`   Priority distribution:`, queueStats.priorityDistribution);
    console.log(`   Average importance: ${queueStats.averageImportance.toFixed(1)}/100`);
    
    // Cache stats
    const cacheStats = fileWatcher.getAnalysisCacheStats();
    console.log(`\n💾 Analysis Cache Statistics:`);
    console.log(`   Cache size: ${cacheStats.size} entries`);
    if (cacheStats.oldestEntry) {
      console.log(`   Oldest entry: ${cacheStats.oldestEntry.toISOString()}`);
      console.log(`   Newest entry: ${cacheStats.newestEntry?.toISOString()}`);
    }
    
    // Processed files summary
    console.log(`\n📋 Processed Files Summary:`);
    const sortedFiles = processedFiles.sort((a, b) => b.importance - a.importance);
    sortedFiles.forEach((file, index) => {
      console.log(`   ${index + 1}. ${file.path} (${file.priority}, ${file.importance}/100)`);
    });
    
    // Stop watcher
    await fileWatcher.stop();
    console.log('\n✅ File watcher stopped');
    
  } finally {
    // Cleanup
    await fs.rm(testDir, { recursive: true, force: true });
    console.log('🧹 Test directory cleaned up');
  }
  
  console.log('\n🎉 Intelligent File Watching test completed!');
}

if (require.main === module) {
  testIntelligentFileWatching().catch(console.error);
}

module.exports = { testIntelligentFileWatching };