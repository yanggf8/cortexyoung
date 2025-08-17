import { QueryRequest, QueryResponse } from './types';
import { SemanticSearcher } from './searcher';
import { CodebaseIndexer } from './indexer';

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

    const result = await this.searcher.search(query);
    
    // Add context optimization hints to help Claude Code learn
    const enhancedResult = {
      ...result,
      context_optimization: {
        tool_used: 'semantic_search',
        query_complexity: this.assessQueryComplexity(params.query),
        token_efficiency: result.context_package?.token_efficiency || 0,
        critical_coverage: result.dependency_chain?.completeness_score || 0,
        suggested_mmr_preset: this.suggestMMRPreset(params.query),
        follow_up_suggestions: this.generateFollowUpSuggestions(result, params.query),
        optimization_tips: this.generateOptimizationTips(result, params)
      }
    };

    return enhancedResult;
  }

  private assessQueryComplexity(query: string): 'simple' | 'medium' | 'complex' {
    const words = query.split(/\s+/).length;
    const hasSpecialTerms = /\b(implement|understand|analyze|debug|refactor|architecture)\b/i.test(query);
    
    if (words > 10 || hasSpecialTerms) return 'complex';
    if (words > 5) return 'medium';
    return 'simple';
  }

  private suggestMMRPreset(query: string): string {
    if (/\b(debug|error|fix|bug|issue)\b/i.test(query)) return 'high-relevance';
    if (/\b(understand|learn|explore|architecture|overview)\b/i.test(query)) return 'high-diversity';
    return 'balanced';
  }

  private generateFollowUpSuggestions(result: any, query: string): string[] {
    const suggestions: string[] = [];
    
    if (result.chunks?.length > 15) {
      suggestions.push('Consider using code_intelligence for comprehensive analysis of complex results');
    }
    
    if (/\b(debug|error)\b/i.test(query)) {
      suggestions.push('Use trace_execution_path to understand error propagation');
    }
    
    if (result.context_package?.groups?.length > 3) {
      suggestions.push('Use relationship_analysis to understand connections between components');
    }
    
    return suggestions;
  }

  private generateOptimizationTips(result: any, params: any): string[] {
    const tips: string[] = [];
    
    if (!params.multi_hop?.enabled) {
      tips.push('Enable multi_hop for better dependency context');
    }
    
    if (result.context_package?.token_efficiency < 0.7) {
      tips.push('Consider more specific query terms to improve relevance');
    }
    
    if (result.chunks?.length < 5) {
      tips.push('Try broader search terms or increase max_chunks');
    }
    
    return tips;
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

export class RelationshipAnalysisHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher) {
    super();
  }

  async handle(params: any): Promise<any> {
    const { analysis_type, starting_symbols, target_symbols, max_depth, relationship_filters } = params;
    
    // Create a specialized query for relationship analysis
    const query: QueryRequest = {
      task: `Analyze ${analysis_type} relationships starting from: ${starting_symbols.join(', ')}`,
      max_chunks: 30,
      multi_hop: {
        enabled: true,
        max_hops: max_depth || 3,
        relationship_types: relationship_filters || ['calls', 'imports', 'data_flow'],
        hop_decay: 0.9,
        focus_symbols: starting_symbols,
        include_paths: true
      },
      context_mode: 'structured'
    };

    const result = await this.searcher.search(query);
    
    return {
      analysis_type,
      starting_symbols,
      target_symbols,
      relationships_found: result.metadata.relationship_paths || [],
      context: result.context_package,
      visualization: `Analysis of ${analysis_type} for symbols: ${starting_symbols.join(', ')}`,
      confidence_scores: result.metadata.confidence_scores
    };
  }
}

export class TraceExecutionPathHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher) {
    super();
  }

  async handle(params: any): Promise<any> {
    const { entry_point, trace_type, include_data_flow, max_execution_depth } = params;
    
    const query: QueryRequest = {
      task: `Trace execution path starting from ${entry_point}`,
      max_chunks: 25,
      multi_hop: {
        enabled: true,
        max_hops: max_execution_depth || 5,
        relationship_types: include_data_flow ? ['calls', 'data_flow', 'throws'] : ['calls', 'throws'],
        hop_decay: 0.8,
        focus_symbols: [entry_point],
        traversal_direction: trace_type === 'backward_trace' ? 'backward' : 'forward'
      },
      context_mode: 'adaptive'
    };

    const result = await this.searcher.search(query);
    
    return {
      entry_point,
      trace_type,
      execution_path: result.metadata.relationship_paths || [],
      data_flow_included: include_data_flow,
      context: result.context_package,
      execution_summary: `Execution trace from ${entry_point} with ${result.context_chunks?.length || 0} steps discovered`
    };
  }
}

export class FindCodePatternsHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher) {
    super();
  }

  async handle(params: any): Promise<any> {
    const { pattern_type, pattern_description, scope, confidence_threshold, max_results } = params;
    
    const searchTask = pattern_description || 
      `Find ${pattern_type} patterns in the codebase`;
    
    const query: QueryRequest = {
      task: searchTask,
      max_chunks: max_results || 10,
      multi_hop: {
        enabled: true,
        max_hops: 2,
        relationship_types: ['calls', 'imports', 'extends', 'implements'],
        hop_decay: 0.9,
        min_strength: confidence_threshold || 0.7
      },
      context_mode: 'structured'
    };

    // Adjust scope based on parameter
    if (scope === 'file' || scope === 'module') {
      query.max_chunks = Math.min(query.max_chunks || 10, 5);
    }

    const result = await this.searcher.search(query);
    
    return {
      pattern_type,
      pattern_description: searchTask,
      scope,
      patterns_found: (result.context_chunks || []).map((chunk, index) => ({
        file: chunk.file_path,
        line_range: `${chunk.start_line}-${chunk.end_line}`,
        confidence: chunk.similarity_score,
        description: `Pattern match ${index + 1}: ${chunk.function_name || 'code block'}`,
        code_excerpt: chunk.content.substring(0, 200) + '...'
      })),
      total_matches: result.context_chunks?.length || 0,
      confidence_scores: result.metadata.confidence_scores
    };
  }
}

export class RealTimeStatusHandler extends BaseHandler {
  constructor(private indexer: CodebaseIndexer) {
    super();
  }

  async handle(params: any): Promise<any> {
    const stats = this.indexer.getRealTimeStats();
    
    return {
      realTimeEnabled: stats.isWatching,
      invalidatedChunks: stats.invalidatedChunks,
      contextFreshness: stats.invalidatedChunks === 0 ? 'fresh' : 'stale',
      lastUpdate: new Date().toISOString(),
      status: stats.isWatching ? 'active' : 'disabled',
      fileWatchingActive: stats.isWatching,
      pendingUpdates: stats.invalidatedChunks,
      systemInfo: {
        realTimeUpdatesSupported: true,
        fileWatcherType: 'chokidar',
        semanticFilteringEnabled: true
      }
    };
  }
}