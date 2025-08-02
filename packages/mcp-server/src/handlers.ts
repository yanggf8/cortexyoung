import { QueryRequest, QueryResponse } from '@cortexyoung/shared';
import { SemanticSearcher } from '@cortexyoung/core';

export abstract class BaseHandler {
  abstract handle(params: any): Promise<any>;
}

export class SemanticSearchHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher) {
    super();
  }

  async handle(params: any): Promise<QueryResponse> {
    const query: QueryRequest = {
      task: params.query,
      max_chunks: params.max_chunks || 20,
      file_filters: params.file_filters,
      include_tests: params.include_tests || false,
      multi_hop: params.multi_hop || { enabled: true, max_hops: 2, relationship_types: ['calls'], hop_decay: 0.8 },
      context_mode: params.context_mode || 'structured'
    };

    return await this.searcher.search(query);
  }
}

export class ContextualReadHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher) {
    super();
  }

  async handle(params: any): Promise<any> {
    // TODO: Implement contextual file reading
    // This would read a specific file but include semantically related context
    const filePath = params.file_path;
    const semanticContext = params.semantic_context;
    const maxTokens = params.max_context_tokens || 2000;

    // For now, delegate to semantic search focused on the specific file
    const query: QueryRequest = {
      task: semanticContext || `Read and understand ${filePath}`,
      file_filters: [filePath],
      max_chunks: 10,
      context_mode: 'structured'
    };

    const result = await this.searcher.search(query);
    
    return {
      file_path: filePath,
      content: result.context_package,
      semantic_context: semanticContext,
      token_estimate: result.metadata.token_estimate
    };
  }
}

export class CodeIntelligenceHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher) {
    super();
  }

  async handle(params: any): Promise<QueryResponse> {
    const query: QueryRequest = {
      task: params.task,
      max_chunks: Math.floor((params.max_context_tokens || 4000) / 200), // Rough token estimation
      recency_weight: params.recency_weight || 0.3,
      multi_hop: {
        enabled: true,
        max_hops: 3,
        relationship_types: ['calls', 'imports', 'data_flow'],
        hop_decay: 0.7
      },
      context_mode: 'adaptive'
    };

    if (params.focus_areas) {
      query.file_filters = params.focus_areas.map((area: string) => `*${area}*`);
    }

    return await this.searcher.search(query);
  }
}