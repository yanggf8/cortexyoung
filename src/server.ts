// Import MCP SDK types for reference but use simpler implementation
// import { Server } from '@modelcontextprotocol/sdk/server/index.js';
// import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import express, { Request, Response } from 'express';
import cors from 'cors';
import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import { SemanticSearchHandler, ContextualReadHandler, CodeIntelligenceHandler, RelationshipAnalysisHandler, TraceExecutionPathHandler, FindCodePatternsHandler, RealTimeStatusHandler } from './mcp-handlers';
import { IndexHealthChecker } from './index-health-checker';
import { HierarchicalStageTracker } from './hierarchical-stages';
import { EnhancedHierarchicalStageTracker } from './enhanced-hierarchical-stages';
import { conditionalLogger } from './utils/console-logger';
import { CORTEX_TOOLS } from './mcp-tools';
import { MMRConfigManager, createMMRConfigFromEnvironment } from './mmr-config-manager';
import { error } from './logging-utils';
import { cortexConfig } from './env-config';
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

export class CortexMCPServer {
  private indexer: CodebaseIndexer;
  private searcher: SemanticSearcher;
  private handlers: Map<string, any> = new Map();
  private httpServer: any;
  private logger: Logger;
  private stageTracker: HierarchicalStageTracker;

  constructor(
    indexer: CodebaseIndexer,
    searcher: SemanticSearcher,
    loggerOrFile?: Logger | string,
    stageTracker?: HierarchicalStageTracker
  ) {
    this.indexer = indexer;
    this.searcher = searcher;
    
    // Accept either a Logger instance or create new one from file path
    if (loggerOrFile && typeof loggerOrFile === 'object' && 'info' in loggerOrFile) {
      this.logger = loggerOrFile as Logger;
    } else {
      this.logger = new Logger(loggerOrFile as string);
    }
    
    this.stageTracker = stageTracker || new HierarchicalStageTracker(this.logger);
    this.setupHandlers();
    this.setupIPC();
    this.logger.info('CortexMCPServer initialized');
  }

