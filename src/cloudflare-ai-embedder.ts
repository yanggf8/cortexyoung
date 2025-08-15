import { IEmbedder, EmbedOptions, EmbeddingResult, EmbeddingMetadata, PerformanceStats, ProviderHealth, ProviderMetrics } from './types';
import { error } from './logging-utils';

const WORKER_URL = 'https://cortex-embedder.yanggf.workers.dev';
const BATCH_SIZE = 100; // As per Cloudflare's documented limit

interface EmbeddingResponse {
  embeddings: number[][];
}

interface SingleEmbeddingResponse {
    embeddings: number[];
}

// Circuit breaker for API resilience
class CircuitBreaker {
  private state: "CLOSED" | "OPEN" | "HALF_OPEN" = "CLOSED";
  private failures = 0;
  private lastFailureTime = 0;
  private successCount = 0;

  constructor(
    private failureThreshold: number = 5,
    private recoveryTimeout: number = 60000, // 1 minute
    private successThreshold: number = 2 // Successes needed to close circuit
  ) {}

  async execute<T>(operation: () => Promise<T>): Promise<T> {
    if (this.state === "OPEN") {
      if (Date.now() - this.lastFailureTime < this.recoveryTimeout) {
        throw new Error("Circuit breaker is OPEN - API temporarily unavailable");
      }
      // Try to recover
      this.state = "HALF_OPEN";
      this.successCount = 0;
    }

    try {
      const result = await operation();
      this.onSuccess();
      return result;
    } catch (err) {
      this.onFailure();
      throw err;
    }
  }

  private onSuccess(): void {
    this.failures = 0;
    
    if (this.state === "HALF_OPEN") {
      this.successCount++;
      if (this.successCount >= this.successThreshold) {
        this.state = "CLOSED";
      }
    }
  }

  private onFailure(): void {
    this.failures++;
    this.lastFailureTime = Date.now();
    
    if (this.failures >= this.failureThreshold) {
      this.state = "OPEN";
    }
  }

  getState(): "CLOSED" | "OPEN" | "HALF_OPEN" {
    return this.state;
  }

  getStats() {
    return {
      state: this.state,
      failures: this.failures,
      lastFailureTime: this.lastFailureTime,
      isHealthy: this.state === "CLOSED"
    };
  }
}

// Token bucket rate limiter
class TokenBucket {
  private tokens: number;
  private lastRefill: number;

  constructor(
    private capacity: number = 100, // 100 requests
    private refillRate: number = 100, // 100 tokens per minute
    private window: number = 60000 // 1 minute window
  ) {
    this.tokens = capacity;
    this.lastRefill = Date.now();
  }

  async acquire(tokens: number = 1): Promise<void> {
    this.refillTokens();
    
    if (this.tokens >= tokens) {
      this.tokens -= tokens;
      return;
    }
    
    // Calculate wait time for next token availability
    const tokensNeeded = tokens - this.tokens;
    const waitTime = (tokensNeeded / this.refillRate) * this.window;
    
    if (waitTime > 5000) { // Don't wait more than 5 seconds
      throw new Error(`Rate limit exceeded. Try again in ${Math.round(waitTime / 1000)} seconds`);
    }
    
    // Short wait for token availability
    await new Promise(resolve => setTimeout(resolve, waitTime));
    this.refillTokens();
    this.tokens -= tokens;
  }

  private refillTokens(): void {
    const now = Date.now();
    const timePassed = now - this.lastRefill;
    const tokensToAdd = Math.floor((timePassed / this.window) * this.refillRate);
    
    this.tokens = Math.min(this.capacity, this.tokens + tokensToAdd);
    this.lastRefill = now;
  }

  getRemaining(): number {
    this.refillTokens();
    return this.tokens;
  }

  getStats() {
    return {
      available: this.getRemaining(),
      capacity: this.capacity,
      refillRate: this.refillRate,
      windowMs: this.window
    };
  }
}

export class CloudflareAIEmbedder implements IEmbedder {
  public readonly providerId = "cloudflare.workers-ai.bge-small";
  public readonly modelId = "@cf/baai/bge-small-en-v1.5"; 
  public readonly dimensions = 384;
  public readonly maxBatchSize = 100;
  public readonly normalization = "l2" as const;
  
  // Resilience components
  private circuitBreaker = new CircuitBreaker(5, 60000, 2); // 5 failures, 1min timeout, 2 successes to recover
  private rateLimiter = new TokenBucket(100, 100, 60000); // 100 req/min capacity and refill rate
  
  // Internal metrics tracking
  private metrics = {
    requestCount: 0,
    totalDuration: 0,
    errorCount: 0,
    lastSuccess: 0,
    totalEmbeddings: 0
  };
  
  private startTime = Date.now();

