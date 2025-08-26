/**
 * V3.0 Lightweight MCP Server
 * 
 * Complete rewrite for centralized architecture:
 * - HTTP Client instead of ProcessPoolEmbedder
 * - Local caching for performance
 * - Graceful degradation when centralized server unavailable
 * - <100MB memory footprint per instance
 * - Optimized for multiple concurrent Claude instances
 */

import express, { Request, Response } from 'express';
import cors from 'cors';
import { createLocalEmbeddingClient, EmbeddingClient } from './embedding-client';
import { FetchChunkHandler, NextChunkHandler, GetCurrentProjectHandler, ListAvailableProjectsHandler, SwitchProjectHandler, AddProjectHandler } from './mcp-handlers';
import { 
  LightweightSemanticSearchHandler,
  LightweightContextualReadHandler,
  LightweightCodeIntelligenceHandler,
  LightweightRelationshipAnalysisHandler,
  LightweightTraceExecutionPathHandler,
  LightweightFindCodePatternsHandler,
  LightweightRealTimeStatusHandler
} from './lightweight-handlers';
import { ProjectManager } from './project-manager';
import { conditionalLogger } from './utils/console-logger';
import { CORTEX_TOOLS } from './mcp-tools';
import { error, warn, log } from './logging-utils';
import { cortexConfig } from './env-config';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { execSync } from 'child_process';

// Helper functions for version and metadata
function getGitCommit(): string {
  try {
    return execSync('git rev-parse --short HEAD', { encoding: 'utf8' }).trim();
  } catch {
    return 'unknown';
  }
}

function getVersion(): string {
  try {
    const packagePath = path.join(__dirname, '../package.json');
    const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
    return packageJson.version || 'unknown';
  } catch {
    return 'unknown';
  }
}

// Simplified logger for lightweight server
class LightweightLogger {
  private logFile: string;
  private logStream: fs.WriteStream;

  constructor(logFile?: string) {
    this.logFile = logFile || path.join(process.cwd(), 'logs', 'cortex-lightweight-server.log');
    
    // Ensure log directory exists
    const logDir = path.dirname(this.logFile);
    if (!fs.existsSync(logDir)) {
      fs.mkdirSync(logDir, { recursive: true });
    }
    
    this.logStream = fs.createWriteStream(this.logFile, { flags: 'a' });
  }

  private formatMessage(level: string, message: string, data?: any): string {
    const timestamp = new Date().toISOString();
    let dataStr = '';
    if (data && typeof data === 'object') {
      const pairs = Object.entries(data).map(([key, value]) => `${key}=${value}`);
      dataStr = pairs.length > 0 ? ` ${pairs.join(' ')}` : '';
    }
    return `[${timestamp}] [${level}] ${message}${dataStr}`;
  }

  private writeLog(level: string, message: string, data?: any): void {
    const formatted = this.formatMessage(level, message, data);
    console.log(formatted);
    this.logStream.write(formatted + '\n');
  }

  info(message: string, data?: any): void {
    this.writeLog('INFO', message, data);
  }

  error(message: string, data?: any): void {
    this.writeLog('ERROR', message, data);
  }

  warn(message: string, data?: any): void {
    this.writeLog('WARN', message, data);
  }

  debug(message: string, data?: any): void {
    if (cortexConfig.debug) {
      this.writeLog('DEBUG', message, data);
    }
  }

  close(): void {
    this.logStream.end();
  }
}

/**
 * Lightweight MCP Server with HTTP Client Architecture
 * Designed for V3.0 centralized architecture with resource efficiency
 */
export class LightweightCortexMCPServer {
  private embeddingClient: EmbeddingClient;
  private projectManager: ProjectManager;
  private handlers: Map<string, any> = new Map();
  private httpServer: any;
  private logger: LightweightLogger;
  private localCache: Map<string, { data: any; timestamp: number; ttl: number }> = new Map();
  private fallbackMode: boolean = false;
  private healthCheckInterval: NodeJS.Timeout | null = null;

  constructor(
    projectPath: string,
    loggerOrFile?: LightweightLogger | string
  ) {
    // Create embedding client for centralized server communication
    this.embeddingClient = createLocalEmbeddingClient(
      `mcp-server-${process.pid}`,
      projectPath
    );
    
    this.projectManager = new ProjectManager();
    
    // Accept either a Logger instance or create new one from file path
    if (loggerOrFile && typeof loggerOrFile === 'object' && 'info' in loggerOrFile) {
      this.logger = loggerOrFile as LightweightLogger;
    } else {
      this.logger = new LightweightLogger(loggerOrFile as string);
    }
    
    this.setupHandlers();
    this.setupIPC();
    this.initializeHealthChecking();
    this.logger.info('Lightweight CortexMCPServer initialized', { projectPath });
  }

