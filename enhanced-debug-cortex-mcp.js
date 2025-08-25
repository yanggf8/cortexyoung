#!/usr/bin/env node

/**
 * Enhanced Cortex MCP Debug Script with Multi-Instance Monitoring
 * Helps diagnose connection issues and provides comprehensive logging
 */

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');
const os = require('os');

console.log('🔍 Enhanced Cortex MCP Debug Script v2.0');
console.log('==========================================');

// Check log directory
const logDir = path.join(os.homedir(), '.cortex', 'multi-instance-logs');
console.log(`📁 Log Directory: ${logDir}`);

if (!fs.existsSync(logDir)) {
  console.log('📂 Creating log directory...');
  fs.mkdirSync(logDir, { recursive: true });
}

// Check active sessions
const sessionFile = path.join(logDir, 'active-sessions.json');
if (fs.existsSync(sessionFile)) {
  try {
    const sessions = JSON.parse(fs.readFileSync(sessionFile, 'utf8'));
    const activeSessions = Object.keys(sessions).length;
    console.log(`📊 Active Sessions: ${activeSessions}`);
    
    if (activeSessions > 0) {
      console.log('\n👥 Current Active Sessions:');
      Object.values(sessions).forEach((session, index) => {
        const uptime = Math.floor((Date.now() - session.startTime) / 1000);
        const lastActivity = Math.floor((Date.now() - session.lastActivity) / 1000);
        console.log(`   ${index + 1}. PID: ${session.pid} | Claude: ${session.claudeInstance} | Status: ${session.status} | Uptime: ${uptime}s | Last Activity: ${lastActivity}s ago`);
      });
    }
  } catch (error) {
    console.log('⚠️  Could not parse active sessions file');
  }
} else {
  console.log('📝 No active sessions file found (normal for first run)');
}

// Test cortex-mcp startup with enhanced logging
console.log('\n🚀 Starting Enhanced Cortex MCP Test...');
console.log('=============================================');

const cortexMcpPath = path.join(__dirname, 'cortex-mcp.js');
if (!fs.existsSync(cortexMcpPath)) {
  console.error('❌ cortex-mcp.js not found');
  process.exit(1);
}

console.log('✅ cortex-mcp.js found');

// Test startup with multi-instance logging
const testProcess = spawn('node', [cortexMcpPath], {
  env: {
    ...process.env,
    MCP_MULTI_INSTANCE: 'true',
    CORTEX_SKIP_CLEANUP: 'true',
    CORTEX_DEBUG: 'true'
  },
  stdio: ['pipe', 'pipe', 'pipe']
});

let stdout = '';
let stderr = '';
const startTime = Date.now();

testProcess.stdout.on('data', (data) => {
  const output = data.toString();
  stdout += output;
  console.log(`[STDOUT] ${output.trim()}`);
});

testProcess.stderr.on('data', (data) => {
  const output = data.toString();
  stderr += output;
  console.log(`[STDERR] ${output.trim()}`);
});

testProcess.on('error', (error) => {
  console.error('❌ Process error:', error.message);
});

// Monitor for 10 seconds then analyze
setTimeout(() => {
  testProcess.kill('SIGTERM');
  
  const runtime = Date.now() - startTime;
  
  console.log('\n📊 Enhanced Debug Analysis');
  console.log('===========================');
  
  // Basic health checks
  if (stderr.includes('Connected and Ready')) {
    console.log('✅ Server startup: SUCCESS');
  } else if (stderr.includes('Starting')) {
    console.log('⏳ Server startup: IN PROGRESS (may need more time)');
  } else {
    console.log('❌ Server startup: FAILED or NO OUTPUT');
  }
  
  // Multi-instance analysis
  if (stderr.includes('multi-instance mode') || process.env.MCP_MULTI_INSTANCE) {
    console.log('✅ Multi-instance mode: ENABLED');
  } else {
    console.log('⚠️  Multi-instance mode: DISABLED');
  }
  
  // Connection analysis
  if (stderr.includes('Centralized embedding server not available')) {
    console.log('ℹ️  Centralized server: NOT AVAILABLE (using fallback mode - normal)');
  } else if (stderr.includes('health check')) {
    console.log('ℹ️  Health checks: RUNNING');
  }
  
  // Log file analysis
  console.log('\n📁 Log File Analysis');
  console.log('=====================');
  
  if (fs.existsSync(sessionFile)) {
    try {
      const sessions = JSON.parse(fs.readFileSync(sessionFile, 'utf8'));
      const sessionCount = Object.keys(sessions).length;
      console.log(`📊 Total sessions after test: ${sessionCount}`);
      
      // Find conflicts
      const projectPaths = Object.values(sessions).map(s => s.projectPath);
      const duplicates = projectPaths.filter((path, index) => projectPaths.indexOf(path) !== index);
      if (duplicates.length > 0) {
        console.log(`⚠️  Project conflicts detected: ${duplicates.length} duplicate paths`);
      } else {
        console.log('✅ No project conflicts detected');
      }
    } catch (error) {
      console.log('⚠️  Could not analyze session file');
    }
  }
  
  // Check individual session logs
  const logFiles = fs.readdirSync(logDir).filter(file => file.endsWith('.log') && file !== 'global.log');
  console.log(`📝 Individual session logs: ${logFiles.length}`);
  
  if (logFiles.length > 0) {
    const latestLogFile = logFiles[logFiles.length - 1];
    const latestLogPath = path.join(logDir, latestLogFile);
    try {
      const logContent = fs.readFileSync(latestLogPath, 'utf8').trim();
      const lines = logContent.split('\n');
      console.log(`📄 Latest log (${latestLogFile}): ${lines.length} entries`);
      
      // Show last few entries
      console.log('\n📋 Recent Log Entries:');
      lines.slice(-3).forEach(line => {
        try {
          const entry = JSON.parse(line);
          console.log(`   [${entry.timestamp}] ${entry.type}: ${entry.message}`);
        } catch (parseError) {
          console.log(`   ${line.substring(0, 100)}...`);
        }
      });
    } catch (error) {
      console.log('⚠️  Could not read latest log file');
    }
  }
  
  // Recommendations
  console.log('\n💡 Multi-Instance Troubleshooting Guide');
  console.log('========================================');
  
  if (stderr.includes('Connected and Ready')) {
    console.log('✅ Your Cortex MCP server is working correctly!');
    console.log('   - Try the health check tool: @cortex-multi_instance_health');
    console.log('   - Logs are available at: ~/.cortex/multi-instance-logs/');
  } else {
    console.log('⚠️  Startup issues detected. Try:');
    console.log('   1. Check if Node.js version is 16+');
    console.log('   2. Run: npm run build');
    console.log('   3. Check for port conflicts');
    console.log('   4. Try single instance first before multiple');
  }
  
  console.log('\n📞 For Connection Failures:');
  console.log('   1. Use @cortex-multi_instance_health in Claude Code');
  console.log('   2. Check ~/.cortex/multi-instance-logs/active-sessions.json');
  console.log('   3. Look for session conflicts (same project, multiple PIDs)');
  console.log('   4. Restart Claude Code instances if needed');
  
  console.log('\n🛠️  Advanced Debugging:');
  console.log(`   - Session file: ${sessionFile}`);
  console.log(`   - Log directory: ${logDir}`);
  console.log('   - Environment: MCP_MULTI_INSTANCE=true CORTEX_SKIP_CLEANUP=true');
  
  process.exit(0);
}, 10000); // 10 seconds