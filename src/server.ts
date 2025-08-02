import { MCPToolRequest, MCPToolResponse } from './types';
import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import { SemanticSearchHandler, ContextualReadHandler, CodeIntelligenceHandler } from './mcp-handlers';

export class CortexMCPServer {
  private indexer: CodebaseIndexer;
  private searcher: SemanticSearcher;
  private handlers: Map<string, any> = new Map();
  private server: any;

  constructor(
    indexer: CodebaseIndexer,
    searcher: SemanticSearcher
  ) {
    this.indexer = indexer;
    this.searcher = searcher;
    this.setupHandlers();
  }

  private setupHandlers(): void {
    this.handlers.set('semantic_search', new SemanticSearchHandler(this.searcher));
    this.handlers.set('contextual_read', new ContextualReadHandler(this.searcher));
    this.handlers.set('code_intelligence', new CodeIntelligenceHandler(this.searcher));
  }

  async handleToolCall(request: MCPToolRequest): Promise<MCPToolResponse> {
    try {
      const handler = this.handlers.get(request.method);
      
      if (!handler) {
        return {
          error: {
            code: -32601,
            message: `Method not found: ${request.method}`
          },
          id: request.id
        };
      }

      const result = await handler.handle(request.params);
      
      return {
        result,
        id: request.id
      };
    } catch (error) {
      return {
        error: {
          code: -32603,
          message: error instanceof Error ? error.message : 'Internal error',
          data: error
        },
        id: request.id
      };
    }
  }

  async start(port: number = 3001): Promise<void> {
    // For now, implement a simple JSON-RPC over HTTP server
    // In a full implementation, this would use the MCP protocol specification
    const http = require('http');
    
    const server = http.createServer(async (req: any, res: any) => {
      // Enable CORS
      res.setHeader('Access-Control-Allow-Origin', '*');
      res.setHeader('Access-Control-Allow-Methods', 'POST, GET, OPTIONS');
      res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
      
      if (req.method === 'OPTIONS') {
        res.writeHead(200);
        res.end();
        return;
      }
      
      if (req.method === 'GET' && req.url === '/tools') {
        // Return available tools
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          tools: this.getAvailableTools()
        }));
        return;
      }
      
      if (req.method === 'POST' && req.url === '/call') {
        let body = '';
        req.on('data', (chunk: any) => {
          body += chunk.toString();
        });
        
        req.on('end', async () => {
          try {
            const request = JSON.parse(body);
            const response = await this.handleToolCall(request);
            
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify(response));
          } catch (error) {
            res.writeHead(400, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({
              error: {
                code: -32700,
                message: 'Parse error'
              }
            }));
          }
        });
        return;
      }
      
      // Default response
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({
        error: {
          code: -32404,
          message: 'Endpoint not found'
        }
      }));
    });
    
    await new Promise<void>((resolve) => {
      server.listen(port, () => {
        console.log(`🚀 Cortex MCP Server listening on port ${port}`);
        console.log(`📋 Available endpoints:`);
        console.log(`   GET  http://localhost:${port}/tools - List available tools`);
        console.log(`   POST http://localhost:${port}/call  - Execute tool calls`);
        resolve();
      });
    });
    
    this.server = server;
  }

  async stop(): Promise<void> {
    console.log('Cortex MCP Server stopping');
    if (this.server) {
      await new Promise<void>((resolve) => {
        this.server.close(() => {
          console.log('✅ Server stopped');
          resolve();
        });
      });
    }
  }

  getAvailableTools(): any[] {
    return [
      {
        name: 'semantic_search',
        description: 'Semantic code search using vector embeddings',
        inputSchema: {
          type: 'object',
          properties: {
            query: { type: 'string', description: 'Natural language search query' },
            max_chunks: { type: 'number', default: 20 },
            file_filters: { type: 'array', items: { type: 'string' } },
            include_related: { type: 'boolean', default: true }
          },
          required: ['query']
        }
      },
      {
        name: 'contextual_read',
        description: 'Read files with semantic context awareness',
        inputSchema: {
          type: 'object',
          properties: {
            file_path: { type: 'string' },
            semantic_context: { type: 'string' },
            max_context_tokens: { type: 'number', default: 2000 }
          },
          required: ['file_path']
        }
      },
      {
        name: 'code_intelligence',
        description: 'High-level semantic codebase analysis',
        inputSchema: {
          type: 'object',
          properties: {
            task: { type: 'string' },
            focus_areas: { type: 'array', items: { type: 'string' } },
            recency_weight: { type: 'number', default: 0.3 },
            max_context_tokens: { type: 'number', default: 4000 }
          },
          required: ['task']
        }
      }
    ];
  }
}

// Main startup function
async function main() {
  const repoPath = process.argv[2] || process.cwd();
  const port = parseInt(process.env.PORT || '8765');
  
  console.log(`🔧 Initializing Cortex MCP Server...`);
  console.log(`📁 Repository: ${repoPath}`);
  console.log(`🌐 Port: ${port}`);
  
  try {
    // Initialize indexer and searcher
    console.log(`📊 Indexing repository...`);
    const indexer = new CodebaseIndexer(repoPath);
    
    const indexResponse = await indexer.indexRepository({
      repository_path: repoPath,
      mode: 'full'
    });
    
    console.log(`✅ Indexing complete: ${indexResponse.chunks_processed} chunks processed`);
    
    // Get searcher from indexer
    const searcher = (indexer as any).searcher; // Access the searcher instance
    
    // Create and start MCP server
    const mcpServer = new CortexMCPServer(indexer, searcher);
    
    console.log(`🚀 Starting MCP server...`);
    await mcpServer.start(port);
    
    // Handle graceful shutdown
    process.on('SIGINT', async () => {
      console.log(`\n🛑 Received SIGINT, shutting down gracefully...`);
      await mcpServer.stop();
      process.exit(0);
    });
    
    process.on('SIGTERM', async () => {
      console.log(`\n🛑 Received SIGTERM, shutting down gracefully...`);
      await mcpServer.stop();
      process.exit(0);
    });
    
  } catch (error) {
    console.error('❌ Failed to start Cortex MCP Server:', error);
    process.exit(1);
  }
}

// Start server if this file is run directly
if (require.main === module) {
  main().catch(console.error);
}