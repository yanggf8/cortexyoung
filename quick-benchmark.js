#!/usr/bin/env node

/**
 * Quick Benchmark Test for Adaptive Process Pool Implementation
 * Tests key features without full embedding run
 */

const { spawn } = require('child_process');
const os = require('os');

class QuickBenchmark {
  constructor() {
    this.startTime = Date.now();
  }

  async runQuickTest(name, command, args, timeout = 45000) {
    console.log(`\n🧪 Testing ${name}...`);
    
    return new Promise((resolve) => {
      const startTime = Date.now();
      const proc = spawn(command, args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        timeout
      });
      
      let stdout = '';
      let stderr = '';
      let features = {
        memoryMonitoring: false,
        processScaling: false,
        gracefulShutdown: false,
        adaptiveFeatures: false
      };
      
      proc.stdout.on('data', (data) => {
        stdout += data.toString();
      });
      
      proc.stderr.on('data', (data) => {
        const chunk = data.toString();
        stderr += chunk;
        
        // Check for adaptive features
        if (chunk.includes('Memory Status:')) features.memoryMonitoring = true;
        if (chunk.includes('Growing process pool:')) features.processScaling = true;
        if (chunk.includes('Adaptive Process Pool Strategy:')) features.adaptiveFeatures = true;
        if (chunk.includes('Graceful shutdown')) features.gracefulShutdown = true;
        
        // Auto-terminate after seeing key features
        if (name === 'Startup and Adaptive Features' && 
            chunk.includes('processes ready') && 
            features.adaptiveFeatures && 
            features.memoryMonitoring) {
          setTimeout(() => proc.kill('SIGINT'), 2000);
        }
      });
      
      proc.on('close', (code) => {
        const duration = Date.now() - startTime;
        resolve({
          name,
          duration,
          success: code === 0 || code === 130, // 130 = SIGINT
          features,
          logs: stderr,
          analysis: this.analyzeOutput(stderr, features)
        });
      });
      
      // Force timeout
      setTimeout(() => {
        proc.kill('SIGKILL');
        resolve({
          name,
          duration: Date.now() - startTime,
          success: false,
          error: 'Timeout',
          features
        });
      }, timeout);
    });
  }

  analyzeOutput(logs, features) {
    const analysis = {};
    
    // Extract system memory info
    const memoryMatch = logs.match(/System Memory: (\d+)MB total/);
    if (memoryMatch) {
      analysis.detectedMemoryMB = parseInt(memoryMatch[1], 10);
    }
    
    // Extract CPU info
    const cpuMatch = logs.match(/CPU Cores: (\d+) total/);
    if (cpuMatch) {
      analysis.detectedCores = parseInt(cpuMatch[1], 10);
    }
    
    // Extract process scaling info
    const startingMatch = logs.match(/Starting: (\d+) processes/);
    const maximumMatch = logs.match(/Maximum: (\d+) processes/);
    if (startingMatch && maximumMatch) {
      analysis.startingProcesses = parseInt(startingMatch[1], 10);
      analysis.maxProcesses = parseInt(maximumMatch[1], 10);
      analysis.scalingRatio = analysis.startingProcesses / analysis.maxProcesses;
    }
    
    // Check memory thresholds
    const thresholdMatch = logs.match(/Stop at (\d+)%, Resume at (\d+)%/);
    if (thresholdMatch) {
      analysis.memoryStopThreshold = parseInt(thresholdMatch[1], 10);
      analysis.memoryResumeThreshold = parseInt(thresholdMatch[2], 10);
    }
    
    // Check for actual memory readings
    const memoryStatusMatch = logs.match(/Memory Status: (\d+)MB used \/ (\d+)MB total \(([^)]+)%\)/);
    if (memoryStatusMatch) {
      analysis.actualMemoryUsedMB = parseInt(memoryStatusMatch[1], 10);
      analysis.actualMemoryTotalMB = parseInt(memoryStatusMatch[2], 10);
      analysis.actualMemoryPercent = parseFloat(memoryStatusMatch[3]);
    }
    
    return analysis;
  }

  async testGracefulShutdownQuick() {
    console.log('\n🛑 Testing Graceful Shutdown (Quick)...');
    
    return new Promise((resolve) => {
      const startTime = Date.now();
      const proc = spawn('npm', ['run', 'demo'], { stdio: ['pipe', 'pipe', 'pipe'] });
      
      let stderr = '';
      let shutdownDetected = false;
      let processesSpawned = 0;
      
      proc.stderr.on('data', (data) => {
        const chunk = data.toString();
        stderr += chunk;
        
        // Count processes spawned
        if (chunk.includes('ready with isolated FastEmbedding')) {
          processesSpawned++;
        }
        
        // Trigger shutdown after some processes are ready
        if (processesSpawned >= 3 && !shutdownDetected) {
          setTimeout(() => {
            console.log('   Sending SIGINT...');
            proc.kill('SIGINT');
            shutdownDetected = true;
          }, 1000);
        }
        
        if (chunk.includes('Graceful shutdown initiated')) {
          console.log('   ✅ Shutdown sequence started');
        }
        
        if (chunk.includes('acknowledged abort')) {
          console.log('   ✅ Child process acknowledged shutdown');
        }
      });
      
      proc.on('close', (code) => {
        const duration = Date.now() - startTime;
        const success = shutdownDetected && (code === 0 || code === 130);
        
        resolve({
          name: 'Graceful Shutdown',
          duration,
          success,
          processesSpawned,
          shutdownDetected,
          analysis: {
            shutdownTime: duration,
            properSignalHandling: success,
            childProcessesHandled: stderr.includes('acknowledged')
          }
        });
      });
      
      setTimeout(() => {
        if (!shutdownDetected) {
          proc.kill('SIGKILL');
          resolve({
            name: 'Graceful Shutdown',
            success: false,
            error: 'Startup too slow for test'
          });
        }
      }, 60000);
    });
  }

  printResults(results) {
    console.log('\n' + '='.repeat(70));
    console.log('📊 QUICK ADAPTIVE PROCESS POOL BENCHMARK RESULTS');
    console.log('='.repeat(70));
    
    console.log(`\n🖥️  System: ${os.platform()} | CPUs: ${os.cpus().length} | RAM: ${Math.round(os.totalmem()/(1024*1024*1024))}GB`);
    
    results.forEach(result => {
      const status = result.success ? '✅' : '❌';
      const duration = (result.duration / 1000).toFixed(1);
      console.log(`\n${status} ${result.name} (${duration}s)`);
      
      if (result.features) {
        console.log('   Features Detected:');
        Object.entries(result.features).forEach(([feature, detected]) => {
          console.log(`     ${detected ? '✅' : '❌'} ${feature}`);
        });
      }
      
      if (result.analysis) {
        console.log('   Analysis:');
        Object.entries(result.analysis).forEach(([key, value]) => {
          console.log(`     ${key}: ${value}`);
        });
      }
      
      if (result.error) {
        console.log(`   ❌ Error: ${result.error}`);
      }
    });
    
    const totalTests = results.length;
    const passedTests = results.filter(r => r.success).length;
    const successRate = ((passedTests / totalTests) * 100).toFixed(1);
    const totalDuration = (Date.now() - this.startTime) / 1000;
    
    console.log(`\n🎯 Summary:`);
    console.log(`   Tests Passed: ${passedTests}/${totalTests} (${successRate}%)`);
    console.log(`   Total Time: ${totalDuration.toFixed(1)}s`);
    console.log(`   Status: ${passedTests === totalTests ? '🎉 EXCELLENT' : passedTests >= totalTests * 0.8 ? '👍 GOOD' : '⚠️ NEEDS WORK'}`);
    
    console.log('\n' + '='.repeat(70));
  }

  async run() {
    console.log('🚀 Quick Adaptive Process Pool Benchmark');
    console.log(`📅 ${new Date().toISOString()}`);
    
    const results = [];
    
    try {
      // Test 1: Startup and adaptive features
      const startupTest = await this.runQuickTest(
        'Startup and Adaptive Features',
        'npm', ['run', 'demo'],
        45000
      );
      results.push(startupTest);
      
      // Test 2: Graceful shutdown
      const shutdownTest = await this.testGracefulShutdownQuick();
      results.push(shutdownTest);
      
      this.printResults(results);
      
      return results;
      
    } catch (error) {
      console.error('❌ Benchmark failed:', error);
      return [];
    }
  }
}

// Run if called directly
if (require.main === module) {
  const benchmark = new QuickBenchmark();
  benchmark.run().then((results) => {
    const allPassed = results.every(r => r.success);
    process.exit(allPassed ? 0 : 1);
  }).catch(error => {
    console.error('❌ Benchmark error:', error);
    process.exit(1);
  });
}

module.exports = QuickBenchmark;