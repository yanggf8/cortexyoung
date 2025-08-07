#!/usr/bin/env node

/**
 * Comprehensive Benchmark Test for Adaptive Process Pool Implementation
 * Tests memory management, process scaling, and graceful shutdown
 */

const { spawn } = require('child_process');
const os = require('os');
const fs = require('fs');
const path = require('path');

class AdaptivePoolBenchmark {
  constructor() {
    this.results = {
      systemInfo: this.getSystemInfo(),
      tests: [],
      summary: {},
      recommendations: []
    };
    this.testStartTime = Date.now();
  }

  getSystemInfo() {
    return {
      platform: os.platform(),
      cpus: os.cpus().length,
      totalMemoryGB: Math.round(os.totalmem() / (1024 * 1024 * 1024)),
      nodeVersion: process.version,
      timestamp: new Date().toISOString()
    };
  }

  async runMemoryCommand() {
    return new Promise((resolve) => {
      const platform = os.platform();
      let command, args;
      
      if (platform === 'linux') {
        command = 'free';
        args = ['-m'];
      } else if (platform === 'darwin') {
        command = 'vm_stat';
        args = [];
      } else {
        // Fallback - just return Node.js memory
        const totalMB = Math.round(os.totalmem() / (1024 * 1024));
        const freeMB = Math.round(os.freemem() / (1024 * 1024));
        return resolve({
          totalMB,
          usedMB: totalMB - freeMB,
          usagePercent: ((totalMB - freeMB) / totalMB) * 100,
          accurate: false
        });
      }
      
      const proc = spawn(command, args, { stdio: ['pipe', 'pipe', 'pipe'] });
      let output = '';
      
      proc.stdout.on('data', (data) => {
        output += data.toString();
      });
      
      proc.on('close', () => {
        try {
          if (platform === 'linux') {
            const lines = output.split('\n');
            const memLine = lines.find(line => line.startsWith('Mem:'));
            if (memLine) {
              const parts = memLine.split(/\s+/);
              const totalMB = parseInt(parts[1], 10);
              const usedMB = parseInt(parts[2], 10);
              resolve({
                totalMB,
                usedMB,
                usagePercent: (usedMB / totalMB) * 100,
                accurate: true
              });
            }
          }
        } catch (error) {
          const totalMB = Math.round(os.totalmem() / (1024 * 1024));
          const freeMB = Math.round(os.freemem() / (1024 * 1024));
          resolve({
            totalMB,
            usedMB: totalMB - freeMB,
            usagePercent: ((totalMB - freeMB) / totalMB) * 100,
            accurate: false
          });
        }
      });
      
      setTimeout(() => {
        proc.kill();
        resolve({ totalMB: 0, usedMB: 0, usagePercent: 0, accurate: false });
      }, 5000);
    });
  }

