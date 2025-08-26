// V3.0 Lightweight MCP Server with HTTP Client Architecture
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
import { error, warn } from './logging-utils';
import { cortexConfig } from './env-config';
import { CodebaseIndexer } from './indexer';
import { StartupStageTracker } from './startup-stages';
import { IndexHealthChecker } from './index-health-checker';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { execSync } from 'child_process';

// Helper function to get git commit hash
function getGitCommit(): string {
  try {
    return execSync('git rev-parse --short HEAD', { encoding: 'utf8' }).trim();
  } catch {
    return 'unknown';
  }
}

// Helper function to get version from package.json  
function getVersion(): string {
  try {
    const packagePath = path.join(__dirname, '../package.json');
    const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
    return packageJson.version || 'unknown';
  } catch {
    return 'unknown';
  }
}

// Logger class for file and console logging
class Logger {
  private logFile: string;
  private logStream: fs.WriteStream;

  constructor(logFile?: string) {
    this.logFile = logFile || cortexConfig.logFile || path.join(process.cwd(), 'logs', 'cortex-server.log');
    
    // Ensure log directory exists
    const logDir = path.dirname(this.logFile);
    if (!fs.existsSync(logDir)) {
      fs.mkdirSync(logDir, { recursive: true });
    }
    
    // Create write stream for log file
    this.logStream = fs.createWriteStream(this.logFile, { flags: 'a' });
    
    this.info('Logger initialized', { logFile: this.logFile });
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
    
    // Write to console
    console.log(formatted);
    
    // Write to file
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

export class LightweightCortexMCPServer {
  private embeddingClient: EmbeddingClient;
  private projectManager: ProjectManager;
  private handlers: Map<string, any> = new Map();
  private httpServer: any;
  private logger: Logger;
  private localCache: Map<string, { data: any; timestamp: number; ttl: number }> = new Map();
  private fallbackMode: boolean = false;
  public indexer?: CodebaseIndexer;
  public stageTracker: StartupStageTracker;

  constructor(
    projectPath: string,
    loggerOrFile?: Logger | string
  ) {
    // Create embedding client for centralized server communication
    this.embeddingClient = createLocalEmbeddingClient(
      `mcp-server-${process.pid}`,
      projectPath
    );
    
    this.projectManager = new ProjectManager();
    
    // Accept either a Logger instance or create new one from file path
    if (loggerOrFile && typeof loggerOrFile === 'object' && 'info' in loggerOrFile) {
      this.logger = loggerOrFile as Logger;
    } else {
      this.logger = new Logger(loggerOrFile as string);
    }
    
    // Initialize stage tracker
    this.stageTracker = new StartupStageTracker(this.logger);
    
    this.setupHandlers();
    this.setupIPC();
    this.initializeHealthChecking();
    this.logger.info('Lightweight CortexMCPServer initialized');
  }

  /**
   * Initialize health checking for centralized server connection
   */
  private initializeHealthChecking(): void {
    // Check centralized server health every 30 seconds
    setInterval(async () => {
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
        type: 'server_ready',
        pid: process.pid,
        timestamp: new Date().toISOString()
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
            status: 'healthy',
            uptime: process.uptime(),
            memoryUsage: process.memoryUsage(),
            pid: process.pid,
            startup: this.stageTracker.getProgressSummary()
          }
        });
        break;

      case 'startup_progress':
        this.sendIPCResponse({
          type: 'progress_response', 
          requestId,
          data: this.stageTracker.getProgressSummary()
        });
        break;

      case 'server_status':
        this.sendIPCResponse({
          type: 'status_response',
          requestId,
          data: {
            serverRunning: !!this.httpServer,
            port: cortexConfig.port,
            stages: this.stageTracker.getProgressData(),
            currentStage: this.stageTracker.getCurrentStage(),
            progress: this.stageTracker.getProgress()
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

  private getAvailableTools() {
    return Object.values(CORTEX_TOOLS);
  }

  async startHttp(port: number = 3001): Promise<void> {
    this.logger.info('Starting MCP Server with HTTP transport', { port });
    
    const app = express();
    app.use(express.json());
    app.use(cors({
      origin: '*',
      methods: ['GET', 'POST', 'OPTIONS'],
      allowedHeaders: ['Content-Type', 'Authorization'],
      credentials: false
    }));

    // Enhanced health check endpoint with startup stage info
    app.get('/health', (req: Request, res: Response) => {
      res.json(this.stageTracker.getHealthData());
    });

    // Startup progress endpoint
    app.get('/progress', (req: Request, res: Response) => {
      res.json(this.stageTracker.getProgressData());
    });

    // Current stage endpoint (for quick status checks)
    app.get('/status', (req: Request, res: Response) => {
      res.json(this.stageTracker.getStatusData());
    });

    // Enhanced metrics endpoint for embedding providers
    app.get('/metrics/embeddings', async (req: Request, res: Response) => {
      try {
        const metrics = await this.getEmbeddingMetrics();
        res.json(metrics);
      } catch (error) {
        this.logger.error('Error getting embedding metrics', { error: error instanceof Error ? error.message : String(error) });
        res.status(500).json({ 
          error: 'Failed to get embedding metrics',
          message: error instanceof Error ? error.message : String(error)
        });
      }
    });

    // MCP endpoint health check (for Claude Code health checking)
    app.get('/mcp', (req: Request, res: Response) => {
      res.json({
        status: 'healthy',
        server: 'cortex-mcp-server',
        version: '2.1.6',
        ready: true,
        timestamp: Date.now(),
        protocolVersion: '2024-11-05',
        capabilities: {
          tools: {}
        }
      });
    });

    // MCP endpoint for JSON-RPC communication
    app.post('/mcp', async (req: Request, res: Response) => {
      try {
        this.logger.info('Received MCP request', { 
          method: req.body?.method,
          id: req.body?.id 
        });

        // Handle JSON-RPC requests manually for now
        // In a full implementation, this would use StreamableHTTPServerTransport
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
                name: 'cortex-mcp-server',
                version: '2.1.0'
              }
            },
            id
          };
        } else if (method === 'tools/list') {
          // Return available tools directly
          response = {
            jsonrpc: '2.0',
            result: {
              tools: this.getAvailableTools()
            },
            id
          };
        } else if (method === 'tools/call') {
          try {
            // Handle tool call directly
            const { name, arguments: args } = params;
            let toolResponse;
            
            // Dynamic tool handling - get handler from registry
            const handler = this.handlers.get(name);
            if (!handler) {
              throw new Error(`Unknown tool: ${name}`);
            }

            const result = await handler.handle(args);
            toolResponse = {
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
          // Handle notifications/initialized - this is a notification, not a request
          // Notifications don't require a response, but we'll return a simple acknowledgment
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
        this.logger.info('MCP HTTP Server started', { 
          port,
          endpoints: {
            health: `GET http://localhost:${port}/health`,
            mcp: `POST http://localhost:${port}/mcp`
          }
        });
        resolve();
      });
    });
  }

  private async getEmbeddingMetrics() {
    const metrics = {
      timestamp: new Date().toISOString(),
      providers: {} as any,
      system: {
        uptime: process.uptime(),
        totalChunks: 0,
        cacheHitRate: 0,
        storageSync: {
          lastLocalSync: null as string | null,
          lastGlobalSync: null as string | null
        }
      },
      performance: {
        queryResponseTime: {
          p50: 0,
          p95: 0,
          p99: 0
        },
        throughput: 0
      }
    };

    try {
      // Try to get metrics from the indexer's unified embedder if available
      const unifiedEmbedder = (this.indexer as any).unifiedEmbedder;
      
      if (unifiedEmbedder) {
        // Get basic provider metrics
        const health = await unifiedEmbedder.getHealth();
        const providerMetrics = await unifiedEmbedder.getMetrics();
        
        metrics.providers[unifiedEmbedder.providerId] = {
          health: health.status,
          details: health.details,
          uptime: health.uptime,
          errorRate: health.errorRate,
          requestCount: providerMetrics.requestCount,
          avgDuration: providerMetrics.avgDuration,
          totalEmbeddings: providerMetrics.totalEmbeddings,
          lastSuccess: new Date(providerMetrics.lastSuccess).toISOString()
        };

        // Add provider-specific metrics
        if (unifiedEmbedder.providerId.includes('cloudflare')) {
          // CloudflareAI specific metrics
          try {
            const circuitBreakerStats = unifiedEmbedder.getCircuitBreakerStats();
            const rateLimiterStats = unifiedEmbedder.getRateLimiterStats();
            
            metrics.providers[unifiedEmbedder.providerId] = {
              ...metrics.providers[unifiedEmbedder.providerId],
              circuitBreakerState: circuitBreakerStats.state,
              rateLimitRemaining: rateLimiterStats.available,
              rateLimitCapacity: rateLimiterStats.capacity,
              failures: circuitBreakerStats.failures,
              isHealthy: circuitBreakerStats.isHealthy
            };
          } catch (error) {
            this.logger.warn('Could not get Cloudflare-specific metrics', { error });
          }
        } else if (unifiedEmbedder.providerId.includes('process-pool')) {
          // ProcessPool specific metrics
          try {
            const poolStatus = unifiedEmbedder.getPoolStatus();
            const cacheStats = unifiedEmbedder.getCacheStats();
            
            metrics.providers[unifiedEmbedder.providerId] = {
              ...metrics.providers[unifiedEmbedder.providerId],
              activeProcesses: poolStatus.activeProcesses,
              resourceUtilization: {
                cpuConstrained: poolStatus.cpuConstrained,
                memoryConstrained: poolStatus.memoryConstrained
              },
              cacheHitRate: cacheStats.hitRate,
              cacheSize: cacheStats.size,
              memoryUsage: cacheStats.memoryMB
            };
            
            metrics.system.cacheHitRate = cacheStats.hitRate;
          } catch (error) {
            this.logger.warn('Could not get ProcessPool-specific metrics', { error });
          }
        }
      }

      // Get vector store information
      try {
        const vectorStore = (this.indexer as any).vectorStore;
        if (vectorStore) {
          metrics.system.totalChunks = vectorStore.getChunkCount ? vectorStore.getChunkCount() : 0;
          
          // Get storage sync information
          const storageInfo = await vectorStore.getStorageInfo();
          if (storageInfo) {
            metrics.system.storageSync = {
              lastLocalSync: storageInfo.local.lastModified?.toISOString() || null,
              lastGlobalSync: storageInfo.global.lastModified?.toISOString() || null
            };
          }
        }
      } catch (error) {
        this.logger.warn('Could not get vector store metrics', { error });
      }

      // Add startup stage information  
      const currentStage = this.stageTracker.getCurrentStage();
      const progress = this.stageTracker.getProgress();
      
      (metrics.system as any).startupStage = currentStage?.name || 'unknown';
      (metrics.system as any).startupProgress = progress.overallProgress;
      (metrics.system as any).isReady = this.stageTracker.isReady();

    } catch (error) {
      this.logger.error('Error collecting embedding metrics', { error });
      throw error;
    }

    return metrics;
  }

  async stop(): Promise<void> {
    this.logger.info('Stopping MCP Server');
    if (this.httpServer) {
      await new Promise<void>((resolve) => {
        this.httpServer.close(() => {
          this.logger.info('MCP HTTP Server stopped successfully');
          resolve();
        });
      });
    }
    this.logger.close();
  }
}

// Helper function for intelligent indexing mode detection with health checks
async function getIntelligentIndexMode(indexer: CodebaseIndexer, logger: Logger): Promise<'full' | 'incremental'> {
  try {
    // Access the vector store to check if valid index exists
    const vectorStore = (indexer as any).vectorStore;
    await vectorStore.initialize();
    
    const hasExistingIndex = await vectorStore.hasValidIndex();
    
    if (!hasExistingIndex) {
      logger.info('🧠 Intelligent mode: No existing embeddings found, using full indexing');
      return 'full';
    }

    // Perform quick health check to determine if rebuild is needed
    logger.info('🩺 Running health check on existing index...');
    
    const quickHealth = await vectorStore.quickHealthCheck();
    
    // Only run expensive full health check if quick check fails
    let healthResult;
    if (!quickHealth.healthy) {
      logger.info(`⚠️  Quick health check failed: ${quickHealth.reason}, running detailed analysis...`);
      const healthChecker = new IndexHealthChecker(process.cwd(), vectorStore);
      healthResult = await healthChecker.shouldRebuild();
    } else {
      // Create a simple healthy result for quick path
      healthResult = {
        shouldRebuild: false,
        reason: 'Index is healthy (quick check)',
        mode: 'incremental' as const
      };
    }
    
    if (healthResult.shouldRebuild) {
      logger.info(`🧠 Intelligent mode: ${healthResult.reason}, using ${healthResult.mode} indexing`);
      return healthResult.mode;
    } else {
      logger.info('🧠 Intelligent mode: Index is healthy, using incremental indexing');
      return 'incremental';
    }
  } catch (error) {
    logger.warn('Error in intelligent mode detection, defaulting to full indexing', { 
      error: error instanceof Error ? error.message : error 
    });
    return 'full';
  }
}

// Check if another Cortex server is already running
const checkForRunningServer = async (port: number): Promise<boolean> => {
  try {
    const { execSync } = require('child_process');
    const result = execSync(`lsof -ti :${port} 2>/dev/null || echo ""`, { encoding: 'utf8' });
    return result.trim() !== '';
  } catch (error) {
    return false; // Assume no server if we can't check
  }
};

// Count existing Cortex server processes
const countCortexProcesses = async (): Promise<number> => {
  try {
    const { execSync } = require('child_process');
    const result = execSync('ps aux | grep -c "cortex.*server\\|server.*js" | grep -v grep || echo "1"', { encoding: 'utf8' });
    return Math.max(1, parseInt(result.trim()) || 1); // At least count this process
  } catch (error) {
    return 1;
  }
};

// Check and clean up orphaned embedding processes at startup
const cleanupOrphanedProcesses = async (): Promise<number> => {
  try {
    const { execSync } = require('child_process');
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

// Main startup function for V3.0 Lightweight MCP Server
async function main() {
  // Parse command line arguments
  const args = process.argv.slice(2);
  const repoPath = args.find(arg => !arg.startsWith('--')) || process.cwd();
  const port = cortexConfig.port || 3001;  // Use different default port for lightweight server
  const logFile = cortexConfig.logFile;
  
  // Check for server reentrance and cleanup orphaned processes
  const isPortOccupied = await checkForRunningServer(port);
  const cortexProcessCount = await countCortexProcesses();
  
  if (isPortOccupied && cortexProcessCount > 1) {
    console.log('');
    console.log('🚨 SERVER ALREADY RUNNING: Another Cortex server is using this port');
    console.log('');
    console.log(`📡 Port ${port} is already in use`);
    console.log(`🔄 Found ${cortexProcessCount} Cortex server processes running`);
    console.log('');
    console.log('🛠️  SOLUTIONS:');
    console.log('   1. Use the existing server (no action needed)');
    console.log(`   2. Stop existing server: pkill -f "server.*js" or npm run shutdown`);
    console.log(`   3. Use different port: PORT=8766 npm run start`);
    console.log(`   4. Check server status: curl http://localhost:${port}/mcp/health`);
    console.log('');
    console.log('ℹ️  Cortex is designed to run one instance per repository');
    console.log('   Multiple instances can cause resource conflicts and data corruption');
    console.log('');
    process.exit(0); // Exit gracefully, not an error
  }
  
  // Clean up orphaned processes from previous crashed sessions
  await cleanupOrphanedProcesses();
  
  // Check for demo mode
  const isDemoMode = args.includes('--demo');
  const forceReindex = args.includes('--reindex') || args.includes('--force-rebuild') || cortexConfig.forceRebuild;
  const forceFullMode = args.includes('--full');
  const enableRealTime = !args.includes('--no-watch') && !cortexConfig.disableRealTime;
  
  // Create main logger first
  const logger = new Logger(logFile);
  
  // Startup metadata header
  const version = getVersion();
  const commit = getGitCommit(); 
  const nodeVersion = process.version;
  const platform = os.platform();
  const pid = process.pid;
  
  // Enhanced startup header
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
    console.log(''); // Separator before stages
  } else {
    logger.info(`[Startup] Cortex Lightweight MCP Server v3.0 version=${version} commit=${commit} pid=${pid} node=${nodeVersion} platform=${platform} port=${port}`);
    logger.info(`[Startup] Project path=${repoPath} logFile=${logFile || 'default'}`);
    logger.info('🎯 Lightweight Cortex MCP Server Starting...');
  }
  
  try {
    // V3.0 Lightweight Architecture - No heavy indexing or embedding processes
    
    logger.info('🚀 V3.0 Lightweight MCP Server - Connecting to centralized embedding server');
    
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

// Handle resource-related errors with helpful messages
const handleResourceError = (error: Error) => {
  console.log('');
  
  if (error.message.includes('System memory too high')) {
    console.log('🚨 RESOURCE ERROR: System Memory Too High');
    console.log('');
    console.log('💾 The system is using too much memory to safely start Cortex');
    console.log('⚠️  Starting anyway could cause system crashes or data corruption');
  } else if (error.message.includes('Too many embedding processes')) {
    console.log('🚨 PROCESS ERROR: Orphaned Processes Detected');
    console.log('');
    console.log('🔄 Too many embedding processes from previous Cortex sessions');
    console.log('⚠️  These orphaned processes consume memory and cause conflicts');
  } else if (error.message.includes('EADDRINUSE') || error.message.includes('port')) {
    console.log('🚨 PORT ERROR: Address Already In Use');
    console.log('');
    console.log('📡 Another application is already using the server port');
    console.log('⚠️  Multiple Cortex instances can cause data corruption');
  } else {
    console.log('🚨 STARTUP ERROR: Failed to start Cortex server');
    console.log('');
    console.log('⚠️  An unexpected error occurred during server initialization');
  }
  
  console.log('');
  console.log('🆘 GENERAL TROUBLESHOOTING:');
  console.log('   • Check system resources: free -h && ps aux | grep node');
  console.log('   • Clean up processes: npm run shutdown');
  console.log('   • Try cloud mode: npm run start:cloudflare');
  console.log('   • Restart system if issues persist');
  console.log('   • Report issues: https://github.com/anthropics/claude-code/issues');
  console.log('');
  console.log(`📋 Error Details: ${error.message}`);
  console.log('');
};

// Start server if this file is run directly
if (require.main === module) {
  main().catch((error) => {
    handleResourceError(error);
    process.exit(1);
  });
}