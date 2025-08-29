import express, { Request, Response } from 'express';
import cors from 'cors';
import { spawn, ChildProcess } from 'child_process';
import fastq from 'fastq';
import * as path from 'path';
import * as os from 'os';
import * as fs from 'fs';
import { CodeChunk, IEmbedder, EmbedOptions, EmbeddingResult, EmbeddingMetadata, PerformanceStats, ProviderHealth, ProviderMetrics } from './types';
import { log, warn, error } from './logging-utils';
import { MemoryMappedCache } from './memory-mapped-cache';
import { ContextEnhancementLayer } from './context-enhancement-layer';
import { CentralizedHandlers, createCentralizedHandlers } from './centralized-handlers';
import { ProcessPoolEmbedder } from './process-pool-embedder';

interface ServerStatus {
  processPool: {
    activeProcesses: number;
    maxProcesses: number;
    queueSize: number;
    activeBatches: Array<{
      id: string;
      progress: number;
      chunkCount: number;
      eta: string;
    }>;
  };
  system: {
    memoryUsage: number;
    memoryMB: number;
    totalMemoryGB: number;
    cpuUsage: number;
    uptime: string;
  };
  activeClients: Array<{
    clientId: string;
    project: string;
    lastActivity: string;
  }>;
  clientCount: number;
  stats: {
    totalRequests: number;
    requestsToday: number;
    averageResponseTime: number;
    errorRate: number;
  };
  autoShutdown: {
    enabled: boolean;
    noClientsTimeout: number;
    idleTimeout: number;
    lastRequestTime: string;
    shutdownScheduled: boolean;
  };
}

interface EmbeddingRequest {
  chunks: CodeChunk[];
  options: EmbedOptions;
  clientId?: string;
  projectPath?: string;
}

interface SemanticSearchRequest {
  query: string;
  options: any;
  clientId?: string;
  projectPath?: string;
}

interface ClientInfo {
  project: string;
  lastActivity: Date;
  registeredAt: Date;
  pid?: number;
}

interface AutoShutdownConfig {
  enabled: boolean;
  noClientsTimeoutMs: number;  // Shutdown after no clients for this duration
  idleTimeoutMs: number;       // Shutdown after no requests for this duration
}

/**
 * Centralized HTTP Embedding Server for Cortex V3.0
 * 
 * Consolidates ProcessPool management across multiple Claude Code instances
 * Provides enhanced semantic search with project context awareness
 */
export class CortexEmbeddingServer implements IEmbedder {
  public readonly providerId = "cortex.centralized.bge-small-v1.5";
  public readonly modelId = "@cortex/bge-small-en-v1.5";
  public readonly dimensions = 384;
  public readonly maxBatchSize = 800;
  public readonly normalization = "l2" as const;

  private app: express.Application;
  private processPool?: ProcessPoolEmbedder;
  private contextEnhancer?: ContextEnhancementLayer;
  private centralizedHandlers?: CentralizedHandlers;
  private server: any;
  private startTime: number;
  private activeClients: Map<string, ClientInfo> = new Map();
  private requestCount = 0;
  private requestsToday = 0;
  private responseTimes: number[] = [];
  private errors = 0;
  private isInitialized = false;
  private lastRequestTime: Date = new Date();
  private shutdownTimer: NodeJS.Timeout | null = null;
  private autoShutdownConfig: AutoShutdownConfig = {
    enabled: process.env.CORTEX_AUTO_SHUTDOWN !== 'false',
    noClientsTimeoutMs: parseInt(process.env.CORTEX_NO_CLIENTS_TIMEOUT || '300000'), // 5 minutes
    idleTimeoutMs: parseInt(process.env.CORTEX_IDLE_TIMEOUT || '1800000') // 30 minutes
  };

  constructor(private port: number = 3001) {
    this.app = express();
    this.startTime = Date.now();
    this.setupMiddleware();
    this.setupRoutes();
  }

