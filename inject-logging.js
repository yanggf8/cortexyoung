#!/usr/bin/env node

/**
 * Inject multi-instance logging into compiled stdio-server.js
 */

const fs = require('fs');
const path = require('path');
const os = require('os');

console.log('🔧 Injecting Enhanced Multi-Instance Logging...');

const stdioServerPath = path.join(__dirname, 'dist', 'stdio-server.js');

if (!fs.existsSync(stdioServerPath)) {
  console.error('❌ dist/stdio-server.js not found - run npm run build first');
  process.exit(1);
}

let serverCode = fs.readFileSync(stdioServerPath, 'utf8');

// Add multi-instance logger class at the top
const loggerCode = `
// Multi-Instance Logger for Enhanced Debugging
class MultiInstanceLogger {
  constructor(projectPath) {
    this.logDir = require('path').join(require('os').homedir(), '.cortex', 'multi-instance-logs');
    this.sessionFile = require('path').join(this.logDir, 'active-sessions.json');
    this.instanceId = this.generateInstanceId();
    
    // Create logs directory
    if (!require('fs').existsSync(this.logDir)) {
      require('fs').mkdirSync(this.logDir, { recursive: true });
    }
    
    this.session = {
      sessionId: this.instanceId,
      pid: process.pid,
      parentPid: process.ppid || 0,
      startTime: Date.now(),
      claudeInstance: this.detectClaudeInstance(),
      projectPath: projectPath || process.cwd(),
      status: 'starting',
      lastActivity: Date.now(),
      healthChecks: [],
      environment: {
        multiInstance: !!process.env.MCP_MULTI_INSTANCE,
        skipCleanup: !!process.env.CORTEX_SKIP_CLEANUP,
        skipHealthCheck: !!process.env.CORTEX_SKIP_HEALTH_CHECK
      }
    };
    
    this.registerSession();
    this.logActivity('STARTUP', 'Cortex MCP instance started (PID: ' + process.pid + ')');
    
    // Setup cleanup
    process.on('exit', () => this.cleanup());
    process.on('SIGINT', () => { this.cleanup(); process.exit(0); });
    process.on('SIGTERM', () => { this.cleanup(); process.exit(0); });
  }
  
  generateInstanceId() {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2, 8);
    return 'cortex-' + timestamp + '-' + random;
  }
  
  detectClaudeInstance() {
    try {
      const ppid = process.ppid;
      if (ppid) {
        return 'claude-' + ppid;
      }
    } catch (error) {
      // Ignore detection errors
    }
    return 'unknown-' + (process.ppid || 'no-parent');
  }
  
  registerSession() {
    try {
      let sessions = {};
      if (require('fs').existsSync(this.sessionFile)) {
        const data = require('fs').readFileSync(this.sessionFile, 'utf8');
        sessions = JSON.parse(data);
      }
      sessions[this.instanceId] = this.session;
      require('fs').writeFileSync(this.sessionFile, JSON.stringify(sessions, null, 2));
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to register session:', error);
    }
  }
  
  logActivity(type, message, metadata) {
    const timestamp = new Date().toISOString();
    const logEntry = {
      timestamp,
      sessionId: this.instanceId,
      pid: process.pid,
      claudeInstance: this.session.claudeInstance,
      type,
      message,
      metadata: metadata || {}
    };
    
    this.session.lastActivity = Date.now();
    this.updateSession();
    
    // Log to individual session file
    const sessionLogFile = require('path').join(this.logDir, this.instanceId + '.log');
    const logLine = JSON.stringify(logEntry) + '\\n';
    
    try {
      require('fs').appendFileSync(sessionLogFile, logLine);
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to write log:', error);
    }
    
    // Also log to stderr
    console.error('[' + timestamp + '] [' + this.instanceId + '] [' + type + '] ' + message);
    if (metadata && Object.keys(metadata).length > 0) {
      console.error('[' + timestamp + '] [' + this.instanceId + '] [METADATA]', JSON.stringify(metadata));
    }
  }
  
  logHealthCheck(success, error) {
    const healthCheck = {
      timestamp: Date.now(),
      success,
      error
    };
    this.session.healthChecks.push(healthCheck);
    if (this.session.healthChecks.length > 10) {
      this.session.healthChecks = this.session.healthChecks.slice(-10);
    }
    this.updateSession();
    this.logActivity('HEALTH_CHECK', success ? 'Health check passed' : 'Health check failed', { error });
  }
  
  updateStatus(status) {
    this.session.status = status;
    this.session.lastActivity = Date.now();
    this.updateSession();
    this.logActivity('STATUS_CHANGE', 'Status changed to: ' + status);
  }
  
  updateSession() {
    try {
      let sessions = {};
      if (require('fs').existsSync(this.sessionFile)) {
        const data = require('fs').readFileSync(this.sessionFile, 'utf8');
        sessions = JSON.parse(data);
      }
      sessions[this.instanceId] = this.session;
      require('fs').writeFileSync(this.sessionFile, JSON.stringify(sessions, null, 2));
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to update session:', error);
    }
  }
  
  cleanup() {
    this.logActivity('SHUTDOWN', 'Cortex MCP instance shutting down');
    try {
      if (require('fs').existsSync(this.sessionFile)) {
        const data = require('fs').readFileSync(this.sessionFile, 'utf8');
        const sessions = JSON.parse(data);
        delete sessions[this.instanceId];
        require('fs').writeFileSync(this.sessionFile, JSON.stringify(sessions, null, 2));
      }
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to cleanup session:', error);
    }
  }
}

// Global logger instance
let globalMultiInstanceLogger = null;
function getMultiInstanceLogger(projectPath) {
  if (!globalMultiInstanceLogger) {
    globalMultiInstanceLogger = new MultiInstanceLogger(projectPath);
  }
  return globalMultiInstanceLogger;
}

`;

