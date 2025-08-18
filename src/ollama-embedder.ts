import * as http from 'http';
import { IEmbedder, EmbedOptions, EmbeddingResult, EmbeddingMetadata, PerformanceStats, ProviderHealth, ProviderMetrics } from './types';
import { log, warn, error } from './logging-utils';

export interface OllamaConfig {
  host?: string;
  port?: number;
  model?: string;
  timeout?: number;
}

export class OllamaEmbedder implements IEmbedder {
  readonly providerId = 'ollama';
  readonly modelId: string;
  readonly dimensions = 768; // nomic-embed-text dimensions
  readonly maxBatchSize = 32; // Conservative batch size for Ollama
  readonly normalization = 'l2' as const;

  private host: string;
  private port: number;
  private timeout: number;
  private isInitialized = false;
  private requestCount = 0;
  private totalDuration = 0;
  private errorCount = 0;
  private lastSuccess = 0;

  constructor(config: OllamaConfig = {}) {
    this.host = config.host || 'localhost';
    this.port = config.port || 11434;
    this.modelId = config.model || 'nomic-embed-text:latest';
    this.timeout = config.timeout || 30000; // 30s timeout
  }

  async initialize(): Promise<void> {
    log(`[OllamaEmbedder] Initializing with model: ${this.modelId}`);
    
    try {
      // Test connection and model availability
      const health = await this.getHealth();
      if (health.status !== 'healthy') {
        throw new Error(`Ollama not healthy: ${health.details}`);
      }
      
      this.isInitialized = true;
      log(`[OllamaEmbedder] Successfully initialized`);
    } catch (err) {
      error(`[OllamaEmbedder] Initialization failed: ${err instanceof Error ? err.message : err}`);
      throw err;
    }
  }

  async embedBatch(texts: string[], options?: EmbedOptions): Promise<EmbeddingResult> {
    if (!this.isInitialized) {
      await this.initialize();
    }

    const startTime = Date.now();
    const embeddings: number[][] = [];
    
    log(`[OllamaEmbedder] Processing batch of ${texts.length} texts`);

    try {
      // Process texts sequentially for now (can optimize to parallel later)
      for (let i = 0; i < texts.length; i++) {
        const text = texts[i];
        const embedding = await this.generateSingleEmbedding(text);
        embeddings.push(embedding);
        
        if ((i + 1) % 10 === 0 || i === texts.length - 1) {
          log(`[OllamaEmbedder] Processed ${i + 1}/${texts.length} embeddings`);
        }
      }

      const duration = Date.now() - startTime;
      this.requestCount++;
      this.totalDuration += duration;
      this.lastSuccess = Date.now();

      const metadata: EmbeddingMetadata = {
        providerId: this.providerId,
        modelId: this.modelId,
        batchSize: texts.length,
        processedAt: Date.now(),
        requestId: options?.requestId
      };

      const performance: PerformanceStats = {
        duration,
        memoryDelta: 0, // Could measure this if needed
        processId: process.pid
      };

      log(`[OllamaEmbedder] Batch completed in ${duration}ms`);

      return {
        embeddings,
        metadata,
        performance
      };

    } catch (err) {
      this.errorCount++;
      error(`[OllamaEmbedder] Batch processing failed: ${err instanceof Error ? err.message : err}`);
      throw err;
    }
  }

  private async generateSingleEmbedding(text: string): Promise<number[]> {
    return new Promise((resolve, reject) => {
      const postData = JSON.stringify({
        model: this.modelId,
        prompt: text
      });

      const options = {
        hostname: this.host,
        port: this.port,
        path: '/api/embeddings',
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(postData)
        },
        timeout: this.timeout
      };

      const req = http.request(options, (res) => {
        let data = '';
        
        res.on('data', (chunk) => {
          data += chunk;
        });
        
        res.on('end', () => {
          try {
            const response = JSON.parse(data);
            
            if (response.embedding && Array.isArray(response.embedding)) {
              resolve(response.embedding);
            } else {
              reject(new Error(`Invalid response format: ${JSON.stringify(response)}`));
            }
          } catch (parseError) {
            reject(new Error(`Failed to parse response: ${parseError instanceof Error ? parseError.message : parseError}`));
          }
        });
      });

      req.on('error', (err) => {
        reject(new Error(`HTTP request failed: ${err.message}`));
      });

      req.on('timeout', () => {
        req.destroy();
        reject(new Error(`Request timeout after ${this.timeout}ms`));
      });

      req.write(postData);
      req.end();
    });
  }

  async getHealth(): Promise<ProviderHealth> {
    try {
      // Test basic connectivity
      const testStart = Date.now();
      const testEmbedding = await this.generateSingleEmbedding('test');
      const responseTime = Date.now() - testStart;
      
      return {
        status: 'healthy',
        details: `Ollama ${this.modelId} responding in ${responseTime}ms, ${testEmbedding.length} dimensions`,
        lastCheck: Date.now(),
        uptime: process.uptime() * 1000,
        errorRate: this.requestCount > 0 ? this.errorCount / this.requestCount : 0
      };
    } catch (err) {
      return {
        status: 'unhealthy',
        details: err instanceof Error ? err.message : String(err),
        lastCheck: Date.now(),
        errorRate: this.requestCount > 0 ? this.errorCount / this.requestCount : 1
      };
    }
  }

  async getMetrics(): Promise<ProviderMetrics> {
    return {
      requestCount: this.requestCount,
      avgDuration: this.requestCount > 0 ? this.totalDuration / this.requestCount : 0,
      errorRate: this.requestCount > 0 ? this.errorCount / this.requestCount : 0,
      lastSuccess: this.lastSuccess,
      totalEmbeddings: this.requestCount, // Approximate
      cacheHitRate: 0 // Ollama doesn't have caching in our implementation
    };
  }
}
