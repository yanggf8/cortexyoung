// ===================== V3.0 LIGHTWEIGHT HANDLERS =====================
// These handlers use HTTP client to communicate with centralized embedding server
// with local caching and graceful degradation for performance and reliability

import { BaseHandler } from './mcp-handlers';
import { EmbeddingClient } from './embedding-client';
import { ProjectManager } from './project-manager';
import { warn } from './logging-utils';

/**
 * Base class for lightweight handlers with HTTP client, caching, and fallback
 */
export abstract class LightweightBaseHandler extends BaseHandler {
  constructor(
    protected embeddingClient: EmbeddingClient, 
    protected projectManager: ProjectManager,
    protected server: any // LightweightCortexMCPServer instance for cache access
  ) {
    super();
  }

  protected async makeRequestWithCache(
    operation: string,
    params: any,
    cacheKeyBase: string,
    fallbackFn?: () => Promise<any>
  ): Promise<any> {
    // Generate cache key
    const cacheKey = `${cacheKeyBase}_${JSON.stringify(params).slice(0, 100)}`;
    
    // Try cache first
    const cached = this.server.getCachedResult(cacheKey);
    if (cached) {
      return this.optimizeForMCP(cached, params);
    }

    try {
      // Try centralized server request
      const method = this.embeddingClient[operation as keyof EmbeddingClient] as Function;
      const response = await method.call(this.embeddingClient, ...Object.values(params));
      
      if (response.success && response.data) {
        // Cache successful response
        this.server.setCachedResult(cacheKey, response.data, 300000); // 5min TTL
        return this.optimizeForMCP(response.data, params);
      } else {
        throw new Error(response.error || 'Server returned unsuccessful response');
      }
    } catch (error: any) {
      warn(`[${operation}] Centralized server request failed: ${error.message}`);
      
      // Try fallback if available
      if (fallbackFn && !this.server.isInFallbackMode()) {
        try {
          const fallbackResult = await fallbackFn();
          // Cache fallback result with shorter TTL
          this.server.setCachedResult(cacheKey, fallbackResult, 60000); // 1min TTL
          return this.optimizeForMCP(fallbackResult, params);
        } catch (fallbackError: any) {
          warn(`[${operation}] Fallback also failed: ${fallbackError.message}`);
        }
      }
      
      // Return minimal error response
      return {
        error: `Service temporarily unavailable: ${error.message}`,
        fallback_available: !!fallbackFn,
        suggested_action: fallbackFn ? 'Try again later or use basic mode' : 'Check centralized server status'
      };
    }
  }
}

export class LightweightSemanticSearchHandler extends LightweightBaseHandler {
  async handle(params: any): Promise<any> {
    return this.makeRequestWithCache(
      'semanticSearch',
      {
        query: params.query,
        maxChunks: params.max_chunks || 5,
        fileFilters: params.file_filters,
        includeTests: params.include_tests
      },
      `semantic_search_${params.query}`,
      // Fallback: Basic text search if centralized server unavailable
      async () => {
        return {
          chunks: [],
          summary: `Basic search for: ${params.query}`,
          files: [],
          fallback_mode: true,
          message: 'Results from basic fallback mode - limited functionality'
        };
      }
    );
  }
}

export class LightweightContextualReadHandler extends LightweightBaseHandler {
  async handle(params: any): Promise<any> {
    // For contextual read, we can provide basic file reading as fallback
    const fallbackFn = async () => {
      const fs = require('fs');
      const path = require('path');
      
      try {
        const fullPath = path.resolve(params.file_path);
        const content = fs.readFileSync(fullPath, 'utf-8');
        
        return {
          file_path: params.file_path,
          content: content,
          semantic_context: params.semantic_context || 'Basic file read (fallback mode)',
          token_estimate: Math.floor(content.length / 4), // rough estimate
          fallback_mode: true
        };
      } catch (error: any) {
        throw new Error(`Cannot read file: ${error.message}`);
      }
    };

    return this.makeRequestWithCache(
      'contextualRead',
      {
        filePath: params.file_path,
        semanticContext: params.semantic_context,
        maxContextTokens: params.max_context_tokens
      },
      `contextual_read_${params.file_path}`,
      fallbackFn
    );
  }
}

