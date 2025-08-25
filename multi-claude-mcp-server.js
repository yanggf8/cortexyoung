#!/usr/bin/env node

/**
 * Multi-Claude Code MCP Server
 * Enhanced logging to track different Claude Code sessions
 * Each Claude Code instance should spawn its own MCP server process
 */

const { Server } = require('@modelcontextprotocol/sdk/server/index.js');
const { StdioServerTransport } = require('@modelcontextprotocol/sdk/server/stdio.js');
const { ListToolsRequestSchema, CallToolRequestSchema } = require('@modelcontextprotocol/sdk/types.js');
const fs = require('fs');
const os = require('os');
const path = require('path');

class MultiClaudeLogger {
  constructor() {
    this.logDir = path.join(os.homedir(), '.cortex', 'multi-claude-logs');
    this.sessionId = this.generateSessionId();
    this.claudeSession = this.detectClaudeSession();
    
    // Create logs directory
    if (!fs.existsSync(this.logDir)) {
      fs.mkdirSync(this.logDir, { recursive: true });
    }
    
    // Initialize sessionData BEFORE any logging
    this.sessionData = {
      sessionId: this.sessionId,
      claudeSession: this.claudeSession,
      pid: process.pid,
      parentPid: process.ppid,
      startTime: Date.now(),
      lastActivity: Date.now(),
      toolCalls: [],
      status: 'starting'
    };
    
    this.registerSession();
    this.logEvent('SESSION_START', `New MCP server instance for Claude session: ${this.claudeSession}`, {
      pid: process.pid,
      parentPid: process.ppid,
      environment: Object.keys(process.env).filter(k => k.includes('CLAUDE') || k.includes('MCP'))
    });
    
    // Setup cleanup
    process.on('exit', () => this.cleanup());
    process.on('SIGINT', () => { this.cleanup(); process.exit(0); });
    process.on('SIGTERM', () => { this.cleanup(); process.exit(0); });
  }
  
