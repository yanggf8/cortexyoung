import axios, { AxiosInstance, AxiosRequestConfig } from 'axios';
import { CodeChunk, EmbedOptions, EmbeddingResult, ProviderHealth, ProviderMetrics } from './types';
import { log, warn, error } from './logging-utils';

interface ClientConfig {
  serverUrl: string;
  timeout?: number;
  retries?: number;
  clientId?: string;
  projectPath?: string;
}

interface SemanticSearchOptions {
  maxChunks?: number;
  fileFilters?: string[];
  recencyWeight?: number;
  includeTests?: boolean;
}

interface CodeIntelligenceOptions {
  maxChunks?: number;
  context?: string;
}

interface RelationshipAnalysisOptions {
  analysisType: 'call_graph' | 'dependency_map' | 'data_flow' | 'impact_analysis';
  startingSymbols?: string[];
  maxDepth?: number;
  includeTests?: boolean;
}

interface TraceExecutionOptions {
  targetFunction?: string;
  maxDepth?: number;
  includeAsync?: boolean;
}

interface FindCodePatternsOptions {
  patternType: 'structural' | 'behavioral' | 'architectural';
}

interface CentralizedResponse<T = any> {
  success: boolean;
  data?: T;
  error?: string;
  metadata: {
    processingTime: number;
    contextEnhanced: boolean;
    clientId?: string;
    projectPath?: string;
    timestamp: number;
  };
}

/**
 * HTTP Client for Cortex Centralized Embedding Server
 * 
 * Provides a clean interface for MCP servers to communicate
 * with the centralized HTTP embedding server
 */
export class EmbeddingClient {
  private httpClient: AxiosInstance;
  private config: ClientConfig;
  private circuitBreaker: {
    failures: number;
    lastFailureTime: number;
    state: 'closed' | 'open' | 'half-open';
  };

  constructor(config: ClientConfig) {
    this.config = {
      timeout: 30000,
      retries: 3,
      ...config
    };

    this.httpClient = axios.create({
      baseURL: this.config.serverUrl,
      timeout: this.config.timeout,
      headers: {
        'Content-Type': 'application/json',
        'User-Agent': 'Cortex-MCP-Client/3.0',
        ...(this.config.clientId && { 'X-Client-Id': this.config.clientId }),
        ...(this.config.projectPath && { 'X-Project-Path': this.config.projectPath })
      }
    });

    this.circuitBreaker = {
      failures: 0,
      lastFailureTime: 0,
      state: 'closed'
    };

    this.setupRequestInterceptors();
    this.setupResponseInterceptors();
  }

  /**
   * Semantic search with context enhancement
   */
  async semanticSearch(
    query: string,
    options: SemanticSearchOptions = {}
  ): Promise<CentralizedResponse> {
    return this.makeRequest('/semantic-search-enhanced', 'POST', {
      query,
      maxChunks: options.maxChunks || 5,
      fileFilters: options.fileFilters,
      recencyWeight: options.recencyWeight,
      includeTests: options.includeTests,
      projectPath: this.config.projectPath,
      clientId: this.config.clientId
    });
  }

  /**
   * Code intelligence analysis
   */
  async codeIntelligence(
    task: string,
    options: CodeIntelligenceOptions = {}
  ): Promise<CentralizedResponse> {
    return this.makeRequest('/code-intelligence', 'POST', {
      task,
      context: options.context,
      maxChunks: options.maxChunks || 10,
      projectPath: this.config.projectPath,
      clientId: this.config.clientId
    });
  }

  /**
   * Relationship analysis
   */
  async relationshipAnalysis(
    options: RelationshipAnalysisOptions
  ): Promise<CentralizedResponse> {
    return this.makeRequest('/relationship-analysis', 'POST', {
      analysisType: options.analysisType,
      startingSymbols: options.startingSymbols || [],
      maxDepth: options.maxDepth || 3,
      includeTests: options.includeTests || false,
      projectPath: this.config.projectPath,
      clientId: this.config.clientId
    });
  }

  /**
   * Trace execution path
   */
  async traceExecutionPath(
    entryPoint: string,
    options: TraceExecutionOptions = {}
  ): Promise<CentralizedResponse> {
    return this.makeRequest('/trace-execution-path', 'POST', {
      entryPoint,
      targetFunction: options.targetFunction,
      maxDepth: options.maxDepth || 5,
      includeAsync: options.includeAsync || false,
      projectPath: this.config.projectPath,
      clientId: this.config.clientId
    });
  }

  /**
   * Find code patterns
   */
  async findCodePatterns(
    pattern: string,
    options: FindCodePatternsOptions
  ): Promise<CentralizedResponse> {
    return this.makeRequest('/find-code-patterns', 'POST', {
      pattern,
      patternType: options.patternType,
      projectPath: this.config.projectPath,
      clientId: this.config.clientId
    });
  }

  /**
   * Generate embeddings
   */
  async embedBatch(texts: string[], options?: EmbedOptions): Promise<CentralizedResponse> {
    return this.makeRequest('/embed', 'POST', {
      texts,
      options
    });
  }

  /**
   * Health check
   */
  async healthCheck(): Promise<CentralizedResponse> {
    return this.makeRequest('/health', 'GET');
  }

  /**
   * Get server status
   */
  async getStatus(): Promise<CentralizedResponse> {
    return this.makeRequest('/status', 'GET');
  }

  /**
   * Get server metrics
   */
  async getMetrics(): Promise<CentralizedResponse> {
    return this.makeRequest('/metrics', 'GET');
  }