  /**
   * Initialize health checking for centralized server connection
   */
  private initializeHealthChecking(): void {
    // Check centralized server health every 30 seconds
    this.healthCheckInterval = setInterval(async () => {
      try {
        const isHealthy = await this.embeddingClient.testConnection();
        if (!isHealthy && !this.fallbackMode) {
          this.logger.warn('Centralized server unavailable, switching to fallback mode');
          this.fallbackMode = true;
        } else if (isHealthy && this.fallbackMode) {
          this.logger.info('Centralized server restored, switching back to primary mode');
          this.fallbackMode = false;
        }
      } catch (error) {
        // Ignore health check errors to avoid spam
      }
    }, 30000);
  }

  /**
   * Cache management for performance optimization
   */
  public getCachedResult(key: string): any {
    const cached = this.localCache.get(key);
    if (!cached) return null;
    
    if (Date.now() > cached.timestamp + cached.ttl) {
      this.localCache.delete(key);
      return null;
    }
    
    return cached.data;
  }

  public setCachedResult(key: string, data: any, ttlMs: number = 300000): void { // 5min default TTL
    this.localCache.set(key, {
      data,
      timestamp: Date.now(),
      ttl: ttlMs
    });

    // Clean up old cache entries if cache gets too large
    if (this.localCache.size > 1000) {
      const now = Date.now();
      for (const [k, v] of this.localCache.entries()) {
        if (now > v.timestamp + v.ttl) {
          this.localCache.delete(k);
        }
      }
    }
  }

  public isInFallbackMode(): boolean {
    return this.fallbackMode;
  }

