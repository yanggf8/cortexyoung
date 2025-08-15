import { CodeChunk, CORTEX_PROGRAM_VERSION, CORTEX_SCHEMA_VERSION, ModelInfo } from './types';
import { VectorStore } from './vector-store';
import { SchemaValidator } from './schema-validator';
import { log } from './logging-utils';
import { StoragePaths } from './storage-constants';
import * as fs from 'fs/promises';

interface PersistedIndex {
  version: string;
  schemaVersion: string;
  timestamp: number;
  repositoryPath: string;
  chunks: CodeChunk[];
  fileHashes: Record<string, string>; // file path -> content hash for fast comparison
  metadata: {
    totalChunks: number;
    lastIndexed: number;
    embeddingModel: string;
    modelInfo?: ModelInfo;
  };
}

interface IndexDelta {
  added: CodeChunk[];
  updated: CodeChunk[];
  removed: string[]; // chunk_ids
  fileChanges: {
    added: string[];
    modified: string[];
    deleted: string[];
  };
}

export class PersistentVectorStore extends VectorStore {
  private repositoryPath: string;
  private indexDir: string;
  private localIndexPath: string;
  private globalIndexPath: string;
  private metadataPath: string;
  private globalMetadataPath: string;
  private deltaPath: string;
  private globalDeltaPath: string;
  private fileHashes: Map<string, string> = new Map(); // file path -> content hash

  constructor(repositoryPath: string, indexDir: string = '.cortex') {
    super(); // Call parent constructor
    this.repositoryPath = repositoryPath;
    this.indexDir = indexDir;
    
    // Get all storage paths using centralized utility
    const paths = StoragePaths.getAllPaths(repositoryPath, indexDir);
    
    // Local storage paths
    this.localIndexPath = paths.local.indexPath;
    this.metadataPath = paths.local.metadataPath;
    this.deltaPath = paths.local.deltaPath;
    
    // Global storage paths
    this.globalIndexPath = paths.global.indexPath;
    this.globalMetadataPath = paths.global.metadataPath;
    this.globalDeltaPath = paths.global.deltaPath;
  }


  private static initializedPaths: Set<string> = new Set();
  private initialized: boolean = false;

