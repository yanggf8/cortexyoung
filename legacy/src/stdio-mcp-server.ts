#!/usr/bin/env node

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  Tool,
} from '@modelcontextprotocol/sdk/types.js';
import { ProactiveContextEngine } from './core/proactive-context-engine.js';
import { StdioMcpLogger } from './stdio-mcp-logger.js';

class ClaudeCatMultiInstanceServer {
  private server: Server;
  private contextEngine: ProactiveContextEngine;
  private logger: StdioMcpLogger;
  private isReady: boolean = false;
  private resourceMonitor: NodeJS.Timeout | null = null;

  constructor() {
    // Initialize enhanced logging first
    this.logger = new StdioMcpLogger();
    
    // Capture detailed environment information
    this.logger.logToFile(`ENVIRONMENT: Node.js ${process.version}, PID: ${process.pid}, PPID: ${process.ppid}`);
    this.logger.logToFile(`PROCESS_ENV: CLAUDE_SESSION_ID=${process.env.CLAUDE_SESSION_ID}, CLAUDE_DESKTOP_SESSION=${process.env.CLAUDE_DESKTOP_SESSION}`);
    this.logger.logToFile(`STDIO_INITIAL: stdin.isTTY=${process.stdin.isTTY}, stdout.isTTY=${process.stdout.isTTY}, stderr.isTTY=${process.stderr.isTTY}`);
    this.logger.logToFile(`ARGS: ${JSON.stringify(process.argv)}`);
    this.logger.logToFile(`CWD: ${process.cwd()}`);
    
    this.server = new Server(
      {
        name: `claudecat-multi-instance-${this.logger.getSessionId()}`,
        version: '1.0.0',
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    this.contextEngine = new ProactiveContextEngine();
    this.setupToolHandlers();
    this.setupEngineEventHandlers();
    this.setupErrorHandling();
    
    this.logger.logToFile('ClaudeCat MCP server initialized');
  }

  private setupToolHandlers() {
    this.logger.logToFile('Setting up MCP tool handlers');
    
    // List available tools
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      this.logger.logToFile('Tools list requested');
      
      return {
        tools: [
          {
            name: 'get_project_context',
            description: 'Get current project context and implementation patterns',
            inputSchema: {
              type: 'object',
              properties: {},
              additionalProperties: false,
            },
          },
          {
            name: 'get_implementation_patterns',
            description: 'Get detected implementation patterns for auth, API responses, and error handling',
            inputSchema: {
              type: 'object',
              properties: {},
              additionalProperties: false,
            },
          },
          {
            name: 'get_critical_guardrails',
            description: 'Get critical guardrails based on detected project patterns',
            inputSchema: {
              type: 'object',
              properties: {},
              additionalProperties: false,
            },
          },
          {
            name: 'force_context_update',
            description: 'Force immediate re-detection of project patterns and CLAUDE.md update',
            inputSchema: {
              type: 'object',
              properties: {},
              additionalProperties: false,
            },
          },
          {
            name: 'get_engine_status',
            description: 'Get status information about the ClaudeCat engine',
            inputSchema: {
              type: 'object',
              properties: {},
              additionalProperties: false,
            },
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
        ] satisfies Tool[],
      };
    });

    // Handle tool calls
    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;
      
      this.logger.logToFile(`Tool invoked: ${name} with args: ${JSON.stringify(args)}`);

      try {
        let result: any;
        
        switch (name) {
          case 'get_project_context':
            result = await this.handleGetProjectContext();
            break;

          case 'get_implementation_patterns':
            result = await this.handleGetImplementationPatterns();
            break;

          case 'get_critical_guardrails':
            result = await this.handleGetCriticalGuardrails();
            break;

          case 'force_context_update':
            result = await this.handleForceContextUpdate();
            break;

          case 'get_engine_status':
            result = await this.handleGetEngineStatus();
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
        return result;
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : String(error);
        this.logger.logToFile(`Tool ${name} failed: ${errorMessage}`);
        
        return {
          content: [
            {
              type: 'text',
              text: `Error executing ${name}: ${errorMessage}`,
            },
          ],
        };
      }
    });
  }