  private setupHandlers(): void {
    this.handlers.set('semantic_search', new SemanticSearchHandler(this.searcher));
    this.handlers.set('contextual_read', new ContextualReadHandler(this.searcher));
    this.handlers.set('code_intelligence', new CodeIntelligenceHandler(this.searcher));
    this.handlers.set('relationship_analysis', new RelationshipAnalysisHandler(this.searcher));
    this.handlers.set('trace_execution_path', new TraceExecutionPathHandler(this.searcher));
    this.handlers.set('find_code_patterns', new FindCodePatternsHandler(this.searcher));
    this.handlers.set('real_time_status', new RealTimeStatusHandler(this.indexer));
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

// Main startup function
async function main() {
  // Parse command line arguments
  const args = process.argv.slice(2);
  const repoPath = args.find(arg => !arg.startsWith('--')) || process.cwd();
  const port = cortexConfig.port;
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
  
  // Create stage tracker with logger to prevent duplicate output
  // Use enhanced tracker if new logging is enabled, otherwise fallback to original
  const stageTracker = cortexConfig.enableNewLogging 
    ? new EnhancedHierarchicalStageTracker(logger)
    : new HierarchicalStageTracker(logger);
  
  // Startup metadata header
  const version = getVersion();
  const commit = getGitCommit(); 
  const nodeVersion = process.version;
  const platform = os.platform();
  const pid = process.pid;
  
  // Enhanced startup header with new logging
  if (cortexConfig.enableNewLogging) {
    conditionalLogger.ready(`Cortex MCP Server v${version} (${commit})`, {
      metadata: { 
        pid, 
        node: nodeVersion,
        platform,
        port 
      }
    });
    conditionalLogger.ok(`Repo: ${repoPath}`, {
      metadata: { 
        logFile: logFile || 'default'
      }
    });
    console.log(''); // Separator before stages
  } else {
    logger.info(`[Startup] Cortex MCP Server version=${version} commit=${commit} pid=${pid} node=${nodeVersion} platform=${platform} port=${port}`);
    logger.info(`[Startup] Repository path=${repoPath} logFile=${logFile || 'default'}`);
    logger.info('🎯 Cortex MCP Server Starting...');
  }
  
  try {
    // ==================== STAGE 1: Initialization & Pre-flight ====================
    stageTracker.startStage('stage_1');
    
    // 1.1 Server Initialization
    stageTracker.startSubstep('stage_1', '1.1', 'Logger setup, repository validation, MCP server components');
    logger.info('Initializing Cortex MCP Server', { repoPath, port });
    stageTracker.completeSubstep('stage_1', '1.1', 'Server components initialized');
    
    // 1.2 Cache & Storage Health Check
    stageTracker.startSubstep('stage_1', '1.2', 'Storage comparison, cache loading, health validation');
    logger.info('Starting repository indexing');
    const indexer = new CodebaseIndexer(repoPath);
    
    // Determine indexing mode
    let indexMode: 'full' | 'incremental' | 'reindex';
    if (forceReindex || cortexConfig.indexMode === 'reindex') {
      indexMode = 'reindex';
      if (cortexConfig.enableNewLogging) {
        conditionalLogger.warn('Force rebuild requested', { 
          metadata: { mode: 'reindex' },
          reason: 'User requested complete rebuild'
        });
      } else {
        logger.info('🔄 Force rebuild requested, using reindex mode');
      }
    } else if (forceFullMode) {
      indexMode = 'full';
      if (cortexConfig.enableNewLogging) {
        conditionalLogger.ok('Full indexing mode requested', { metadata: { mode: 'full' } });
      } else {
        logger.info('🔄 Full mode requested, using full indexing');
      }
    } else if (cortexConfig.indexMode) {
      indexMode = cortexConfig.indexMode as 'full' | 'incremental';
      if (cortexConfig.enableNewLogging) {
        conditionalLogger.ok('Explicit indexing mode', { metadata: { mode: indexMode } });
      } else {
        logger.info('Using explicit indexing mode', { mode: indexMode });
      }
    } else {
      indexMode = await getIntelligentIndexMode(indexer, logger);
    }
    stageTracker.completeSubstep('stage_1', '1.2', `Index healthy → Using ${indexMode} mode`);
    
    // 1.3 AI Model Loading
    stageTracker.startSubstep('stage_1', '1.3', 'BGE-small-en-v1.5 initialization and readiness');
    
    // Pre-initialize both embedder and storage coordinator to avoid duplication in Stage 2
    await Promise.all([
      (indexer as any).embedder.getModelInfo(),
      (indexer as any).storageCoordinator.initialize()
    ]);
    
    stageTracker.completeSubstep('stage_1', '1.3', 'BGE-small-en-v1.5 ready');
    
    stageTracker.completeStage('stage_1');
    
    // ==================== STAGE 2: Code Intelligence Indexing ====================
    stageTracker.startStage('stage_2');
    
    // 2.1 Repository Analysis
    stageTracker.startSubstep('stage_2', '2.1', 'File discovery, delta analysis, change categorization');
    
    // Index the repository
    const indexResponse = await indexer.indexRepository({
      repository_path: repoPath,
      mode: indexMode
    });
    
    // Determine what happened during indexing
    const deltaInfo = indexResponse.chunks_processed > 0 
      ? `+${indexResponse.chunks_processed} chunks processed` 
      : 'No changes detected, using cache';
    stageTracker.completeSubstep('stage_2', '2.1', deltaInfo);
    
    // 2.2 Embedding Generation (if chunks were processed)
    if (indexResponse.chunks_processed > 0) {
      stageTracker.startSubstep('stage_2', '2.2', 'Process pool setup, chunk processing, resource monitoring');
      stageTracker.completeSubstep('stage_2', '2.2', `${indexResponse.chunks_processed} chunks embedded`);
    } else {
      stageTracker.startSubstep('stage_2', '2.2', 'No new embeddings needed, using cached data');
      stageTracker.completeSubstep('stage_2', '2.2', 'Embeddings loaded from cache');
    }
    
    // 2.3 Relationship Analysis
    stageTracker.startSubstep('stage_2', '2.3', 'Dependency mapping, symbol extraction, graph building');
    
    // Initialize relationship engine (uses cache if available)
    const searcher = (indexer as any).searcher;
    
    // Build file map for relationship analysis (this is quick since files are already read)
    const files = new Map<string, string>();
    const gitScanner = (indexer as any).gitScanner;
    const allFiles = await gitScanner.scanRepository('full');
    
    for (const filePath of allFiles.files) {
      try {
        const content = await gitScanner.readFile(filePath);
        files.set(filePath, content);
      } catch (error) {
        logger.warn('Failed to read file for relationships', { filePath, error: error instanceof Error ? error.message : error });
      }
    }
    
    await searcher.initializeRelationshipEngine(files);
    stageTracker.completeSubstep('stage_2', '2.3', 'Relationship graph ready');
    
    // 2.4 Vector Storage Commit
    stageTracker.startSubstep('stage_2', '2.4', 'Database updates, storage persistence, synchronization');
    const storageInfo = `${indexResponse.chunks_processed} chunks ${indexResponse.chunks_processed > 0 ? 'processed' : 'loaded from cache'}`;
    stageTracker.completeSubstep('stage_2', '2.4', `Storage committed: ${storageInfo}`);
    
    stageTracker.completeStage('stage_2');
    
    logger.info('Repository indexing completed', { 
      chunksProcessed: indexResponse.chunks_processed,
      timeMs: indexResponse.time_taken_ms
    });
    
    // If in demo mode, exit after indexing is complete
    if (isDemoMode) {
      logger.info('🎯 Demo mode complete - indexing finished successfully');
      logger.info(`📊 Processed ${indexResponse.chunks_processed} chunks in ${indexResponse.time_taken_ms}ms`);
      logger.info('✅ Demo completed, exiting...');
      process.exit(0);
    }
    
    // ==================== STAGE 3: Server Activation ====================
    stageTracker.startStage('stage_3');
    
    // 3.1 MCP Server Startup
    stageTracker.startSubstep('stage_3', '3.1', 'HTTP transport, endpoint registration, service availability');
    
    // Create and start MCP server (pass logger instance to avoid double creation)
    const mcpServer = new CortexMCPServer(indexer, searcher, logger, stageTracker);
    
    // MCP server startup message handled by startHttp method
    await mcpServer.startHttp(port);
    
    stageTracker.completeSubstep('stage_3', '3.1', `HTTP server ready at http://localhost:${port}`);
    
    stageTracker.completeStage('stage_3');
    
    // Optional: Enable real-time file watching
    if (enableRealTime) {
      logger.info('🔄 Enabling real-time file watching...');
      try {
        await indexer.enableRealTimeUpdates();
        logger.info('✅ Real-time file watching enabled successfully');
        logger.info('📡 MCP tools will now reflect live codebase changes');
      } catch (error) {
        logger.warn('⚠️  Failed to enable real-time watching', { 
          error: error instanceof Error ? error.message : 'Unknown error' 
        });
        logger.info('📊 Server will continue with static indexing mode');
      }
    } else {
      logger.info('📊 Real-time watching disabled (use --no-watch flag or DISABLE_REAL_TIME=true to disable when needed)');
    }
    
    // Final startup summary
    stageTracker.logStartupSummary();
    
    // Handle graceful shutdown
    process.on('SIGINT', async () => {
      logger.info('Received SIGINT, initiating graceful shutdown');
      if (enableRealTime) {
        logger.info('🔄 Disabling real-time file watching...');
        await indexer.disableRealTimeUpdates();
      }
      await mcpServer.stop();
      process.exit(0);
    });
    
    process.on('SIGTERM', async () => {
      logger.info('Received SIGTERM, initiating graceful shutdown');
      if (enableRealTime) {
        logger.info('🔄 Disabling real-time file watching...');
        await indexer.disableRealTimeUpdates();
      }
      await mcpServer.stop();
      process.exit(0);
    });
    
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    logger.error('Failed to start Cortex MCP Server', { error: errorMessage });
    
    // Log failure in hierarchical tracker if available
    if (stageTracker.getCurrentStage()) {
      const currentStage = stageTracker.getCurrentStage()!;
      stageTracker.failStage(currentStage.id, errorMessage);
    }
    
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