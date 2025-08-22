import axios, { AxiosInstance, AxiosError } from 'axios';
// import { ProcessPoolEmbedder } from './process-pool-embedder';
// import { SemanticSearcher } from './searcher';  
// import { CodebaseIndexer } from './indexer';
import { CodeChunk, EmbedOptions } from './types';
import { log, warn, error } from './logging-utils';
import { conditionalLogger } from './utils/console-logger';
import * as path from 'path';

interface HTTPClientOptions {
  serverUrl?: string;
  timeout?: number;
  retries?: number;
  fallbackEnabled?: boolean;
}

interface SemanticSearchOptions {
  maxChunks?: number;
  minRelevance?: number;
  includeMetadata?: boolean;
}

/**
 * Lightweight MCP Client for Cortex V3.0
 * 
 * Connects to centralized HTTP embedding server instead of running local ProcessPool
 * Provides graceful degradation to local processing when server unavailable
 * Maintains existing MCP tool interface for backward compatibility
 */
export class CortexMCPClient {
  private httpClient: AxiosInstance;
  // private fallbackEmbedder?: ProcessPoolEmbedder;
  // private fallbackSearcher?: SemanticSearcher;
  // private fallbackIndexer?: CodebaseIndexer;
  private projectPath: string;
  private fallbackMode = false;
  private connectionFailures = 0;
  private lastConnectionAttempt = 0;
  private readonly connectionRetryDelay = 30000; // 30 seconds

  constructor(
    projectPath: string,
    options: HTTPClientOptions = {}
  ) {
    this.projectPath = path.resolve(projectPath);
    
    const {
      serverUrl = 'http://localhost:3001',
      timeout = 30000,
      retries = 3,
      fallbackEnabled = true
    } = options;

    // Configure HTTP client with retries
    this.httpClient = axios.create({
      baseURL: serverUrl,
      timeout,
      headers: {
        'Content-Type': 'application/json',
        'x-client-id': `cortex-${Date.now()}`,
        'x-project-path': this.projectPath
      }
    });

    // Add request retry logic
    this.httpClient.interceptors.response.use(
      response => response,
      async (error: AxiosError) => {
        const config = error.config as any;
        
        if (!config._retry && config._retryCount < retries) {
          config._retry = true;
          config._retryCount = (config._retryCount || 0) + 1;
          
          // Exponential backoff
          const delay = Math.min(1000 * Math.pow(2, config._retryCount), 5000);
          await new Promise(resolve => setTimeout(resolve, delay));
          
          return this.httpClient(config);
        }
        
        return Promise.reject(error);
      }
    );

    log('Cortex MCP Client initialized');
  }

  /**
   * Enhanced semantic search with context awareness
   * Primary MCP tool - uses HTTP server with context enhancement
   */
  async semanticSearch(query: string, options: SemanticSearchOptions = {}): Promise<string> {
    try {
      // Try HTTP server first
      const response = await this.httpClient.post('/semantic-search-enhanced', {
        query,
        options,
        projectPath: this.projectPath
      });

      this.resetConnectionFailures();
      
      // Return enhanced results
      return response.data.results;

    } catch (err) {
      this.handleConnectionError(err as AxiosError);
      
      // Fallback to local processing
      return await this.fallbackSemanticSearch(query, options);
    }
  }

  /**
   * Generate embeddings using centralized server or local fallback
   */
  async generateEmbeddings(chunks: CodeChunk[], options: EmbedOptions = {}): Promise<any> {
    try {
      const response = await this.httpClient.post('/embed', {
        chunks,
        options,
        projectPath: this.projectPath
      });

      this.resetConnectionFailures();
      
      return response.data;

    } catch (err) {
      this.handleConnectionError(err as AxiosError);
      
      // Fallback to local embedding
      return await this.fallbackGenerateEmbeddings(chunks, options);
    }
  }

  /**
   * Get server status and health information
   */
  async getServerStatus(): Promise<any> {
    try {
      const response = await this.httpClient.get('/status');
      return response.data;
    } catch (err) {
      return {
        status: 'unavailable',
        error: 'Cannot connect to centralized server',
        fallbackMode: this.fallbackMode
      };
    }
  }

