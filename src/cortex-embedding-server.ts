import express, { Request, Response } from 'express';
import cors from 'cors';
import { ProcessPoolEmbedder } from './process-pool-embedder';
import { ContextEnhancer } from './context-enhancer';
import { CodeChunk, EmbedOptions } from './types';
import { log, warn, error } from './logging-utils';
import { conditionalLogger } from './utils/console-logger';
import * as os from 'os';

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
  stats: {
    totalRequests: number;
    requestsToday: number;
    averageResponseTime: number;
    errorRate: number;
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

/**
 * Centralized HTTP Embedding Server for Cortex V3.0
 * 
 * Consolidates ProcessPool management across multiple Claude Code instances
 * Provides enhanced semantic search with project context awareness
 */
export class CortexEmbeddingServer {
  private app: express.Application;
  private processPool?: ProcessPoolEmbedder;
  private contextEnhancer?: ContextEnhancer;
  private server: any;
  private startTime: number;
  private activeClients: Map<string, { project: string; lastActivity: Date }> = new Map();
  private requestCount = 0;
  private requestsToday = 0;
  private responseTimes: number[] = [];
  private errors = 0;

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
        this.activeClients.set(clientId, {
          project: projectPath,
          lastActivity: new Date()
        });
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

    // Core embedding generation endpoint
    this.app.post('/embed', async (req: Request, res: Response) => {
      try {
        const { chunks, options, clientId, projectPath }: EmbeddingRequest = req.body;
        
        if (!chunks || !Array.isArray(chunks)) {
          return res.status(400).json({ error: 'Invalid chunks provided' });
        }

        // TODO: Implement actual embedding generation
        const result = {
          embeddings: chunks.map(() => Array.from({ length: 384 }, () => Math.random())),
          metadata: { processed: chunks.length },
          stats: { duration: 100, memoryUsed: 1024 }
        };
        
        res.json({
          embeddings: result.embeddings,
          metadata: result.metadata,
          stats: result.stats
        });
        
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
        const { query, options, projectPath }: SemanticSearchRequest = req.body;
        
        if (!query) {
          return res.status(400).json({ error: 'Query is required' });
        }

        if (!projectPath) {
          return res.status(400).json({ error: 'Project path is required for context enhancement' });
        }

        const startTime = Date.now();

        // TODO: Integrate with actual semantic search from ProcessPool
        // For now, simulate semantic search results
        const mockSemanticResults = `// Semantic search results for: ${query}
// Found relevant code chunks:
function authenticate(req, res, next) {
  const token = req.headers.authorization;
  // JWT validation logic here
  next();
}

class UserService {
  async validateUser(credentials) {
    // User validation implementation
  }
}`;

        // Apply context enhancement  
        const enhancement = await this.contextEnhancer!.enhanceSemanticResults(
          mockSemanticResults,
          query,
          projectPath,
          { maxTokens: 150 }
        );

        const processingTime = Date.now() - startTime;

        res.json({
          results: enhancement.results,
          contextEnhanced: enhancement.stats.enhanced,
          enhancementStats: enhancement.stats,
          metadata: {
            query,
            projectPath,
            resultCount: 2, // Mock result count
            processingTime,
            tokensAdded: enhancement.stats.tokensAdded
          }
        });
        
      } catch (err) {
        this.errors++;
        error('Enhanced semantic search failed', { error: err });
        res.status(500).json({ error: 'Enhanced semantic search failed' });
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
      stats: {
        totalRequests: this.requestCount,
        requestsToday: this.requestsToday,
        averageResponseTime: this.responseTimes.length > 0 
          ? Math.round(this.responseTimes.reduce((a, b) => a + b, 0) / this.responseTimes.length)
          : 0,
        errorRate: this.requestCount > 0 ? (this.errors / this.requestCount) * 100 : 0
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

  async start(): Promise<void> {
    try {
      // Initialize ProcessPool and ContextEnhancer
      log('Initializing ProcessPool for centralized embedding...');
      // TODO: Initialize actual ProcessPool
      // this.processPool = new ProcessPoolEmbedder();
      
      log('Initializing Context Enhancement Layer...');
      this.contextEnhancer = new ContextEnhancer();
      
      // Start HTTP server
      this.server = this.app.listen(this.port, () => {
        log(`Cortex V3.0 Embedding Server started on port ${this.port}`);
      });

      // Graceful shutdown handling
      process.on('SIGINT', this.shutdown.bind(this));
      process.on('SIGTERM', this.shutdown.bind(this));

    } catch (err) {
      error('Failed to start Cortex Embedding Server', { error: err });
      process.exit(1);
    }
  }

  private async shutdown(): Promise<void> {
    log('Shutting down Cortex Embedding Server...');
    
    if (this.server) {
      this.server.close();
    }
    
    // TODO: Cleanup ProcessPool when implemented
    
    log('Cortex Embedding Server shutdown complete');
    process.exit(0);
  }
}

// Main execution
if (require.main === module) {
  const port = parseInt(process.env.CORTEX_EMBEDDING_SERVER_PORT || '3001');
  const server = new CortexEmbeddingServer(port);
  server.start();
}