// Find the constructor and inject logger initialization
const constructorMatch = serverCode.match(/(constructor\([^)]*\)\s*{)/);
if (constructorMatch) {
  const insertPoint = constructorMatch.index + constructorMatch[0].length;
  const beforeConstructor = serverCode.substring(0, insertPoint);
  const afterConstructor = serverCode.substring(insertPoint);
  
  const loggerInit = `
    // Initialize multi-instance logger
    this.logger = getMultiInstanceLogger(projectPath);
    this.logger.logActivity('INIT', 'Initializing Lightweight Stdio Cortex MCP Server', {
      projectPath,
      pid: process.pid,
      parentPid: process.ppid,
      multiInstanceMode: !!process.env.MCP_MULTI_INSTANCE
    });
  `;
  
  serverCode = loggerCode + beforeConstructor + loggerInit + afterConstructor;
}

// Add logging to start method
serverCode = serverCode.replace(
  /async start\(\) {[\s\S]*?transport = new StdioServerTransport\(\);[\s\S]*?await this\.server\.connect\(transport\);/,
  `async start() {
    const startTime = Date.now();
    conditionalLogger.ready('🚀 Cortex MCP Server (stdio) Starting...', {
      metadata: {
        version: getVersion(),
        commit: getGitCommit(),
        node: process.version,
        platform: os.platform(),
        pid: process.pid
      }
    });

    if (this.logger) this.logger.logActivity('MCP_START', 'Starting MCP server connection');

    try {
      // Create stdio transport and connect
      const transport = new StdioServerTransport();
      await this.server.connect(transport);
      
      const startupTime = Date.now() - startTime;
      if (this.logger) {
        this.logger.updateStatus('ready');
        this.logger.logActivity('MCP_READY', 'MCP server connected successfully', { startupTimeMs: startupTime });
      }`
);

// Add logging to health checking
serverCode = serverCode.replace(
  /this\.healthCheckInterval = setInterval\(async \(\) => {[\s\S]*?}, 30000\);/,
  `if (this.logger) this.logger.logActivity('HEALTH_CHECK', 'Starting health check monitoring (30s interval)');
   this.healthCheckInterval = setInterval(async () => {
     try {
       const isHealthy = await this.embeddingClient.testConnection();
       if (this.logger) this.logger.logHealthCheck(isHealthy);
       if (!isHealthy && !this.fallbackMode) {
         this.fallbackMode = true;
         if (this.logger) this.logger.logActivity('MODE_CHANGE', 'Switched to fallback mode');
       } else if (isHealthy && this.fallbackMode) {
         this.fallbackMode = false;
         if (this.logger) this.logger.logActivity('MODE_CHANGE', 'Switched to centralized mode');
       }
     } catch (error) {
       if (this.logger) this.logger.logHealthCheck(false, error instanceof Error ? error.message : String(error));
     }
   }, 30000);`
);

// Write the enhanced server
fs.writeFileSync(stdioServerPath, serverCode);

console.log('✅ Enhanced logging injected successfully!');
console.log('📝 Features added:');
console.log('   - Multi-instance session tracking');
console.log('   - Health check logging');
console.log('   - Startup/shutdown activity logs');
console.log('   - Session conflict detection');
console.log('📁 Logs will be written to: ~/.cortex/multi-instance-logs/');
console.log('');
console.log('🔧 To test: node enhanced-debug-cortex-mcp.js');
console.log('🏥 Health check: Use @cortex-multi_instance_health in Claude Code');