  private setupMiddleware(): void {
    this.app.use(cors());
    this.app.use(express.json({ limit: '10mb' }));
    
    // Request logging and metrics
    this.app.use((req, res, next) => {
      const startTime = Date.now();
      const clientId = req.headers['x-client-id'] as string || 'unknown';
      const projectPath = req.headers['x-project-path'] as string || 'unknown';
      
      // Track active clients
      if (clientId !== 'unknown') {
        const now = new Date();
        const existing = this.activeClients.get(clientId);
        this.activeClients.set(clientId, {
          project: projectPath,
          lastActivity: now,
          registeredAt: existing?.registeredAt || now,
          pid: existing?.pid
        });
        this.lastRequestTime = now;
      }
      
      res.on('finish', () => {
        const responseTime = Date.now() - startTime;
        this.responseTimes.push(responseTime);
        this.requestCount++;
        
        // Keep only last 100 response times for average calculation
        if (this.responseTimes.length > 100) {
          this.responseTimes.shift();
        }
        
        log('HTTP Request completed');
      });
      
      next();
    });
  }

  private setupRoutes(): void {
    // Health check endpoint
    this.app.get('/health', (req: Request, res: Response) => {
      res.json({ 
        status: 'healthy',
        uptime: Date.now() - this.startTime,
        processPool: this.processPool ? 'ready' : 'initializing'
      });
    });

    // Client registration endpoint
    this.app.post('/register-client', (req: Request, res: Response) => {
      try {
        const { clientId, project, pid } = req.body;
        
        if (!clientId) {
          return res.status(400).json({ error: 'clientId is required' });
        }
        
        const now = new Date();
        this.activeClients.set(clientId, {
          project: project || 'unknown',
          lastActivity: now,
          registeredAt: now,
          pid: pid
        });
        
        log(`Client registered: ${clientId} (PID: ${pid || 'unknown'}, Project: ${project || 'unknown'}) - Total clients: ${this.activeClients.size}`);
        
        // Cancel any pending shutdown since we have a new client
        this.cancelShutdown();
        
        res.json({ success: true, clientId, registeredAt: now.toISOString(), totalClients: this.activeClients.size });
        
      } catch (err) {
        error('Client registration failed', { error: err });
        res.status(500).json({ error: 'Registration failed' });
      }
    });

    // Client deregistration endpoint
    this.app.post('/deregister-client', (req: Request, res: Response) => {
      try {
        const { clientId } = req.body;
        
        if (!clientId) {
          return res.status(400).json({ error: 'clientId is required' });
        }
        
        const wasRegistered = this.activeClients.has(clientId);
        if (wasRegistered) {
          this.activeClients.delete(clientId);
          log(`Client deregistered: ${clientId} - Remaining clients: ${this.activeClients.size}`);
          
          // Check if we should shutdown after client removal
          this.scheduleShutdownCheck();
        }
        
        res.json({ success: true, clientId, wasRegistered, totalClients: this.activeClients.size });
        
      } catch (err) {
        error('Client deregistration failed', { error: err });
        res.status(500).json({ error: 'Deregistration failed' });
      }
    });


    // Core embedding generation endpoint
    this.app.post('/embed', async (req: Request, res: Response) => {
      try {
        const { texts, options } = req.body;
        
        if (!Array.isArray(texts)) {
          return res.status(400).json({ error: 'texts must be an array' });
        }
        
        const result = await this.centralizedHandlers!.handleEmbedBatch(texts, options);
        res.json(result);
        
      } catch (err) {
        this.errors++;
        error('Embedding generation failed', { error: err });
        res.status(500).json({ error: 'Embedding generation failed' });
      }
    });

    // Basic semantic search endpoint (without context enhancement)
    this.app.post('/semantic-search', async (req: Request, res: Response) => {
      try {
        const { query, options }: SemanticSearchRequest = req.body;
        
        if (!query) {
          return res.status(400).json({ error: 'Query is required' });
        }

        // TODO: Implement semantic search using ProcessPool
        // For now, return placeholder response
        res.json({
          results: `Semantic search results for: ${query}`,
          metadata: {
            query,
            resultCount: 0,
            processingTime: 0
          }
        });
        
      } catch (err) {
        this.errors++;
        error('Semantic search failed', { error: err });
        res.status(500).json({ error: 'Semantic search failed' });
      }
    });

    // Enhanced semantic search with context enhancement
    this.app.post('/semantic-search-enhanced', async (req: Request, res: Response) => {
      try {
        const { query, options = {}, projectPath, clientId } = req.body;
        
        if (!query) {
          return res.status(400).json({ error: 'Query is required' });
        }

        const result = await this.centralizedHandlers!.handleSemanticSearch({
          query,
          ...options,
          projectPath,
          clientId
        });
        
        res.json(result);
        
      } catch (err) {
        this.errors++;
        error('Enhanced semantic search failed', { error: err });
        res.status(500).json({ error: 'Enhanced semantic search failed' });
      }
    });

    // Code intelligence endpoint
    this.app.post('/code-intelligence', async (req: Request, res: Response) => {
      try {
        const { task, options = {}, projectPath, clientId } = req.body;
        
        if (!task) {
          return res.status(400).json({ error: 'Task is required' });
        }

        const result = await this.centralizedHandlers!.handleCodeIntelligence({
          task,
          ...options,
          projectPath,
          clientId
        });
        
        res.json(result);
        
      } catch (err) {
        this.errors++;
        error('Code intelligence failed', { error: err });
        res.status(500).json({ error: 'Code intelligence failed' });
      }
    });

    // Relationship analysis endpoint
    this.app.post('/relationship-analysis', async (req: Request, res: Response) => {
      try {
        const { analysisType, options = {}, projectPath, clientId } = req.body;
        
        if (!analysisType) {
          return res.status(400).json({ error: 'Analysis type is required' });
        }

        const result = await this.centralizedHandlers!.handleRelationshipAnalysis({
          analysisType,
          ...options,
          projectPath,
          clientId
        });
        
        res.json(result);
        
      } catch (err) {
        this.errors++;
        error('Relationship analysis failed', { error: err });
        res.status(500).json({ error: 'Relationship analysis failed' });
      }
    });

    // Trace execution path endpoint
    this.app.post('/trace-execution-path', async (req: Request, res: Response) => {
      try {
        const { entryPoint, options = {}, projectPath, clientId } = req.body;
        
        if (!entryPoint) {
          return res.status(400).json({ error: 'Entry point is required' });
        }

        const result = await this.centralizedHandlers!.handleTraceExecutionPath({
          entryPoint,
          ...options,
          projectPath,
          clientId
        });
        
        res.json(result);
        
      } catch (err) {
        this.errors++;
        error('Trace execution path failed', { error: err });
        res.status(500).json({ error: 'Trace execution path failed' });
      }
    });

    // Find code patterns endpoint
    this.app.post('/find-code-patterns', async (req: Request, res: Response) => {
      try {
        const { pattern, patternType, options = {}, projectPath, clientId } = req.body;
        
        if (!pattern || !patternType) {
          return res.status(400).json({ error: 'Pattern and pattern type are required' });
        }

        const result = await this.centralizedHandlers!.handleFindCodePatterns({
          pattern,
          patternType,
          ...options,
          projectPath,
          clientId
        });
        
        res.json(result);
        
      } catch (err) {
        this.errors++;
        error('Find code patterns failed', { error: err });
        res.status(500).json({ error: 'Find code patterns failed' });
      }
    });

    // Embedding health monitoring endpoint
    this.app.post('/embedding-health', (req: Request, res: Response) => {
      try {
        const healthData = this.getEmbeddingHealth();
        res.json(healthData);
      } catch (err) {
        this.errors++;
        error('Embedding health check failed', { error: err });
        res.status(500).json({ error: 'Embedding health check failed' });
      }
    });

    // Server status endpoint for monitoring
    this.app.get('/status', (req: Request, res: Response) => {
      const status = this.getServerStatus();
      res.json(status);
    });

    // Server dashboard endpoint
    this.app.get('/dashboard', (req: Request, res: Response) => {
      const status = this.getServerStatus();
      const dashboard = this.renderDashboard(status);
      
      res.setHeader('Content-Type', 'text/plain');
      res.send(dashboard);
    });
  }