  private setupEngineEventHandlers() {
    this.contextEngine.on('context-updated', (context) => {
      const logMessage = {
        project: `${context.projectType} (${context.language})`,
        framework: context.framework,
        patterns: {
          auth: context.implementationPatterns.authentication.confidence,
          api: context.implementationPatterns.apiResponses.confidence,
          errors: context.implementationPatterns.errorHandling.confidence,
        },
      };
      
      this.logger.logToFile(`PROJECT_CONTEXT_UPDATED: ${JSON.stringify(logMessage)}`);
      console.log('📋 Project context updated:', logMessage);
    });

    this.contextEngine.on('context-error', (error) => {
      this.logger.logToFile(`CONTEXT_ENGINE_ERROR: ${error.message}`);
      console.error('❌ Context engine error:', error.message);
    });

    this.contextEngine.on('startup-complete', () => {
      this.isReady = true;
      this.logger.updateSessionStatus('ready');
      this.logger.logToFile('STARTUP_COMPLETE: ClaudeCat startup complete');
      console.log('🎯 ClaudeCat startup complete - Claude Code context ready');
    });
  }

  private async handleGetProjectContext() {
    const context = this.contextEngine.getCurrentContext();
    
    const summary = {
      projectType: context.projectType,
      language: context.language,
      framework: context.framework,
      packageManager: context.packageManager,
      lastUpdated: context.lastUpdated,
      keyDirectories: context.directories.map(d => `${d.path}/ (${d.purpose})`),
      coreDependencies: context.dependencies.slice(0, 10),
      scripts: context.scripts,
    };

    const contextText = `**Multi-Instance Project Context**

**Claude Session**: ${this.logger.getClaudeSession()}  
**MCP Instance**: ${this.logger.getSessionId()}  
**Process ID**: ${process.pid}

**Project**: ${summary.projectType}
**Language**: ${summary.language}  
**Framework**: ${summary.framework}
**Package Manager**: ${summary.packageManager}

**Key Directories**:
${summary.keyDirectories.map(d => `- ${d}`).join('\n')}

**Core Dependencies**: ${summary.coreDependencies.join(', ')}

**Scripts**:
- dev: \`${summary.scripts.dev || 'Not detected'}\`
- build: \`${summary.scripts.build || 'Not detected'}\`
- test: \`${summary.scripts.test || 'Not detected'}\`

**Last Updated**: ${summary.lastUpdated}

✅ **Multi-Instance Support**: Each Claude Code instance has its own MCP server  
📊 **Session Isolation**: Context is isolated per Claude session`;

    return {
      content: [
        {
          type: 'text',
          text: contextText,
        },
      ],
    };
  }

  private async handleGetImplementationPatterns() {
    const patterns = this.contextEngine.getImplementationPatternsSummary();
    const context = this.contextEngine.getCurrentContext();
    const impl = context.implementationPatterns;

    const patternsText = `**Implementation Patterns Detected**

**Claude Session**: ${this.logger.getClaudeSession()}  
**MCP Instance**: ${this.logger.getSessionId()}

**Authentication** (${impl.authentication.confidence}% confidence):
- User Property: \`${impl.authentication.userProperty}\`
- Token Storage: ${impl.authentication.tokenLocation}
- Error Response: ${impl.authentication.errorResponse.format}
- Middleware Pattern: ${impl.authentication.middlewarePattern}
${impl.authentication.evidence.length > 0 ? `- Evidence: ${impl.authentication.evidence.slice(0, 2).join(', ')}` : ''}

**API Responses** (${impl.apiResponses.confidence}% confidence):
- Success Format: ${impl.apiResponses.successFormat}
- Error Format: ${impl.apiResponses.errorFormat}
- Status Codes: ${impl.apiResponses.statusCodeUsage}
- Wrapper Pattern: ${impl.apiResponses.wrapperPattern}
${impl.apiResponses.evidence.length > 0 ? `- Evidence: ${impl.apiResponses.evidence.slice(0, 2).join(', ')}` : ''}

**Error Handling** (${impl.errorHandling.confidence}% confidence):
- Catch Pattern: ${impl.errorHandling.catchPattern}
- Error Structure: ${impl.errorHandling.errorStructure}
- Logging Integration: ${impl.errorHandling.loggingIntegration}
- Propagation Style: ${impl.errorHandling.propagationStyle}
${impl.errorHandling.evidence.length > 0 ? `- Evidence: ${impl.errorHandling.evidence.slice(0, 2).join(', ')}` : ''}

**Pattern Summary**: ${patterns.confidence}

✅ **Multi-Instance Ready**: Patterns detected separately per Claude session`;

    return {
      content: [
        {
          type: 'text',
          text: patternsText,
        },
      ],
    };
  }

