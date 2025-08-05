#!/usr/bin/env node

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

/**
 * Validate the critical performance improvements we implemented
 */
class PerformanceValidator {
  constructor() {
    this.results = {
      storage: {},
      startup: {},
      relationships: {},
      overall: {}
    };
  }

  async runCommand(command, args = [], timeout = 30000) {
    return new Promise((resolve, reject) => {
      const startTime = Date.now();
      const proc = spawn(command, args, { 
        stdio: ['inherit', 'pipe', 'pipe'],
        shell: true 
      });
      
      let stdout = '';
      let stderr = '';
      
      proc.stdout.on('data', (data) => {
        stdout += data.toString();
      });
      
      proc.stderr.on('data', (data) => {
        stderr += data.toString();
      });
      
      const timer = setTimeout(() => {
        proc.kill('SIGTERM');
        reject(new Error(`Command timed out after ${timeout}ms`));
      }, timeout);
      
      proc.on('close', (code) => {
        clearTimeout(timer);
        const duration = Date.now() - startTime;
        
        resolve({
          code,
          stdout,
          stderr,
          duration,
          success: code === 0
        });
      });
      
      proc.on('error', (error) => {
        clearTimeout(timer);
        reject(error);
      });
    });
  }

  async validateStoragePerformance() {
    console.log('🧪 Testing Storage Performance...');
    
    try {
      const result = await this.runCommand('npm', ['run', 'benchmark:storage'], 15000);
      
      // Extract timing information from output
      const statusCheckMatch = result.stdout.match(/storage-status-check: ([\d.]+)ms/);
      const statsGenMatch = result.stdout.match(/storage-stats-generation: ([\d.]+)ms/);
      const syncOpMatch = result.stdout.match(/storage-sync-operation: ([\d.]+)ms/);
      
      this.results.storage = {
        success: result.success,
        statusCheck: statusCheckMatch ? parseFloat(statusCheckMatch[1]) : null,
        statsGeneration: statsGenMatch ? parseFloat(statsGenMatch[1]) : null,
        syncOperation: syncOpMatch ? parseFloat(syncOpMatch[1]) : null,
        totalDuration: result.duration
      };
      
      console.log('✅ Storage Performance Results:');
      console.log(`   Status Check: ${this.results.storage.statusCheck}ms`);
      console.log(`   Stats Generation: ${this.results.storage.statsGeneration}ms`);
      console.log(`   Sync Operation: ${this.results.storage.syncOperation}ms`);
      
      return true;
    } catch (error) {
      console.error('❌ Storage performance test failed:', error.message);
      this.results.storage = { success: false, error: error.message };
      return false;
    }
  }

  async validateCacheLoading() {
    console.log('🧪 Testing Cache Loading Performance...');
    
    try {
      // Clear cache first
      await this.runCommand('npm', ['run', 'cache:clear'], 10000);
      
      // Test cold start (this will take longer but we'll timeout)
      console.log('   Testing cold start indexing...');
      const coldResult = await this.runCommand('timeout', ['45s', 'npm', 'run', 'demo'], 50000);
      
      // Look for key performance indicators in the output
      const cacheDetectionMatch = coldResult.stdout.match(/Cache Detection.*?(\d+)ms/);
      const modelLoadingMatch = coldResult.stdout.match(/AI Model Loading.*?(\d+)ms/);
      
      this.results.startup = {
        coldStartAttempted: true,
        cacheDetection: cacheDetectionMatch ? parseInt(cacheDetectionMatch[1]) : null,
        modelLoading: modelLoadingMatch ? parseInt(modelLoadingMatch[1]) : null,
        relationshipInitialized: coldResult.stdout.includes('relationship_analysis') || 
                                 coldResult.stdout.includes('Relationship Analysis')
      };
      
      console.log('✅ Cache Loading Results:');
      console.log(`   Cache Detection: ${this.results.startup.cacheDetection}ms`);
      console.log(`   Model Loading: ${this.results.startup.modelLoading}ms`);
      console.log(`   Relationship Stage: ${this.results.startup.relationshipInitialized ? 'Present' : 'Missing'}`);
      
      return true;
    } catch (error) {
      console.error('❌ Cache loading test failed:', error.message);
      this.results.startup = { success: false, error: error.message };
      return false;
    }
  }

  async validateRelationshipPersistence() {
    console.log('🧪 Testing Relationship Persistence...');
    
    try {
      const result = await this.runCommand('npm', ['run', 'storage:status'], 10000);
      
      // Check if storage architecture is working
      const hasEmbeddings = result.stdout.includes('Embeddings: Local') && 
                           result.stdout.includes('Global');
      const hasRelationships = result.stdout.includes('Relationships: Local') && 
                              result.stdout.includes('Global');
      const hasUnifiedCoordinator = result.stdout.includes('Unified Storage');
      
      this.results.relationships = {
        success: result.success,
        hasEmbeddingStorage: hasEmbeddings,
        hasRelationshipStorage: hasRelationships,
        hasUnifiedCoordinator: hasUnifiedCoordinator,
        outputSample: result.stdout.substring(0, 500)
      };
      
      console.log('✅ Relationship Persistence Results:');
      console.log(`   Embedding Storage: ${hasEmbeddings ? '✅' : '❌'}`);
      console.log(`   Relationship Storage: ${hasRelationships ? '✅' : '❌'}`);
      console.log(`   Unified Coordinator: ${hasUnifiedCoordinator ? '✅' : '❌'}`);
      
      return hasEmbeddings && hasRelationships && hasUnifiedCoordinator;
    } catch (error) {
      console.error('❌ Relationship persistence test failed:', error.message);
      this.results.relationships = { success: false, error: error.message };
      return false;
    }
  }