export class LightweightCodeIntelligenceHandler extends LightweightBaseHandler {
  async handle(params: any): Promise<any> {
    return this.makeRequestWithCache(
      'codeIntelligence',
      {
        task: params.task,
        context: params.context,
        maxChunks: params.max_context_tokens ? Math.floor(params.max_context_tokens / 200) : 10
      },
      `code_intelligence_${params.task}`,
      // Fallback: Return basic task acknowledgment
      async () => {
        return {
          chunks: [],
          summary: `Code intelligence request: ${params.task}`,
          analysis_type: 'basic',
          focus_areas: params.focus_areas || [],
          fallback_mode: true,
          message: 'Centralized server unavailable - using basic mode'
        };
      }
    );
  }
}

export class LightweightRelationshipAnalysisHandler extends LightweightBaseHandler {
  async handle(params: any): Promise<any> {
    return this.makeRequestWithCache(
      'relationshipAnalysis',
      {
        analysisType: params.analysis_type,
        startingSymbols: params.starting_symbols,
        maxDepth: params.max_depth,
        includeTests: params.include_tests
      },
      `relationship_analysis_${params.analysis_type}_${params.starting_symbols?.join('_')}`,
      // Fallback: Basic symbol listing
      async () => {
        return {
          analysis_type: params.analysis_type,
          starting_symbols: params.starting_symbols || [],
          relationships_found: [],
          visualization: `Basic relationship analysis for: ${params.starting_symbols?.join(', ') || 'symbols'}`,
          fallback_mode: true,
          chunks: [],
          summary: 'Relationship analysis unavailable - centralized server required'
        };
      }
    );
  }
}

export class LightweightTraceExecutionPathHandler extends LightweightBaseHandler {
  async handle(params: any): Promise<any> {
    return this.makeRequestWithCache(
      'traceExecutionPath',
      {
        entryPoint: params.entry_point,
        targetFunction: params.target_function,
        maxDepth: params.max_execution_depth,
        includeAsync: params.include_data_flow
      },
      `trace_execution_${params.entry_point}`,
      // Fallback: Basic execution trace stub
      async () => {
        return {
          entry_point: params.entry_point,
          execution_path: [],
          execution_summary: `Execution trace from ${params.entry_point} - centralized server required`,
          fallback_mode: true,
          chunks: [],
          summary: 'Execution path tracing unavailable in fallback mode'
        };
      }
    );
  }
}

export class LightweightFindCodePatternsHandler extends LightweightBaseHandler {
  async handle(params: any): Promise<any> {
    return this.makeRequestWithCache(
      'findCodePatterns',
      {
        pattern: params.pattern_description || `${params.pattern_type} patterns`,
        patternType: params.pattern_type
      },
      `find_patterns_${params.pattern_type}_${params.pattern_description}`,
      // Fallback: Basic pattern search stub
      async () => {
        return {
          pattern_type: params.pattern_type,
          pattern_description: params.pattern_description,
          patterns_found: [],
          total_matches: 0,
          fallback_mode: true,
          chunks: [],
          summary: 'Pattern analysis requires centralized server - unavailable in fallback mode'
        };
      }
    );
  }
}

export class LightweightRealTimeStatusHandler extends BaseHandler {
  constructor(
    private embeddingClient: EmbeddingClient,
    private server: any
  ) {
    super();
  }

  async handle(params: any): Promise<any> {
    // Check centralized server status instead of local real-time watching
    try {
      const status = await this.embeddingClient.getStatus();
      
      if (status.success) {
        return {
          status: 'active',
          context_freshness: 'managed_by_centralized_server',
          server_status: 'connected',
          fallback_mode: this.server.isInFallbackMode()
        };
      } else {
        return {
          status: 'degraded',
          context_freshness: 'unknown',
          server_status: 'disconnected',
          fallback_mode: true
        };
      }
    } catch (error: any) {
      return {
        status: 'offline',
        context_freshness: 'unavailable',
        server_status: 'disconnected',
        fallback_mode: true,
        error: 'Cannot connect to centralized server'
      };
    }
  }
}