  async initialize(): Promise<void> {
    // Prevent duplicate initialization across all instances for the same path
    const key = `${this.repositoryPath}:${this.indexDir}`;
    
    if (this.initialized) {
      log('[VectorStore] Already initialized, skipping');
      return;
    }

    if (PersistentVectorStore.initializedPaths.has(key)) {
      log('[VectorStore] Path already initialized by another instance, skipping load');
      this.initialized = true;
      return;
    }

    // Ensure both index directories exist in parallel
    await Promise.all([
      fs.mkdir(this.localIndexPath, { recursive: true }),
      fs.mkdir(this.deltaPath, { recursive: true }),
      fs.mkdir(this.globalIndexPath, { recursive: true }),
      fs.mkdir(this.globalDeltaPath, { recursive: true })
    ]);
    
    // Check existence and get stats in parallel
    const [localExists, globalExists, localStats, globalStats] = await Promise.all([
      this.indexExists(),
      this.globalIndexExists(),
      this.indexExists().then(exists => exists ? fs.stat(this.metadataPath).catch(() => null) : null),
      this.globalIndexExists().then(exists => exists ? fs.stat(this.globalMetadataPath).catch(() => null) : null)
    ]);
    
    if (globalExists && localExists && localStats && globalStats) {
      // Quick timestamp comparison - skip expensive chunk counting if timestamps are identical
      const localTime = localStats.mtime.getTime();
      const globalTime = globalStats.mtime.getTime();
      
      if (Math.abs(localTime - globalTime) < 1000) { // Within 1 second
        log('[StorageCompare] Timestamps nearly identical, using local storage (quick path)');
        await this.loadPersistedIndex(false);
        this.initialized = true;
        PersistentVectorStore.initializedPaths.add(key);
        return;
      }
      
      // Parallel chunk count reading for comparison
      const [localData, globalData] = await Promise.all([
        fs.readFile(this.metadataPath, 'utf-8').then(data => JSON.parse(data)).catch(() => ({ chunks: [] })),
        fs.readFile(this.globalMetadataPath, 'utf-8').then(data => JSON.parse(data)).catch(() => ({ chunks: [] }))
      ]);
      
      const localChunks = localData.chunks?.length || 0;
      const globalChunks = globalData.chunks?.length || 0;
      
      log('[StorageCompare] Storage comparison started');
      log(`[StorageCompare] Local chunks=${localChunks} modified=${localStats.mtime.toISOString()} path=${this.metadataPath}`);
      log(`[StorageCompare] Global chunks=${globalChunks} modified=${globalStats.mtime.toISOString()} path=${this.globalMetadataPath}`);
      
      // Check if both storages are empty/corrupt (0 chunks)
      if (localChunks === 0 && globalChunks === 0) {
        log('[StorageCompare] Both storages have 0 chunks - neither qualifies for comparison');
        log('[StorageCompare] No valid storage to load - initialize() will complete without loading any data');
        this.initialized = true;
        PersistentVectorStore.initializedPaths.add(key);
        return;
      }
      
      // Determine winner and load + sync in parallel where possible
      if (localChunks === 0 && globalChunks > 0) {
        log('[StorageCompare] Winner=global reason=local_empty loading=global');
        await this.loadPersistedIndex(true);
        // Sync in background - don't wait
        this.syncToLocal().catch(err => log(`[StorageSync] Background sync failed: ${err}`));
      } else if (globalChunks === 0 && localChunks > 0) {
        log('[StorageCompare] Winner=local reason=global_empty loading=local');
        await this.loadPersistedIndex(false);
        // Sync in background - don't wait
        this.syncToGlobal().catch(err => log(`[StorageSync] Background sync failed: ${err}`));
      } else {
        // Both have valid chunks - compare timestamps
        const globalIsNewer = globalStats.mtime > localStats.mtime;
        log(`[StorageCompare] Winner=${globalIsNewer ? 'global' : 'local'} reason=${globalIsNewer ? 'newer' : 'newer'} loading=${globalIsNewer ? 'global' : 'local'}`);
        
        if (globalIsNewer) {
          await this.loadPersistedIndex(true);
          // Sync in background - don't wait
          this.syncToLocal().catch(err => log(`[StorageSync] Background sync failed: ${err}`));
        } else {
          await this.loadPersistedIndex(false);
          // Sync in background - don't wait
          this.syncToGlobal().catch(err => log(`[StorageSync] Background sync failed: ${err}`));
        }
      }
    } else if (globalExists) {
      log('[StorageLoad] Loading from global storage - local not found');
      await this.loadPersistedIndex(true);
      // Sync in background - don't wait
      this.syncToLocal().catch(err => log(`[StorageSync] Background sync failed: ${err}`));
    } else if (localExists) {
      log('[StorageLoad] Loading from local storage - global not found');
      await this.loadPersistedIndex(false);
      // Sync in background - don't wait
      this.syncToGlobal().catch(err => log(`[StorageSync] Background sync failed: ${err}`));
    }

    this.initialized = true;
    PersistentVectorStore.initializedPaths.add(key);
  }

