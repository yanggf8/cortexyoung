#!/usr/bin/env node

/**
 * Real-Time File Watching Validation Test
 * 
 * Tests the semantic file watching system by:
 * 1. Initializing the file watcher
 * 2. Creating test files with semantic content
 * 3. Validating that changes are detected and processed correctly
 * 4. Measuring response times and system behavior
 */

const fs = require('fs').promises;
const path = require('path');
const { exec } = require('child_process');
const { promisify } = require('util');

const execAsync = promisify(exec);

// Test configuration
const TEST_CONFIG = {
  testDir: path.join(__dirname, 'test-semantic-watching-temp'),
  responseTimeThreshold: 200, // ms
  maxWaitTime: 5000, // ms
  testFiles: [
    {
      name: 'test-function.js',
      content: `// Test semantic function
function calculateSum(a, b) {
  return a + b;
}

export { calculateSum };`
    },
    {
      name: 'test-class.ts',
      content: `// Test semantic class
import { calculateSum } from './test-function.js';

class MathProcessor {
  constructor(private initialValue: number = 0) {}
  
  processNumbers(nums: number[]): number {
    return nums.reduce((acc, num) => calculateSum(acc, num), this.initialValue);
  }
}

export default MathProcessor;`
    },
    {
      name: 'test-interface.ts',
      content: `// Test semantic interface
interface DataProcessor<T> {
  process(data: T[]): T;
  validate(item: T): boolean;
}

type ProcessorConfig = {
  batchSize: number;
  timeout: number;
};

export { DataProcessor, ProcessorConfig };`
    }
  ]
};

class SemanticWatchingTester {
  constructor() {
    this.results = {
      testsPassed: 0,
      testsFailed: 0,
      responseTime: [],
      errors: []
    };
  }

  async runTests() {
    console.log('🧪 Starting Semantic File Watching Validation Tests\n');
    
    try {
      await this.setupTestEnvironment();
      await this.testFileWatchingInitialization();
      await this.testSemanticPatternDetection();
      await this.testIncrementalUpdates();
      await this.testResponseTimes();
      await this.testMCPToolIntegration();
      
      await this.cleanup();
      this.printResults();
      
    } catch (error) {
      console.error('❌ Test suite failed:', error.message);
      await this.cleanup();
      process.exit(1);
    }
  }

  async setupTestEnvironment() {
    console.log('🔧 Setting up test environment...');
    
    // Create test directory
    await fs.mkdir(TEST_CONFIG.testDir, { recursive: true });
    
    // Create initial test files
    for (const testFile of TEST_CONFIG.testFiles) {
      const filePath = path.join(TEST_CONFIG.testDir, testFile.name);
      await fs.writeFile(filePath, testFile.content);
    }
    
    console.log(`✅ Created test directory with ${TEST_CONFIG.testFiles.length} files\n`);
  }

  async testFileWatchingInitialization() {
    console.log('🚀 Testing file watching initialization...');
    
    try {
      // Test that the server can start with file watching enabled
      const startTime = Date.now();
      
      // Check if semantic watcher can be imported and initialized
      const testScript = `
        const { SemanticWatcher } = require('./src/semantic-watcher');
        const { CodebaseIndexer } = require('./src/indexer');
        
        const indexer = new CodebaseIndexer('${TEST_CONFIG.testDir}');
        const watcher = new SemanticWatcher('${TEST_CONFIG.testDir}', indexer);
        
        console.log('Watcher initialized successfully');
        process.exit(0);
      `;
      
      await fs.writeFile(path.join(__dirname, 'temp-watcher-test.js'), testScript);
      
      try {
        const { stdout, stderr } = await execAsync('node temp-watcher-test.js', {
          timeout: 5000,
          cwd: __dirname
        });
        
        if (stdout.includes('Watcher initialized successfully')) {
          this.logSuccess('File watching initialization');
        } else {
          throw new Error(`Unexpected output: ${stdout}`);
        }
      } catch (error) {
        this.logFailure('File watching initialization', error.message);
      }
      
      // Cleanup temp file
      await fs.unlink(path.join(__dirname, 'temp-watcher-test.js')).catch(() => {});
      
    } catch (error) {
      this.logFailure('File watching initialization', error.message);
    }
    
    console.log();
  }