  private getServerStatus(): ServerStatus {
    const memUsage = process.memoryUsage();
    const totalMemory = os.totalmem();
    const freeMemory = os.freemem();
    const usedMemoryPercent = ((totalMemory - freeMemory) / totalMemory) * 100;

    // Clean up stale clients (inactive for >5 minutes)
    const fiveMinutesAgo = new Date(Date.now() - 5 * 60 * 1000);
    for (const [clientId, client] of this.activeClients.entries()) {
      if (client.lastActivity < fiveMinutesAgo) {
        this.activeClients.delete(clientId);
      }
    }

    return {
      processPool: {
        activeProcesses: 1, // Mock for V3.0 demo
        maxProcesses: 8,
        queueSize: 0,
        activeBatches: [] // TODO: Get from ProcessPool
      },
      system: {
        memoryUsage: Math.round(usedMemoryPercent),
        memoryMB: Math.round(memUsage.rss / 1024 / 1024),
        totalMemoryGB: Math.round(totalMemory / 1024 / 1024 / 1024),
        cpuUsage: 0, // TODO: Implement CPU monitoring
        uptime: this.formatUptime(Date.now() - this.startTime)
      },
      activeClients: Array.from(this.activeClients.entries()).map(([clientId, client]) => ({
        clientId,
        project: client.project,
        lastActivity: client.lastActivity.toLocaleTimeString()
      })),
      clientCount: this.activeClients.size,
      stats: {
        totalRequests: this.requestCount,
        requestsToday: this.requestsToday,
        averageResponseTime: this.responseTimes.length > 0 
          ? Math.round(this.responseTimes.reduce((a, b) => a + b, 0) / this.responseTimes.length)
          : 0,
        errorRate: this.requestCount > 0 ? (this.errors / this.requestCount) * 100 : 0
      },
      autoShutdown: {
        enabled: this.autoShutdownConfig.enabled,
        noClientsTimeout: this.autoShutdownConfig.noClientsTimeoutMs,
        idleTimeout: this.autoShutdownConfig.idleTimeoutMs,
        lastRequestTime: this.lastRequestTime.toISOString(),
        shutdownScheduled: Boolean(this.shutdownTimer)
      }
    };
  }

