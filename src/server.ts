import { MCPToolRequest, MCPToolResponse } from './types';
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
  private server: any;
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

  async handleToolCall(request: MCPToolRequest): Promise<MCPToolResponse> {
    const startTime = Date.now();
    this.logger.info('Received tool call', { 
      method: request.method, 
      id: request.id,
      params: request.params ? Object.keys(request.params) : []
    });

    try {
      const handler = this.handlers.get(request.method);
      
      if (!handler) {
        this.logger.warn('Method not found', { method: request.method, id: request.id });
        return {
          error: {
            code: -32601,
            message: `Method not found: ${request.method}`
          },
          id: request.id
        };
      }

      const result = await handler.handle(request.params);
      const duration = Date.now() - startTime;
      
      this.logger.info('Tool call completed', { 
        method: request.method, 
        id: request.id, 
        duration: `${duration}ms`,
        success: true
      });
      
      return {
        result,
        id: request.id
      };
    } catch (error) {
      const duration = Date.now() - startTime;
      this.logger.error('Tool call failed', { 
        method: request.method, 
        id: request.id, 
        duration: `${duration}ms`,
        error: error instanceof Error ? error.message : 'Unknown error'
      });
      
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
        this.logger.info('MCP Server started', { 
          port,
          endpoints: {
            tools: `GET http://localhost:${port}/tools`,
            call: `POST http://localhost:${port}/call`
          }
        });
        resolve();
      });
    });
    
    this.server = server;
  }

  async stop(): Promise<void> {
    this.logger.info('Stopping MCP Server');
    if (this.server) {
      await new Promise<void>((resolve) => {
        this.server.close(() => {
          this.logger.info('MCP Server stopped successfully');
          resolve();
        });
      });
    }
    this.logger.close();
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
    
    logger.info('Starting MCP server');
    await mcpServer.start(port);
    
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