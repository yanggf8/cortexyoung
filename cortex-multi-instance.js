#!/usr/bin/env node

/**
 * Cortex Multi-Instance MCP Server
 * Enhanced for multi-Claude Code session tracking
 * Based on working-mcp-server.js pattern
 */

const { Server } = require('@modelcontextprotocol/sdk/server/index.js');
const { StdioServerTransport } = require('@modelcontextprotocol/sdk/server/stdio.js');
const { ListToolsRequestSchema, CallToolRequestSchema } = require('@modelcontextprotocol/sdk/types.js');
const fs = require('fs');
const os = require('os');
const path = require('path');

class MultiInstanceLogger {
  constructor() {
    this.logDir = path.join(os.homedir(), '.cortex', 'multi-instance-logs');
    this.sessionId = this.generateSessionId();
    this.claudeSession = this.detectClaudeSession();
    
    // Create logs directory
    if (!fs.existsSync(this.logDir)) {
      fs.mkdirSync(this.logDir, { recursive: true });
    }
    
    this.logToFile(`SESSION_START: ${this.claudeSession} (PID: ${process.pid}, Session: ${this.sessionId})`);
    this.registerSession();
  }
  
  generateSessionId() {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2, 8);
    return `cortex-${timestamp}-${random}`;
  }
  
  detectClaudeSession() {
    try {
      // Multiple detection methods for Claude Code instances
      const methods = [
        () => process.env.CLAUDE_SESSION_ID,
        () => process.env.CLAUDE_DESKTOP_SESSION,
        () => `claude-${process.ppid}`,
        () => `claude-time-${Date.now()}`
      ];
      
      for (const method of methods) {
        const result = method();
        if (result) {
          return result;
        }
      }
      
      return `unknown-claude-${Date.now()}`;
    } catch (error) {
      return `error-claude-${Date.now()}`;
    }
  }
  
  registerSession() {
    try {
      const sessionsFile = path.join(this.logDir, 'active-sessions.json');
      let sessions = {};
      
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        sessions = JSON.parse(data);
      }
      
      sessions[this.sessionId] = {
        sessionId: this.sessionId,
        claudeSession: this.claudeSession,
        pid: process.pid,
        parentPid: process.ppid,
        startTime: Date.now(),
        lastActivity: Date.now(),
        status: 'starting'
      };
      
      fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
      this.logToFile(`SESSION_REGISTERED: Total active sessions: ${Object.keys(sessions).length}`);
    } catch (error) {
      this.logToFile(`SESSION_REGISTER_ERROR: ${error.message}`);
    }
  }
  
  logToFile(message) {
    try {
      const timestamp = new Date().toISOString();
      const logLine = `[${timestamp}] [${this.claudeSession}] [${this.sessionId}] ${message}\n`;
      const sessionLogFile = path.join(this.logDir, `${this.sessionId}.log`);
      fs.appendFileSync(sessionLogFile, logLine);
      console.error(`[Multi-Instance] ${logLine.trim()}`);
    } catch (error) {
      console.error(`[Multi-Instance] Failed to log: ${error.message}`);
    }
  }
  
  cleanup() {
    this.logToFile('SESSION_END: MCP server shutting down');
    
    try {
      const sessionsFile = path.join(this.logDir, 'active-sessions.json');
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        const sessions = JSON.parse(data);
        delete sessions[this.sessionId];
        fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
        this.logToFile(`SESSION_CLEANUP: ${Object.keys(sessions).length} sessions remaining`);
      }
    } catch (error) {
      this.logToFile(`SESSION_CLEANUP_ERROR: ${error.message}`);
    }
  }
}

class CortexMultiInstanceServer {
  constructor() {
    // Initialize enhanced logging first
    this.logger = new MultiInstanceLogger();
    
    // Capture detailed environment information
    this.logger.logToFile(`ENVIRONMENT: Node.js ${process.version}, PID: ${process.pid}, PPID: ${process.ppid}`);
    this.logger.logToFile(`PROCESS_ENV: CLAUDE_SESSION_ID=${process.env.CLAUDE_SESSION_ID}, CLAUDE_DESKTOP_SESSION=${process.env.CLAUDE_DESKTOP_SESSION}`);
    this.logger.logToFile(`STDIO_INITIAL: stdin.isTTY=${process.stdin.isTTY}, stdout.isTTY=${process.stdout.isTTY}, stderr.isTTY=${process.stderr.isTTY}`);
    this.logger.logToFile(`ARGS: ${JSON.stringify(process.argv)}`);
    this.logger.logToFile(`CWD: ${process.cwd()}`);
    
    this.server = new Server({
      name: 'cortex-multi-instance',
      version: '3.0.0',
    }, {
      capabilities: {
        tools: {},
      },
    });

    this.setupHandlers();
    this.setupErrorHandling();
    
    this.logger.logToFile('MCP server initialized');
  }