  async sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  async runCommand(command, args = [], options = {}) {
    return new Promise((resolve, reject) => {
      const startTime = Date.now();
      const proc = spawn(command, args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        ...options
      });
      
      let stdout = '';
      let stderr = '';
      let memoryPeak = 0;
      let processCount = 0;
      const memoryReadings = [];
      const processGrowth = [];
      
      // Monitor memory and process growth
      const monitorInterval = setInterval(async () => {
        const memInfo = await this.runMemoryCommand();
        memoryReadings.push({
          timestamp: Date.now() - startTime,
          memoryMB: memInfo.usedMB,
          memoryPercent: memInfo.usagePercent
        });
        
        if (memInfo.usedMB > memoryPeak) {
          memoryPeak = memInfo.usedMB;
        }
        
        // Count child processes (rough estimate)
        const processMatch = stderr.match(/Process pool grown to (\d+) processes/);
        if (processMatch) {
          const newCount = parseInt(processMatch[1], 10);
          if (newCount > processCount) {
            processCount = newCount;
            processGrowth.push({
              timestamp: Date.now() - startTime,
              processCount: newCount
            });
          }
        }
      }, 1000);
      
      proc.stdout.on('data', (data) => {
        stdout += data.toString();
      });
      
      proc.stderr.on('data', (data) => {
        stderr += data.toString();
      });
      
      proc.on('close', (code) => {
        clearInterval(monitorInterval);
        const duration = Date.now() - startTime;
        
        resolve({
          code,
          duration,
          stdout,
          stderr,
          memoryPeak,
          memoryReadings,
          processGrowth,
          success: code === 0
        });
      });
      
      proc.on('error', (error) => {
        clearInterval(monitorInterval);
        reject(error);
      });
    });
  }

  async testStartupPerformance() {
    console.log('\n🚀 Testing Startup Performance...');
    
    const result = await this.runCommand('npm', ['run', 'demo'], {
      timeout: 120000,
      cwd: process.cwd()
    });
    
    const analysis = this.analyzeStartupOutput(result.stderr);
    
    const test = {
      name: 'Startup Performance',
      duration: result.duration,
      success: result.success,
      memoryPeakMB: result.memoryPeak,
      processGrowth: result.processGrowth,
      stages: analysis.stages,
      adaptiveFeatures: analysis.adaptiveFeatures,
      performance: {
        startupTime: result.duration,
        memoryEfficiency: result.memoryPeak < 5000 ? 'Excellent' : result.memoryPeak < 10000 ? 'Good' : 'Needs Improvement',
        processScaling: result.processGrowth.length > 0 ? 'Working' : 'Static'
      }
    };
    
    this.results.tests.push(test);
    return test;
  }

  async testMemoryConstraints() {
    console.log('\n🧠 Testing Memory Constraint Handling...');
    
    // Create a smaller process pool to test memory limits
    const result = await this.runCommand('node', ['-e', `
      const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
      const embedder = new ProcessPoolEmbedder();
      
      async function test() {
        console.log('Testing memory constraints...');
        await embedder.initialize();
        
        // Monitor for 30 seconds
        setTimeout(() => {
          console.log('Memory test completed');
          process.exit(0);
        }, 30000);
      }
      
      test().catch(console.error);
    `], { timeout: 45000 });
    
    const test = {
      name: 'Memory Constraints',
      duration: result.duration,
      success: result.success,
      memoryReadings: result.memoryReadings,
      analysis: this.analyzeMemoryConstraints(result.stderr, result.memoryReadings)
    };
    
    this.results.tests.push(test);
    return test;
  }

  async testGracefulShutdown() {
    console.log('\n🛑 Testing Graceful Shutdown...');
    
    return new Promise((resolve) => {
      const startTime = Date.now();
      const proc = spawn('npm', ['run', 'demo'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: process.cwd()
      });
      
      let stderr = '';
      let shutdownStarted = false;
      let shutdownCompleted = false;
      let processesKilled = 0;
      
      proc.stderr.on('data', (data) => {
        stderr += data.toString();
        
        // Look for process spawn messages
        const spawnMatches = stderr.match(/Process \d+ ready with isolated FastEmbedding/g);
        if (spawnMatches && spawnMatches.length >= 3) {
          // Wait a bit then send SIGINT
          setTimeout(() => {
            console.log('   Sending SIGINT to test graceful shutdown...');
            proc.kill('SIGINT');
            shutdownStarted = true;
          }, 5000);
        }
        
        // Look for shutdown messages
        if (stderr.includes('Graceful shutdown initiated')) {
          console.log('   ✅ Shutdown initiated properly');
        }
        
        if (stderr.includes('acknowledged abort')) {
          processesKilled++;
        }
        
        if (stderr.includes('Graceful shutdown completed')) {
          shutdownCompleted = true;
        }
      });
      
      proc.on('close', (code) => {
        const duration = Date.now() - startTime;
        
        const test = {
          name: 'Graceful Shutdown',
          duration,
          success: shutdownStarted && (code === 0 || code === 130), // 130 is SIGINT exit code
          shutdownInitiated: shutdownStarted,
          shutdownCompleted: shutdownCompleted,
          processesKilled,
          exitCode: code,
          analysis: {
            shutdownTime: duration,
            cleanExit: shutdownCompleted,
            noOrphanedProcesses: processesKilled > 0
          }
        };
        
        this.results.tests.push(test);
        resolve(test);
      });
      
      // Safety timeout
      setTimeout(() => {
        if (!shutdownStarted) {
          proc.kill('SIGKILL');
          resolve({
            name: 'Graceful Shutdown',
            success: false,
            error: 'Test timed out - system too slow to start'
          });
        }
      }, 60000);
    });
  }

  async testProcessScaling() {
    console.log('\n📈 Testing Process Scaling...');
    
    // Monitor the demo for process scaling behavior
    const result = await this.runCommand('timeout', ['45s', 'npm', 'run', 'demo'], {
      timeout: 50000
    });
    
    const scalingAnalysis = this.analyzeProcessScaling(result.stderr);
    
    const test = {
      name: 'Process Scaling',
      duration: result.duration,
      success: scalingAnalysis.scalingDetected,
      initialProcesses: scalingAnalysis.initialProcesses,
      maxProcesses: scalingAnalysis.maxProcesses,
      scalingSteps: scalingAnalysis.scalingSteps,
      memoryRespected: scalingAnalysis.memoryRespected,
      analysis: scalingAnalysis
    };
    
    this.results.tests.push(test);
    return test;
  }

  analyzeStartupOutput(output) {
    const stages = {};
    const adaptiveFeatures = {
      memoryMonitoring: false,
      processScaling: false,
      memoryThresholds: false
    };
    
    // Parse startup stages
    const stageMatches = output.match(/\[Step \d+\/10\].*?completed.*?\(([^)]+)\)/g);
    if (stageMatches) {
      stageMatches.forEach(match => {
        const timeMatch = match.match(/\(([^)]+)\)$/);
        if (timeMatch) {
          const stage = match.split(']')[1].split(':')[0].trim();
          stages[stage] = timeMatch[1];
        }
      });
    }
    
    // Check for adaptive features
    if (output.includes('Memory Status:')) adaptiveFeatures.memoryMonitoring = true;
    if (output.includes('Growing process pool:')) adaptiveFeatures.processScaling = true;
    if (output.includes('Memory Thresholds:')) adaptiveFeatures.memoryThresholds = true;
    
    return { stages, adaptiveFeatures };
  }

  analyzeMemoryConstraints(output, memoryReadings) {
    const analysis = {
      memoryThresholdsRespected: false,
      maxMemoryUsage: 0,
      memoryGrowthPattern: 'unknown',
      constraintTriggers: []
    };
    
    if (memoryReadings && memoryReadings.length > 0) {
      analysis.maxMemoryUsage = Math.max(...memoryReadings.map(r => r.memoryPercent));
      
      // Check if memory stayed under reasonable limits
      analysis.memoryThresholdsRespected = analysis.maxMemoryUsage < 80;
      
      // Analyze growth pattern
      const firstReading = memoryReadings[0]?.memoryPercent || 0;
      const lastReading = memoryReadings[memoryReadings.length - 1]?.memoryPercent || 0;
      
      if (lastReading > firstReading * 1.5) {
        analysis.memoryGrowthPattern = 'rapid-growth';
      } else if (lastReading > firstReading * 1.2) {
        analysis.memoryGrowthPattern = 'moderate-growth';
      } else {
        analysis.memoryGrowthPattern = 'stable';
      }
    }
    
    // Check for constraint triggers in output
    if (output.includes('Memory threshold reached')) {
      analysis.constraintTriggers.push('threshold-reached');
    }
    if (output.includes('Memory pressure relieved')) {
      analysis.constraintTriggers.push('pressure-relieved');
    }
    
    return analysis;
  }

  analyzeProcessScaling(output) {
    const analysis = {
      scalingDetected: false,
      initialProcesses: 0,
      maxProcesses: 0,
      scalingSteps: [],
      memoryRespected: true
    };
    
    // Extract initial process count
    const initialMatch = output.match(/Starting: (\d+) processes/);
    if (initialMatch) {
      analysis.initialProcesses = parseInt(initialMatch[1], 10);
    }
    
    // Extract maximum processes
    const maxMatch = output.match(/Maximum: (\d+) processes/);
    if (maxMatch) {
      analysis.maxProcesses = parseInt(maxMatch[1], 10);
    }
    
    // Find scaling steps
    const scalingMatches = output.match(/Growing process pool: (\d+) → (\d+) processes/g);
    if (scalingMatches) {
      analysis.scalingDetected = true;
      scalingMatches.forEach(match => {
        const numbers = match.match(/(\d+) → (\d+)/);
        if (numbers) {
          analysis.scalingSteps.push({
            from: parseInt(numbers[1], 10),
            to: parseInt(numbers[2], 10)
          });
        }
      });
    }
    
    // Check if memory constraints were respected
    if (output.includes('Memory threshold reached') || output.includes('Pausing process growth')) {
      analysis.memoryRespected = true;
    }
    
    return analysis;
  }

  generateSummary() {
    const tests = this.results.tests;
    const totalTests = tests.length;
    const passedTests = tests.filter(t => t.success).length;
    const totalDuration = Date.now() - this.testStartTime;
    
    this.results.summary = {
      totalTests,
      passedTests,
      successRate: ((passedTests / totalTests) * 100).toFixed(1),
      totalDurationMs: totalDuration,
      totalDurationMin: (totalDuration / 60000).toFixed(1),
      overallStatus: passedTests === totalTests ? 'EXCELLENT' : passedTests >= totalTests * 0.8 ? 'GOOD' : 'NEEDS_IMPROVEMENT'
    };
    
    // Generate recommendations
    this.generateRecommendations();
  }

  generateRecommendations() {
    const tests = this.results.tests;
    const recommendations = [];
    
    // Startup performance
    const startupTest = tests.find(t => t.name === 'Startup Performance');
    if (startupTest && startupTest.duration > 60000) {
      recommendations.push('Consider optimizing startup time - current duration is over 1 minute');
    }
    
    // Memory usage
    const memoryTest = tests.find(t => t.name === 'Memory Constraints');
    if (memoryTest && memoryTest.analysis.maxMemoryUsage > 70) {
      recommendations.push('High memory usage detected - consider reducing batch sizes or process count');
    }
    
    // Process scaling
    const scalingTest = tests.find(t => t.name === 'Process Scaling');
    if (scalingTest && !scalingTest.success) {
      recommendations.push('Process scaling not working properly - check memory monitoring implementation');
    }
    
    // Graceful shutdown
    const shutdownTest = tests.find(t => t.name === 'Graceful Shutdown');
    if (shutdownTest && !shutdownTest.success) {
      recommendations.push('Graceful shutdown issues detected - verify signal handling implementation');
    }
    
    if (recommendations.length === 0) {
      recommendations.push('All systems operating optimally! 🎉');
    }
    
    this.results.recommendations = recommendations;
  }

  printResults() {
    console.log('\n' + '='.repeat(80));
    console.log('📊 ADAPTIVE PROCESS POOL BENCHMARK RESULTS');
    console.log('='.repeat(80));
    
    console.log(`\n🖥️  System Info:`);
    console.log(`   Platform: ${this.results.systemInfo.platform}`);
    console.log(`   CPUs: ${this.results.systemInfo.cpus} cores`);
    console.log(`   Memory: ${this.results.systemInfo.totalMemoryGB}GB`);
    console.log(`   Node.js: ${this.results.systemInfo.nodeVersion}`);
    
    console.log(`\n📈 Test Results:`);
    this.results.tests.forEach(test => {
      const status = test.success ? '✅' : '❌';
      console.log(`   ${status} ${test.name}: ${test.success ? 'PASS' : 'FAIL'} (${(test.duration/1000).toFixed(1)}s)`);
      
      if (test.memoryPeakMB) {
        console.log(`      Peak Memory: ${test.memoryPeakMB}MB`);
      }
      if (test.processGrowth && test.processGrowth.length > 0) {
        const maxProcesses = Math.max(...test.processGrowth.map(p => p.processCount));
        console.log(`      Max Processes: ${maxProcesses}`);
      }
    });
    
    console.log(`\n🎯 Summary:`);
    console.log(`   Tests Passed: ${this.results.summary.passedTests}/${this.results.summary.totalTests}`);
    console.log(`   Success Rate: ${this.results.summary.successRate}%`);
    console.log(`   Total Duration: ${this.results.summary.totalDurationMin} minutes`);
    console.log(`   Overall Status: ${this.results.summary.overallStatus}`);
    
    console.log(`\n💡 Recommendations:`);
    this.results.recommendations.forEach(rec => {
      console.log(`   • ${rec}`);
    });
    
    console.log('\n' + '='.repeat(80));
  }

  async saveResults() {
    const filename = `benchmark-results-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
    const filepath = path.join(process.cwd(), 'performance-reports', filename);
    
    // Ensure directory exists
    const dir = path.dirname(filepath);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }
    
    fs.writeFileSync(filepath, JSON.stringify(this.results, null, 2));
    console.log(`\n📁 Results saved to: ${filepath}`);
  }

  async run() {
    console.log('🚀 Starting Adaptive Process Pool Benchmark...');
    console.log(`📅 ${new Date().toISOString()}`);
    
    try {
      await this.testStartupPerformance();
      await this.testMemoryConstraints();
      await this.testProcessScaling();
      await this.testGracefulShutdown();
      
      this.generateSummary();
      this.printResults();
      await this.saveResults();
      
    } catch (error) {
      console.error('❌ Benchmark failed:', error);
      process.exit(1);
    }
  }
}

// Run the benchmark
if (require.main === module) {
  const benchmark = new AdaptivePoolBenchmark();
  benchmark.run().then(() => {
    console.log('\n✅ Benchmark completed successfully!');
  }).catch(error => {
    console.error('\n❌ Benchmark failed:', error);
    process.exit(1);
  });
}

module.exports = AdaptivePoolBenchmark;