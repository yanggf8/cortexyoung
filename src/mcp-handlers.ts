import { QueryRequest, QueryResponse } from './types';
import { SemanticSearcher } from './searcher';
import { CodebaseIndexer } from './indexer';
import { cacheText, getCachedChunk, getNextChunk } from './utils/chunk-cache';
import { ProjectManager, ProjectInfo } from './project-manager';
import { EmbeddingClient } from './embedding-client';
import { warn, error } from './logging-utils';

export abstract class BaseHandler {
  abstract handle(params: any): Promise<any>;

  // Shared MCP optimization for all handlers - MINIMAL data only
  protected optimizeForMCP(result: any, params: any): any {
    const CHUNK_SIZE = typeof params.chunk_size === 'number' ? params.chunk_size : 20000;
    
    // Create ultra-lean MCP response - only what Claude Code actually needs
    const mcpOptimizedResult: any = {
      // Essential chunks data only
      chunks: result.chunks?.map((chunk: any) => ({
        file_path: chunk.file_path,
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        content: chunk.content,
        // Only include symbol name if it exists and is meaningful
        ...(chunk.symbol_name && chunk.symbol_name !== 'section_0' ? { symbol_name: chunk.symbol_name } : {})
      })) || [],
      
      // Minimal summary only
      summary: result.context_package?.summary || `Found ${result.chunks?.length || 0} relevant code chunks`,
      
      // Essential file list for reference
      files: [...new Set(result.chunks?.map((chunk: any) => chunk.file_path) || [])],
      
      // Preserve tool-specific fields that are actually useful
      ...(result.analysis_type ? { analysis_type: result.analysis_type } : {}),
      ...(result.starting_symbols ? { starting_symbols: result.starting_symbols } : {}),
      ...(result.entry_point ? { entry_point: result.entry_point } : {}),
      ...(result.pattern_type ? { pattern_type: result.pattern_type } : {}),
      ...(result.patterns_found ? { patterns_found: result.patterns_found } : {}),
      ...(result.execution_path ? { execution_path: result.execution_path } : {}),
      ...(result.relationships_found ? { relationships_found: result.relationships_found } : {}),
      ...(result.visualization ? { visualization: result.visualization } : {})
    };

    // Check if response needs chunking
    const resultString = JSON.stringify(mcpOptimizedResult, null, 2);
    
    if (resultString.length > CHUNK_SIZE) {
      const { key, total } = cacheText(resultString, CHUNK_SIZE);
      const first = getCachedChunk(key, 1);
      
      if (!first) return mcpOptimizedResult;
      
      return `Response too large (${resultString.length} chars); returning first chunk. Use fetch-chunk or next-chunk with cacheKey to continue.\ncacheKey: ${key}\nchunk: 1/${total}\n\n${first.chunk}`;
    }

    return mcpOptimizedResult;
  }
}

export class SemanticSearchHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher, private projectManager?: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<QueryResponse | string> {
    // Use project manager's current searcher if available, otherwise fall back to constructor searcher
    const activeSearcher = this.projectManager?.getCurrentSearcher() || this.searcher;
    if (!activeSearcher) {
      throw new Error('No active project or searcher available. Use switch_project to set up a project.');
    }
    const query: QueryRequest = {
      task: params.query,
      max_chunks: params.max_chunks || 20,
      file_filters: params.file_filters,
      include_tests: params.include_tests || false,
      multi_hop: params.multi_hop || { enabled: true, max_hops: 2, relationship_types: ['calls'], hop_decay: 0.8 },
      context_mode: params.context_mode || 'structured'
    };

    const result = await activeSearcher.search(query);
    
    // Add semantic search specific context optimization
    const enhancedResult = {
      ...result,
      context_optimization: {
        tool_used: 'semantic_search',
        query_complexity: this.assessQueryComplexity(params.query),
        response_optimized_for: 'mcp_client',
        follow_up_suggestions: this.generateFollowUpSuggestions(result, params.query),
        optimization_tips: this.generateOptimizationTips(result, params)
      }
    };

    // Use shared MCP optimization
    return this.optimizeForMCP(enhancedResult, params);
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
  constructor(private searcher: SemanticSearcher, private projectManager?: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    // Use project manager's current searcher if available, otherwise fall back to constructor searcher
    const activeSearcher = this.projectManager?.getCurrentSearcher() || this.searcher;
    if (!activeSearcher) {
      throw new Error('No active project or searcher available. Use switch_project to set up a project.');
    }

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

    const result = await activeSearcher.search(query);
    
    return {
      file_path: filePath,
      content: result.context_package,
      semantic_context: semanticContext,
      token_estimate: result.metadata.token_estimate
    };
  }
}