  /**
   * Test connection to server
   */
  async testConnection(): Promise<boolean> {
    try {
      const response = await this.healthCheck();
      return response.success;
    } catch (error) {
      warn(`[EmbeddingClient] Connection test failed: ${error.message}`);
      return false;
    }
  }

  /**
   * Update client configuration
   */
  updateConfig(updates: Partial<ClientConfig>): void {
    this.config = { ...this.config, ...updates };
    
    // Update headers if needed
    const headers = this.httpClient.defaults.headers;
    if (updates.clientId) {
      headers['X-Client-Id'] = updates.clientId;
    }
    if (updates.projectPath) {
      headers['X-Project-Path'] = updates.projectPath;
    }
  }

  /**
   * Get current configuration
   */
  getConfig(): ClientConfig {
    return { ...this.config };
  }

  // Private methods

  private async makeRequest(
    url: string,
    method: 'GET' | 'POST' = 'GET',
    data?: any
  ): Promise<CentralizedResponse> {
    // Check circuit breaker
    if (this.isCircuitOpen()) {
      throw new Error('Circuit breaker is open - server may be unavailable');
    }

    const startTime = Date.now();
    let attempt = 0;
    const maxRetries = this.config.retries || 3;

    while (attempt < maxRetries) {
      try {
        const requestConfig: AxiosRequestConfig = {
          method,
          url,
          ...(data && { data })
        };

        log(`[EmbeddingClient] ${method} ${url} (attempt ${attempt + 1})`);
        const response = await this.httpClient.request(requestConfig);

        // Reset circuit breaker on success
        this.resetCircuitBreaker();

        const processingTime = Date.now() - startTime;
        log(`[EmbeddingClient] Request completed in ${processingTime}ms`);

        return response.data;

      } catch (error) {
        attempt++;
        this.recordFailure();

        if (attempt >= maxRetries) {
          const processingTime = Date.now() - startTime;
          error(`[EmbeddingClient] Request failed after ${maxRetries} attempts: ${error.message}`);
          
          return {
            success: false,
            error: error.message,
            metadata: {
              processingTime,
              contextEnhanced: false,
              clientId: this.config.clientId,
              projectPath: this.config.projectPath,
              timestamp: Date.now()
            }
          };
        }

        // Exponential backoff for retries
        const backoffDelay = Math.min(1000 * Math.pow(2, attempt - 1), 5000);
        await this.delay(backoffDelay);
      }
    }

    // This should never be reached due to the throw above, but TypeScript requires it
    throw new Error('Unexpected end of makeRequest method');
  }

  private setupRequestInterceptors(): void {
    this.httpClient.interceptors.request.use(
      (config) => {
        // Add request ID for tracking
        config.headers['X-Request-Id'] = this.generateRequestId();
        config.metadata = { startTime: Date.now() };
        return config;
      },
      (error) => Promise.reject(error)
    );
  }

  private setupResponseInterceptors(): void {
    this.httpClient.interceptors.response.use(
      (response) => {
        const duration = Date.now() - (response.config as any).metadata?.startTime;
        log(`[EmbeddingClient] Response received in ${duration}ms`);
        return response;
      },
      (error) => {
        if (error.response) {
          // Server responded with error status
          error(`[EmbeddingClient] Server error: ${error.response.status} ${error.response.statusText}`);
        } else if (error.request) {
          // No response received
          error(`[EmbeddingClient] Network error: ${error.message}`);
        } else {
          // Request configuration error
          error(`[EmbeddingClient] Request error: ${error.message}`);
        }
        return Promise.reject(error);
      }
    );
  }

  // Circuit breaker implementation
  
  private isCircuitOpen(): boolean {
    const now = Date.now();
    const timeSinceLastFailure = now - this.circuitBreaker.lastFailureTime;

    if (this.circuitBreaker.state === 'open') {
      // Try to half-open after 60 seconds
      if (timeSinceLastFailure > 60000) {
        this.circuitBreaker.state = 'half-open';
        log('[EmbeddingClient] Circuit breaker half-open - testing connection');
        return false;
      }
      return true;
    }

    return false;
  }

  private recordFailure(): void {
    this.circuitBreaker.failures++;
    this.circuitBreaker.lastFailureTime = Date.now();

    // Open circuit after 5 consecutive failures
    if (this.circuitBreaker.failures >= 5 && this.circuitBreaker.state === 'closed') {
      this.circuitBreaker.state = 'open';
      warn('[EmbeddingClient] Circuit breaker opened due to consecutive failures');
    }
  }

  private resetCircuitBreaker(): void {
    if (this.circuitBreaker.failures > 0) {
      log('[EmbeddingClient] Circuit breaker reset - connection restored');
    }
    this.circuitBreaker.failures = 0;
    this.circuitBreaker.state = 'closed';
  }

  // Utility methods

  private generateRequestId(): string {
    return `req_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  }

  private delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}

/**
 * Factory function to create an embedding client
 */
export function createEmbeddingClient(config: ClientConfig): EmbeddingClient {
  return new EmbeddingClient(config);
}

/**
 * Default client for local development
 */
export function createLocalEmbeddingClient(
  clientId?: string,
  projectPath?: string
): EmbeddingClient {
  return new EmbeddingClient({
    serverUrl: 'http://localhost:8766',
    clientId,
    projectPath,
    timeout: 30000,
    retries: 3
  });
}

/**
 * High-performance client with optimized settings
 */
export function createOptimizedEmbeddingClient(
  serverUrl: string,
  clientId?: string,
  projectPath?: string
): EmbeddingClient {
  return new EmbeddingClient({
    serverUrl,
    clientId,
    projectPath,
    timeout: 60000,  // Longer timeout for complex operations
    retries: 5       // More retries for reliability
  });
}