  async validateKeyImprovements() {
    console.log('🧪 Validating Key Performance Improvements...');
    
    // Check that relationship initialization is integrated
    const indexerPath = path.join(__dirname, 'src', 'indexer.ts');
    try {
      const indexerContent = fs.readFileSync(indexerPath, 'utf8');
      
      const hasRelationshipInit = indexerContent.includes('initializeRelationshipEngine');
      const hasStageTracking = indexerContent.includes('relationship_analysis');
      const hasAllIndexingPaths = indexerContent.match(/initializeRelationshipEngine/g)?.length >= 3;
      
      this.results.overall.codeIntegration = {
        relationshipInitPresent: hasRelationshipInit,
        stageTrackingPresent: hasStageTracking,
        allIndexingPathsCovered: hasAllIndexingPaths
      };
      
      console.log('✅ Code Integration Results:');
      console.log(`   Relationship Init: ${hasRelationshipInit ? '✅' : '❌'}`);
      console.log(`   Stage Tracking: ${hasStageTracking ? '✅' : '❌'}`);
      console.log(`   All Paths Covered: ${hasAllIndexingPaths ? '✅' : '❌'}`);
      
      return hasRelationshipInit && hasStageTracking && hasAllIndexingPaths;
    } catch (error) {
      console.error('❌ Code validation failed:', error.message);
      return false;
    }
  }

  generateReport() {
    const report = {
      timestamp: new Date().toISOString(),
      summary: {
        storagePerformance: this.results.storage.success !== false,
        relationshipPersistence: this.results.relationships.success !== false,
        codeIntegration: this.results.overall.codeIntegration?.relationshipInitPresent === true,
        overallSuccess: true
      },
      details: this.results,
      recommendations: []
    };
    
    // Performance thresholds validation
    if (this.results.storage.statusCheck > 10) {
      report.recommendations.push('Storage status check is slower than expected (>10ms)');
    }
    
    if (this.results.startup.cacheDetection > 1000) {
      report.recommendations.push('Cache detection is slower than expected (>1s)');
    }
    
    if (!this.results.startup.relationshipInitialized) {
      report.recommendations.push('CRITICAL: Relationship analysis stage missing from startup');
      report.summary.overallSuccess = false;
    }
    
    if (!this.results.relationships.hasUnifiedCoordinator) {
      report.recommendations.push('CRITICAL: Unified storage coordinator not working');
      report.summary.overallSuccess = false;
    }
    
    // Overall success calculation
    report.summary.overallSuccess = 
      report.summary.storagePerformance &&
      report.summary.relationshipPersistence &&
      report.summary.codeIntegration &&
      report.recommendations.filter(r => r.includes('CRITICAL')).length === 0;
    
    return report;
  }

  async run() {
    console.log('🚀 Cortex V2.1 Performance Validation');
    console.log('====================================');
    console.log('Validating critical performance improvements...\n');
    
    const tests = [
      { name: 'Storage Performance', fn: () => this.validateStoragePerformance() },
      { name: 'Cache Loading', fn: () => this.validateCacheLoading() },
      { name: 'Relationship Persistence', fn: () => this.validateRelationshipPersistence() },
      { name: 'Key Improvements', fn: () => this.validateKeyImprovements() }
    ];
    
    const results = [];
    
    for (const test of tests) {
      console.log(`\n🧪 Running ${test.name}...`);
      try {
        const success = await test.fn();
        results.push({ name: test.name, success });
        console.log(`${success ? '✅' : '❌'} ${test.name}: ${success ? 'PASSED' : 'FAILED'}`);
      } catch (error) {
        results.push({ name: test.name, success: false, error: error.message });
        console.log(`❌ ${test.name}: FAILED - ${error.message}`);
      }
    }
    
    const report = this.generateReport();
    
    console.log('\n📊 Validation Summary');
    console.log('====================');
    console.log(`Overall Result: ${report.summary.overallSuccess ? '✅ PASSED' : '❌ FAILED'}`);
    console.log(`Tests Passed: ${results.filter(r => r.success).length}/${results.length}`);
    
    if (report.recommendations.length > 0) {
      console.log('\n💡 Recommendations:');
      report.recommendations.forEach(rec => {
        const icon = rec.includes('CRITICAL') ? '🚨' : '💡';
        console.log(`   ${icon} ${rec}`);
      });
    }
    
    // Save detailed report
    const reportsDir = path.join(__dirname, 'performance-reports');
    if (!fs.existsSync(reportsDir)) {
      fs.mkdirSync(reportsDir, { recursive: true });
    }
    
    const reportFile = path.join(reportsDir, `validation-report-${Date.now()}.json`);
    fs.writeFileSync(reportFile, JSON.stringify(report, null, 2));
    console.log(`\n📄 Detailed report: ${path.relative(__dirname, reportFile)}`);
    
    process.exit(report.summary.overallSuccess ? 0 : 1);
  }
}

// Run validation if this script is executed directly
if (require.main === module) {
  const validator = new PerformanceValidator();
  validator.run().catch(console.error);
}

module.exports = { PerformanceValidator };