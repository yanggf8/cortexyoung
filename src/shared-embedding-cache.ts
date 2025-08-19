import * as fs from 'fs/promises';
import * as path from 'path';
import { log, warn, error } from './logging-utils';

interface CachedEmbedding {
  embedding: number[];
  contentHash: string;
  timestamp: number;
  hitCount: number;
  lastAccessed: number;
}

interface CacheMetadata {
  version: string;
  totalEntries: number;
  lastUpdated: number;
  maxSize: number;
}

/**
 * Shared embedding cache that persists to disk and can be shared
 * across multiple ProcessPoolEmbedder instances and process restarts
 */
export class SharedEmbeddingCache {
  private cache: Map<string, CachedEmbedding> = new Map();
  private cacheStats = { hits: 0, misses: 0, total: 0, evictions: 0 };
  private cacheFilePath: string;
  private metadataFilePath: string;
  private isLoaded = false;
  private isSaving = false;
  private readonly maxSize: number;
  private readonly evictionThreshold: number;
  private readonly evictionPercentage: number;
  
  // Auto-save configuration
  private saveInterval?: NodeJS.Timeout;
  private readonly AUTO_SAVE_INTERVAL = 30000; // Save every 30 seconds
  private readonly SAVE_BATCH_SIZE = 100; // Save after 100 operations
  private unsavedChanges = 0;

  constructor(
    cacheDir: string, 
    maxSize: number = 10000,
    evictionThreshold: number = 0.8,
    evictionPercentage: number = 0.2
  ) {
    this.maxSize = maxSize;
    this.evictionThreshold = evictionThreshold;
    this.evictionPercentage = evictionPercentage;
    
    // Create cache file paths
    this.cacheFilePath = path.join(cacheDir, 'shared-embedding-cache.json');
    this.metadataFilePath = path.join(cacheDir, 'cache-metadata.json');
    
    // Start auto-save interval
    this.startAutoSave();
  }

  /**
   * Initialize the shared cache by loading from disk
   */
  async initialize(): Promise<void> {
    if (this.isLoaded) return;

    try {
      await this.loadFromDisk();
      this.isLoaded = true;
      log(`[SharedCache] Initialized with ${this.cache.size} cached embeddings`);
    } catch (error) {
      warn(`[SharedCache] Failed to load cache from disk: ${error}. Starting with empty cache.`);
      this.isLoaded = true;
    }
  }

  /**
   * Get embedding from cache
   */
  get(contentHash: string): number[] | null {
    if (!this.isLoaded) {
      throw new Error('SharedEmbeddingCache not initialized. Call initialize() first.');
    }

    this.cacheStats.total++;
    const cached = this.cache.get(contentHash);
    
    if (cached && this.validateCacheEntry(cached)) {
      // Update LRU data
      cached.hitCount++;
      cached.lastAccessed = Date.now();
      this.cacheStats.hits++;
      
      // Increment unsaved changes
      this.unsavedChanges++;
      if (this.unsavedChanges >= this.SAVE_BATCH_SIZE) {
        this.saveToDiskAsync(); // Non-blocking save
      }
      
      return cached.embedding;
    } else {
      // Remove invalid entry if exists
      if (cached) {
        this.cache.delete(contentHash);
        this.unsavedChanges++;
      }
      
      this.cacheStats.misses++;
      return null;
    }
  }

  /**
   * Store embedding in cache
   */
  set(contentHash: string, embedding: number[]): void {
    if (!this.isLoaded) {
      throw new Error('SharedEmbeddingCache not initialized. Call initialize() first.');
    }

    const now = Date.now();
    const cached: CachedEmbedding = {
      embedding,
      contentHash,
      timestamp: now,
      hitCount: 1,
      lastAccessed: now
    };

    this.cache.set(contentHash, cached);
    this.unsavedChanges++;

    // Check if eviction is needed
    if (this.cache.size > this.maxSize * this.evictionThreshold) {
      this.evictLRU();
    }

    // Auto-save if batch threshold reached
    if (this.unsavedChanges >= this.SAVE_BATCH_SIZE) {
      this.saveToDiskAsync();
    }
  }

  /**
   * Get cache statistics
   */
  getStats() {
    const hitRate = this.cacheStats.total > 0 ? (this.cacheStats.hits / this.cacheStats.total * 100) : 0;
    return {
      size: this.cache.size,
      maxSize: this.maxSize,
      utilizationRate: (this.cache.size / this.maxSize * 100).toFixed(1) + '%',
      hits: this.cacheStats.hits,
      misses: this.cacheStats.misses,
      total: this.cacheStats.total,
      hitRate: hitRate.toFixed(1) + '%',
      evictions: this.cacheStats.evictions,
      unsavedChanges: this.unsavedChanges
    };
  }

