import { CodeChunk, EmbeddingCache, EmbeddingCacheEntry, CacheStats } from './types';
import { ProcessPoolEmbedder } from './process-pool-embedder';
import * as fs from 'fs/promises';
import * as path from 'path';
import * as crypto from 'crypto';
import * as os from 'os';

interface CachePartition {
  cached: CodeChunk[];
  uncached: CodeChunk[];
  cacheHits: string[]; // content_hash values that were cache hits
}

export class CachedEmbedder extends ProcessPoolEmbedder {
  private cache: EmbeddingCache = {};
  private stats: CacheStats = {
    total_entries: 0,
    cache_hits: 0,
    cache_misses: 0,
    hit_rate: 0,
    last_cleanup: new Date().toISOString(),
    size_bytes: 0
  };
  private cacheFilePath: string;
  private globalCacheFilePath: string;
  private repositoryPath: string;

  constructor(repositoryPath: string, processCount?: number) {
    super();
    this.repositoryPath = repositoryPath;
    this.cacheFilePath = path.join(repositoryPath, '.cortex', 'embedding-cache.json');
    
    // Global cache path using same hash as existing dual storage
    const repoHash = crypto.createHash('sha256')
      .update(path.basename(repositoryPath) + repositoryPath)
      .digest('hex')
      .substring(0, 16);
    
    const globalCacheDir = path.join(os.homedir(), '.claude', 'cortex-embeddings', `${path.basename(repositoryPath)}-${repoHash}`);
    this.globalCacheFilePath = path.join(globalCacheDir, 'embedding-cache.json');
  }

  async initialize(chunkCount?: number): Promise<void> {
    await super.initialize(chunkCount);
    await this.loadCache();
  }

  async processAllEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    const startTime = Date.now();
    
    // Partition chunks into cached vs uncached
    const partition = await this.partitionChunks(chunks);
    
    console.log(`📦 Cache: ${partition.cached.length} hits, ${partition.uncached.length} misses`);
    
    // Update stats
    this.stats.cache_hits += partition.cached.length;
    this.stats.cache_misses += partition.uncached.length;
    this.stats.hit_rate = this.stats.cache_hits / (this.stats.cache_hits + this.stats.cache_misses);
    
    // Generate embeddings only for uncached chunks
    let newEmbeddings: CodeChunk[] = [];
    if (partition.uncached.length > 0) {
      console.log(`🔄 Generating ${partition.uncached.length} new embeddings...`);
      newEmbeddings = await super.processAllEmbeddings(partition.uncached);
      
      // Update cache with new embeddings
      await this.updateEmbeddingCache(newEmbeddings);
    }
    
    // Merge cached + new embeddings, preserving original order
    const result = this.mergeResults(chunks, partition.cached, newEmbeddings);
    
    const timeTaken = Date.now() - startTime;
    console.log(`✅ Cache-aware embedding completed in ${timeTaken}ms`);
    console.log(`   📊 Hit rate: ${(this.stats.hit_rate * 100).toFixed(1)}%`);
    