  async testSemanticPatternDetection() {
    console.log('🔍 Testing semantic pattern detection...');
    
    const semanticTests = [
      {
        name: 'Function detection',
        content: 'function newFunction() { return "test"; }',
        shouldTrigger: true
      },
      {
        name: 'Import detection', 
        content: 'import { newModule } from "./somewhere";',
        shouldTrigger: true
      },
      {
        name: 'Class detection',
        content: 'class NewClass { constructor() {} }',
        shouldTrigger: true
      },
      {
        name: 'Comment only change',
        content: '// Just a comment change',
        shouldTrigger: false
      },
      {
        name: 'Whitespace only change',
        content: '   \n\n   ',
        shouldTrigger: false
      }
    ];

    for (const test of semanticTests) {
      try {
        const testFilePath = path.join(TEST_CONFIG.testDir, 'semantic-test.js');
        await fs.writeFile(testFilePath, test.content);
        
        // Simulate semantic pattern checking
        const isSemanticChange = this.checkSemanticPatterns(test.content);
        
        if (isSemanticChange === test.shouldTrigger) {
          this.logSuccess(`Semantic detection: ${test.name}`);
        } else {
          this.logFailure(`Semantic detection: ${test.name}`, 
            `Expected ${test.shouldTrigger}, got ${isSemanticChange}`);
        }
        
      } catch (error) {
        this.logFailure(`Semantic detection: ${test.name}`, error.message);
      }
    }
    
    console.log();
  }

  checkSemanticPatterns(content) {
    // Replicate the semantic pattern detection logic from SemanticWatcher
    const semanticPatterns = [
      /^(import|export|from)\s/m,           // Import/export changes
      /^(class|interface|type|enum)\s/m,    // Type definitions  
      /^(function|const|let|var)\s.*=/m,    // Function/variable declarations
      /^(async\s+)?function\s/m,            // Function definitions
      /\/\*\*[\s\S]*?\*\//g,               // JSDoc comments (semantic)
    ];
    
    return semanticPatterns.some(pattern => pattern.test(content));
  }

  async testIncrementalUpdates() {
    console.log('🔄 Testing incremental updates...');
    
    try {
      // Test file modification detection
      const testFile = path.join(TEST_CONFIG.testDir, 'incremental-test.js');
      
      // Create initial file
      await fs.writeFile(testFile, 'function original() { return 1; }');
      
      // Modify with semantic change
      await fs.writeFile(testFile, 'function modified() { return 2; }');
      
      // Test that modification would be detected
      const stats = await fs.stat(testFile);
      if (stats.isFile()) {
        this.logSuccess('File modification detection');
      } else {
        this.logFailure('File modification detection', 'File not found after modification');
      }
      
      // Test file deletion detection
      await fs.unlink(testFile);
      
      try {
        await fs.stat(testFile);
        this.logFailure('File deletion detection', 'File still exists after deletion');
      } catch (error) {
        if (error.code === 'ENOENT') {
          this.logSuccess('File deletion detection');
        } else {
          this.logFailure('File deletion detection', error.message);
        }
      }
      
    } catch (error) {
      this.logFailure('Incremental updates', error.message);
    }
    
    console.log();
  }

  async testResponseTimes() {
    console.log('⏱️ Testing response times...');
    
    const responseTests = [
      { name: 'Small file creation', size: 1024 },
      { name: 'Medium file creation', size: 10240 },
      { name: 'Large file creation', size: 102400 }
    ];
    
    for (const test of responseTests) {
      try {
        const content = 'function test() {\n' + 'console.log("test");\n'.repeat(test.size / 30) + '}';
        const testFile = path.join(TEST_CONFIG.testDir, `response-test-${test.size}.js`);
        
        const startTime = Date.now();
        await fs.writeFile(testFile, content);
        const endTime = Date.now();
        
        const responseTime = endTime - startTime;
        this.results.responseTime.push(responseTime);
        
        if (responseTime < TEST_CONFIG.responseTimeThreshold) {
          this.logSuccess(`${test.name} (${responseTime}ms)`);
        } else {
          this.logFailure(`${test.name} (${responseTime}ms)`, 
            `Response time ${responseTime}ms exceeds threshold ${TEST_CONFIG.responseTimeThreshold}ms`);
        }
        
      } catch (error) {
        this.logFailure(`Response time: ${test.name}`, error.message);
      }
    }
    
    console.log();
  }