  async indexExists(): Promise<boolean> {
    try {
      await fs.access(this.metadataPath);
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Check if a valid index exists with actual chunks
   * @param useGlobal Whether to check global or local storage
   * @returns true if index exists and has valid chunks, false otherwise
   */
  async hasValidIndex(useGlobal: boolean = false): Promise<boolean> {
    try {
      const indexPath = useGlobal ? this.globalMetadataPath : this.metadataPath;
      await fs.access(indexPath);
      
      const indexData = JSON.parse(await fs.readFile(indexPath, 'utf-8'));
      const chunkCount = indexData.chunks?.length || 0;
      
      return chunkCount > 0;
    } catch {
      return false;
    }
  }

  async globalIndexExists(): Promise<boolean> {
    try {
      await fs.access(this.globalMetadataPath);
      return true;
    } catch {
      return false;
    }
  }

  async loadPersistedIndex(useGlobal: boolean = false): Promise<boolean> {
    try {
      const indexPath = useGlobal ? this.globalMetadataPath : this.metadataPath;
      const source = useGlobal ? 'global (~/.claude)' : 'local (.cortex)';
      const fullPath = useGlobal ? this.globalMetadataPath : this.metadataPath;
      
      log(`[StorageLoad] Loading persisted embeddings source=${source} path=${fullPath}`);
      const startTime = Date.now();
      
      const indexData = await fs.readFile(indexPath, 'utf-8');
      const persistedIndex: PersistedIndex = JSON.parse(indexData);
      
      // Load chunks
      this.chunks.clear();
      for (const chunk of persistedIndex.chunks) {
        this.chunks.set(chunk.chunk_id, chunk);
      }
      
      // Load file hashes for fast file-level comparison
      this.fileHashes.clear();
      if (persistedIndex.fileHashes) {
        for (const [filePath, hash] of Object.entries(persistedIndex.fileHashes)) {
          this.fileHashes.set(filePath, hash);
        }
      }
      
      const loadTime = Date.now() - startTime;
      log(`[StorageLoad] Loaded chunks=${persistedIndex.chunks.length} source=${source} path=${fullPath} duration=${loadTime}ms`);
      log(`[StorageLoad] Index metadata totalChunks=${persistedIndex.metadata.totalChunks} embeddingModel=${persistedIndex.metadata.embeddingModel} lastIndexed=${persistedIndex.metadata.lastIndexed}`);
      
      return true;
    } catch (error) {
      log(`[StorageLoad] Failed to load persisted index error=${error instanceof Error ? error.message : error}`);
      return false;
    }
  }

  async savePersistedIndex(modelInfo?: ModelInfo): Promise<void> {
    try {
      log('[StorageSave] Saving embeddings to both local and global storage');
      const startTime = Date.now();
      
      const persistedIndex: PersistedIndex = {
        version: '1.0.0', // Legacy version field
        schemaVersion: CORTEX_SCHEMA_VERSION,
        timestamp: Date.now(),
        repositoryPath: this.repositoryPath,
        chunks: Array.from(this.chunks.values()),
        fileHashes: Object.fromEntries(this.fileHashes.entries()),
        metadata: {
          totalChunks: this.chunks.size,
          lastIndexed: Date.now(),
          embeddingModel: modelInfo?.name || 'BGE-small-en-v1.5',
          modelInfo
        }
      };
      
      // Optimize JSON serialization for large datasets
      const indexData = JSON.stringify(persistedIndex, null, 0); // No pretty printing for faster writes
      
      // Save to both storages in parallel with optimized writes
      const saveLocal = async () => {
        const localTempPath = this.metadataPath + '.tmp';
        await fs.writeFile(localTempPath, indexData, { encoding: 'utf8' });
        await fs.rename(localTempPath, this.metadataPath);
      };

      const saveGlobal = async () => {
        const globalTempPath = this.globalMetadataPath + '.tmp';
        await fs.writeFile(globalTempPath, indexData, { encoding: 'utf8' });
        await fs.rename(globalTempPath, this.globalMetadataPath);
      };
      
      // Execute storage operations in parallel
      await Promise.all([saveLocal(), saveGlobal()]);
      
      const saveTime = Date.now() - startTime;
      log(`[StorageSave] Saved chunks=${persistedIndex.chunks.length} duration=${saveTime}ms`);
      log(`[StorageSave] Local path=${this.metadataPath}`);
      log(`[StorageSave] Global path=${this.globalMetadataPath}`);
    } catch (error) {
      log(`[StorageSave] Failed to save persisted index error=${error instanceof Error ? error.message : error}`);
      throw error;
    }
  }

  /**
   * Quick health check that skips expensive validation when possible
   */
  async quickHealthCheck(): Promise<{ healthy: boolean; reason?: string }> {
    try {
      // Check if index exists
      const indexExists = await this.indexExists();
      if (!indexExists) {
        return { healthy: false, reason: 'No index found' };
      }

      // Quick metadata validation
      const metadata = await this.getMetadata();
      if (!metadata || !metadata.totalChunks || metadata.totalChunks === 0) {
        return { healthy: false, reason: 'Empty or corrupt index' };
      }

      // Check if chunks match metadata count
      if (this.chunks.size !== metadata.totalChunks) {
        return { healthy: false, reason: 'Chunk count mismatch' };
      }

      return { healthy: true };
    } catch (error) {
      return { healthy: false, reason: `Health check failed: ${error instanceof Error ? error.message : 'Unknown error'}` };
    }
  }

  async calculateFileDelta(files: string[], chunkHashCalculator?: (filePath: string) => Promise<string>): Promise<IndexDelta> {
    const delta: IndexDelta = {
      added: [],
      updated: [],
      removed: [],
      fileChanges: {
        added: [],
        modified: [],
        deleted: []
      }
    };

    // Check each file for changes by comparing stored chunks with current chunks
    for (const filePath of files) {
      try {
        // Get stored chunks for this file
        const storedChunks = Array.from(this.chunks.values())
          .filter(chunk => chunk.file_path === filePath);
        
        if (storedChunks.length === 0) {
          // New file - no existing chunks
          delta.fileChanges.added.push(filePath);
        } else {
          // Use file content hash for fast comparison
          if (chunkHashCalculator) {
            const currentHash = await chunkHashCalculator(filePath);
            const storedHash = this.fileHashes.get(filePath);
            
            if (!storedHash) {
              // No stored hash (old data format) - treat as modified to rebuild file hash
              delta.fileChanges.modified.push(filePath);
              delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
            } else if (storedHash !== currentHash) {
              // File changed at content level
              delta.fileChanges.modified.push(filePath);
              delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
            }
            // If hashes match, file is unchanged - no action needed
          } else {
            // Fallback: treat as modified if no calculator provided
            delta.fileChanges.modified.push(filePath);
            delta.removed.push(...storedChunks.map(chunk => chunk.chunk_id));
          }
        }
      } catch (error) {
        log(`[VectorStore] Failed to process file file=${filePath} error=${error instanceof Error ? error.message : error}`);
      }
    }

    // Check for deleted files - files that have chunks but aren't in current file list
    const filesWithChunks = new Set(Array.from(this.chunks.values()).map(chunk => chunk.file_path));
    for (const filePath of filesWithChunks) {
      if (!files.includes(filePath)) {
        delta.fileChanges.deleted.push(filePath);
        
        // Remove chunks for deleted files
        const deletedChunks = Array.from(this.chunks.values())
          .filter(chunk => chunk.file_path === filePath);
        delta.removed.push(...deletedChunks.map(chunk => chunk.chunk_id));
      }
    }

    return delta;
  }

  async applyDelta(delta: IndexDelta): Promise<void> {
    // Remove deleted chunks
    for (const chunkId of delta.removed) {
      this.chunks.delete(chunkId);
    }

    // Add new chunks
    for (const chunk of delta.added) {
      this.chunks.set(chunk.chunk_id, chunk);
    }

    // Update modified chunks
    for (const chunk of delta.updated) {
      this.chunks.set(chunk.chunk_id, chunk);
    }

    log(`[StorageDelta] Applied delta added=${delta.added.length} updated=${delta.updated.length} removed=${delta.removed.length}`);
  }

  async syncToGlobal(): Promise<void> {
    try {
      if (await this.indexExists()) {
        log(`[StorageSync] Syncing local embeddings to global storage from=${this.metadataPath} to=${this.globalMetadataPath}`);
        const indexData = await fs.readFile(this.metadataPath, 'utf-8');
        const globalTempPath = this.globalMetadataPath + '.tmp';
        await fs.writeFile(globalTempPath, indexData);
        await fs.rename(globalTempPath, this.globalMetadataPath);
        log('[StorageSync] Synced to global storage');
      }
    } catch (error) {
      log(`[StorageSync] Failed to sync to global storage error=${error instanceof Error ? error.message : error}`);
    }
  }

  async syncToLocal(): Promise<void> {
    try {
      if (await this.globalIndexExists()) {
        // Check if local is outdated
        const localExists = await this.indexExists();
        let shouldSync = true;
        
        if (localExists) {
          const [localStats, globalStats] = await Promise.all([
            fs.stat(this.metadataPath),
            fs.stat(this.globalMetadataPath)
          ]);
          
          // Only sync if global is newer
          shouldSync = globalStats.mtime > localStats.mtime;
        }
        
        if (shouldSync) {
          // Get chunk count from global before syncing
          let globalChunks = 0;
          try {
            const globalData = JSON.parse(await fs.readFile(this.globalMetadataPath, 'utf-8'));
            globalChunks = globalData.chunks?.length || 0;
          } catch (error) {
            log('[StorageSync] Could not read global chunk count');
          }
          
          log(`[StorageSync] Syncing global embeddings to local storage chunks=${globalChunks} from=${this.globalMetadataPath} to=${this.metadataPath}`);
          const indexData = await fs.readFile(this.globalMetadataPath, 'utf-8');
          const localTempPath = this.metadataPath + '.tmp';
          await fs.writeFile(localTempPath, indexData);
          await fs.rename(localTempPath, this.metadataPath);
          log(`[StorageSync] Synced chunks=${globalChunks} to local storage`);
        } else {
          log('[StorageSync] Local storage is up to date');
        }
      }
    } catch (error) {
      log(`[StorageSync] Failed to sync to local storage error=${error instanceof Error ? error.message : error}`);
    }
  }

  async getStorageInfo(): Promise<{
    local: { exists: boolean; path: string; lastModified?: Date };
    global: { exists: boolean; path: string; lastModified?: Date };
  }> {
    const [localExists, globalExists] = await Promise.all([
      this.indexExists(),
      this.globalIndexExists()
    ]);

    const info: {
      local: { exists: boolean; path: string; lastModified?: Date };
      global: { exists: boolean; path: string; lastModified?: Date };
    } = {
      local: { exists: localExists, path: this.metadataPath },
      global: { exists: globalExists, path: this.globalMetadataPath }
    };

    if (localExists) {
      const localStats = await fs.stat(this.metadataPath);
      info.local.lastModified = localStats.mtime;
    }

    if (globalExists) {
      const globalStats = await fs.stat(this.globalMetadataPath);
      info.global.lastModified = globalStats.mtime;
    }

    return info;
  }

  getChunksByFile(filePath: string): CodeChunk[] {
    return Array.from(this.chunks.values()).filter(chunk => chunk.file_path === filePath);
  }

  compareChunks(oldChunks: CodeChunk[], newChunks: CodeChunk[]): { toAdd: CodeChunk[], toKeep: CodeChunk[], toRemove: CodeChunk[] } {
    const oldChunkMap = new Map(oldChunks.map(c => [c.content_hash, c]));
    const newChunkMap = new Map(newChunks.map(c => [c.content_hash, c]));

    const toAdd: CodeChunk[] = [];
    const toKeep: CodeChunk[] = [];
    const toRemove: CodeChunk[] = [];

    for (const [hash, chunk] of newChunkMap.entries()) {
      if (oldChunkMap.has(hash)) {
        const oldChunk = oldChunkMap.get(hash)!;
        // Preserve existing embedding
        chunk.embedding = oldChunk.embedding;
        toKeep.push(chunk);
        oldChunkMap.delete(hash); // Remove from map to track remaining (deleted) chunks
      } else {
        toAdd.push(chunk);
      }
    }

    // Any remaining chunks in oldChunkMap were removed
    toRemove.push(...oldChunkMap.values());

    return { toAdd, toKeep, toRemove };
  }

  // Override upsertChunks to ensure persistence and populate file hashes
  async upsertChunks(chunks: CodeChunk[]): Promise<void> {
    await super.upsertChunks(chunks);
    
    // Note: File hashes are updated separately via setFileHash() method during indexing
  }

  // Set file content hash for delta detection
  setFileHash(filePath: string, contentHash: string): void {
    this.fileHashes.set(filePath, contentHash);
  }

  // Enhanced getStats with persistence information
  async getStats(): Promise<{
    total_chunks: number;
    totalFiles: number;
    indexSize: string;
    lastUpdated: Date;
  }> {
    const stats = await fs.stat(this.metadataPath).catch(() => null);
    const indexSize = stats ? `${(stats.size / 1024 / 1024).toFixed(2)} MB` : 'Unknown';
    
    // Count unique files from chunks
    const uniqueFiles = new Set(Array.from(this.chunks.values()).map(chunk => chunk.file_path));
    
    return {
      total_chunks: this.chunks.size,
      totalFiles: uniqueFiles.size,
      indexSize,
      lastUpdated: stats?.mtime || new Date()
    };
  }

  // Override clear to also clear persistence
  async clear(): Promise<void> {
    await super.clear();
    
    try {
      await fs.rm(this.localIndexPath, { recursive: true, force: true });
      await fs.mkdir(this.localIndexPath, { recursive: true });
      await fs.mkdir(this.deltaPath, { recursive: true });
    } catch (error) {
      log(`[VectorStore] Failed to clear index directory error=${error instanceof Error ? error.message : error}`);
    }
  }

  async clearIndex(): Promise<void> {
    await this.clear();
  }

  getChunkCount(): number {
    return this.chunks.size;
  }

  getAllChunks(): CodeChunk[] {
    return Array.from(this.chunks.values());
  }

  getFileHashCount(): number {
    // Return count of unique files from chunks
    const uniqueFiles = new Set(Array.from(this.chunks.values()).map(chunk => chunk.file_path));
    return uniqueFiles.size;
  }

  hasFileHashes(): boolean {
    // Always return true since we calculate from chunks
    return this.chunks.size > 0;
  }

  async getMetadata(): Promise<any> {
    try {
      if (await this.indexExists()) {
        const indexData = JSON.parse(await fs.readFile(this.metadataPath, 'utf-8'));
        return indexData.metadata || {};
      }
    } catch (error) {
      if (error instanceof SyntaxError && error.message.includes('JSON')) {
        log(`[VectorStore] JSON parsing error in metadata file path=${this.metadataPath} error=${error.message}`);
        log(`[VectorStore] Check for merge conflicts or corrupted JSON content`);
      } else {
        log(`[VectorStore] Could not load metadata error=${error instanceof Error ? error.message : error}`);
      }
    }
    return null;
  }

  async updateMetadata(metadata: any): Promise<void> {
    try {
      let indexData: any = { chunks: [], metadata: {} };
      
      if (await this.indexExists()) {
        indexData = JSON.parse(await fs.readFile(this.metadataPath, 'utf-8'));
      }
      
      indexData.metadata = { ...indexData.metadata, ...metadata };
      
      const tempPath = this.metadataPath + '.tmp';
      await fs.writeFile(tempPath, JSON.stringify(indexData, null, 2));
      await fs.rename(tempPath, this.metadataPath);
    } catch (error) {
      log(`[VectorStore] Could not update metadata error=${error instanceof Error ? error.message : error}`);
    }
  }

  // Methods needed by redundancy checker
  async listAllVectors(): Promise<Array<{id: string, vector: number[], metadata?: any}>> {
    const chunks = this.getAllChunks();
    return chunks.map(chunk => ({
      id: chunk.chunk_id,
      vector: chunk.embedding || [],
      metadata: {
        file_path: chunk.file_path,
        content: chunk.content
      }
    }));
  }

  async deleteVector(id: string): Promise<void> {
    this.chunks.delete(id);
    // Persist the change
    await this.savePersistedIndex();
  }

  async close(): Promise<void> {
    // No-op for now, could be used for cleanup
  }
}