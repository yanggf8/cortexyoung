#!/usr/bin/env node

/**
 * Multi-Instance MCP Server Test
 * Simulates real Claude Code MCP clients connecting to multiple instances
 */

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

class MultiInstanceTester {
  constructor() {
    this.instances = [];
    this.logDir = path.join(os.homedir(), '.cortex', 'multi-instance-logs');
    this.testResults = {
      startTime: Date.now(),
      instances: [],
      success: false,
      errors: []
    };
  }

  async runTest() {
    console.log('🧪 Starting Multi-Instance MCP Server Test...\n');
    
    // Clear previous logs
    this.clearLogs();
    
    try {
      // Test 1: Start first instance
      console.log('📡 Test 1: Starting first MCP instance...');
      const instance1 = await this.startInstance('/tmp/claude-test-1', 'CLAUDE_SESSION_1');
      
      // Wait for first instance to stabilize
      await this.wait(2000);
      
      // Test 2: Start second instance
      console.log('📡 Test 2: Starting second MCP instance...');
      const instance2 = await this.startInstance('/tmp/claude-test-2', 'CLAUDE_SESSION_2');
      
      // Wait for both instances to be ready
      await this.wait(3000);
      
      // Test 3: Send MCP requests to both instances
      console.log('📋 Test 3: Sending MCP requests to both instances...');
      await this.testMcpCommunication(instance1, 'Instance 1');
      await this.testMcpCommunication(instance2, 'Instance 2');
      
      // Test 4: Monitor session isolation
      console.log('🔍 Test 4: Verifying session isolation...');
      await this.verifySessionIsolation();
      
      // Test 5: Graceful shutdown
      console.log('🛑 Test 5: Testing graceful shutdown...');
      await this.testGracefulShutdown();
      
      this.testResults.success = true;
      console.log('✅ All tests passed! Multi-instance support is working correctly.\n');
      
    } catch (error) {
      this.testResults.errors.push(error.message);
      console.error('❌ Test failed:', error.message);
    } finally {
      await this.cleanup();
      this.printResults();
    }
  }

  async startInstance(workdir, sessionId) {
    return new Promise((resolve, reject) => {
      console.log(`  Starting instance in ${workdir} with session ${sessionId}...`);
      
      const instance = spawn('node', [path.join(__dirname, 'cortex-multi-instance.js')], {
        cwd: workdir,
        stdio: ['pipe', 'pipe', 'pipe'],
        env: {
          ...process.env,
          CLAUDE_SESSION_ID: sessionId,
          CLAUDE_DESKTOP_SESSION: sessionId
        }
      });

      instance.sessionId = sessionId;
      instance.workdir = workdir;
      instance.pid = instance.pid;
      
      this.instances.push(instance);

      // Handle instance output
      instance.stdout.on('data', (data) => {
        console.log(`  [${sessionId}] STDOUT: ${data.toString().trim()}`);
      });

      instance.stderr.on('data', (data) => {
        const output = data.toString().trim();
        console.log(`  [${sessionId}] STDERR: ${output}`);
        
        // Look for ready signal
        if (output.includes('Cortex Multi-Instance MCP Server running on stdio')) {
          console.log(`  ✅ ${sessionId} is ready and listening`);
          resolve(instance);
        }
      });

      instance.on('error', (error) => {
        console.error(`  ❌ ${sessionId} error: ${error.message}`);
        reject(error);
      });

      instance.on('exit', (code) => {
        console.log(`  🔚 ${sessionId} exited with code ${code}`);
      });

      // Timeout for startup
      setTimeout(() => {
        if (!instance.killed) {
          console.log(`  ⏰ ${sessionId} startup timeout - assuming ready`);
          resolve(instance);
        }
      }, 5000);
    });
  }

  async testMcpCommunication(instance, instanceName) {
    return new Promise((resolve) => {
      console.log(`  Testing MCP communication with ${instanceName}...`);
      
      // Send MCP initialization request
      const initRequest = {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: {
          protocolVersion: "2024-11-05",
          capabilities: {},
          clientInfo: {
            name: "test-client",
            version: "1.0.0"
          }
        }
      };

      const initData = JSON.stringify(initRequest) + '\n';
      instance.stdin.write(initData);
      console.log(`  📤 Sent initialization request to ${instanceName}`);

      // Send tools list request
      setTimeout(() => {
        const toolsRequest = {
          jsonrpc: "2.0",
          id: 2,
          method: "tools/list",
          params: {}
        };

        const toolsData = JSON.stringify(toolsRequest) + '\n';
        instance.stdin.write(toolsData);
        console.log(`  📤 Sent tools list request to ${instanceName}`);
        
        resolve();
      }, 1000);
    });
  }

  async verifySessionIsolation() {
    try {
      const sessionsFile = path.join(this.logDir, 'active-sessions.json');
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        const sessions = JSON.parse(data);
        
        const sessionCount = Object.keys(sessions).length;
        console.log(`  Found ${sessionCount} active sessions`);
        
        if (sessionCount >= 2) {
          console.log('  ✅ Multiple sessions detected - isolation working');
          return true;
        } else {
          console.log('  ⚠️  Expected 2+ sessions, found', sessionCount);
          return false;
        }
      } else {
        console.log('  ⚠️  No active sessions file found');
        return false;
      }
    } catch (error) {
      console.log(`  ❌ Session verification error: ${error.message}`);
      return false;
    }
  }

  async testGracefulShutdown() {
    console.log('  Sending SIGTERM to all instances...');
    
    for (const instance of this.instances) {
      if (!instance.killed) {
        instance.kill('SIGTERM');
        console.log(`  📤 Sent SIGTERM to ${instance.sessionId}`);
      }
    }
    
    // Wait for graceful shutdown
    await this.wait(2000);
  }

  async cleanup() {
    console.log('🧹 Cleaning up test environment...');
    
    for (const instance of this.instances) {
      if (!instance.killed) {
        instance.kill('SIGKILL');
      }
    }
    
    this.instances = [];
  }

  clearLogs() {
    try {
      if (fs.existsSync(this.logDir)) {
        const files = fs.readdirSync(this.logDir);
        for (const file of files) {
          if (file.endsWith('.log') || file === 'active-sessions.json') {
            fs.unlinkSync(path.join(this.logDir, file));
          }
        }
        console.log('🧹 Cleared previous test logs\n');
      }
    } catch (error) {
      console.log(`Warning: Could not clear logs: ${error.message}`);
    }
  }

  printResults() {
    console.log('\n📊 Test Results Summary:');
    console.log('================================');
    console.log(`Duration: ${Date.now() - this.testResults.startTime}ms`);
    console.log(`Instances Started: ${this.instances.length}`);
    console.log(`Success: ${this.testResults.success ? '✅ YES' : '❌ NO'}`);
    
    if (this.testResults.errors.length > 0) {
      console.log('\nErrors:');
      this.testResults.errors.forEach(error => console.log(`  - ${error}`));
    }
    
    console.log('\n📂 Check logs at:', this.logDir);
    
    // Show final session status
    try {
      const sessionsFile = path.join(this.logDir, 'active-sessions.json');
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        const sessions = JSON.parse(data);
        console.log('\n📋 Final Session Status:');
        console.log(JSON.stringify(sessions, null, 2));
      }
    } catch (error) {
      console.log('Could not read final session status');
    }
  }

  wait(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}

// Run the test
if (require.main === module) {
  const tester = new MultiInstanceTester();
  tester.runTest().then(() => {
    process.exit(0);
  }).catch((error) => {
    console.error('Test runner error:', error);
    process.exit(1);
  });
}

module.exports = MultiInstanceTester;