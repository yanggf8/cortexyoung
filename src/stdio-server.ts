// Import MCP SDK types
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js';

// Import existing Cortex components
import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import { ProjectManager } from './project-manager';
import { CORTEX_TOOLS } from './mcp-tools';

// Import handlers
import { 
  SemanticSearchHandler,
  ContextualReadHandler,
  CodeIntelligenceHandler,
  RelationshipAnalysisHandler,
  TraceExecutionPathHandler,
  FindCodePatternsHandler,
  RealTimeStatusHandler,
  FetchChunkHandler,
  NextChunkHandler,
  GetCurrentProjectHandler,
  ListAvailableProjectsHandler,
  SwitchProjectHandler,
  AddProjectHandler
} from './mcp-handlers';

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

class StdioCortexMCPServer {
  private server: Server;
  private indexer: CodebaseIndexer;
  private searcher: SemanticSearcher;
  private projectManager: ProjectManager;
  private handlers: Map<string, any> = new Map();

  constructor(
    indexer: CodebaseIndexer,
    searcher: SemanticSearcher,
    projectManager: ProjectManager
  ) {
    // Create MCP server
    this.server = new Server(
      {
        name: 'cortex-mcp-server',
        version: '2.1.0',
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );
    
    this.indexer = indexer;
    this.searcher = searcher;
    this.projectManager = projectManager;
    
    this.setupHandlers();
    this.setupEventHandlers();
  }

  private setupHandlers(): void {
    // Core semantic analysis handlers
    this.handlers.set('semantic_search', new SemanticSearchHandler(this.searcher, this.projectManager));
    this.handlers.set('contextual_read', new ContextualReadHandler(this.searcher, this.projectManager));
    this.handlers.set('code_intelligence', new CodeIntelligenceHandler(this.searcher, this.projectManager));
    this.handlers.set('relationship_analysis', new RelationshipAnalysisHandler(this.searcher, this.projectManager));
    this.handlers.set('trace_execution_path', new TraceExecutionPathHandler(this.searcher, this.projectManager));
    this.handlers.set('find_code_patterns', new FindCodePatternsHandler(this.searcher, this.projectManager));
    this.handlers.set('real_time_status', new RealTimeStatusHandler(this.indexer));
    
    // Chunking handlers
    this.handlers.set('fetch_chunk', new FetchChunkHandler());
    this.handlers.set('next_chunk', new NextChunkHandler());
    
    // Project management handlers
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
    conditionalLogger.start('🛑 Shutting down Cortex MCP Server (stdio)');
    
    try {
      // Disable real-time updates if enabled
      await this.indexer.disableRealTimeUpdates();
    } catch (error: any) {
      conditionalLogger.warn('Error disabling real-time updates', { 
        metadata: { error: error.message }
      });
    }

    // Close server connection
    this.server.close();
    conditionalLogger.ok('✅ Cortex MCP Server (stdio) Stopped');
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

    // Initialize indexer with intelligent mode detection
    const indexer = new CodebaseIndexer(repoPath);
    
    // Determine indexing mode
    let indexMode: 'full' | 'incremental' | 'reindex';
    if (forceReindex || cortexConfig.indexMode === 'reindex') {
      indexMode = 'reindex';
      conditionalLogger.warn('Force rebuild requested', { 
        metadata: { mode: 'reindex' }
      });
    } else if (forceFullMode) {
      indexMode = 'full';
      conditionalLogger.ok('Full indexing mode requested', { 
        metadata: { mode: 'full' }
      });
    } else if (cortexConfig.indexMode) {
      indexMode = cortexConfig.indexMode as 'full' | 'incremental';
      conditionalLogger.ok('Explicit indexing mode', { 
        metadata: { mode: indexMode }
      });
    } else {
      // Use intelligent mode detection
      const hasValidIndex = await (indexer as any).vectorStore.hasValidIndex();
      indexMode = hasValidIndex ? 'incremental' : 'full';
      conditionalLogger.start(`🧠 Intelligent mode: Using ${indexMode} indexing`, {
        metadata: { hasValidIndex, mode: indexMode }
      });
    }

    // Index repository
    conditionalLogger.start('📚 Indexing repository...', {
      metadata: { mode: indexMode }
    });
    const indexResponse = await indexer.indexRepository({
      repository_path: repoPath,
      mode: indexMode
    });

    const deltaInfo = indexResponse.chunks_processed > 0 
      ? `+${indexResponse.chunks_processed} chunks processed` 
      : 'No changes detected, using cache';
    
    conditionalLogger.ok('📊 Indexing complete', { 
      metadata: { 
        chunks: indexResponse.chunks_processed,
        time: `${indexResponse.time_taken_ms}ms`,
        mode: deltaInfo
      }
    });

    // Initialize searcher and project manager
    const searcher = (indexer as any).searcher;
    const projectManager = new ProjectManager();
    await projectManager.initializeWithCurrentDirectory();

    // If in demo mode, exit after indexing is complete
    if (isDemoMode) {
      conditionalLogger.ok('🎯 Demo mode complete - indexing finished successfully');
      conditionalLogger.start(`📊 Processed ${indexResponse.chunks_processed} chunks in ${indexResponse.time_taken_ms}ms`);
      conditionalLogger.ok('✅ Demo completed, exiting...');
      process.exit(0);
    }

    // Enable real-time file watching if requested
    if (enableRealTime) {
      conditionalLogger.start('🔄 Enabling real-time file watching...', {
        metadata: { enabled: true }
      });
      try {
        await indexer.enableRealTimeUpdates();
        conditionalLogger.ok('✅ Real-time file watching enabled successfully');
      } catch (error: any) {
        conditionalLogger.warn('⚠️  Failed to enable real-time watching', { 
          metadata: { error: error.message }
        });
      }
    }

    // Create and start stdio MCP server
    const stdioServer = new StdioCortexMCPServer(indexer, searcher, projectManager);
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

export { StdioCortexMCPServer };