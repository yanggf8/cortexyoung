// Import MCP SDK types for reference but use simpler implementation
// import { Server } from '@modelcontextprotocol/sdk/server/index.js';
// import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import express, { Request, Response } from 'express';
import cors from 'cors';
import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import { SemanticSearchHandler, ContextualReadHandler, CodeIntelligenceHandler, RelationshipAnalysisHandler, TraceExecutionPathHandler, FindCodePatternsHandler } from './mcp-handlers';
import { IndexHealthChecker } from './index-health-checker';
import { StartupStageTracker } from './startup-stages';
import { CORTEX_TOOLS } from './mcp-tools';
import * as fs from 'fs';
import * as path from 'path';

// Logger class for file and console logging
class Logger {
  private logFile: string;
  private logStream: fs.WriteStream;

  constructor(logFile?: string) {
    this.logFile = logFile || path.join(process.cwd(), 'logs', 'cortex-server.log');
    
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
    const dataStr = data ? ` | ${JSON.stringify(data)}` : '';
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
    if (process.env.DEBUG === 'true') {
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
  private stageTracker: StartupStageTracker;

  constructor(
    indexer: CodebaseIndexer,
    searcher: SemanticSearcher,
    logFile?: string,
    stageTracker?: StartupStageTracker
  ) {
    this.indexer = indexer;
    this.searcher = searcher;
    this.logger = new Logger(logFile);
    this.stageTracker = stageTracker || new StartupStageTracker();
    this.setupHandlers();
    this.logger.info('CortexMCPServer initialized');
  }

  private setupHandlers(): void {
    this.handlers.set('semantic_search', new SemanticSearchHandler(this.searcher));
    this.handlers.set('contextual_read', new ContextualReadHandler(this.searcher));
    this.handlers.set('code_intelligence', new CodeIntelligenceHandler(this.searcher));
    this.handlers.set('relationship_analysis', new RelationshipAnalysisHandler(this.searcher));
    this.handlers.set('trace_execution_path', new TraceExecutionPathHandler(this.searcher));
    this.handlers.set('find_code_patterns', new FindCodePatternsHandler(this.searcher));
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
      const progress = this.stageTracker.getProgress();
      const currentStage = this.stageTracker.getCurrentStage();
      const isReady = this.stageTracker.isReady();
      const stats = this.stageTracker.getStageStats();
      
      // Determine health status based on startup progress
      let healthStatus: 'healthy' | 'starting' | 'indexing' | 'error';
      
      if (isReady) {
        healthStatus = 'healthy';
      } else if (progress.overallStatus === 'failed') {
        healthStatus = 'error';
      } else if (progress.overallStatus === 'indexing') {
        healthStatus = 'indexing';
      } else {
        healthStatus = 'starting';
      }
      
      const response: any = {
        status: healthStatus,
        server: 'cortex-mcp-server',
        version: '2.1.0',
        ready: isReady,
        timestamp: Date.now()
      };
      
      // Add startup progress info if not fully ready
      if (!isReady) {
        response.startup = {
          stage: currentStage?.name || 'Unknown',
          progress: Math.round(progress.overallProgress),
          eta: progress.estimatedTimeRemaining ? Math.round(progress.estimatedTimeRemaining / 1000) : null,
          completed: stats.completed,
          total: stats.total,
          details: currentStage?.details
        };
      }
      
      // Add any failed stages
      if (stats.failed > 0) {
        const failedStages = progress.stages
          .filter(stage => stage.status === 'failed')
          .map(stage => ({ name: stage.name, error: stage.error }));
        response.errors = failedStages;
      }
      
      res.json(response);
    });

    // Startup progress endpoint
    app.get('/progress', (req: Request, res: Response) => {
      const progress = this.stageTracker.getProgress();
      res.json({
        ...progress,
        server: 'cortex-mcp-server',
        version: '2.1.0',
        timestamp: Date.now()
      });
    });

    // Current stage endpoint (for quick status checks)
    app.get('/status', (req: Request, res: Response) => {
      const progress = this.stageTracker.getProgress();
      const currentStage = this.stageTracker.getCurrentStage();
      const stats = this.stageTracker.getStageStats();
      
      res.json({
        status: progress.overallStatus,
        ready: this.stageTracker.isReady(),
        progress: progress.overallProgress,
        currentStage: currentStage?.name,
        stageProgress: currentStage?.progress,
        stages: `${stats.completed}/${stats.total}`,
        eta: progress.estimatedTimeRemaining ? Math.round(progress.estimatedTimeRemaining / 1000) : null,
        server: 'cortex-mcp-server',
        timestamp: Date.now()
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
              protocolVersion: '2025-01-07',
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
    // Access the vector store to check if index exists
    const vectorStore = (indexer as any).vectorStore;
    await vectorStore.initialize();
    
    const hasExistingIndex = await vectorStore.indexExists();
    
    if (!hasExistingIndex) {
      logger.info('🧠 Intelligent mode: No existing embeddings found, using full indexing');
      return 'full';
    }

    // Perform health check to determine if rebuild is needed
    logger.info('🩺 Running health check on existing index...');
    const healthChecker = new IndexHealthChecker(process.cwd(), vectorStore);
    const healthResult = await healthChecker.shouldRebuild();
    
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

// Main startup function
async function main() {
  const repoPath = process.argv[2] || process.cwd();
  const port = parseInt(process.env.PORT || '8765');
  const logFile = process.env.LOG_FILE;
  
  // Create stage tracker and main logger
  const stageTracker = new StartupStageTracker();
  const logger = new Logger(logFile);
  
  stageTracker.startStage('server_init', `Repository: ${repoPath}, Port: ${port}`);
  logger.info('Initializing Cortex MCP Server', { repoPath, port });
  
  try {
    stageTracker.completeStage('server_init');
    stageTracker.startStage('cache_check', 'Checking for existing embeddings cache');
    
    // Initialize indexer and searcher
    logger.info('Starting repository indexing');
    const indexer = new CodebaseIndexer(repoPath);
    
    // Use intelligent mode by default, or explicit mode if specified
    let indexMode: 'full' | 'incremental' | 'reindex';
    
    if (process.env.INDEX_MODE === 'reindex' || process.env.FORCE_REBUILD === 'true') {
      indexMode = 'reindex';
      logger.info('🔄 Force rebuild requested, using reindex mode');
      stageTracker.updateStageProgress('cache_check', 50, 'Force rebuild requested (reindex mode)');
    } else if (process.env.INDEX_MODE) {
      indexMode = process.env.INDEX_MODE as 'full' | 'incremental';
      logger.info('Using explicit indexing mode', { mode: indexMode });
      stageTracker.updateStageProgress('cache_check', 100, `Using explicit mode: ${indexMode}`);
    } else {
      indexMode = await getIntelligentIndexMode(indexer, logger);
      stageTracker.updateStageProgress('cache_check', 100, `Intelligent mode selected: ${indexMode}`);
    }
    
    stageTracker.completeStage('cache_check');
    stageTracker.startStage('file_scan', 'Scanning repository for code files');
    stageTracker.updateStageProgress('file_scan', 50, 'Analyzing repository structure');
    
    const indexResponse = await indexer.indexRepository({
      repository_path: repoPath,
      mode: indexMode
    });
    
    stageTracker.completeStage('vector_storage', `Indexed ${indexResponse.chunks_processed} chunks`);
    
    logger.info('Repository indexing completed', { 
      chunksProcessed: indexResponse.chunks_processed,
      timeMs: indexResponse.time_taken_ms
    });
    
    // Get searcher from indexer
    const searcher = (indexer as any).searcher; // Access the searcher instance
    
    stageTracker.startStage('mcp_ready', 'Starting MCP server');
    
    // Create and start MCP server
    const mcpServer = new CortexMCPServer(indexer, searcher, logFile, stageTracker);
    
    logger.info('Starting MCP server with HTTP transport', { port });
    await mcpServer.startHttp(port);
    
    stageTracker.completeStage('mcp_ready', 'MCP server ready to accept requests');
    
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
    logger.error('Failed to start Cortex MCP Server', { error: error instanceof Error ? error.message : error });
    process.exit(1);
  }
}

// Start server if this file is run directly
if (require.main === module) {
  main().catch(console.error);
}