  async embedBatch(texts: string[], options?: EmbedOptions): Promise<EmbeddingResult> {
    const startTime = Date.now();
    const startMemory = process.memoryUsage().heapUsed;
    
    this.metrics.requestCount++;
    
    try {
      // Apply rate limiting first
      await this.rateLimiter.acquire();
      
      const allEmbeddings: number[][] = [];
      let retries = 0;

      for (let i = 0; i < texts.length; i += BATCH_SIZE) {
        const batch = texts.slice(i, i + BATCH_SIZE);
        
        // Execute with circuit breaker protection
        const embeddings = await this.circuitBreaker.execute(async () => {
          const response = await fetch(WORKER_URL, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ texts: batch }),
          });

          if (!response.ok) {
            const errorText = await response.text();
            
            // Handle specific error cases
            if (response.status === 429) {
              throw new Error(`Rate limit exceeded: ${errorText}`);
            } else if (response.status >= 500) {
              throw new Error(`Server error (${response.status}): ${errorText}`);
            } else {
              throw new Error(`API error (${response.status}): ${errorText}`);
            }
          }

          const data = (await response.json()) as EmbeddingResponse;
          return data.embeddings;
        });
        
        allEmbeddings.push(...embeddings);
        
        // Small delay between batches to be respectful to the API
        if (i + BATCH_SIZE < texts.length) {
          await new Promise(resolve => setTimeout(resolve, 100));
        }
      }
      
      const duration = Date.now() - startTime;
      const memoryDelta = Math.round((process.memoryUsage().heapUsed - startMemory) / 1024 / 1024);
      
      this.metrics.totalDuration += duration;
      this.metrics.lastSuccess = Date.now();
      this.metrics.totalEmbeddings += allEmbeddings.length;
      
      return {
        embeddings: allEmbeddings,
        metadata: {
          providerId: this.providerId,
          modelId: this.modelId,
          batchSize: texts.length,
          processedAt: Date.now(),
          requestId: options?.requestId
        },
        performance: {
          duration,
          memoryDelta,
          retries
        }
      };
      
    } catch (err) {
      this.metrics.errorCount++;
      error(`Error embedding batch with Cloudflare AI (${this.circuitBreaker.getState()}):`, err);
      throw err;
    }
  }
  
  // Legacy method for backward compatibility
  async embed(texts: string[]): Promise<number[][]> {
    const result = await this.embedBatch(texts);
    return result.embeddings;
  }

  async embedSingle(text: string): Promise<number[]> {
    const result = await this.embedBatch([text]);
    return result.embeddings[0];
  }
  
  async getHealth(): Promise<ProviderHealth> {
    try {
      const circuitBreakerStats = this.circuitBreaker.getStats();
      const rateLimiterStats = this.rateLimiter.getStats();
      
      const errorRate = this.metrics.requestCount > 0 
        ? this.metrics.errorCount / this.metrics.requestCount 
        : 0;
      
      let status: "healthy" | "degraded" | "unhealthy" = "healthy";
      let details = "Service operational";
      
      // Check circuit breaker state first
      if (circuitBreakerStats.state === "OPEN") {
        status = "unhealthy";
        details = `Circuit breaker OPEN - API temporarily unavailable (${circuitBreakerStats.failures} failures)`;
      } else if (circuitBreakerStats.state === "HALF_OPEN") {
        status = "degraded";
        details = `Circuit breaker HALF_OPEN - Testing API recovery`;
      } else if (rateLimiterStats.available < 10) {
        status = "degraded";
        details = `Rate limit nearly exhausted (${rateLimiterStats.available}/${rateLimiterStats.capacity} requests remaining)`;
      } else if (errorRate > 0.3) {
        status = "unhealthy";
        details = `High error rate: ${(errorRate * 100).toFixed(1)}%`;
      } else if (errorRate > 0.1) {
        status = "degraded";
        details = `Elevated error rate: ${(errorRate * 100).toFixed(1)}%`;
      } else {
        details = `Circuit: ${circuitBreakerStats.state}, Rate limit: ${rateLimiterStats.available}/${rateLimiterStats.capacity}`;
      }
      
      return {
        status,
        details,
        lastCheck: Date.now(),
        uptime: Date.now() - this.startTime,
        errorRate
      };
      
    } catch (err) {
      return {
        status: "unhealthy",
        details: `Health check failed: ${err instanceof Error ? err.message : String(err)}`,
        lastCheck: Date.now(),
        uptime: Date.now() - this.startTime,
        errorRate: 1.0
      };
    }
  }
  
  async getMetrics(): Promise<ProviderMetrics> {
    const avgDuration = this.metrics.requestCount > 0 
      ? this.metrics.totalDuration / this.metrics.requestCount 
      : 0;
      
    const errorRate = this.metrics.requestCount > 0 
      ? this.metrics.errorCount / this.metrics.requestCount 
      : 0;
    
    return {
      requestCount: this.metrics.requestCount,
      avgDuration,
      errorRate,
      lastSuccess: this.metrics.lastSuccess,
      totalEmbeddings: this.metrics.totalEmbeddings
    };
  }
  
  // Additional methods for enhanced observability
  getCircuitBreakerState(): "CLOSED" | "OPEN" | "HALF_OPEN" {
    return this.circuitBreaker.getState();
  }
  
  getRateLimitRemaining(): number {
    return this.rateLimiter.getRemaining();
  }
  
  getCircuitBreakerStats() {
    return this.circuitBreaker.getStats();
  }
  
  getRateLimiterStats() {
    return this.rateLimiter.getStats();
  }
}