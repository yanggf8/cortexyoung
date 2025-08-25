#!/usr/bin/env node

/**
 * Minimal Cortex MCP Server for Testing
 */

const { Server } = require('@modelcontextprotocol/sdk/server/index.js');
const { StdioServerTransport } = require('@modelcontextprotocol/sdk/server/stdio.js');
const { ListToolsRequestSchema, CallToolRequestSchema } = require('@modelcontextprotocol/sdk/types.js');

class MinimalCortexMCPServer {
  constructor() {
    this.server = new Server({
      name: 'cortex-minimal-test-server',
      version: '1.0.0',
    }, {
      capabilities: {
        tools: {},
      },
    });

    this.setupHandlers();
    this.setupErrorHandling();
  }

  setupHandlers() {
    // List tools
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      return {
        tools: [
          {
            name: 'test_tool',
            description: 'Simple test tool to verify MCP connection',
            inputSchema: {
              type: 'object',
              properties: {
                message: {
                  type: 'string',
                  description: 'Test message',
                  default: 'Hello from Cortex MCP!'
                }
              },
              required: []
            }
          }
        ]
      };
    });

    // Handle tool calls
    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;
      
      if (name === 'test_tool') {
        const message = args?.message || 'Hello from Cortex MCP!';
        return {
          content: [
            {
              type: 'text',
              text: `✅ Cortex MCP Server is working! Message: ${message}`
            }
          ]
        };
      }
      
      throw new Error(`Unknown tool: ${name}`);
    });
  }

  setupErrorHandling() {
    this.server.onerror = (error) => {
      console.error('[MCP Error]', error);
    };

    process.on('SIGINT', () => {
      console.error('[Cortex] Received SIGINT, shutting down...');
      process.exit(0);
    });

    process.on('SIGTERM', () => {
      console.error('[Cortex] Received SIGTERM, shutting down...');
      process.exit(0);
    });
  }

  async run() {
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    console.error('[INFO] Minimal Cortex MCP Server running on stdio (this STDERR message is by design)');
  }
}

// Start the server
const server = new MinimalCortexMCPServer();
server.run().catch((error) => {
  console.error('Failed to start Cortex MCP server:', error);
  process.exit(1);
});