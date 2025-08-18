/**
 * Child process cache that coordinates with main process SharedArrayBuffer
 * via IPC for shared embedding caching
 */

class ChildProcessCache {
  constructor() {
    this.localCache = new Map(); // Local cache for this process
    this.pendingRequests = new Map(); // Track async cache requests
    this.requestId = 0;
    this.maxEntries = 1000; // Smaller cache for child process
    
    // Set up IPC message handler
    if (process.send) {
      process.on('message', (message) => {
        this.handleMessage(message);
      });
    }
  }

  /**
   * Get embedding from cache (checks local first, then main process)
   */
  async get(contentHash) {
    // Check local cache first (fastest)
    const localResult = this.localCache.get(contentHash);
    if (localResult) {
      this.updateLocalCacheStats(localResult, true);
      return new Float32Array(localResult.embedding);
    }

    // Check main process shared cache via IPC
    return new Promise((resolve) => {
      const requestId = ++this.requestId;
      this.pendingRequests.set(requestId, resolve);
      
      process.send({
        type: 'cache_get',
        contentHash,
        requestId
      });
      
      // Timeout after 100ms to avoid blocking
      setTimeout(() => {
        if (this.pendingRequests.has(requestId)) {
          this.pendingRequests.delete(requestId);
          resolve(null); // Cache miss
        }
      }, 100);
    });
  }

  /**
   * Store embedding in cache (local + notify main process)
   */
  set(contentHash, embedding) {
    const embeddingArray = Array.from(embedding);
    
    // Store in local cache
    this.setLocal(contentHash, embeddingArray);
    
    // Notify main process to store in shared cache
    if (process.send) {
      process.send({
        type: 'cache_set',
        contentHash,
        embedding: embeddingArray
      });
    }
  }

  /**
   * Store in local cache with LRU eviction
   */
  setLocal(contentHash, embedding) {
    // Evict if needed
    if (this.localCache.size >= this.maxEntries) {
      this.evictLRU();
    }

    const entry = {
      embedding,
      timestamp: Date.now(),
      hitCount: 1,
      lastAccessed: Date.now()
    };

    this.localCache.set(contentHash, entry);
  }

  /**
   * Handle IPC messages from main process
   */
  handleMessage(message) {
    switch (message.type) {
      case 'cache_init':
        this.handleCacheInit(message);
        break;
      case 'cache_get_response':
        this.handleCacheGetResponse(message);
        break;
      case 'cache_clear':
        this.localCache.clear();
        break;
    }
  }

  /**
   * Handle cache initialization from main process
   */
  handleCacheInit(message) {
    const { config } = message;
    this.maxEntries = config.maxEntries || 1000;
    // console.error(`[ChildCache] Initialized with maxEntries=${this.maxEntries}`);
  }

  /**
   * Handle cache get response from main process
   */
  handleCacheGetResponse(message) {
    const { requestId, embedding } = message;
    const resolver = this.pendingRequests.get(requestId);
    
    if (resolver) {
      this.pendingRequests.delete(requestId);
      
      if (embedding) {
        const embeddingArray = new Float32Array(embedding);
        // Cache locally for future use
        this.setLocal(message.contentHash || 'unknown', embedding);
        resolver(embeddingArray);
      } else {
        resolver(null); // Cache miss
      }
    }
  }

  /**
   * Update local cache access statistics
   */
  updateLocalCacheStats(entry, isHit) {
    if (isHit) {
      entry.hitCount++;
      entry.lastAccessed = Date.now();
    }
  }

  /**
   * Evict least recently used entry from local cache
   */
  evictLRU() {
    if (this.localCache.size === 0) return;

    let oldestKey = null;
    let oldestTime = Date.now();

    for (const [key, entry] of this.localCache.entries()) {
      if (entry.lastAccessed < oldestTime) {
        oldestTime = entry.lastAccessed;
        oldestKey = key;
      }
    }

    if (oldestKey) {
      this.localCache.delete(oldestKey);
    }
  }

  /**
   * Get cache statistics
   */
  getStats() {
    let totalHits = 0;
    for (const entry of this.localCache.values()) {
      totalHits += entry.hitCount;
    }

    return {
      type: 'ChildProcessCache',
      localEntries: this.localCache.size,
      maxEntries: this.maxEntries,
      totalHits,
      pendingRequests: this.pendingRequests.size
    };
  }

  /**
   * Clear all caches
   */
  clear() {
    this.localCache.clear();
    this.pendingRequests.clear();
  }
}

module.exports = { ChildProcessCache };