  /**
   * Force save to disk
   */
  async save(): Promise<void> {
    await this.saveToDisk();
  }

  /**
   * Shutdown and save cache
   */
  async shutdown(): Promise<void> {
    if (this.saveInterval) {
      clearInterval(this.saveInterval);
      this.saveInterval = undefined;
    }

    if (this.unsavedChanges > 0) {
      await this.saveToDisk();
    }

    log(`[SharedCache] Shutdown complete. Final stats: ${JSON.stringify(this.getStats())}`);
  }

  /**
   * Validate cache entry
   */
  private validateCacheEntry(cached: CachedEmbedding): boolean {
    return Array.isArray(cached.embedding) && 
           cached.embedding.length > 0 &&
           typeof cached.contentHash === 'string' &&
           cached.timestamp > 0;
  }

  /**
   * Evict least recently used entries
   */
  private evictLRU(): void {
    const targetRemoval = Math.floor(this.cache.size * this.evictionPercentage);
    if (targetRemoval <= 0) return;

    // Sort by lastAccessed (oldest first)
    const entries = Array.from(this.cache.entries()).sort((a, b) => 
      a[1].lastAccessed - b[1].lastAccessed
    );

    for (let i = 0; i < targetRemoval && i < entries.length; i++) {
      this.cache.delete(entries[i][0]);
      this.cacheStats.evictions++;
    }

    this.unsavedChanges += targetRemoval;
    log(`[SharedCache] Evicted ${targetRemoval} LRU entries. Cache size: ${this.cache.size}`);
  }

  /**
   * Load cache from disk
   */
  private async loadFromDisk(): Promise<void> {
    try {
      // Load metadata first
      const metadataExists = await this.fileExists(this.metadataFilePath);
      let metadata: CacheMetadata | null = null;
      
      if (metadataExists) {
        const metadataContent = await fs.readFile(this.metadataFilePath, 'utf-8');
        metadata = JSON.parse(metadataContent);
      }

      // Load cache data
      const cacheExists = await this.fileExists(this.cacheFilePath);
      if (cacheExists) {
        const cacheContent = await fs.readFile(this.cacheFilePath, 'utf-8');
        const cacheData = JSON.parse(cacheContent);

        // Reconstruct Map from serialized data
        this.cache.clear();
        for (const [key, value] of Object.entries(cacheData)) {
          if (this.validateCacheEntry(value as CachedEmbedding)) {
            this.cache.set(key, value as CachedEmbedding);
          }
        }

        log(`[SharedCache] Loaded ${this.cache.size} entries from disk`);
        if (metadata) {
          log(`[SharedCache] Cache metadata: version=${metadata.version}, lastUpdated=${new Date(metadata.lastUpdated).toISOString()}`);
        }
      }
    } catch (error) {
      throw new Error(`Failed to load cache: ${error}`);
    }
  }

  /**
   * Save cache to disk (blocking)
   */
  private async saveToDisk(): Promise<void> {
    if (this.isSaving) return; // Prevent concurrent saves
    this.isSaving = true;

    try {
      // Ensure cache directory exists
      await fs.mkdir(path.dirname(this.cacheFilePath), { recursive: true });

      // Save cache data
      const cacheData = Object.fromEntries(this.cache.entries());
      await fs.writeFile(this.cacheFilePath, JSON.stringify(cacheData, null, 2));

      // Save metadata
      const metadata: CacheMetadata = {
        version: '1.0',
        totalEntries: this.cache.size,
        lastUpdated: Date.now(),
        maxSize: this.maxSize
      };
      await fs.writeFile(this.metadataFilePath, JSON.stringify(metadata, null, 2));

      this.unsavedChanges = 0;
      log(`[SharedCache] Saved ${this.cache.size} entries to disk`);
    } catch (err) {
      error(`[SharedCache] Failed to save cache: ${err}`);
      throw err;
    } finally {
      this.isSaving = false;
    }
  }

  /**
   * Save cache to disk (non-blocking)
   */
  private saveToDiskAsync(): void {
    this.saveToDisk().catch(err => {
      warn(`[SharedCache] Async save failed: ${err}`);
    });
  }

  /**
   * Check if file exists
   */
  private async fileExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath);
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Start auto-save interval
   */
  private startAutoSave(): void {
    this.saveInterval = setInterval(() => {
      if (this.unsavedChanges > 0 && this.isLoaded) {
        this.saveToDiskAsync();
      }
    }, this.AUTO_SAVE_INTERVAL);
  }
}