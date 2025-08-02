import { MCPToolRequest, MCPToolResponse } from '@cortexyoung/shared';
import { CodebaseIndexer, SemanticSearcher } from '@cortexyoung/core';
import { SemanticSearchHandler, ContextualReadHandler, CodeIntelligenceHandler } from './handlers';

export class CortexMCPServer {
  private indexer: CodebaseIndexer;
  private searcher: SemanticSearcher;
  private handlers: Map<string, any> = new Map();

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
    // TODO: Implement HTTP/WebSocket server for MCP protocol
    console.log(`Cortex MCP Server starting on port ${port}`);
  }

  async stop(): Promise<void> {
    console.log('Cortex MCP Server stopping');
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