export class CodeIntelligenceHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher, private projectManager?: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    // Use project manager's current searcher if available, otherwise fall back to constructor searcher
    const activeSearcher = this.projectManager?.getCurrentSearcher() || this.searcher;
    if (!activeSearcher) {
      throw new Error('No active project or searcher available. Use switch_project to set up a project.');
    }

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

    const result = await activeSearcher.search(query);
    
    // Add code intelligence specific context optimization
    const enhancedResult = {
      ...result,
      context_optimization: {
        tool_used: 'code_intelligence',
        analysis_type: 'comprehensive',
        focus_areas: params.focus_areas || [],
        response_optimized_for: 'mcp_client'
      }
    };

    // Use shared MCP optimization
    return this.optimizeForMCP(enhancedResult, params);
  }
}

export class RelationshipAnalysisHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher, private projectManager?: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    // Use project manager's current searcher if available, otherwise fall back to constructor searcher
    const activeSearcher = this.projectManager?.getCurrentSearcher() || this.searcher;
    if (!activeSearcher) {
      throw new Error('No active project or searcher available. Use switch_project to set up a project.');
    }

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

    const result = await activeSearcher.search(query);
    
    // Create relationship analysis specific response
    const enhancedResult = {
      ...result,
      analysis_type,
      starting_symbols,
      target_symbols,
      relationships_found: result.metadata?.relationship_paths || [],
      visualization: `Analysis of ${analysis_type} for symbols: ${starting_symbols.join(', ')}`,
      confidence_scores: result.metadata?.confidence_scores || [],
      context_optimization: {
        tool_used: 'relationship_analysis',
        analysis_type: analysis_type,
        starting_symbols: starting_symbols,
        response_optimized_for: 'mcp_client'
      }
    };

    // Use shared MCP optimization
    return this.optimizeForMCP(enhancedResult, params);
  }
}

export class TraceExecutionPathHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher, private projectManager?: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    // Use project manager's current searcher if available, otherwise fall back to constructor searcher
    const activeSearcher = this.projectManager?.getCurrentSearcher() || this.searcher;
    if (!activeSearcher) {
      throw new Error('No active project or searcher available. Use switch_project to set up a project.');
    }

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

    const result = await activeSearcher.search(query);
    
    // Create execution path specific response
    const enhancedResult = {
      ...result,
      entry_point,
      trace_type,
      execution_path: result.metadata?.relationship_paths || [],
      data_flow_included: include_data_flow,
      execution_summary: `Execution trace from ${entry_point} with ${result.context_chunks?.length || 0} steps discovered`,
      context_optimization: {
        tool_used: 'trace_execution_path',
        entry_point: entry_point,
        trace_type: trace_type,
        response_optimized_for: 'mcp_client'
      }
    };

    // Use shared MCP optimization
    return this.optimizeForMCP(enhancedResult, params);
  }
}

export class FindCodePatternsHandler extends BaseHandler {
  constructor(private searcher: SemanticSearcher, private projectManager?: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    // Use project manager's current searcher if available, otherwise fall back to constructor searcher
    const activeSearcher = this.projectManager?.getCurrentSearcher() || this.searcher;
    if (!activeSearcher) {
      throw new Error('No active project or searcher available. Use switch_project to set up a project.');
    }

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

    const result = await activeSearcher.search(query);
    
    // Create pattern finding specific response
    const enhancedResult = {
      ...result,
      pattern_type,
      pattern_description: searchTask,
      scope,
      patterns_found: (result.context_chunks || []).map((chunk: any, index: number) => ({
        file: chunk.file_path,
        line_range: `${chunk.start_line}-${chunk.end_line}`,
        confidence: chunk.similarity_score || chunk.relevance_score,
        description: `Pattern match ${index + 1}: ${chunk.function_name || chunk.symbol_name || 'code block'}`,
        code_excerpt: chunk.content.substring(0, 200) + '...'
      })),
      total_matches: result.context_chunks?.length || 0,
      confidence_scores: result.metadata?.confidence_scores || [],
      context_optimization: {
        tool_used: 'find_code_patterns',
        pattern_type: pattern_type,
        scope: scope,
        response_optimized_for: 'mcp_client'
      }
    };

    // Use shared MCP optimization
    return this.optimizeForMCP(enhancedResult, params);
  }
}

export class RealTimeStatusHandler extends BaseHandler {
  constructor(private indexer: CodebaseIndexer) {
    super();
  }

  async handle(params: any): Promise<any> {
    const stats = this.indexer.getRealTimeStats();
    
    // Ultra-minimal status for Claude Code
    return {
      status: stats.isWatching ? 'active' : 'disabled',
      context_freshness: stats.invalidatedChunks === 0 ? 'fresh' : 'stale',
      pending_updates: stats.invalidatedChunks
    };
  }
}

export class FetchChunkHandler extends BaseHandler {
  constructor() {
    super();
  }

