#!/usr/bin/env node

const { spawn } = require('child_process');
const fs = require('fs/promises');
const path = require('path');

/**
 * Compare embedding strategies performance
 */
class EmbeddingStrategyComparison {
  constructor() {
    this.results = {};
    this.repoPath = process.cwd();
  }

  async runCommand(command, env = {}, timeout = 300000) {
    return new Promise((resolve, reject) => {
      const startTime = Date.now();
      
      console.log(`🧪 Running: ${command}`);
      
      const proc = spawn('npm', ['run', command], {
        stdio: ['inherit', 'pipe', 'pipe'],
        env: { ...process.env, ...env },
        shell: true
      });

      let stdout = '';
      let stderr = '';

      proc.stdout.on('data', (data) => {
        const output = data.toString();
        stdout += output;
        // Show real-time output for key metrics
        if (output.includes('📊') || output.includes('✅') || output.includes('🎯')) {
          process.stdout.write(output);
        }
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

  extractMetrics(output) {
    const metrics = {};
    
    // Extract various performance metrics from output
    const patterns = {
      strategy: /🎯 Embedding strategy: (\w+)/,
      totalTime: /📊 Total time: ([\d.]+)s/,
      chunksPerSecond: /📊 Chunks\/second: ([\d.]+)/,
      peakMemory: /📊 Peak memory: ([\d.]+)MB/,
      batchTime: /📊 Average batch time: ([\d.]+)s/,
      chunksProcessed: /chunks_processed: (\d+)/,
      indexingTime: /Indexing completed in (\d+)ms/,
      embeddingTime: /Generated embeddings for \d+ chunks.*?(\d+)ms/,
      processCount: /Initializing (\d+) processes/
    };

    for (const [key, pattern] of Object.entries(patterns)) {
      const match = output.match(pattern);
      if (match) {
        metrics[key] = isNaN(parseFloat(match[1])) ? match[1] : parseFloat(match[1]);
      }
    }

    return metrics;
  }

  async benchmarkStrategy(strategy, description) {
    console.log(`\n🚀 Benchmarking ${description}`);
    console.log('='.repeat(50));

    try {
      // Clear cache first for fair comparison
      await this.runCommand('cache:clear', {}, 10000);
      
      // Run the strategy
      const result = await this.runCommand('demo:full', { 
        EMBEDDING_STRATEGY: strategy 
      }, 600000); // 10 minute timeout
      
      const metrics = this.extractMetrics(result.stdout);
      
      return {
        strategy,
        success: result.success,
        duration: result.duration,
        metrics,
        output: result.stdout.substring(0, 1000) // Keep sample output
      };
      
    } catch (error) {
      console.error(`❌ ${description} failed:`, error.message);
      return {
        strategy,
        success: false,
        error: error.message,
        duration: 0,
        metrics: {}
      };
    }
  }

  async runComparison() {
    console.log('🏁 Embedding Strategy Performance Comparison');
    console.log('==========================================');
    console.log(`📁 Repository: ${this.repoPath}`);
    console.log(`⏰ Start time: ${new Date().toISOString()}\n`);

    const strategies = [
      { name: 'original', description: 'Original Strategy (Single-threaded)' },
      { name: 'process-pool', description: 'ProcessPool Strategy (Multi-process)' }
    ];

    const results = [];

    for (const strategy of strategies) {
      const result = await this.benchmarkStrategy(strategy.name, strategy.description);
      results.push(result);
      
      console.log(`\n📊 ${strategy.description} Results:`);
      if (result.success) {
        console.log(`   ✅ Success: ${(result.duration / 1000).toFixed(1)}s total`);
        if (result.metrics.totalTime) console.log(`   📊 Embedding time: ${result.metrics.totalTime}s`);
        if (result.metrics.chunksPerSecond) console.log(`   📊 Throughput: ${result.metrics.chunksPerSecond.toFixed(1)} chunks/s`);
        if (result.metrics.peakMemory) console.log(`   📊 Peak memory: ${result.metrics.peakMemory}MB`);
        if (result.metrics.processCount) console.log(`   📊 Processes: ${result.metrics.processCount}`);
      } else {
        console.log(`   ❌ Failed: ${result.error}`);
      }
      
      // Wait between tests
      console.log('\n⏸️ Waiting 10 seconds before next test...');
      await new Promise(resolve => setTimeout(resolve, 10000));
    }

    // Generate comparison report
    const report = this.generateComparisonReport(results);
    await this.saveReport(report);
    
    return report;
  }

  generateComparisonReport(results) {
    const original = results.find(r => r.strategy === 'original');
    const processPool = results.find(r => r.strategy === 'process-pool');
    
    const report = {
      timestamp: new Date().toISOString(),
      repository: this.repoPath,
      results: results,
      comparison: null,
      recommendation: null
    };

    if (original?.success && processPool?.success) {
      const speedRatio = processPool.metrics.chunksPerSecond / original.metrics.chunksPerSecond;
      const memoryRatio = processPool.metrics.peakMemory / original.metrics.peakMemory;
      const timeRatio = original.metrics.totalTime / processPool.metrics.totalTime;
      
      report.comparison = {
        speedRatio: speedRatio,
        memoryRatio: memoryRatio,
        timeRatio: timeRatio,
        originalFaster: speedRatio < 1,
        processPoolFaster: speedRatio > 1,
        memoryIncrease: memoryRatio - 1,
        timeReduction: 1 - (1 / timeRatio)
      };

      // Generate recommendation
      if (speedRatio > 1.2 && memoryRatio < 3) {
        report.recommendation = {
          strategy: 'process-pool',
          reason: `ProcessPool is ${speedRatio.toFixed(1)}x faster with acceptable memory usage (${memoryRatio.toFixed(1)}x)`
        };
      } else if (speedRatio < 0.8) {
        report.recommendation = {
          strategy: 'original',
          reason: `Original is ${(1/speedRatio).toFixed(1)}x faster than ProcessPool`
        };
      } else {
        report.recommendation = {
          strategy: 'auto',
          reason: `Performance is similar - use auto selection based on dataset size`
        };
      }
    } else {
      report.recommendation = {
        strategy: 'original',
        reason: 'ProcessPool strategy failed - use original as fallback'
      };
    }

    return report;
  }

  printSummary(report) {
    console.log('\n📊 Strategy Comparison Summary');
    console.log('=============================');
    
    const original = report.results.find(r => r.strategy === 'original');
    const processPool = report.results.find(r => r.strategy === 'process-pool');
    
    if (original?.success) {
      console.log(`\n🔵 Original Strategy:`);
      console.log(`   Time: ${original.metrics.totalTime}s`);
      console.log(`   Throughput: ${original.metrics.chunksPerSecond?.toFixed(1)} chunks/s`);
      console.log(`   Memory: ${original.metrics.peakMemory}MB`);
    }
    
    if (processPool?.success) {
      console.log(`\n🔴 ProcessPool Strategy:`);
      console.log(`   Time: ${processPool.metrics.totalTime}s`);
      console.log(`   Throughput: ${processPool.metrics.chunksPerSecond?.toFixed(1)} chunks/s`);
      console.log(`   Memory: ${processPool.metrics.peakMemory}MB`);
      console.log(`   Processes: ${processPool.metrics.processCount}`);
    }
    
    if (report.comparison) {
      const comp = report.comparison;
      console.log(`\n📈 Performance Comparison:`);
      console.log(`   Speed ratio: ${comp.speedRatio.toFixed(2)}x (ProcessPool vs Original)`);
      console.log(`   Memory ratio: ${comp.memoryRatio.toFixed(2)}x (ProcessPool vs Original)`);
      console.log(`   Time reduction: ${(comp.timeReduction * 100).toFixed(1)}%`);
      
      if (comp.processPoolFaster) {
        console.log(`   🚀 ProcessPool is ${comp.speedRatio.toFixed(1)}x faster`);
      } else {
        console.log(`   🐌 Original is ${(1/comp.speedRatio).toFixed(1)}x faster`);
      }
    }
    
    if (report.recommendation) {
      console.log(`\n💡 Recommendation: ${report.recommendation.strategy.toUpperCase()}`);
      console.log(`   Reason: ${report.recommendation.reason}`);
    }
  }

  async saveReport(report) {
    const reportsDir = path.join(this.repoPath, 'performance-reports');
    await fs.mkdir(reportsDir, { recursive: true }).catch(() => {});
    
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const filename = `strategy-comparison-${timestamp}.json`;
    const reportPath = path.join(reportsDir, filename);
    
    await fs.writeFile(reportPath, JSON.stringify(report, null, 2));
    console.log(`\n📄 Detailed report saved: ${path.relative(this.repoPath, reportPath)}`);
    
    return reportPath;
  }
}

// Usage instructions
function showUsage() {
  console.log(`
🏁 Embedding Strategy Comparison Tool

Usage:
  npm run benchmark:compare                 # Compare both strategies
  
Manual Strategy Testing:
  npm run demo:original                     # Test original strategy
  npm run demo:process-pool                 # Test ProcessPool strategy
  npm run demo:auto                         # Use auto selection
  
Individual Benchmarks:
  npm run benchmark:original                # Benchmark original only
  npm run benchmark:process-pool            # Benchmark ProcessPool only

Environment Variables:
  EMBEDDING_STRATEGY=original|process-pool|auto
  EMBEDDING_BATCH_SIZE=50                   # For original strategy
  EMBEDDING_PROCESS_COUNT=8                 # For ProcessPool strategy

Examples:
  EMBEDDING_STRATEGY=original npm run demo
  EMBEDDING_PROCESS_COUNT=4 npm run server:process-pool
`);
}

// Run comparison if executed directly
async function main() {
  const args = process.argv.slice(2);
  
  if (args.includes('--help') || args.includes('-h')) {
    showUsage();
    return;
  }
  
  const comparison = new EmbeddingStrategyComparison();
  
  try {
    const report = await comparison.runComparison();
    comparison.printSummary(report);
    
    console.log('\n🎉 Strategy comparison complete!');
    
  } catch (error) {
    console.error('\n❌ Comparison failed:', error.message);
    process.exit(1);
  }
}

if (require.main === module) {
  main().catch(console.error);
}

module.exports = { EmbeddingStrategyComparison };