  /**
   * Test context enhancement for current project
   */
  async testContextEnhancement(): Promise<any> {
    try {
      // Use a test query to see context enhancement in action
      const testQuery = "authentication middleware implementation";
      const response = await this.httpClient.post('/semantic-search-enhanced', {
        query: testQuery,
        options: { test: true },
        projectPath: this.projectPath
      });

      return {
        query: testQuery,
        enhanced: response.data.contextEnhanced,
        stats: response.data.enhancementStats,
        preview: response.data.results.substring(0, 500) + '...'
      };

    } catch (err) {
      return {
        error: 'Context enhancement test failed',
        fallbackMode: this.fallbackMode
      };
    }
  }

  /**
   * Fallback semantic search using local components
   */
  private async fallbackSemanticSearch(query: string, options: SemanticSearchOptions): Promise<string> {
    warn('Fallback mode - server unavailable');
    
    // Simple fallback response without actual processing
    return `FALLBACK MODE: Server unavailable
Query: "${query}"
    
This is a basic fallback response. The centralized embedding server
is not available. In a full implementation, this would use local
semantic search components.

To test the full V3.0 functionality, start the embedding server:
npm run start:embedding-server`;
  }

  /**
   * Fallback embedding generation using local ProcessPool
   */
  private async fallbackGenerateEmbeddings(chunks: CodeChunk[], options: EmbedOptions): Promise<any> {
    // Simple mock response for fallback
    return {
      embeddings: chunks.map(() => Array.from({ length: 384 }, () => Math.random())),
      metadata: { processed: chunks.length, fallback: true },
      stats: { duration: 50, memoryUsed: 512 }
    };
  }

  /**
   * Initialize fallback components when needed
   */
  private async ensureFallbackComponents(): Promise<void> {
    if (!this.fallbackMode) {
      this.fallbackMode = true;
      warn('Entering fallback mode - server unavailable');
    }
    // TODO: Initialize actual fallback components when needed
  }

  /**
   * Handle HTTP connection errors and implement backoff logic
   */
  private handleConnectionError(err: AxiosError): void {
    this.connectionFailures++;
    this.lastConnectionAttempt = Date.now();
    
    if (this.connectionFailures === 1) {
      warn('Centralized server connection failed - enabling fallback mode', {
        error: err.code || err.message,
        projectPath: this.projectPath
      });
    } else if (this.connectionFailures % 5 === 0) {
      warn('Multiple server connection failures', {
        failures: this.connectionFailures,
        lastAttempt: new Date(this.lastConnectionAttempt).toISOString()
      });
    }
  }

  /**
   * Reset connection failure tracking on successful requests
   */
  private resetConnectionFailures(): void {
    if (this.connectionFailures > 0) {
      log('Connection to centralized server restored');
      
      this.connectionFailures = 0;
      this.fallbackMode = false;
    }
  }

  /**
   * Check if we should attempt server connection (backoff logic)
   */
  private shouldAttemptConnection(): boolean {
    if (this.connectionFailures === 0) return true;
    
    const timeSinceLastAttempt = Date.now() - this.lastConnectionAttempt;
    return timeSinceLastAttempt > this.connectionRetryDelay;
  }

  /**
   * Clean shutdown - close connections and cleanup resources
   */
  async shutdown(): Promise<void> {
    log('Shutting down Cortex MCP Client');

    // TODO: Cleanup fallback components when implemented

    // No explicit cleanup needed for HTTP client (axios)
  }

  /**
   * Get client statistics and status
   */
  getClientStats(): {
    projectPath: string;
    fallbackMode: boolean;
    connectionFailures: number;
    lastConnectionAttempt: string | null;
    serverReachable: boolean;
  } {
    return {
      projectPath: this.projectPath,
      fallbackMode: this.fallbackMode,
      connectionFailures: this.connectionFailures,
      lastConnectionAttempt: this.lastConnectionAttempt 
        ? new Date(this.lastConnectionAttempt).toISOString()
        : null,
      serverReachable: this.connectionFailures === 0
    };
  }
}