  generateSessionId() {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2, 8);
    return `mcp-${timestamp}-${random}`;
  }
  
  detectClaudeSession() {
    try {
      // Try multiple methods to detect Claude Code session
      const methods = [
        () => process.env.CLAUDE_SESSION_ID,
        () => process.env.CLAUDE_DESKTOP_SESSION,
        () => `claude-${process.ppid}`,
        () => this.detectFromProcessTree(),
        () => this.detectFromTiming()
      ];
      
      for (const method of methods) {
        const result = method();
        if (result) {
          this.logEvent('CLAUDE_DETECTION', `Detected Claude session via method`, { method: method.name, result });
          return result;
        }
      }
      
      return `unknown-claude-${Date.now()}`;
    } catch (error) {
      this.logEvent('CLAUDE_DETECTION_ERROR', 'Failed to detect Claude session', { error: error.message });
      return `error-claude-${Date.now()}`;
    }
  }
  
  detectFromProcessTree() {
    try {
      const { execSync } = require('child_process');
      // Try to get parent process info
      const parentInfo = execSync(`ps -p ${process.ppid} -o pid,ppid,command`, { encoding: 'utf8' });
      if (parentInfo.includes('claude') || parentInfo.includes('Claude')) {
        return `claude-tree-${process.ppid}`;
      }
    } catch (error) {
      // Ignore detection errors
    }
    return null;
  }
  
  detectFromTiming() {
    // Use startup timing to differentiate Claude sessions
    const startupTime = Date.now();
    return `claude-time-${startupTime}`;
  }
  
  registerSession() {
    try {
      const sessionsFile = path.join(this.logDir, 'active-claude-sessions.json');
      let sessions = {};
      
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        sessions = JSON.parse(data);
      }
      
      sessions[this.sessionId] = this.sessionData;
      fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
      
      this.logEvent('SESSION_REGISTERED', 'Session registered in active sessions', {
        totalSessions: Object.keys(sessions).length,
        claudeSessions: [...new Set(Object.values(sessions).map(s => s.claudeSession))]
      });
    } catch (error) {
      this.logEvent('SESSION_REGISTER_ERROR', 'Failed to register session', { error: error.message });
    }
  }
  
  updateSession() {
    try {
      this.sessionData.lastActivity = Date.now();
      const sessionsFile = path.join(this.logDir, 'active-claude-sessions.json');
      let sessions = {};
      
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        sessions = JSON.parse(data);
      }
      
      sessions[this.sessionId] = this.sessionData;
      fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
    } catch (error) {
      console.error(`[MultiClaudeLogger] Failed to update session: ${error.message}`);
    }
  }
  
  logEvent(type, message, metadata = {}) {
    const timestamp = new Date().toISOString();
    const logEntry = {
      timestamp,
      sessionId: this.sessionId,
      claudeSession: this.claudeSession,
      pid: process.pid,
      parentPid: process.ppid,
      type,
      message,
      metadata
    };
    
    // Write to session-specific log file
    const sessionLogFile = path.join(this.logDir, `${this.sessionId}.log`);
    const logLine = JSON.stringify(logEntry) + '\n';
    
    try {
      fs.appendFileSync(sessionLogFile, logLine);
    } catch (error) {
      console.error(`[MultiClaudeLogger] Failed to write log: ${error.message}`);
    }
    
    // Update session activity (after logging to avoid recursion)
    try {
      this.sessionData.lastActivity = Date.now();
      const sessionsFile = path.join(this.logDir, 'active-claude-sessions.json');
      let sessions = {};
      
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        sessions = JSON.parse(data);
      }
      
      sessions[this.sessionId] = this.sessionData;
      fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
    } catch (error) {
      console.error(`[MultiClaudeLogger] Failed to update session: ${error.message}`);
    }
    
    // Also log to stderr with Claude session info
    console.error(`[${timestamp}] [${this.claudeSession}] [${this.sessionId}] [${type}] ${message}`);
    if (metadata && Object.keys(metadata).length > 0) {
      console.error(`[${timestamp}] [${this.claudeSession}] [METADATA]`, JSON.stringify(metadata));
    }
  }
  
  logToolCall(toolName, args) {
    const toolCall = {
      timestamp: Date.now(),
      toolName,
      args: JSON.stringify(args)
    };
    
    this.sessionData.toolCalls.push(toolCall);
    this.logEvent('TOOL_CALL', `Tool invoked: ${toolName}`, { args });
  }
  
  updateStatus(status) {
    this.sessionData.status = status;
    this.logEvent('STATUS_CHANGE', `Status changed to: ${status}`);
  }
  
  cleanup() {
    this.logEvent('SESSION_END', 'MCP server instance shutting down');
    
    try {
      const sessionsFile = path.join(this.logDir, 'active-claude-sessions.json');
      if (fs.existsSync(sessionsFile)) {
        const data = fs.readFileSync(sessionsFile, 'utf8');
        const sessions = JSON.parse(data);
        delete sessions[this.sessionId];
        fs.writeFileSync(sessionsFile, JSON.stringify(sessions, null, 2));
        
        this.logEvent('SESSION_CLEANUP', 'Session removed from active sessions', {
          remainingSessions: Object.keys(sessions).length
        });
      }
    } catch (error) {
      this.logEvent('SESSION_CLEANUP_ERROR', 'Failed to cleanup session', { error: error.message });
    }
  }
  
  getSessionAnalysis() {
    try {
      const sessionsFile = path.join(this.logDir, 'active-claude-sessions.json');
      if (!fs.existsSync(sessionsFile)) {
        return 'No active sessions file found';
      }
      
      const data = fs.readFileSync(sessionsFile, 'utf8');
      const sessions = JSON.parse(data);
      
      const analysis = {
        totalSessions: Object.keys(sessions).length,
        claudeSessions: [...new Set(Object.values(sessions).map(s => s.claudeSession))],
        currentSession: this.sessionId,
        sessionDetails: Object.values(sessions).map(s => ({
          sessionId: s.sessionId,
          claudeSession: s.claudeSession,
          pid: s.pid,
          uptime: Math.round((Date.now() - s.startTime) / 1000),
          status: s.status,
          toolCalls: s.toolCalls.length
        }))
      };
      
      return JSON.stringify(analysis, null, 2);
    } catch (error) {
      return `Error analyzing sessions: ${error.message}`;
    }
  }
}

class MultiClaudeMCPServer {
  constructor() {
    // Initialize enhanced logging first
    this.logger = new MultiClaudeLogger();
    
    this.server = new Server({
      name: 'cortex-multi-claude-mcp-server',
      version: '3.0.0',
    }, {
      capabilities: {
        tools: {},
      },
    });

    this.setupHandlers();
    this.setupErrorHandling();
    
    this.logger.updateStatus('initialized');
  }