  private renderDashboard(status: ServerStatus): string {
    return `
🎯 Cortex V3.0 Centralized Embedding Server
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📊 RESOURCE STATUS
├─ Process Pool: ${status.processPool.activeProcesses}/${status.processPool.maxProcesses} processes
├─ Memory Usage: ${status.system.memoryUsage}% (${status.system.memoryMB}MB / ${status.system.totalMemoryGB}GB)
├─ Queue Size: ${status.processPool.queueSize} pending tasks
└─ Uptime: ${status.system.uptime}

👥 ACTIVE CLAUDE CODE CLIENTS (${status.activeClients.length})
${status.activeClients.length > 0 
  ? status.activeClients.map(c => `├─ ${c.clientId}: ${c.project} (${c.lastActivity})`).join('\n')
  : '└─ No active clients'}

📈 PERFORMANCE METRICS
├─ Total Requests: ${status.stats.totalRequests}
├─ Avg Response Time: ${status.stats.averageResponseTime}ms
├─ Error Rate: ${status.stats.errorRate.toFixed(2)}%
└─ Success Rate: ${(100 - status.stats.errorRate).toFixed(2)}%

🔄 ACTIVE BATCHES
${status.processPool.activeBatches.length > 0
  ? status.processPool.activeBatches.map(b => `├─ ${b.id}: ${b.progress}% (${b.chunkCount} chunks)`).join('\n')
  : '└─ No active embedding batches'}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Status: ${status.processPool.activeProcesses > 0 ? '✅ OPERATIONAL' : '⚠️  INITIALIZING'}
`;
  }

  private formatUptime(ms: number): string {
    const seconds = Math.floor(ms / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);

    if (days > 0) return `${days}d ${hours % 24}h ${minutes % 60}m`;
    if (hours > 0) return `${hours}h ${minutes % 60}m ${seconds % 60}s`;
    if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
    return `${seconds}s`;
  }

