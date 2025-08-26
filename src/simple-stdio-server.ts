#!/usr/bin/env node

/**
 * Simple Cortex MCP Server - stdio transport
 * Simplified version optimized for multi-instance compatibility
 * Based on successful qclimcp pattern
 */

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js';
import { CORTEX_TOOLS } from './mcp-tools';

// Import only essential handlers
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

import { createLocalEmbeddingClient, EmbeddingClient } from './embedding-client';
import { ProjectManager } from './project-manager';

class SimpleCortexMCPServer {
  private server: Server;
  private embeddingClient: EmbeddingClient;
  private projectManager: ProjectManager;
  private cache: Map<string, { data: any; timestamp: number; ttl: number }> = new Map();

  constructor() {
    this.server = new Server(
      {
        name: 'cortex-mcp-server',
        version: '2.1.6',
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    // Initialize embedding client and project manager
    this.embeddingClient = createLocalEmbeddingClient('simple-stdio-server', process.cwd());
    this.projectManager = new ProjectManager();

    this.setupToolHandlers();
    this.setupErrorHandling();
  }

  // Cache methods for handler compatibility
  setCachedResult(key: string, data: any, ttl: number): void {
    this.cache.set(key, { data, timestamp: Date.now(), ttl });
  }

  getCachedResult(key: string): any {
    const cached = this.cache.get(key);
    if (cached && Date.now() - cached.timestamp < cached.ttl) {
      return cached.data;
    }
    this.cache.delete(key);
    return null;
  }

  private setupErrorHandling(): void {
    this.server.onerror = (error) => {
      console.error('[MCP Error]', error);
    };

    process.on('SIGINT', async () => {
      await this.server.close();
      process.exit(0);
    });

    process.on('SIGTERM', async () => {
      await this.server.close();
      process.exit(0);
    });
  }

  private setupToolHandlers(): void {
    // List tools
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      return {
        tools: CORTEX_TOOLS,
      };
    });

    // Handle tool calls
    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;

      try {
        switch (name) {
          case 'semantic_search':
            return await new LightweightSemanticSearchHandler(this.embeddingClient, this.projectManager, this).handle(args);
          case 'contextual_read':
            return await new LightweightContextualReadHandler(this.embeddingClient, this.projectManager, this).handle(args);
          case 'code_intelligence':
            return await new LightweightCodeIntelligenceHandler(this.embeddingClient, this.projectManager, this).handle(args);
          case 'relationship_analysis':
            return await new LightweightRelationshipAnalysisHandler(this.embeddingClient, this.projectManager, this).handle(args);
          case 'trace_execution_path':
            return await new LightweightTraceExecutionPathHandler(this.embeddingClient, this.projectManager, this).handle(args);
          case 'find_code_patterns':
            return await new LightweightFindCodePatternsHandler(this.embeddingClient, this.projectManager, this).handle(args);
          case 'real_time_status':
            return await new LightweightRealTimeStatusHandler(this.embeddingClient, this).handle(args);
          case 'fetch_chunk':
            return await new FetchChunkHandler().handle(args);
          case 'next_chunk':
            return await new NextChunkHandler().handle(args);
          case 'get_current_project':
            return await new GetCurrentProjectHandler(this.projectManager).handle(args);
          case 'list_available_projects':
            return await new ListAvailableProjectsHandler(this.projectManager).handle(args);
          case 'switch_project':
            return await new SwitchProjectHandler(this.projectManager).handle(args);
          case 'add_project':
            return await new AddProjectHandler(this.projectManager).handle(args);
          default:
            throw new Error(`Unknown tool: ${name}`);
        }
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : String(error);
        return {
          content: [
            {
              type: 'text',
              text: `Error: ${errorMessage}`,
            },
          ],
        };
      }
    });
  }

  async run(): Promise<void> {
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    console.error('[INFO] Cortex MCP Server running on stdio (this STDERR message is by design)');
  }
}

// Start the server
const server = new SimpleCortexMCPServer();
server.run().catch((error) => {
  console.error('Failed to start server:', error);
  process.exit(1);
});

export { SimpleCortexMCPServer };