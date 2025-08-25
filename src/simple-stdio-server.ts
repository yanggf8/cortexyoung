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

class SimpleCortexMCPServer {
  private server: Server;

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

    this.setupToolHandlers();
    this.setupErrorHandling();
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
            return await LightweightSemanticSearchHandler(args);
          case 'contextual_read':
            return await LightweightContextualReadHandler(args);
          case 'code_intelligence':
            return await LightweightCodeIntelligenceHandler(args);
          case 'relationship_analysis':
            return await LightweightRelationshipAnalysisHandler(args);
          case 'trace_execution_path':
            return await LightweightTraceExecutionPathHandler(args);
          case 'find_code_patterns':
            return await LightweightFindCodePatternsHandler(args);
          case 'real_time_status':
            return await LightweightRealTimeStatusHandler(args);
          case 'fetch_chunk':
            return await FetchChunkHandler(args);
          case 'next_chunk':
            return await NextChunkHandler(args);
          case 'get_current_project':
            return await GetCurrentProjectHandler(args);
          case 'list_available_projects':
            return await ListAvailableProjectsHandler(args);
          case 'switch_project':
            return await SwitchProjectHandler(args);
          case 'add_project':
            return await AddProjectHandler(args);
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