  async handle(params: any): Promise<string> {
    const { cacheKey, chunkIndex } = params;
    
    if (!cacheKey || typeof cacheKey !== 'string') {
      return 'Error: cacheKey is required and must be a string';
    }
    
    if (!chunkIndex || typeof chunkIndex !== 'number' || chunkIndex < 1) {
      return 'Error: chunkIndex is required and must be a positive number';
    }
    
    const cached = getCachedChunk(cacheKey, chunkIndex);
    if (!cached) {
      return `No cached chunk ${chunkIndex} for key ${cacheKey}. The cache may have expired or the chunk index is invalid.`;
    }
    
    return `chunk: ${chunkIndex}/${cached.total} (cacheKey: ${cacheKey})\n\n${cached.chunk}`;
  }
}

export class NextChunkHandler extends BaseHandler {
  constructor() {
    super();
  }

  async handle(params: any): Promise<string> {
    const { cacheKey } = params;
    
    if (!cacheKey || typeof cacheKey !== 'string') {
      return 'Error: cacheKey is required and must be a string';
    }
    
    const next = getNextChunk(cacheKey);
    if (!next) {
      return `No further chunks available for key ${cacheKey}. You may have reached the end or the cache may have expired.`;
    }
    
    return `chunk: ${next.index}/${next.total} (cacheKey: ${cacheKey})\n\n${next.chunk}`;
  }
}

export class GetCurrentProjectHandler extends BaseHandler {
  constructor(private projectManager: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    const currentProject = this.projectManager.getCurrentProject();
    
    if (!currentProject) {
      return {
        status: 'no_project',
        message: 'No project is currently active. Use switch_project or add_project to get started.',
        available_projects: (await this.projectManager.listProjects(false)).length
      };
    }

    // Get project stats if available
    const stats = await this.projectManager.getProjectStats(currentProject.name);
    
    return {
      project_name: currentProject.name,
      project_path: currentProject.path,
      index_status: currentProject.indexStatus,
      last_accessed: new Date(currentProject.lastAccessed).toISOString(),
      stats: stats ? {
        file_count: stats.fileCount,
        chunk_count: stats.chunkCount,
        last_indexed: new Date(stats.lastIndexed).toISOString()
      } : null
    };
  }
}

export class ListAvailableProjectsHandler extends BaseHandler {
  constructor(private projectManager: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    const includeStats = params.include_stats !== false;
    const projects = await this.projectManager.listProjects(includeStats);
    const currentProject = this.projectManager.getCurrentProject();

    return {
      total_projects: projects.length,
      current_project: currentProject?.name || null,
      projects: projects.map(project => ({
        name: project.name,
        path: project.path,
        index_status: project.indexStatus,
        is_current: currentProject?.name === project.name,
        added_at: new Date(project.addedAt).toISOString(),
        last_accessed: new Date(project.lastAccessed).toISOString(),
        ...(includeStats && {
          file_count: project.fileCount,
          index_size: project.indexSize
        }),
        ...(project.error && { error: project.error })
      }))
    };
  }
}

export class SwitchProjectHandler extends BaseHandler {
  constructor(private projectManager: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    const { project_path, project_name, auto_index } = params;
    
    if (!project_path) {
      return {
        error: 'project_path is required'
      };
    }

    try {
      const projectInfo = await this.projectManager.switchProject(
        project_path, 
        project_name, 
        auto_index !== false
      );

      // Get updated stats
      const stats = await this.projectManager.getProjectStats(projectInfo.name);
      
      return {
        success: true,
        project_name: projectInfo.name,
        project_path: projectInfo.path,
        index_status: projectInfo.indexStatus,
        switched_at: new Date().toISOString(),
        stats: stats ? {
          file_count: stats.fileCount,
          chunk_count: stats.chunkCount
        } : null,
        message: `Successfully switched to project: ${projectInfo.name}`
      };

    } catch (error) {
      return {
        error: error instanceof Error ? error.message : 'Unknown error',
        success: false
      };
    }
  }
}

export class AddProjectHandler extends BaseHandler {
  constructor(private projectManager: ProjectManager) {
    super();
  }

  async handle(params: any): Promise<any> {
    const { project_path, project_name, start_indexing } = params;
    
    if (!project_path) {
      return {
        error: 'project_path is required'
      };
    }

    try {
      const projectInfo = await this.projectManager.addProject(
        project_path, 
        project_name, 
        start_indexing !== false
      );

      // Get stats if indexing completed
      const stats = await this.projectManager.getProjectStats(projectInfo.name);
      
      return {
        success: true,
        project_name: projectInfo.name,
        project_path: projectInfo.path,
        index_status: projectInfo.indexStatus,
        added_at: new Date(projectInfo.addedAt).toISOString(),
        stats: stats ? {
          file_count: stats.fileCount,
          chunk_count: stats.chunkCount
        } : null,
        message: `Successfully added project: ${projectInfo.name}`
      };

    } catch (error) {
      return {
        error: error instanceof Error ? error.message : 'Unknown error',
        success: false
      };
    }
  }
}