  private getEmbeddingHealth(): any {
    const logsDir = path.join(os.homedir(), '.cortex', 'multi-instance-logs');
    const activeSessionsFile = path.join(logsDir, 'active-sessions.json');
    
    try {
      // Read active sessions if file exists
      let activeSessions: any = {};
      let totalSessions = 0;
      let staleSessions = 0;
      
      if (fs.existsSync(activeSessionsFile)) {
        const sessionsData = fs.readFileSync(activeSessionsFile, 'utf8');
        activeSessions = JSON.parse(sessionsData);
        totalSessions = Object.keys(activeSessions).length;
        
        // Count stale sessions (older than 5 minutes)
        const fiveMinutesAgo = Date.now() - 5 * 60 * 1000;
        staleSessions = Object.values(activeSessions).filter((session: any) => 
          session.lastActivity < fiveMinutesAgo
        ).length;
      }

      // Get log files count
      let logFiles: string[] = [];
      if (fs.existsSync(logsDir)) {
        logFiles = fs.readdirSync(logsDir)
          .filter(f => f.startsWith('cortex-') && f.endsWith('.log'))
          .sort((a, b) => {
            try {
              const statA = fs.statSync(path.join(logsDir, a));
              const statB = fs.statSync(path.join(logsDir, b));
              return statB.mtime.getTime() - statA.mtime.getTime(); // Most recent first
            } catch {
              return 0;
            }
          });
      }

      // Prepare session summaries (only active ones)
      const sessionSummaries = Object.entries(activeSessions)
        .filter(([_, session]: [string, any]) => 
          session.lastActivity >= Date.now() - 5 * 60 * 1000 // Only sessions active in last 5 minutes
        )
        .map(([sessionId, session]: [string, any]) => ({
          sessionId,
          claudeSession: session.claudeSession,
          pid: session.pid,
          parentPid: session.parentPid,
          uptime: this.formatUptime(Date.now() - session.startTime),
          status: session.status,
          lastActivity: new Date(session.lastActivity).toISOString()
        }));

      return {
        status: 'healthy',
        multiInstanceEnabled: true,
        logsDirectory: logsDir,
        summary: {
          totalActiveSessions: sessionSummaries.length,
          totalHistoricalSessions: totalSessions,
          staleSessions,
          logFilesCount: logFiles.length
        },
        activeSessions: sessionSummaries,
        recentLogFiles: logFiles.slice(0, 5), // Last 5 log files
        diagnostics: {
          centralizedServer: {
            port: this.port,
            uptime: this.formatUptime(Date.now() - this.startTime),
            processPoolReady: Boolean(this.processPool)
          },
          environment: {
            nodeVersion: process.version,
            platform: os.platform(),
            architecture: os.arch(),
            totalMemory: Math.round(os.totalmem() / 1024 / 1024 / 1024) + 'GB'
          }
        },
        timestamp: new Date().toISOString()
      };

    } catch (error) {
      // Return error state if unable to read multi-instance data
      return {
        status: 'error',
        multiInstanceEnabled: false,
        error: error instanceof Error ? error.message : String(error),
        logsDirectory: logsDir,
        diagnostics: {
          centralizedServer: {
            port: this.port,
            uptime: this.formatUptime(Date.now() - this.startTime),
            processPoolReady: Boolean(this.processPool)
          }
        },
        timestamp: new Date().toISOString()
      };
    }
  }

  async start(): Promise<void> {
    try {
      await this.initialize();
      
      // Start HTTP server
      this.server = this.app.listen(this.port, () => {
        log(`[CortexEmbeddingServer] Server started on port ${this.port}`);
        log(`[CortexEmbeddingServer] Dashboard: http://localhost:${this.port}/dashboard`);
        log(`[CortexEmbeddingServer] Status: http://localhost:${this.port}/status`);
      });

      // Note: Signal handlers are managed by startup script for proper cleanup coordination

    } catch (err) {
      error('Failed to start Cortex Embedding Server', { error: err });
      process.exit(1);
    }
  }

  /**
   * Initialize all components
   */
  private async initialize(): Promise<void> {
    if (this.isInitialized) {
      return;
    }

    log('[CortexEmbeddingServer] Initializing centralized embedding server...');
    
    // Initialize ProcessPool
    log('[CortexEmbeddingServer] Initializing ProcessPool...');
    this.processPool = new ProcessPoolEmbedder();
    await this.processPool.initialize();
    
    // Initialize Context Enhancement Layer
    log('[CortexEmbeddingServer] Initializing Context Enhancement Layer...');
    this.contextEnhancer = new ContextEnhancementLayer();
    await this.contextEnhancer.initialize();
    
    // Initialize Centralized Handlers
    log('[CortexEmbeddingServer] Initializing Centralized Handlers...');
    this.centralizedHandlers = createCentralizedHandlers({
      processPool: this.processPool,
      contextEnhancer: this.contextEnhancer
    });
    
    this.isInitialized = true;
    log('[CortexEmbeddingServer] Initialization complete');
    
    // Start shutdown monitoring if enabled
    if (this.autoShutdownConfig.enabled) {
      log(`[CortexEmbeddingServer] Auto-shutdown enabled: No clients timeout=${this.autoShutdownConfig.noClientsTimeoutMs/1000}s, Idle timeout=${this.autoShutdownConfig.idleTimeoutMs/1000}s`);
    }
  }