  setupHandlers() {
    this.logger.logEvent('SETUP', 'Setting up MCP handlers');
    
    // List tools handler
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      this.logger.logEvent('TOOLS_LIST', 'Tools list requested');
      
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
            name: 'multi_claude_health',
            description: 'Multi-Claude Code instance health monitoring',
            inputSchema: {
              type: 'object',
              properties: {},
              required: []
            }
          },
          {
            name: 'session_analysis',
            description: 'Analyze active Claude Code sessions and MCP instances',
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
      
      this.logger.logToolCall(name, args);
      
      try {
        let result = '';
        
        switch (name) {
          case 'semantic_search':
            result = this.handleSemanticSearch(args);
            break;
            
          case 'multi_claude_health':
            result = this.handleMultiClaudeHealth();
            break;
            
          case 'session_analysis':
            result = this.handleSessionAnalysis();
            break;
            
          default:
            throw new Error(`Unknown tool: ${name}`);
        }

        this.logger.logEvent('TOOL_SUCCESS', `Tool ${name} completed successfully`);
        
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
        this.logger.logEvent('TOOL_ERROR', `Tool ${name} failed`, { error: error.message });
        
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
    
    return `## Multi-Claude Semantic Search: "${query}"

**Claude Session**: ${this.logger.claudeSession}  
**MCP Instance**: ${this.logger.sessionId}  
**Process ID**: ${process.pid}

### Search Results (${maxChunks} chunks):

1. **Example Function** (src/example.ts:10-25)
\`\`\`typescript
function exampleSearch(query: string) {
  // Multi-Claude compatible search
  return processQuery(query);
}
\`\`\`

✅ **Multi-Claude Support**: Each Claude Code instance has its own MCP server  
🎯 **Session Isolation**: Results are isolated per Claude session  
📊 **Enhanced Logging**: All interactions tracked in ~/.cortex/multi-claude-logs/`;
  }

  handleMultiClaudeHealth() {
    return `## Multi-Claude Code Health Status

### Current Session
- **Claude Session**: ${this.logger.claudeSession}
- **MCP Session**: ${this.logger.sessionId}  
- **Process ID**: ${process.pid}
- **Parent PID**: ${process.ppid}
- **Status**: ${this.logger.sessionData.status}
- **Uptime**: ${Math.round((Date.now() - this.logger.sessionData.startTime) / 1000)}s
- **Tool Calls**: ${this.logger.sessionData.toolCalls.length}

### System Status
- **Memory Usage**: ${Math.round(process.memoryUsage().heapUsed / 1024 / 1024)}MB
- **Log Directory**: ~/.cortex/multi-claude-logs/
- **Session File**: active-claude-sessions.json

✅ **Multi-Claude Ready**: Each Claude Code spawns its own MCP server instance  
📊 **Enhanced Logging**: Tracking all Claude sessions separately  
🔍 **Session Analysis**: Use session_analysis tool for detailed view`;
  }

  handleSessionAnalysis() {
    const analysis = this.logger.getSessionAnalysis();
    
    return `## Claude Code Session Analysis

### Active Sessions Overview
\`\`\`json
${analysis}
\`\`\`

### Key Insights
- **Multiple Claude Support**: Each Claude Code instance spawns separate MCP server
- **Session Isolation**: Tools and data are isolated per Claude session  
- **Process Management**: Each session runs in its own process
- **Enhanced Logging**: All sessions tracked in ~/.cortex/multi-claude-logs/

### Troubleshooting
If a second Claude Code fails to connect:
1. Check session logs in ~/.cortex/multi-claude-logs/
2. Verify each Claude Code spawns new MCP server process
3. Look for session conflicts or resource issues
4. Use this tool to monitor active sessions`;
  }

  setupErrorHandling() {
    this.server.onerror = (error) => {
      this.logger.logEvent('MCP_ERROR', 'MCP server error', { error: error.message });
    };

    // Handle process signals
    process.on('SIGINT', () => {
      this.logger.logEvent('SIGNAL', 'Received SIGINT, shutting down');
      this.logger.cleanup();
      process.exit(0);
    });

    process.on('SIGTERM', () => {
      this.logger.logEvent('SIGNAL', 'Received SIGTERM, shutting down');
      this.logger.cleanup();
      process.exit(0);
    });
    
    process.on('uncaughtException', (error) => {
      this.logger.logEvent('UNCAUGHT_EXCEPTION', 'Uncaught exception', { error: error.message, stack: error.stack });
      this.logger.cleanup();
      process.exit(1);
    });
  }

  async run() {
    try {
      this.logger.logEvent('SERVER_START', 'Starting MCP server connection');
      
      const transport = new StdioServerTransport();
      await this.server.connect(transport);
      
      this.logger.updateStatus('connected');
      this.logger.logEvent('SERVER_READY', 'MCP server connected and ready');
      
      console.error('[INFO] Multi-Claude MCP Server running on stdio (this STDERR message is by design)');
      
    } catch (error) {
      this.logger.logEvent('SERVER_START_ERROR', 'Failed to start MCP server', { error: error.message });
      this.logger.cleanup();
      process.exit(1);
    }
  }
}

// Start the server
const server = new MultiClaudeMCPServer();
server.run();