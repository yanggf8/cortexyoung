// Import MCP SDK types
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js';

// Import lightweight components for V3.0 architecture
import { createLocalEmbeddingClient, EmbeddingClient } from './embedding-client';
import { ProjectManager } from './project-manager';
import { CORTEX_TOOLS } from './mcp-tools';

// Import lightweight handlers and standard handlers
import { 
  FetchChunkHandler,
  NextChunkHandler,
  GetCurrentProjectHandler,
  ListAvailableProjectsHandler,
  SwitchProjectHandler,
  AddProjectHandler
} from './mcp-handlers';

import { 
  LightweightSemanticSearchHandler,
  LightweightContextualReadHandler,
  LightweightCodeIntelligenceHandler,
  LightweightRelationshipAnalysisHandler,
  LightweightTraceExecutionPathHandler,
  LightweightFindCodePatternsHandler,
  LightweightRealTimeStatusHandler
} from './lightweight-handlers';

// Import utilities
import { conditionalLogger } from './utils/console-logger';
import { cortexConfig } from './env-config';
import * as os from 'os';
import { execSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

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

class LightweightStdioCortexMCPServer {
  private server: Server;
  private embeddingClient: EmbeddingClient;
  private projectManager: ProjectManager;
  private handlers: Map<string, any> = new Map();
  private localCache: Map<string, { data: any; timestamp: number; ttl: number }> = new Map();
  private fallbackMode: boolean = false;
  private healthCheckInterval: NodeJS.Timeout | null = null;

  constructor(
    projectPath: string,
    projectManager: ProjectManager
  ) {
    // Create MCP server
    this.server = new Server(
      {
        name: 'cortex-lightweight-stdio-server',
        version: '3.0.0',
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );
    
    // Create embedding client for centralized server communication
    this.embeddingClient = createLocalEmbeddingClient(
      `stdio-server-${process.pid}`,
      projectPath
    );
    
    this.projectManager = projectManager;
    
    this.setupHandlers();
    this.setupEventHandlers();
    this.initializeHealthChecking();
  }

  /**
   * Initialize health checking for centralized server connection
   */
  private initializeHealthChecking(): void {
    this.healthCheckInterval = setInterval(async () => {
      try {
        const isHealthy = await this.embeddingClient.testConnection();
        if (!isHealthy && !this.fallbackMode) {
          this.fallbackMode = true;
        } else if (isHealthy && this.fallbackMode) {
          this.fallbackMode = false;
        }
      } catch (error) {
        // Ignore health check errors
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

  public setCachedResult(key: string, data: any, ttlMs: number = 300000): void {
    this.localCache.set(key, {
      data,
      timestamp: Date.now(),
      ttl: ttlMs
    });

    // Clean up old cache entries
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
    // Lightweight semantic analysis handlers using HTTP client
    this.handlers.set('semantic_search', new LightweightSemanticSearchHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('contextual_read', new LightweightContextualReadHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('code_intelligence', new LightweightCodeIntelligenceHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('relationship_analysis', new LightweightRelationshipAnalysisHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('trace_execution_path', new LightweightTraceExecutionPathHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('find_code_patterns', new LightweightFindCodePatternsHandler(this.embeddingClient, this.projectManager, this));
    this.handlers.set('real_time_status', new LightweightRealTimeStatusHandler(this.embeddingClient, this));
    
    // Chunking handlers (unchanged)
    this.handlers.set('fetch_chunk', new FetchChunkHandler());
    this.handlers.set('next_chunk', new NextChunkHandler());
    
    // Project management handlers (unchanged)
    this.handlers.set('get_current_project', new GetCurrentProjectHandler(this.projectManager));
    this.handlers.set('list_available_projects', new ListAvailableProjectsHandler(this.projectManager));
    this.handlers.set('switch_project', new SwitchProjectHandler(this.projectManager));
    this.handlers.set('add_project', new AddProjectHandler(this.projectManager));
  }

  private setupEventHandlers(): void {
    // Handle tools list request
    this.server.setRequestHandler(
      ListToolsRequestSchema,
      async () => {
        return {
          tools: Object.values(CORTEX_TOOLS),
        };
      }
    );

    // Handle tool call request
    this.server.setRequestHandler(
      CallToolRequestSchema,
      async (request: any) => {
        const { name, arguments: args } = request.params;
        
        try {
          const handler = this.handlers.get(name);
          if (!handler) {
            throw new Error(`Unknown tool: ${name}`);
          }

          const result = await handler.handle(args);
          
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify(result, null, 2)
              }
            ]
          };
        } catch (error: any) {
          return {
            content: [
              {
                type: 'text', 
                text: `Error: ${error.message || 'Unknown error'}`
              }
            ],
            isError: true
          };
        }
      }
    );

    // Set up error handler
    this.server.onerror = (error) => {
      console.error('MCP server error:', error);
    };

    // Set up close handler
    this.server.onclose = () => {
      console.log('MCP server connection closed');
    };
  }

  async start(): Promise<void> {
    conditionalLogger.ready('🚀 Cortex MCP Server (stdio) Starting...', {
      metadata: {
        version: getVersion(),
        commit: getGitCommit(),
        node: process.version,
        platform: os.platform(),
        pid: process.pid
      }
    });

    // Create stdio transport and connect
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    
    conditionalLogger.ok('✅ Cortex MCP Server (stdio) Connected and Ready');
  }

  async stop(): Promise<void> {
    conditionalLogger.start('🛑 Shutting down Lightweight Cortex MCP Server (stdio)');
    
    // Clear health check interval
    if (this.healthCheckInterval) {
      clearInterval(this.healthCheckInterval);
      this.healthCheckInterval = null;
    }
    
    // Clear cache
    this.localCache.clear();

    // Close server connection
    this.server.close();
    conditionalLogger.ok('✅ Lightweight Cortex MCP Server (stdio) Stopped');
  }
}

// Clean up orphaned embedding processes at startup
const cleanupOrphanedProcesses = async (): Promise<number> => {
  try {
    const countResult = execSync('ps aux | grep -c "external-embedding-process" | grep -v grep || echo "0"', { encoding: 'utf8' });
    const orphanedCount = parseInt(countResult.trim()) || 0;
    
    if (orphanedCount > 0) {
      conditionalLogger.warn('🧹 CLEANUP: Found orphaned embedding processes from previous sessions', {
        metadata: { count: orphanedCount }
      });
      try {
        execSync('pkill -f "external-embedding-process" 2>/dev/null || true');
        conditionalLogger.ok('✅ Orphaned processes cleaned up successfully');
      } catch (cleanupError: any) {
        conditionalLogger.warn('⚠️  Some processes may require manual cleanup', {
          metadata: { error: cleanupError.message }
        });
      }
    }
    
    return orphanedCount;
  } catch (error) {
    return 0;
  }
};

// Command line argument parsing
const isDemoMode = process.argv.includes('--demo');
const forceReindex = process.argv.includes('--reindex') || process.argv.includes('--force-rebuild') || cortexConfig.forceRebuild;
const forceFullMode = process.argv.includes('--full');
const enableRealTime = !process.argv.includes('--no-watch') && !cortexConfig.disableRealTime;
const repoPath = process.argv.find(arg => !arg.startsWith('--') && !arg.includes('ts-node') && !arg.includes('stdio-server.ts')) || process.cwd();

// Main startup function for stdio server
async function main() {
  // Clean up orphaned processes from previous crashed sessions
  await cleanupOrphanedProcesses();
  
  try {
    // Startup metadata header
    const version = getVersion();
    const commit = getGitCommit(); 
    const nodeVersion = process.version;
    const platform = os.platform();
    const pid = process.pid;
    
    conditionalLogger.ready(`Cortex MCP Server (stdio) v${version} (${commit})`, {
      metadata: { 
        pid, 
        node: nodeVersion,
        platform,
        repo: repoPath
      }
    });

    // V3.0 Lightweight Architecture - No heavy indexing processes
    
    // Initialize project manager for multi-project support
    const projectManager = new ProjectManager();
    await projectManager.initializeWithCurrentDirectory();

    // If in demo mode, show lightweight demo message and exit
    if (isDemoMode) {
      conditionalLogger.ok('🎯 Lightweight stdio demo mode - No heavy indexing required');
      conditionalLogger.start('📊 V3.0 architecture uses centralized embedding server instead of local processing');
      conditionalLogger.ok('✅ Demo completed, exiting...');
      process.exit(0);
    }

    // Create and start lightweight stdio MCP server
    const stdioServer = new LightweightStdioCortexMCPServer(repoPath, projectManager);
    
    // Test connection to centralized server
    const embeddingClient = (stdioServer as any).embeddingClient;
    const centralizedServerAvailable = await embeddingClient.testConnection();
    
    if (centralizedServerAvailable) {
      conditionalLogger.ok('✅ Connected to centralized embedding server at localhost:8766');
    } else {
      conditionalLogger.warn('⚠️  Centralized embedding server not available - running in fallback mode');
    }
    
    await stdioServer.start();

    // Handle graceful shutdown
    process.on('SIGINT', async () => {
      conditionalLogger.start('Received SIGINT, initiating graceful shutdown');
      await stdioServer.stop();
      process.exit(0);
    });

    process.on('SIGTERM', async () => {
      conditionalLogger.start('Received SIGTERM, initiating graceful shutdown');
      await stdioServer.stop();
      process.exit(0);
    });

  } catch (error: any) {
    conditionalLogger.fail('Failed to start Cortex MCP Server (stdio)', { 
      metadata: { error: error.message }
    });
    process.exit(1);
  }
}

// Start server if this file is run directly
if (require.main === module) {
  main().catch((error: any) => {
    console.error('Fatal error starting stdio server:', error);
    process.exit(1);
  });
}

export { LightweightStdioCortexMCPServer };