  /**
   * Cancel any pending shutdown
   */
  private cancelShutdown(): void {
    if (this.shutdownTimer) {
      clearTimeout(this.shutdownTimer);
      this.shutdownTimer = null;
      log('[CortexEmbeddingServer] Shutdown cancelled - clients are active');
    }
  }

  /**
   * Schedule a shutdown check after client disconnect
   */
  private scheduleShutdownCheck(): void {
    if (!this.autoShutdownConfig.enabled) {
      return;
    }

    // Cancel any existing timer
    this.cancelShutdown();

    // If no clients remain, start shutdown countdown
    if (this.activeClients.size === 0) {
      log(`[CortexEmbeddingServer] No clients remaining, scheduling shutdown in ${this.autoShutdownConfig.noClientsTimeoutMs/1000}s`);
      
      this.shutdownTimer = setTimeout(async () => {
        if (this.activeClients.size === 0) {
          log('[CortexEmbeddingServer] Auto-shutdown triggered: No active clients');
          await this.gracefulShutdown();
        } else {
          log('[CortexEmbeddingServer] Auto-shutdown cancelled: New clients connected');
        }
      }, this.autoShutdownConfig.noClientsTimeoutMs);
    }
    
    // Also check for idle timeout (no requests)
    const timeSinceLastRequest = Date.now() - this.lastRequestTime.getTime();
    if (timeSinceLastRequest > this.autoShutdownConfig.idleTimeoutMs) {
      log('[CortexEmbeddingServer] Auto-shutdown triggered: Idle timeout exceeded');
      setTimeout(() => this.gracefulShutdown(), 1000);
    }
  }

  /**
   * Perform graceful shutdown
   */
  private async gracefulShutdown(): Promise<void> {
    log('[CortexEmbeddingServer] Initiating graceful shutdown...');
    
    try {
      await this.shutdown();
      process.exit(0);
    } catch (err) {
      error('[CortexEmbeddingServer] Error during graceful shutdown:', err);
      process.exit(1);
    }
  }

  // IEmbedder interface implementation
  async embedBatch(texts: string[], options?: EmbedOptions): Promise<EmbeddingResult> {
    if (!this.processPool) {
      throw new Error('ProcessPool not initialized');
    }
    return this.processPool.embedBatch(texts, options);
  }

  async getHealth(): Promise<ProviderHealth> {
    if (!this.processPool) {
      return {
        status: 'unhealthy',
        details: 'ProcessPool not initialized',
        lastCheck: Date.now()
      };
    }
    return this.processPool.getHealth();
  }

  async getMetrics(): Promise<ProviderMetrics> {
    if (!this.processPool) {
      throw new Error('ProcessPool not initialized');
    }
    return this.processPool.getMetrics();
  }

  async shutdown(): Promise<void> {
    log('[CortexEmbeddingServer] Shutting down gracefully...');
    
    try {
      // Close HTTP server
      if (this.server) {
        await new Promise<void>((resolve) => {
          this.server.close(() => {
            log('[CortexEmbeddingServer] HTTP server closed');
            resolve();
          });
        });
      }
      
      // Shutdown ProcessPool
      if (this.processPool) {
        await this.processPool.shutdown();
        log('[CortexEmbeddingServer] ProcessPool shutdown complete');
      }
      
      log('[CortexEmbeddingServer] Graceful shutdown complete');
      
    } catch (err) {
      error('[CortexEmbeddingServer] Error during shutdown:', err);
      throw err;
    }
  }
}

// Export for use as a module
export default CortexEmbeddingServer;

// Main execution
if (require.main === module) {
  const port = parseInt(process.env.CORTEX_EMBEDDING_SERVER_PORT || '8766');
  const server = new CortexEmbeddingServer(port);
  
  server.start().then(() => {
    log('[CortexEmbeddingServer] Server started successfully');
  }).catch((error) => {
    error('[CortexEmbeddingServer] Failed to start server:', error);
    process.exit(1);
  });
}