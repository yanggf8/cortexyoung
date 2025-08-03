// Import MCP SDK types for reference but use simpler implementation
// import { Server } from '@modelcontextprotocol/sdk/server/index.js';
// import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import express, { Request, Response } from 'express';
import cors from 'cors';
import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import { SemanticSearchHandler, ContextualReadHandler, CodeIntelligenceHandler } from './mcp-handlers';
import * as fs from 'fs';
import * as path from 'path';

// Logger class for file and console logging
class Logger {
  private logFile: string;
  private logStream: fs.WriteStream;

  constructor(logFile?: string) {
    this.logFile = logFile || path.join(process.cwd(), 'logs', 'cortex-server.log');
    
    // Ensure log directory exists
    const logDir = path.dirname(this.logFile);
    if (!fs.existsSync(logDir)) {
      fs.mkdirSync(logDir, { recursive: true });
    }
    
    // Create write stream for log file
    this.logStream = fs.createWriteStream(this.logFile, { flags: 'a' });
    
    this.info('Logger initialized', { logFile: this.logFile });
  }

  private formatMessage(level: string, message: string, data?: any): string {
    const timestamp = new Date().toISOString();
    const dataStr = data ? ` | ${JSON.stringify(data)}` : '';
    return `[${timestamp}] [${level}] ${message}${dataStr}`;
  }

  private writeLog(level: string, message: string, data?: any): void {
    const formatted = this.formatMessage(level, message, data);
    
    // Write to console
    console.log(formatted);
    
    // Write to file
    this.logStream.write(formatted + '\n');
  }

  info(message: string, data?: any): void {
    this.writeLog('INFO', message, data);
  }

  error(message: string, data?: any): void {
    this.writeLog('ERROR', message, data);
  }

  warn(message: string, data?: any): void {
    this.writeLog('WARN', message, data);
  }

  debug(message: string, data?: any): void {
    if (process.env.DEBUG === 'true') {
      this.writeLog('DEBUG', message, data);
    }
  }

  close(): void {
    this.logStream.end();
  }
}

export class CortexMCPServer {
  private indexer: CodebaseIndexer;
  private searcher: SemanticSearcher;
  private handlers: Map<string, any> = new Map();
  private httpServer: any;
  private logger: Logger;

  constructor(
    indexer: CodebaseIndexer,
    searcher: SemanticSearcher,
    logFile?: string
  ) {
    this.indexer = indexer;
    this.searcher = searcher;
    this.logger = new Logger(logFile);
    this.setupHandlers();
    this.logger.info('CortexMCPServer initialized');
  }

  private setupHandlers(): void {
    this.handlers.set('semantic_search', new SemanticSearchHandler(this.searcher));
    this.handlers.set('contextual_read', new ContextualReadHandler(this.searcher));
    this.handlers.set('code_intelligence', new CodeIntelligenceHandler(this.searcher));
  }