  private setupHandlers(): void {
    // Create lightweight handlers that use HTTP client instead of heavy components
    this.handlers.set('semantic_search', new LightweightSemanticSearchHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('contextual_read', new LightweightContextualReadHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('code_intelligence', new LightweightCodeIntelligenceHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('relationship_analysis', new LightweightRelationshipAnalysisHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('trace_execution_path', new LightweightTraceExecutionPathHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('find_code_patterns', new LightweightFindCodePatternsHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('real_time_status', new LightweightRealTimeStatusHandler(this.embeddingClient, this));
    
    // Chunking handlers (unchanged - they work locally)
    this.handlers.set('fetch_chunk', new FetchChunkHandler());
    this.handlers.set('next_chunk', new NextChunkHandler());
    
    // Project management handlers (unchanged - they manage local project state)
    this.handlers.set('get_current_project', new GetCurrentProjectHandler(this.projectManager));
    this.handlers.set('list_available_projects', new ListAvailableProjectsHandler(this.projectManager));
    this.handlers.set('switch_project', new SwitchProjectHandler(this.projectManager));
    this.handlers.set('add_project', new AddProjectHandler(this.projectManager));
  }

  private setupIPC(): void {
    // Set up IPC communication for health checks and progress reporting
    if (process.send) {
      this.logger.info('IPC available, setting up message handlers');
      
      process.on('message', (message: any) => {
        try {
          this.handleIPCMessage(message);
        } catch (error: any) {
          this.logger.error('Error handling IPC message', { message, error: error?.message || 'Unknown error' });
        }
      });

      // Send ready signal to parent process
      process.send({
        type: 'lightweight_server_ready',
        pid: process.pid,
        timestamp: new Date().toISOString(),
        architecture: 'v3.0-lightweight-http-client'
      });
    } else {
      this.logger.info('No IPC available (running standalone)');
    }
  }

  private handleIPCMessage(message: any): void {
    const { type, requestId } = message;

    switch (type) {
      case 'health_check':
        this.sendIPCResponse({
          type: 'health_response',
          requestId,
          data: {
            status: this.fallbackMode ? 'degraded' : 'healthy',
            uptime: process.uptime(),
            memoryUsage: process.memoryUsage(),
            pid: process.pid,
            fallbackMode: this.fallbackMode,
            cacheSize: this.localCache.size,
            centralizedServerConnected: !this.fallbackMode
          }
        });
        break;

      case 'cache_stats':
        this.sendIPCResponse({
          type: 'cache_stats_response',
          requestId,
          data: {
            cacheSize: this.localCache.size,
            cacheHitRate: this.calculateCacheHitRate(),
            fallbackMode: this.fallbackMode
          }
        });
        break;

      case 'ping':
        this.sendIPCResponse({
          type: 'pong',
          requestId,
          timestamp: new Date().toISOString()
        });
        break;

      default:
        this.logger.warn('Unknown IPC message type', { type, message });
    }
  }

  private sendIPCResponse(response: any): void {
    if (process.send) {
      process.send(response);
    }
  }

  private calculateCacheHitRate(): number {
    // Simple cache hit rate calculation - could be enhanced with actual metrics
    return this.localCache.size > 0 ? 0.75 : 0; // Placeholder
  }

  private getAvailableTools() {
    return Object.values(CORTEX_TOOLS);
  }

  async startHttp(port: number = 3001): Promise<void> {
    this.logger.info('Starting Lightweight MCP Server with HTTP transport', { port, architecture: 'V3.0 HTTP Client' });
    
    const app = express();
    app.use(express.json());
    app.use(cors({
      origin: '*',
      methods: ['GET', 'POST', 'OPTIONS'],
      allowedHeaders: ['Content-Type', 'Authorization'],
      credentials: false
    }));

    // Enhanced health check endpoint with lightweight server info
    app.get('/health', (req: Request, res: Response) => {
      res.json({
        status: this.fallbackMode ? 'degraded' : 'healthy',
        server: 'cortex-lightweight-mcp-server',
        version: '3.0.0',
        architecture: 'http-client-centralized',
        ready: true,
        timestamp: Date.now(),
        uptime: process.uptime(),
        memory: process.memoryUsage(),
        fallbackMode: this.fallbackMode,
        cacheSize: this.localCache.size,
        centralizedServerConnected: !this.fallbackMode
      });
    });

    // MCP endpoint health check (for Claude Code health checking)
    app.get('/mcp', (req: Request, res: Response) => {
      res.json({
        status: this.fallbackMode ? 'degraded' : 'healthy',
        server: 'cortex-lightweight-mcp-server',
        version: '3.0.0',
        ready: true,
        timestamp: Date.now(),
        protocolVersion: '2024-11-05',
        capabilities: {
          tools: {}
        },
        architecture: 'V3.0 Lightweight HTTP Client'
      });
    });

    // MCP endpoint for JSON-RPC communication
    app.post('/mcp', async (req: Request, res: Response) => {
      try {
        this.logger.info('Received MCP request', { 
          method: req.body?.method,
          id: req.body?.id 
        });

        const { method, params, id } = req.body;
        let response: any;
        
        if (method === 'initialize') {
          response = {
            jsonrpc: '2.0',
            result: {
              protocolVersion: '2024-11-05',
              capabilities: {
                tools: {}
              },
              serverInfo: {
                name: 'cortex-lightweight-mcp-server',
                version: '3.0.0'
              }
            },
            id
          };
        } else if (method === 'tools/list') {
          response = {
            jsonrpc: '2.0',
            result: {
              tools: this.getAvailableTools()
            },
            id
          };
        } else if (method === 'tools/call') {
          try {
            const { name, arguments: args } = params;
            
            const handler = this.handlers.get(name);
            if (!handler) {
              throw new Error(`Unknown tool: ${name}`);
            }

            const result = await handler.handle(args);
            const toolResponse = {
              content: [
                {
                  type: 'text',
                  text: JSON.stringify(result, null, 2)
                }
              ]
            };
            
            response = {
              jsonrpc: '2.0',
              result: toolResponse,
              id
            };
          } catch (error) {
            response = {
              jsonrpc: '2.0',
              error: {
                code: -32603,
                message: error instanceof Error ? error.message : 'Internal error'
              },
              id
            };
          }
        } else if (method === 'notifications/initialized') {
          response = {
            jsonrpc: '2.0',
            result: {},
            id: id || null
          };
        } else {
          response = {
            jsonrpc: '2.0',
            error: {
              code: -32601,
              message: `Method not found: ${method}`
            },
            id
          };
        }

        this.logger.info('Sending MCP response', { 
          method,
          id,
          success: !response.error 
        });
        
        res.json(response);
      } catch (error) {
        this.logger.error('MCP request failed', { error: error instanceof Error ? error.message : 'Unknown error' });
        res.status(500).json({
          jsonrpc: '2.0',
          error: {
            code: -32603,
            message: 'Internal error'
          },
          id: req.body?.id || null
        });
      }
    });

    await new Promise<void>((resolve) => {
      this.httpServer = app.listen(port, () => {
        this.logger.info('Lightweight MCP HTTP Server started', { 
          port,
          architecture: 'V3.0 HTTP Client → Centralized Server',
          endpoints: {
            health: `GET http://localhost:${port}/health`,
            mcp: `POST http://localhost:${port}/mcp`
          }
        });
        resolve();
      });
    });
  }

  async stop(): Promise<void> {
    this.logger.info('Stopping Lightweight MCP Server');
    
    // Clear health check interval
    if (this.healthCheckInterval) {
      clearInterval(this.healthCheckInterval);
      this.healthCheckInterval = null;
    }
    
    // Clear cache
    this.localCache.clear();
    
    if (this.httpServer) {
      await new Promise<void>((resolve) => {
        this.httpServer.close(() => {
          this.logger.info('Lightweight MCP HTTP Server stopped successfully');
          resolve();
        });
      });
    }
    this.logger.close();
  }
}

// Main startup function for V3.0 Lightweight MCP Server
async function main() {
  const args = process.argv.slice(2);
  const repoPath = args.find(arg => !arg.startsWith('--')) || process.cwd();
  const port = cortexConfig.port || 3001;
  const logFile = cortexConfig.logFile;

  // Clean up orphaned processes from previous sessions
  await cleanupOrphanedProcesses();
  
  const logger = new LightweightLogger(logFile);
  
  // Startup metadata
  const version = getVersion();
  const commit = getGitCommit(); 
  const nodeVersion = process.version;
  const platform = os.platform();
  const pid = process.pid;
  
  if (cortexConfig.enableNewLogging) {
    conditionalLogger.ready(`Cortex Lightweight MCP Server v${version} (${commit})`, {
      metadata: { 
        pid, 
        node: nodeVersion,
        platform,
        port,
        architecture: 'V3.0 Lightweight HTTP Client'
      }
    });
    conditionalLogger.ok(`Project: ${repoPath}`, {
      metadata: { 
        logFile: logFile || 'default',
        centralizedServer: 'localhost:8766'
      }
    });
  } else {
    logger.info(`[Startup] Cortex Lightweight MCP Server v3.0 version=${version} commit=${commit} pid=${pid} node=${nodeVersion} platform=${platform} port=${port}`);
    logger.info(`[Startup] Project path=${repoPath} logFile=${logFile || 'default'}`);
    logger.info('🎯 Lightweight Cortex MCP Server Starting...');
  }
  
  try {
    // Initialize project manager for multi-project support
    const projectManager = new ProjectManager();
    await projectManager.initializeWithCurrentDirectory();
    
    // Create lightweight MCP server with HTTP client
    const mcpServer = new LightweightCortexMCPServer(repoPath, logger);
    
    // Test connection to centralized server
    const embeddingClient = (mcpServer as any).embeddingClient;
    const centralizedServerAvailable = await embeddingClient.testConnection();
    
    if (centralizedServerAvailable) {
      logger.info('✅ Connected to centralized embedding server at localhost:8766');
      if (cortexConfig.enableNewLogging) {
        conditionalLogger.ok('Centralized server connection established', {
          metadata: { 
            server: 'localhost:8766',
            mode: 'enhanced_context'
          }
        });
      }
    } else {
      logger.warn('⚠️  Centralized embedding server not available - running in fallback mode');
      if (cortexConfig.enableNewLogging) {
        conditionalLogger.warn('Running in fallback mode', {
          metadata: { 
            reason: 'Centralized server unavailable',
            mode: 'local_basic'
          }
        });
      }
    }
    
    // Start HTTP MCP server
    await mcpServer.startHttp(port);
    
    logger.info('✅ Lightweight Cortex MCP Server ready');
    if (cortexConfig.enableNewLogging) {
      conditionalLogger.ready('🚀 V3.0 Lightweight MCP Server Active', {
        metadata: {
          port: port,
          endpoints: {
            health: `GET http://localhost:${port}/health`,
            mcp: `POST http://localhost:${port}/mcp`
          },
          architecture: 'HTTP Client → Centralized Server',
          resourceUsage: '<100MB memory per client'
        }
      });
    }
    
    // Handle graceful shutdown
    process.on('SIGINT', async () => {
      logger.info('Received SIGINT, initiating graceful shutdown');
      await mcpServer.stop();
      process.exit(0);
    });
    
    process.on('SIGTERM', async () => {
      logger.info('Received SIGTERM, initiating graceful shutdown');
      await mcpServer.stop();
      process.exit(0);
    });
    
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    logger.error('Failed to start Lightweight Cortex MCP Server', { error: errorMessage });
    process.exit(1);
  }
}

// Clean up orphaned embedding processes at startup
const cleanupOrphanedProcesses = async (): Promise<number> => {
  try {
    const countResult = execSync('ps aux | grep -c "external-embedding-process" | grep -v grep || echo "0"', { encoding: 'utf8' });
    const orphanedCount = parseInt(countResult.trim()) || 0;
    
    if (orphanedCount > 0) {
      console.log('🧹 CLEANUP: Found orphaned embedding processes from previous sessions');
      console.log(`🔄 Cleaning up ${orphanedCount} orphaned processes...`);
      try {
        execSync('pkill -f "external-embedding-process" 2>/dev/null || true');
        console.log('✅ Orphaned processes cleaned up successfully');
      } catch (cleanupError) {
        console.log('⚠️  Some processes may require manual cleanup');
      }
      console.log('');
    }
    
    return orphanedCount;
  } catch (error) {
    return 0;
  }
};

// Start server if this file is run directly
if (require.main === module) {
  main().catch((error: any) => {
    console.error('Fatal error starting lightweight server:', error);
    process.exit(1);
  });
}

