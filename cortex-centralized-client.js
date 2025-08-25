#!/usr/bin/env node

/**
 * Cortex V3.0 Centralized Architecture MCP Client
 * Lightweight MCP client that connects to centralized embedding server
 */

const { Server } = require('@modelcontextprotocol/sdk/server/index.js');
const { StdioServerTransport } = require('@modelcontextprotocol/sdk/server/stdio.js');
const { ListToolsRequestSchema, CallToolRequestSchema } = require('@modelcontextprotocol/sdk/types.js');
const http = require('http');

const CENTRALIZED_SERVER_URL = 'http://localhost:8766';

class LightweightMCPClient {
  constructor() {
    this.server = new Server(
      {
        name: "cortex-centralized",
        version: "3.0.0"
      },
      {
        capabilities: {
          tools: {
            listTools: true,
            callTool: true
          }
        }
      }
    );

    this.setupToolHandlers();
    this.setupProcessHandlers();
  }

  setupProcessHandlers() {
    // Clean exit when stdin closes (Claude Code disconnects)
    process.stdin.on('end', () => {
      console.error(`[${new Date().toISOString()}] STDIN_END: Claude Code disconnected, shutting down`);
      process.exit(0);
    });

    // Handle other signals
    ['SIGINT', 'SIGTERM', 'SIGHUP'].forEach(signal => {
      process.on(signal, () => {
        console.error(`[${new Date().toISOString()}] SIGNAL_${signal}: Shutting down gracefully`);
        process.exit(0);
      });
    });

    process.on('uncaughtException', (error) => {
      console.error(`[${new Date().toISOString()}] UNCAUGHT_EXCEPTION:`, error.message);
      process.exit(1);
    });

    process.on('unhandledRejection', (reason) => {
      console.error(`[${new Date().toISOString()}] UNHANDLED_REJECTION:`, reason);
      process.exit(1);
    });
  }

  setupToolHandlers() {
    // List available tools
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      return {
        tools: [
          {
            name: "semantic_search",
            description: "Advanced semantic search with context enhancement"
          },
          {
            name: "code_intelligence",
            description: "Intelligent code analysis and insights"
          },
          {
            name: "relationship_analysis",
            description: "Analyze code relationships and dependencies"
          },
          {
            name: "real_time_status",
            description: "Get real-time server and resource status"
          },
          {
            name: "fetch_chunk",
            description: "Fetch specific chunk from cached response"
          },
          {
            name: "next_chunk",
            description: "Get next chunk in sequence from cached response"
          }
        ]
      };
    });

    // Handle tool calls by forwarding to centralized server
    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;
      
      try {
        // Map MCP tool names to centralized server endpoints
        const endpointMap = {
          'semantic_search': '/semantic-search-enhanced',
          'code_intelligence': '/code-intelligence',
          'relationship_analysis': '/relationship-analysis',
          'real_time_status': '/health',
          'fetch_chunk': '/fetch-chunk',
          'next_chunk': '/next-chunk'
        };

        const endpoint = endpointMap[name];
        if (!endpoint) {
          throw new Error(`Unknown tool: ${name}`);
        }

        // Forward request to centralized server
        const response = await this.makeHttpRequest(endpoint, args || {});
        
        return {
          content: [
            {
              type: "text",
              text: typeof response === 'string' ? response : JSON.stringify(response, null, 2)
            }
          ]
        };

      } catch (error) {
        return {
          content: [
            {
              type: "text", 
              text: `Error: ${error.message}`
            }
          ],
          isError: true
        };
      }
    });
  }

  async makeHttpRequest(endpoint, data) {
    return new Promise((resolve, reject) => {
      const isHealthCheck = endpoint === '/health';
      const options = {
        hostname: 'localhost',
        port: 8766,
        path: endpoint,
        method: isHealthCheck ? 'GET' : 'POST',
        headers: isHealthCheck ? {} : {
          'Content-Type': 'application/json'
        }
      };

      const req = http.request(options, (res) => {
        let responseData = '';

        res.on('data', (chunk) => {
          responseData += chunk;
        });

        res.on('end', () => {
          try {
            if (res.statusCode >= 200 && res.statusCode < 300) {
              // Try to parse as JSON, fall back to text
              try {
                const jsonResponse = JSON.parse(responseData);
                resolve(jsonResponse);
              } catch {
                resolve(responseData);
              }
            } else {
              reject(new Error(`HTTP ${res.statusCode}: ${responseData}`));
            }
          } catch (error) {
            reject(error);
          }
        });
      });

      req.on('error', (error) => {
        reject(new Error(`Connection failed: ${error.message}. Is centralized server running on ${CENTRALIZED_SERVER_URL}?`));
      });

      // Send data for POST requests
      if (!isHealthCheck && data) {
        req.write(JSON.stringify(data));
      }

      req.end();
    });
  }

  async run() {
    try {
      // Test connection to centralized server
      await this.makeHttpRequest('/health', {});
      console.error(`[${new Date().toISOString()}] Connected to centralized server at ${CENTRALIZED_SERVER_URL}`);
      
      const transport = new StdioServerTransport();
      await this.server.connect(transport);
      console.error(`[${new Date().toISOString()}] Lightweight MCP client ready`);
      
    } catch (error) {
      console.error(`[${new Date().toISOString()}] Failed to connect to centralized server:`, error.message);
      console.error(`[${new Date().toISOString()}] Make sure to start centralized server first: npm run start:centralized`);
      process.exit(1);
    }
  }
}

// Start the lightweight client
const client = new LightweightMCPClient();
client.run().catch(error => {
  console.error(`[${new Date().toISOString()}] Fatal error:`, error);
  process.exit(1);
});