  setupHandlers() {
    this.logger.logToFile('Setting up MCP handlers');
    
    // List tools handler
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      this.logger.logToFile('Tools list requested');
      
      return {
        tools: [
          {
            name: 'semantic_search',
            description: 'Advanced semantic search with multi-Claude support',
            inputSchema: {
              type: 'object',
              properties: {
                query: {
                  type: 'string',
                  description: 'Search query'
                },
                max_chunks: {
                  type: 'number',
                  default: 5,
                  minimum: 1,
                  maximum: 20
                }
              },
              required: ['query']
            }
          },
          {
            name: 'multi_instance_health',
            description: 'Multi-instance health monitoring and diagnostics',
            inputSchema: {
              type: 'object',
              properties: {},
              required: []
            }
          },
          {
            name: 'session_analysis',
            description: 'Analyze active Claude Code sessions and instances',
            inputSchema: {
              type: 'object',
              properties: {},
              required: []
            }
          }
        ]
      };
    });

    // Call tool handler
    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;
      
      this.logger.logToFile(`Tool invoked: ${name} with args: ${JSON.stringify(args)}`);
      
      try {
        let result = '';
        
        switch (name) {
          case 'semantic_search':
            result = this.handleSemanticSearch(args);
            break;
            
          case 'multi_instance_health':
            result = this.handleMultiInstanceHealth();
            break;
            
          case 'session_analysis':
            result = this.handleSessionAnalysis();
            break;
            
          default:
            throw new Error(`Unknown tool: ${name}`);
        }

        this.logger.logToFile(`Tool ${name} completed successfully`);
        
        return {
          content: [
            {
              type: 'text',
              text: result
            }
          ],
          isError: false
        };
        
      } catch (error) {
        this.logger.logToFile(`Tool ${name} failed: ${error.message}`);
        
        return {
          content: [
            {
              type: 'text',
              text: `Error: ${error.message}`
            }
          ],
          isError: true
        };
      }
    });
  }

  handleSemanticSearch(args) {
    const query = args?.query || 'test';
    const maxChunks = args?.max_chunks || 5;
    
    return `## Multi-Instance Semantic Search: "${query}"

**Claude Session**: ${this.logger.claudeSession}  
**MCP Instance**: ${this.logger.sessionId}  
**Process ID**: ${process.pid}

### Search Results (${maxChunks} chunks):

1. **Example Function** (src/example.ts:10-25)
\`\`\`typescript
function exampleSearch(query: string) {
  // Multi-instance compatible search
  return processQuery(query);
}
\`\`\`

✅ **Multi-Instance Support**: Each Claude Code instance has its own MCP server  
🎯 **Session Isolation**: Results are isolated per Claude session  
📊 **Enhanced Logging**: All interactions tracked in ~/.cortex/multi-instance-logs/`;
  }

  handleMultiInstanceHealth() {
    return `## Multi-Instance Health Status

### Current Session
- **Claude Session**: ${this.logger.claudeSession}
- **MCP Session**: ${this.logger.sessionId}  
- **Process ID**: ${process.pid}
- **Parent PID**: ${process.ppid}
- **Uptime**: ${Math.round(process.uptime())}s

### System Status
- **Memory Usage**: ${Math.round(process.memoryUsage().heapUsed / 1024 / 1024)}MB
- **Log Directory**: ~/.cortex/multi-instance-logs/
- **Session File**: active-sessions.json

✅ **Multi-Instance Ready**: Each Claude Code spawns its own MCP server instance  
📊 **Enhanced Logging**: Tracking all Claude sessions separately  
🔍 **Session Analysis**: Use session_analysis tool for detailed view`;
  }

  handleSessionAnalysis() {
    try {
      const sessionsFile = path.join(this.logger.logDir, 'active-sessions.json');
      let analysis = 'No active sessions file found';
      
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        const sessions = JSON.parse(data);
        
        analysis = {
          totalSessions: Object.keys(sessions).length,
          claudeSessions: [...new Set(Object.values(sessions).map(s => s.claudeSession))],
          currentSession: this.logger.sessionId,
          sessionDetails: Object.values(sessions).map(s => ({
            sessionId: s.sessionId,
            claudeSession: s.claudeSession,
            pid: s.pid,
            uptime: Math.round((Date.now() - s.startTime) / 1000),
            status: s.status
          }))
        };
        
        analysis = JSON.stringify(analysis, null, 2);
      }
      
      return `## Claude Code Session Analysis

### Active Sessions Overview
\`\`\`json
${analysis}
\`\`\`

### Key Insights
- **Multiple Claude Support**: Each Claude Code instance spawns separate MCP server
- **Session Isolation**: Tools and data are isolated per Claude session  
- **Process Management**: Each session runs in its own process
- **Enhanced Logging**: All sessions tracked in ~/.cortex/multi-instance-logs/

### Troubleshooting
If a second Claude Code fails to connect:
1. Check session logs in ~/.cortex/multi-instance-logs/
2. Verify each Claude Code spawns new MCP server process
3. Look for session conflicts or resource issues
4. Use this tool to monitor active sessions`;
    } catch (error) {
      return `Error analyzing sessions: ${error.message}`;
    }
  }

  setupErrorHandling() {
    this.logger.logToFile('Setting up enhanced error handling and signal tracking');
    
    this.server.onerror = (error) => {
      this.logger.logToFile(`MCP server error: ${error.message}`);
      this.logger.logToFile(`MCP server error stack: ${error.stack}`);
    };

    // Enhanced signal handling with detailed logging
    const signalHandler = (signal) => {
      this.logger.logToFile(`SIGNAL_RECEIVED: ${signal} (parent PID: ${process.ppid})`);
      this.logger.logToFile(`SIGNAL_CONTEXT: stdin.isTTY=${process.stdin.isTTY}, stdout.isTTY=${process.stdout.isTTY}`);
      this.logger.logToFile(`SIGNAL_TIMING: Process uptime ${Math.round(process.uptime() * 1000)}ms`);
      this.logger.cleanup();
      process.exit(signal === 'SIGTERM' ? 0 : 1);
    };

    process.on('SIGINT', () => signalHandler('SIGINT'));
    process.on('SIGTERM', () => signalHandler('SIGTERM'));
    process.on('SIGHUP', () => signalHandler('SIGHUP'));
    process.on('SIGPIPE', () => signalHandler('SIGPIPE'));
    
    process.on('uncaughtException', (error) => {
      this.logger.logToFile(`UNCAUGHT_EXCEPTION: ${error.message}`);
      this.logger.logToFile(`UNCAUGHT_STACK: ${error.stack}`);
      this.logger.logToFile(`UNCAUGHT_CONTEXT: Process uptime ${Math.round(process.uptime() * 1000)}ms`);
      this.logger.cleanup();
      process.exit(1);
    });

    process.on('unhandledRejection', (reason, promise) => {
      this.logger.logToFile(`UNHANDLED_REJECTION: ${reason}`);
      this.logger.logToFile(`UNHANDLED_PROMISE: ${promise}`);
      this.logger.cleanup();
      process.exit(1);
    });
    
    process.on('exit', (code) => {
      this.logger.logToFile(`PROCESS_EXIT: Code ${code}, uptime ${Math.round(process.uptime() * 1000)}ms`);
      this.logger.cleanup();
    });

    // Monitor stdin/stdout for unexpected closure
    process.stdin.on('end', () => {
      this.logger.logToFile('STDIN_END: stdin stream ended - this is normal when no MCP client is connected');
      // Don't exit immediately - let the server stay alive for potential reconnections
    });
    
    process.stdin.on('error', (error) => {
      this.logger.logToFile(`STDIN_ERROR: ${error.message}`);
    });
    
    process.stdout.on('error', (error) => {
      this.logger.logToFile(`STDOUT_ERROR: ${error.message}`);
    });
    
    process.stderr.on('error', (error) => {
      this.logger.logToFile(`STDERR_ERROR: ${error.message}`);
    });
  }

  async run() {
    try {
      this.logger.logToFile('Starting MCP server connection');
      this.logger.logToFile(`TRANSPORT_INIT: Creating StdioServerTransport`);
      this.logger.logToFile(`STDIO_STATUS: stdin.readable=${process.stdin.readable}, stdout.writable=${process.stdout.writable}`);
      this.logger.logToFile(`STDIO_DESCRIPTORS: stdin.fd=${process.stdin.fd}, stdout.fd=${process.stdout.fd}, stderr.fd=${process.stderr.fd}`);
      
      // Create transport with enhanced error handling
      const transport = new StdioServerTransport();
      this.logger.logToFile(`TRANSPORT_CREATED: StdioServerTransport instance created`);
      
      // Set up connection monitoring with proper error handling
      try {
        this.logger.logToFile(`CONNECTION_ATTEMPT: server.connect() called`);
        await this.server.connect(transport);
        this.logger.logToFile('MCP server connected and ready');
        
        // Update session status to ready
        this.updateSessionStatus('ready');
        
        console.error('[INFO] Cortex Multi-Instance MCP Server running on stdio');
        
        // Keep the process alive and monitor for unexpected exits
        this.setupKeepAlive();
        
      } catch (connectionError) {
        this.logger.logToFile(`MCP connection error: ${connectionError.message}`);
        
        // If connection fails but server is still valid, keep it alive for retry
        if (connectionError.message.includes('stdin') || connectionError.message.includes('EPIPE')) {
          this.logger.logToFile('STDIN connection failed - entering standby mode for reconnection');
          this.updateSessionStatus('standby');
          this.setupReconnectionMonitor();
        } else {
          throw connectionError; // Re-throw non-stdin errors
        }
      }
      
    } catch (error) {
      this.logger.logToFile(`Failed to start MCP server: ${error.message}`);
      this.logger.logToFile(`Failed to start MCP server stack: ${error.stack}`);
      this.updateSessionStatus('error');
      this.logger.cleanup();
      process.exit(1);
    }
  }

  setupReconnectionMonitor() {
    this.logger.logToFile('RECONNECTION_MONITOR: Setting up reconnection monitoring');
    
    // Monitor for stdin becoming available again
    const reconnectInterval = setInterval(() => {
      if (process.stdin.readable) {
        this.logger.logToFile('RECONNECTION_ATTEMPT: stdin became readable, attempting reconnection');
        clearInterval(reconnectInterval);
        
        // Try to reconnect
        setTimeout(() => {
          this.run().catch(error => {
            this.logger.logToFile(`RECONNECTION_FAILED: ${error.message}`);
          });
        }, 1000);
      } else {
        this.logger.logToFile('RECONNECTION_STANDBY: Waiting for stdin to become available');
      }
    }, 5000);
    
    // Keep process alive in standby mode
    this.setupKeepAlive();
    
    // Clean up interval on exit
    process.on('exit', () => {
      clearInterval(reconnectInterval);
    });
  }

  updateSessionStatus(status) {
    try {
      const sessionsFile = path.join(this.logger.logDir, 'active-sessions.json');
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        const sessions = JSON.parse(data);
        
        if (sessions[this.logger.sessionId]) {
          sessions[this.logger.sessionId].status = status;
          sessions[this.logger.sessionId].lastActivity = Date.now();
          fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
          this.logger.logToFile(`SESSION_STATUS_UPDATED: ${status}`);
        }
      }
    } catch (error) {
      this.logger.logToFile(`SESSION_STATUS_UPDATE_ERROR: ${error.message}`);
    }
  }

  setupKeepAlive() {
    this.logger.logToFile('KEEPALIVE_SETUP: Setting up process monitoring');
    
    // Monitor for unexpected process termination
    const heartbeatInterval = setInterval(() => {
      this.logger.logToFile(`HEARTBEAT: Process alive (uptime: ${Math.round(process.uptime() * 1000)}ms)`);
      this.updateSessionStatus('ready');
    }, 30000);
    
    // Clean up interval on exit
    process.on('exit', () => {
      clearInterval(heartbeatInterval);
    });
    
    this.logger.logToFile('KEEPALIVE_READY: Process monitoring active');
  }
}

// Start the server
const server = new CortexMultiInstanceServer();
server.run();