  private getAvailableTools() {
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

  async startHttp(port: number = 3001): Promise<void> {
    this.logger.info('Starting MCP Server with HTTP transport', { port });
    
    const app = express();
    app.use(express.json());
    app.use(cors({
      origin: '*',
      methods: ['GET', 'POST', 'OPTIONS'],
      allowedHeaders: ['Content-Type', 'Authorization'],
      credentials: false
    }));

    // Health check endpoint
    app.get('/health', (req: Request, res: Response) => {
      res.json({ status: 'healthy', server: 'cortex-mcp-server', version: '2.1.0' });
    });

    // MCP endpoint for JSON-RPC communication
    app.post('/mcp', async (req: Request, res: Response) => {
      try {
        this.logger.info('Received MCP request', { 
          method: req.body?.method,
          id: req.body?.id 
        });

        // Handle JSON-RPC requests manually for now
        // In a full implementation, this would use StreamableHTTPServerTransport
        const { method, params, id } = req.body;
        
        let response: any;
        
        if (method === 'initialize') {
          response = {
            jsonrpc: '2.0',
            result: {
              protocolVersion: '2025-01-07',
              capabilities: {
                tools: {}
              },
              serverInfo: {
                name: 'cortex-mcp-server',
                version: '2.1.0'
              }
            },
            id
          };
        } else if (method === 'tools/list') {
          // Return available tools directly
          response = {
            jsonrpc: '2.0',
            result: {
              tools: this.getAvailableTools()
            },
            id
          };
        } else if (method === 'tools/call') {
          try {
            // Handle tool call directly
            const { name, arguments: args } = params;
            let toolResponse;
            
            switch (name) {
              case 'semantic_search': {
                const handler = this.handlers.get('semantic_search');
                const result = await handler.handle(args);
                toolResponse = {
                  content: [
                    {
                      type: 'text',
                      text: JSON.stringify(result, null, 2)
                    }
                  ]
                };
                break;
              }
              case 'contextual_read': {
                const handler = this.handlers.get('contextual_read');
                const result = await handler.handle(args);
                toolResponse = {
                  content: [
                    {
                      type: 'text',
                      text: JSON.stringify(result, null, 2)
                    }
                  ]
                };
                break;
              }
              case 'code_intelligence': {
                const handler = this.handlers.get('code_intelligence');
                const result = await handler.handle(args);
                toolResponse = {
                  content: [
                    {
                      type: 'text',
                      text: JSON.stringify(result, null, 2)
                    }
                  ]
                };
                break;
              }
              default:
                throw new Error(`Unknown tool: ${name}`);
            }
            
            response = {
              jsonrpc: '2.0',
              result: toolResponse,
              id
            };
          } catch (error) {
            response = {
              jsonrpc: '2.0',
              error: {
                code: -32603,
                message: error instanceof Error ? error.message : 'Internal error'
              },
              id
            };
          }
        } else {
          response = {
            jsonrpc: '2.0',
            error: {
              code: -32601,
              message: `Method not found: ${method}`
            },
            id
          };
        }

        this.logger.info('Sending MCP response', { 
          method,
          id,
          success: !response.error 
        });
        
        res.json(response);
      } catch (error) {
        this.logger.error('MCP request failed', { error: error instanceof Error ? error.message : 'Unknown error' });
        res.status(500).json({
          jsonrpc: '2.0',
          error: {
            code: -32603,
            message: 'Internal error'
          },
          id: req.body?.id || null
        });
      }
    });

    await new Promise<void>((resolve) => {
      this.httpServer = app.listen(port, () => {
        this.logger.info('MCP HTTP Server started', { 
          port,
          endpoints: {
            health: `GET http://localhost:${port}/health`,
            mcp: `POST http://localhost:${port}/mcp`
          }
        });
        resolve();
      });
    });
  }

  async stop(): Promise<void> {
    this.logger.info('Stopping MCP Server');
    if (this.httpServer) {
      await new Promise<void>((resolve) => {
        this.httpServer.close(() => {
          this.logger.info('MCP HTTP Server stopped successfully');
          resolve();
        });
      });
    }
    this.logger.close();
  }
}

// Main startup function
async function main() {
  const repoPath = process.argv[2] || process.cwd();
  const port = parseInt(process.env.PORT || '8765');
  const logFile = process.env.LOG_FILE;
  
  // Create main logger
  const logger = new Logger(logFile);
  
  logger.info('Initializing Cortex MCP Server', { repoPath, port });
  
  try {
    // Initialize indexer and searcher
    logger.info('Starting repository indexing');
    const indexer = new CodebaseIndexer(repoPath);
    
    const indexResponse = await indexer.indexRepository({
      repository_path: repoPath,
      mode: 'full'
    });
    
    logger.info('Repository indexing completed', { 
      chunksProcessed: indexResponse.chunks_processed,
      timeMs: indexResponse.time_taken_ms
    });
    
    // Get searcher from indexer
    const searcher = (indexer as any).searcher; // Access the searcher instance
    
    // Create and start MCP server
    const mcpServer = new CortexMCPServer(indexer, searcher, logFile);
    
    logger.info('Starting MCP server with HTTP transport', { port });
    await mcpServer.startHttp(port);
    
    // Handle graceful shutdown
    process.on('SIGINT', async () => {
      logger.info('Received SIGINT, initiating graceful shutdown');
      await mcpServer.stop();
      process.exit(0);
    });
    
    process.on('SIGTERM', async () => {
      logger.info('Received SIGTERM, initiating graceful shutdown');
      await mcpServer.stop();
      process.exit(0);
    });
    
  } catch (error) {
    logger.error('Failed to start Cortex MCP Server', { error: error instanceof Error ? error.message : error });
    process.exit(1);
  }
}

// Start server if this file is run directly
if (require.main === module) {
  main().catch(console.error);
}