    return result;
  }

  private async partitionChunks(chunks: CodeChunk[]): Promise<CachePartition> {
    const cached: CodeChunk[] = [];
    const uncached: CodeChunk[] = [];
    const cacheHits: string[] = [];

    for (const chunk of chunks) {
      const cacheEntry = this.cache[chunk.content_hash];
      
      if (cacheEntry && this.isCacheEntryValid(cacheEntry)) {
        // Cache hit - restore embedding from cache
        chunk.embedding = cacheEntry.embedding;
        cached.push(chunk);
        cacheHits.push(chunk.content_hash);
        
        // Update access stats
        cacheEntry.access_count++;
        cacheEntry.last_accessed = new Date().toISOString();
      } else {
        // Cache miss - needs new embedding
        uncached.push(chunk);
      }
    }

    return { cached, uncached, cacheHits };
  }

  private isCacheEntryValid(entry: EmbeddingCacheEntry): boolean {
    // Check if cache entry is valid (model version, embedding dimensions, etc.)
    return entry.embedding && 
           entry.embedding.length === 384 && // BGE-small-en-v1.5 dimensions
           entry.model_version === 'BGE-small-en-v1.5';
  }

  private async updateEmbeddingCache(newChunks: CodeChunk[]): Promise<void> {
    for (const chunk of newChunks) {
      if (chunk.embedding && chunk.embedding.length > 0) {
        this.cache[chunk.content_hash] = {
          embedding: chunk.embedding,
          created_at: new Date().toISOString(),
          model_version: 'BGE-small-en-v1.5',
          access_count: 1,
          last_accessed: new Date().toISOString(),
          chunk_metadata: {
            file_path: chunk.file_path,
            symbol_name: chunk.symbol_name,
            chunk_type: chunk.chunk_type
          }
        };
      }
    }

    this.stats.total_entries = Object.keys(this.cache).length;
    await this.saveCache();
  }

  private mergeResults(originalChunks: CodeChunk[], cached: CodeChunk[], newEmbeddings: CodeChunk[]): CodeChunk[] {
    // Create lookup maps for efficient merging
    const cachedMap = new Map(cached.map(chunk => [chunk.chunk_id, chunk]));
    const newMap = new Map(newEmbeddings.map(chunk => [chunk.chunk_id, chunk]));
    
    // Preserve original order while merging results
    return originalChunks.map(chunk => {
      return cachedMap.get(chunk.chunk_id) || newMap.get(chunk.chunk_id) || chunk;
    });
  }

  private async loadCache(): Promise<void> {
    try {
      // Try global cache first, then local
      let cacheData: string | null = null;
      let sourceLocation = '';

      try {
        cacheData = await fs.readFile(this.globalCacheFilePath, 'utf-8');
        sourceLocation = 'global';
      } catch {
        try {
          cacheData = await fs.readFile(this.cacheFilePath, 'utf-8');
          sourceLocation = 'local';
        } catch {
          console.log('📦 No existing embedding cache found, starting fresh');
          return;
        }
      }

      if (cacheData) {
        const parsed = JSON.parse(cacheData);
        this.cache = parsed.cache || {};
        this.stats = { ...this.stats, ...parsed.stats };
        console.log(`📦 Loaded embedding cache from ${sourceLocation}: ${this.stats.total_entries} entries`);
        
        // Sync to other location if needed
        if (sourceLocation === 'global') {
          await this.saveCacheToLocation(this.cacheFilePath);
        } else {
          await this.saveCacheToLocation(this.globalCacheFilePath);
        }
      }
    } catch (error) {
      console.warn('⚠️ Failed to load embedding cache:', error);
      this.cache = {};
    }
  }

  private async saveCache(): Promise<void> {
    // Save to both local and global locations
    await Promise.all([
      this.saveCacheToLocation(this.cacheFilePath),
      this.saveCacheToLocation(this.globalCacheFilePath)
    ]);
  }

  private async saveCacheToLocation(filePath: string): Promise<void> {
    try {
      const dir = path.dirname(filePath);
      await fs.mkdir(dir, { recursive: true });
      
      const cacheData = {
        cache: this.cache,
        stats: this.stats,
        updated_at: new Date().toISOString()
      };
      
      await fs.writeFile(filePath, JSON.stringify(cacheData, null, 2));
    } catch (error) {
      console.warn(`⚠️ Failed to save cache to ${filePath}:`, error);
    }
  }

  async getEmbeddingCacheStats(): Promise<CacheStats> {
    this.stats.size_bytes = JSON.stringify(this.cache).length;
    return { ...this.stats };
  }

  async clearCache(): Promise<void> {
    this.cache = {};
    this.stats = {
      total_entries: 0,
      cache_hits: 0,
      cache_misses: 0,
      hit_rate: 0,
      last_cleanup: new Date().toISOString(),
      size_bytes: 0
    };
    
    await this.saveCache();
    console.log('🗑️ Embedding cache cleared');
  }
}