  private async handleGetCriticalGuardrails() {
    const guardrails = this.contextEngine.getCriticalGuardrails();

    if (guardrails.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text: '**Critical Guardrails**\n\n⚠️ **Implementation patterns unclear** - Verify project conventions before making architectural decisions\n\n📊 **Multi-Instance**: This session isolated from other Claude instances',
          },
        ],
      };
    }

    const guardrailsText = `**Critical Guardrails**

**Claude Session**: ${this.logger.getClaudeSession()}

${guardrails.map((g, i) => `${i + 1}. ${g}`).join('\n')}

*These guardrails are automatically generated based on detected implementation patterns with 60%+ confidence.*

📊 **Multi-Instance**: Guardrails are session-specific`;

    return {
      content: [
        {
          type: 'text',
          text: guardrailsText,
        },
      ],
    };
  }

  private async handleForceContextUpdate() {
    const updatedContext = await this.contextEngine.forceUpdate();
    
    const updateText = `**Context Update Complete**

**Claude Session**: ${this.logger.getClaudeSession()}  
**MCP Instance**: ${this.logger.getSessionId()}

Project patterns re-analyzed and CLAUDE.md updated.

**Updated**: ${updatedContext.lastUpdated}
**Confidence Levels**:
- Authentication: ${updatedContext.implementationPatterns.authentication.confidence}%
- API Responses: ${updatedContext.implementationPatterns.apiResponses.confidence}%
- Error Handling: ${updatedContext.implementationPatterns.errorHandling.confidence}%

✅ **Multi-Instance**: Update isolated to this Claude session`;

    return {
      content: [
        {
          type: 'text',
          text: updateText,
        },
      ],
    };
  }

  private async handleGetEngineStatus() {
    const status = this.contextEngine.getStatus();
    
    const statusText = `**ClaudeCat Engine Status**

**Claude Session**: ${this.logger.getClaudeSession()}  
**MCP Instance**: ${this.logger.getSessionId()}  
**Process ID**: ${process.pid}

**Initialization**: ${status.initialized ? '✅ Ready' : '❌ Not Ready'}
**Project Root**: ${status.projectRoot}

**File Watcher**:
- Status: ${status.watcher.isWatching ? '👀 Active' : '💤 Inactive'}
- Files Monitored: ${status.watcher.watchedFileCount}
- Processing Update: ${status.watcher.isProcessingUpdate ? '⏳ Yes' : '✅ No'}
- Last Update: ${status.watcher.lastUpdateTime?.toISOString() || 'Never'}

**Multi-Instance Support**: ✅ Enabled
**Session Isolation**: ✅ Active
**Log Directory**: ~/.claude/logs/claudecat/`;

    return {
      content: [
        {
          type: 'text',
          text: statusText,
        },
      ],
    };
  }

  private handleMultiInstanceHealth() {
    const healthText = `## Multi-Instance Health Status

### Current Session
- **Claude Session**: ${this.logger.getClaudeSession()}
- **MCP Session**: ${this.logger.getSessionId()}  
- **Process ID**: ${process.pid}
- **Parent PID**: ${process.ppid}
- **Uptime**: ${Math.round(process.uptime())}s
- **Status**: ${this.isReady ? 'Ready' : 'Starting'}

### System Status
- **Memory Usage**: ${Math.round(process.memoryUsage().heapUsed / 1024 / 1024)}MB
- **Log Directory**: ~/.claude/logs/claudecat/
- **Session File**: active-sessions.json

✅ **Multi-Instance Ready**: Each Claude Code spawns its own MCP server instance  
📊 **Enhanced Logging**: Tracking all Claude sessions separately  
🔍 **Session Analysis**: Use session_analysis tool for detailed view`;

    return {
      content: [
        {
          type: 'text',
          text: healthText,
        },
      ],
    };
  }

  private handleSessionAnalysis() {
    try {
      const analysis = this.logger.getActiveSessionsInfo();
      
      const analysisText = `## Claude Code Session Analysis

### Active Sessions Overview
\`\`\`json
${JSON.stringify(analysis, null, 2)}
\`\`\`

### Key Insights
- **Multiple Claude Support**: Each Claude Code instance spawns separate MCP server
- **Session Isolation**: Tools and data are isolated per Claude session  
- **Process Management**: Each session runs in its own process
- **Enhanced Logging**: All sessions tracked in ~/.claude/logs/claudecat/

### Troubleshooting
If a second Claude Code fails to connect:
1. Check session logs in ~/.claude/logs/claudecat/
2. Verify each Claude Code spawns new MCP server process
3. Look for session conflicts or resource issues
4. Use this tool to monitor active sessions`;

      return {
        content: [
          {
            type: 'text',
            text: analysisText,
          },
        ],
      };
    } catch (error) {
      return {
        content: [
          {
            type: 'text',
            text: `Error analyzing sessions: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
      };
    }
  }

  private setupErrorHandling() {
    this.logger.logToFile('Setting up enhanced error handling and signal tracking');
    
    this.server.onerror = (error) => {
      this.logger.logToFile(`MCP server error: ${error.message}`);
      this.logger.logToFile(`MCP server error stack: ${error.stack}`);
    };

    // Enhanced signal handling with detailed logging
    const signalHandler = (signal: string) => {
      this.logger.logToFile(`SIGNAL_RECEIVED: ${signal} (parent PID: ${process.ppid})`);
      this.logger.logToFile(`SIGNAL_CONTEXT: stdin.isTTY=${process.stdin.isTTY}, stdout.isTTY=${process.stdout.isTTY}`);
      this.logger.logToFile(`SIGNAL_TIMING: Process uptime ${Math.round(process.uptime() * 1000)}ms`);
      this.cleanup();
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
      this.cleanup();
      process.exit(1);
    });

    process.on('unhandledRejection', (reason) => {
      this.logger.logToFile(`UNHANDLED_REJECTION: ${reason}`);
      this.cleanup();
      process.exit(1);
    });
    
    process.on('exit', (code) => {
      this.logger.logToFile(`PROCESS_EXIT: Code ${code}, uptime ${Math.round(process.uptime() * 1000)}ms`);
      this.cleanup();
    });

    // Monitor stdin/stdout for unexpected closure
    process.stdin.on('end', () => {
      this.logger.logToFile('STDIN_END: stdin stream ended - this is normal when no MCP client is connected');
    });
    
    // Simplified stdio error handling - no logging to prevent EPIPE loops
    process.stdin.on('error', () => {
      // Silent handling - stdio errors are expected in MCP environment
    });
    
    process.stdout.on('error', () => {
      // Silent handling - stdio errors are expected in MCP environment  
    });
    
    process.stderr.on('error', () => {
      // Silent handling - stdio errors are expected in MCP environment
    });
  }

  // Removed complex reconnection logic that could cause resource leaks

  async start() {
    try {
      this.logger.logToFile('Starting ClaudeCat MCP server connection');
      this.logger.logToFile(`TRANSPORT_INIT: Creating StdioServerTransport`);
      this.logger.logToFile(`STDIO_STATUS: stdin.readable=${process.stdin.readable}, stdout.writable=${process.stdout.writable}`);
      this.logger.logToFile(`STDIO_DESCRIPTORS: stdin.fd=${process.stdin.fd}, stdout.fd=${process.stdout.fd}, stderr.fd=${process.stderr.fd}`);
      
      console.log('🚀 Starting ClaudeCat Multi-Instance MCP Server...');
      
      // Initialize the context engine
      this.logger.updateSessionStatus('initializing');
      await this.contextEngine.initialize();
      
      // Create transport with enhanced error handling
      const transport = new StdioServerTransport();
      this.logger.logToFile(`TRANSPORT_CREATED: StdioServerTransport instance created`);
      
      // Simplified connection - no complex error handling
      this.logger.logToFile(`CONNECTION_ATTEMPT: server.connect() called`);
      await this.server.connect(transport);
      this.logger.logToFile('MCP server connected and ready');
      
      // Update session status to ready
      this.logger.updateSessionStatus('ready');
      
      // Start resource monitoring
      this.startResourceMonitoring();
      
    } catch (error) {
      this.logger.logToFile(`Failed to start MCP server: ${error instanceof Error ? error.message : String(error)}`);
      this.logger.logToFile(`Failed to start MCP server stack: ${error instanceof Error ? error.stack : String(error)}`);
      this.logger.updateSessionStatus('error');
      this.cleanup();
      process.exit(1);
    }
  }

  private startResourceMonitoring(): void {
    // Monitor resources every 60 seconds
    this.resourceMonitor = setInterval(() => {
      const memUsage = process.memoryUsage();
      const cpuUsage = process.cpuUsage();
      
      // Check memory usage (alert if over 500MB)
      const memMB = Math.round(memUsage.heapUsed / 1024 / 1024);
      if (memMB > 500) {
        this.logger.logToFile(`RESOURCE_WARNING: High memory usage: ${memMB}MB`);
      }
      
      // Log resource stats periodically
      this.logger.logToFile(`RESOURCE_STATS: Memory=${memMB}MB, Uptime=${Math.round(process.uptime())}s`);
      
    }, 60000); // Every 60 seconds
    
    this.logger.logToFile('RESOURCE_MONITOR: Started resource monitoring');
  }
  
  private cleanup() {
    this.logger.logToFile('Cleanup initiated');
    
    // Stop resource monitoring
    if (this.resourceMonitor) {
      clearInterval(this.resourceMonitor);
      this.resourceMonitor = null;
    }
    
    if (this.contextEngine) {
      this.contextEngine.shutdown();
    }
    this.logger.cleanup();
  }

  async stop() {
    console.log('🛑 Stopping ClaudeCat Multi-Instance MCP Server...');
    this.logger.logToFile('Stop requested');
    await this.contextEngine.shutdown();
    this.logger.cleanup();
    console.log('✅ ClaudeCat Multi-Instance MCP Server stopped');
  }
}

// Main startup function
async function main() {
  const server = new ClaudeCatMultiInstanceServer();
  
  // Note: Signal handlers are already set up in MultiInstanceLogger constructor

  try {
    await server.start();
  } catch (error) {
    console.error('💥 Failed to start multi-instance server:', error);
    process.exit(1);
  }
}

// Run the server if this file is executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  // Add specific debugging for Claude Code MCP connection issues
  console.error(`[DEBUG] MCP server startup: PID=${process.pid}, PPID=${process.ppid}`);
  console.error(`[DEBUG] Node path: ${process.execPath}`);
  console.error(`[DEBUG] Working directory: ${process.cwd()}`);
  console.error(`[DEBUG] Args: ${JSON.stringify(process.argv)}`);
  console.error(`[DEBUG] Environment: CLAUDE_SESSION_ID=${process.env.CLAUDE_SESSION_ID}`);
  console.error(`[DEBUG] stdio: stdin.readable=${process.stdin.readable}, stdout.writable=${process.stdout.writable}`);
  
  main().catch((error) => {
    console.error('💥 Fatal error:', error);
    console.error(`[DEBUG] Error type: ${error.constructor.name}`);
    console.error(`[DEBUG] Error stack: ${error.stack}`);
    process.exit(1);
  });
}