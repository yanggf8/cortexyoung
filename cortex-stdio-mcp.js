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
            description: "BEST FOR: Quick code discovery, finding specific functions/patterns, debugging. Uses advanced semantic search with MMR optimization to find the most relevant code chunks while ensuring diversity.",
            inputSchema: {
              type: "object",
              properties: {
                query: {
                  type: "string",
                  description: "Natural language description of what you're looking for"
                },
                max_chunks: {
                  type: "number",
                  description: "Maximum number of code chunks to return",
                  default: 20,
                  minimum: 1,
                  maximum: 100
                }
              },
              required: ["query"]
            }
          },
          {
            name: "code_intelligence",
            description: "BEST FOR: Complex development tasks, architecture understanding, feature implementation. When you need comprehensive analysis of large codebases or complex architectural patterns.",
            inputSchema: {
              type: "object",
              properties: {
                task: {
                  type: "string",
                  description: "High-level description of the development task"
                },
                max_context_tokens: {
                  type: "number",
                  description: "Maximum tokens for the complete context package",
                  default: 4000,
                  maximum: 16000,
                  minimum: 1000
                }
              },
              required: ["task"]
            }
          },
          {
            name: "relationship_analysis",
            description: "BEST FOR: Understanding code dependencies, impact analysis, refactoring planning. Before making changes, for impact analysis, or understanding how code components connect.",
            inputSchema: {
              type: "object",
              properties: {
                analysis_type: {
                  type: "string",
                  enum: ["call_graph", "dependency_chain", "data_flow", "error_propagation", "impact_analysis"],
                  description: "Type of relationship analysis to perform"
                },
                starting_symbols: {
                  type: "array",
                  items: { type: "string" },
                  description: "Starting points for analysis (function names, class names, file paths)"
                }
              },
              required: ["analysis_type", "starting_symbols"]
            }
          },
          {
            name: "real_time_status",
            description: "BEST FOR: Checking if context is up-to-date, verifying file watching status. Shows real-time file watching status and context freshness for the codebase.",
            inputSchema: {
              type: "object",
              properties: {},
              required: []
            }
          },
          {
            name: "fetch_chunk",
            description: "CHUNKING TOOL: Retrieves a specific chunk from large responses by index (random access). When a Cortex tool returns 'Response too large' with a cacheKey and you need to access a specific chunk number.",
            inputSchema: {
              type: "object",
              properties: {
                cacheKey: {
                  type: "string",
                  description: "The cache key provided in the 'Response too large' message from any Cortex tool"
                },
                chunkIndex: {
                  type: "number",
                  description: "Which chunk to retrieve (1-based index)",
                  minimum: 1
                }
              },
              required: ["cacheKey", "chunkIndex"]
            }
          },
          {
            name: "next_chunk",
            description: "CHUNKING TOOL: Fetches the next chunk in sequence from large responses (sequential access). When a Cortex tool returns 'Response too large' with a cacheKey and you want to read through all chunks sequentially.",
            inputSchema: {
              type: "object",
              properties: {
                cacheKey: {
                  type: "string",
                  description: "The cache key from the 'Response too large' message from any Cortex tool"
                }
              },
              required: ["cacheKey"]
            }
          },
          {
            name: "multi_instance_health",
            description: "HEALTH CHECK: Multi-instance health monitoring and diagnostics. When experiencing MCP connection issues, multiple Claude Code instances, or need to diagnose startup problems.",
            inputSchema: {
              type: "object",
              properties: {},
              required: []
            }
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
          'next_chunk': '/next-chunk',
          'multi_instance_health': '/multi-instance-health'
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