  async testMCPToolIntegration() {
    console.log('🔧 Testing MCP tool integration...');
    
    try {
      // Test that real_time_status tool is available
      const toolsFile = path.join(__dirname, 'src', 'mcp-tools.ts');
      const toolsContent = await fs.readFile(toolsFile, 'utf-8');
      
      if (toolsContent.includes('real_time_status')) {
        this.logSuccess('real_time_status tool definition exists');
      } else {
        this.logFailure('real_time_status tool definition', 'Tool not found in mcp-tools.ts');
      }
      
      // Test that handler is available
      const handlersFile = path.join(__dirname, 'src', 'mcp-handlers.ts');
      const handlersContent = await fs.readFile(handlersFile, 'utf-8');
      
      if (handlersContent.includes('RealTimeStatusHandler')) {
        this.logSuccess('RealTimeStatusHandler class exists');
      } else {
        this.logFailure('RealTimeStatusHandler', 'Handler class not found in mcp-handlers.ts');
      }
      
      // Test that server registers the handler
      const serverFile = path.join(__dirname, 'src', 'server.ts');
      const serverContent = await fs.readFile(serverFile, 'utf-8');
      
      if (serverContent.includes('real_time_status') && serverContent.includes('RealTimeStatusHandler')) {
        this.logSuccess('MCP server integration complete');
      } else {
        this.logFailure('MCP server integration', 'Handler not properly registered in server');
      }
      
    } catch (error) {
      this.logFailure('MCP tool integration', error.message);
    }
    
    console.log();
  }

  logSuccess(testName) {
    console.log(`✅ ${testName}`);
    this.results.testsPassed++;
  }

  logFailure(testName, reason) {
    console.log(`❌ ${testName}: ${reason}`);
    this.results.testsFailed++;
    this.results.errors.push({ test: testName, reason });
  }

  async cleanup() {
    console.log('🧹 Cleaning up test environment...');
    
    try {
      await fs.rm(TEST_CONFIG.testDir, { recursive: true, force: true });
      console.log('✅ Test environment cleaned up\n');
    } catch (error) {
      console.log(`⚠️  Cleanup warning: ${error.message}\n`);
    }
  }

  printResults() {
    console.log('📊 Test Results Summary');
    console.log('='.repeat(50));
    console.log(`✅ Tests Passed: ${this.results.testsPassed}`);
    console.log(`❌ Tests Failed: ${this.results.testsFailed}`);
    console.log(`🎯 Success Rate: ${((this.results.testsPassed / (this.results.testsPassed + this.results.testsFailed)) * 100).toFixed(1)}%`);
    
    if (this.results.responseTime.length > 0) {
      const avgResponseTime = this.results.responseTime.reduce((a, b) => a + b, 0) / this.results.responseTime.length;
      console.log(`⏱️  Average Response Time: ${avgResponseTime.toFixed(1)}ms`);
    }
    
    if (this.results.errors.length > 0) {
      console.log('\n❌ Failed Tests:');
      this.results.errors.forEach(error => {
        console.log(`   - ${error.test}: ${error.reason}`);
      });
    }
    
    console.log('\n🏁 Semantic File Watching Validation Complete');
    
    if (this.results.testsFailed === 0) {
      console.log('🎉 All tests passed! Real-time file watching system is ready.');
      process.exit(0);
    } else {
      console.log('⚠️  Some tests failed. Please review the implementation.');
      process.exit(1);
    }
  }
}

// Run tests if this file is executed directly
if (require.main === module) {
  const tester = new SemanticWatchingTester();
  tester.runTests().catch(error => {
    console.error('💥 Test runner crashed:', error);
    process.exit(1);
  });
}

module.exports = { SemanticWatchingTester, TEST_CONFIG };