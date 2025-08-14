import { CodeChunk, CORTEX_PROGRAM_VERSION, CORTEX_SCHEMA_VERSION, ModelInfo } from './types';
import { VectorStore } from './vector-store';
import { SchemaValidator } from './schema-validator';
import { log } from './logging-utils';
import * as fs from 'fs/promises';
import * as path from 'path';
import * as crypto from 'crypto';
import * as os from 'os';

interface PersistedIndex {
  version: string;
  schemaVersion: string;
  timestamp: number;
  repositoryPath: string;
  chunks: CodeChunk[];
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

  constructor(repositoryPath: string, indexDir: string = '.cortex') {
    super(); // Call parent constructor
    this.repositoryPath = repositoryPath;
    this.indexDir = indexDir;
    
    // Local storage (in repo)
    this.localIndexPath = path.join(repositoryPath, indexDir);
    this.metadataPath = path.join(this.localIndexPath, 'index.json');
    this.deltaPath = path.join(this.localIndexPath, 'deltas');
    
    // Global storage (in ~/.claude)
    const repoHash = this.getRepositoryHash(repositoryPath);
    const claudeDir = path.join(os.homedir(), '.claude', 'cortex-embeddings');
    this.globalIndexPath = path.join(claudeDir, repoHash);
    this.globalMetadataPath = path.join(this.globalIndexPath, 'index.json');
    this.globalDeltaPath = path.join(this.globalIndexPath, 'deltas');
  }

  private getRepositoryHash(repoPath: string): string {
    // Create a consistent hash based on absolute path
    const absolutePath = path.resolve(repoPath);
    const hash = crypto.createHash('sha256').update(absolutePath).digest('hex');
    
    // Use first 16 chars + repo name for readability
    const repoName = path.basename(absolutePath);
    return `${repoName}-${hash.substring(0, 16)}`;
  }

  private static initializedPaths: Set<string> = new Set();
  private initialized: boolean = false;

  async initialize(): Promise<void> {
    // Prevent duplicate initialization across all instances for the same path
    const key = `${this.repositoryPath}:${this.indexDir}`;
    
    if (this.initialized) {
      console.log('📋 Vector store already initialized, skipping...');
      return;
    }

    if (PersistentVectorStore.initializedPaths.has(key)) {
      console.log('📋 Vector store for this path already initialized by another instance, skipping load...');
      this.initialized = true;
      return;
    }

    // Ensure both index directories exist
    await fs.mkdir(this.localIndexPath, { recursive: true });
    await fs.mkdir(this.deltaPath, { recursive: true });
    await fs.mkdir(this.globalIndexPath, { recursive: true });
    await fs.mkdir(this.globalDeltaPath, { recursive: true });
    
    // Try to load existing index with comparison details
    const localExists = await this.indexExists();
    const globalExists = await this.globalIndexExists();
    
    if (globalExists && localExists) {
      // Both exist - compare and choose winner
      const [localStats, globalStats] = await Promise.all([
        fs.stat(this.metadataPath),
        fs.stat(this.globalMetadataPath)
      ]);
      
      // Get chunk counts for comparison
      let localChunks = 0, globalChunks = 0;
      try {
        const localData = JSON.parse(await fs.readFile(this.metadataPath, 'utf-8'));
        const globalData = JSON.parse(await fs.readFile(this.globalMetadataPath, 'utf-8'));
        localChunks = localData.chunks?.length || 0;
        globalChunks = globalData.chunks?.length || 0;
      } catch (error) {
        console.warn('⚠️ Could not read chunk counts for comparison');
      }
      
      const localTime = localStats.mtime.toISOString();
      const globalTime = globalStats.mtime.toISOString();
      const globalIsNewer = globalStats.mtime > localStats.mtime;
      
      log('🔍 Storage Comparison:');
      console.log(`📁 Local:  ${localChunks} chunks, modified ${localTime}`);
      console.log(`🌐 Global: ${globalChunks} chunks, modified ${globalTime}`);
      console.log(`🏆 Winner: ${globalIsNewer ? 'Global (newer)' : 'Local (newer)'} - Loading from ${globalIsNewer ? 'global' : 'local'}`);
      
      if (globalIsNewer) {
        await this.loadPersistedIndex(true);
        await this.syncToLocal();
      } else {
        await this.loadPersistedIndex(false);
        await this.syncToGlobal();
      }
    } else if (globalExists) {
      console.log('🌐 Loading from global storage (~/.claude) - local not found');
      await this.loadPersistedIndex(true);
      await this.syncToLocal();
    } else if (localExists) {
      console.log('📁 Loading from local storage (.cortex) - global not found');
      await this.loadPersistedIndex(false);
      await this.syncToGlobal();
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
      
      console.log(`🔄 Loading persisted embeddings from ${source}...`);
      const startTime = Date.now();
      
      const indexData = await fs.readFile(indexPath, 'utf-8');
      const persistedIndex: PersistedIndex = JSON.parse(indexData);
      
      // Load chunks
      this.chunks.clear();
      for (const chunk of persistedIndex.chunks) {
        this.chunks.set(chunk.chunk_id, chunk);
      }
      
      // Note: File change detection now uses chunk-based comparison
      // No need to load file hashes - we calculate from stored chunks vs current chunks
      
      const loadTime = Date.now() - startTime;
      console.log(`✅ Loaded ${persistedIndex.chunks.length} chunks from ${source} in ${loadTime}ms`);
      console.log(`📊 Index metadata:`, persistedIndex.metadata);
      
      return true;
    } catch (error) {
      console.warn('⚠️ Failed to load persisted index:', error instanceof Error ? error.message : error);
      return false;
    }
  }

  async savePersistedIndex(modelInfo?: ModelInfo): Promise<void> {
    try {
      console.log('💾 Saving embeddings to both local and global storage...');
      const startTime = Date.now();
      
      const persistedIndex: PersistedIndex = {
        version: '1.0.0', // Legacy version field
        schemaVersion: CORTEX_SCHEMA_VERSION,
        timestamp: Date.now(),
        repositoryPath: this.repositoryPath,
        chunks: Array.from(this.chunks.values()),
        metadata: {
          totalChunks: this.chunks.size,
          lastIndexed: Date.now(),
          embeddingModel: modelInfo?.name || 'BGE-small-en-v1.5',
          modelInfo
        }
      };
      
      const indexData = JSON.stringify(persistedIndex, null, 2);
      
      // Save to both storages in parallel for better performance
      const saveLocal = async () => {
        const localTempPath = this.metadataPath + '.tmp';
        await fs.writeFile(localTempPath, indexData);
        await fs.rename(localTempPath, this.metadataPath);
      };

      const saveGlobal = async () => {
        const globalTempPath = this.globalMetadataPath + '.tmp';
        await fs.writeFile(globalTempPath, indexData);
        await fs.rename(globalTempPath, this.globalMetadataPath);
      };
      
      // Execute storage operations in parallel
      await Promise.all([saveLocal(), saveGlobal()]);
      
      const saveTime = Date.now() - startTime;
      console.log(`✅ Saved ${persistedIndex.chunks.length} chunks to both storages in ${saveTime}ms`);
      console.log(`📁 Local: ${this.metadataPath}`);
      console.log(`🌐 Global: ${this.globalMetadataPath}`);
    } catch (error) {
      console.error('❌ Failed to save persisted index:', error instanceof Error ? error.message : error);
      throw error;
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
          // Calculate hash from stored chunks
          const storedChunkContent = storedChunks
            .sort((a, b) => a.start_line - b.start_line) // Ensure consistent order by line number
            .map(chunk => chunk.content)
            .join('');
          const storedHash = crypto.createHash('sha256').update(storedChunkContent).digest('hex');
          
          // Calculate current chunk hash using provided calculator
          if (chunkHashCalculator) {
            const currentHash = await chunkHashCalculator(filePath);
            
            if (storedHash !== currentHash) {
              // File changed - chunk representation differs
              delta.fileChanges.modified.push(filePath);
              
              // Remove old chunks for this file
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
        console.warn(`⚠️ Failed to process file ${filePath}:`, error instanceof Error ? error.message : error);
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

    console.log(`📊 Applied delta: +${delta.added.length} ~${delta.updated.length} -${delta.removed.length} chunks`);
  }

  async syncToGlobal(): Promise<void> {
    try {
      if (await this.indexExists()) {
        console.log('🔄 Syncing local embeddings to global storage...');
        const indexData = await fs.readFile(this.metadataPath, 'utf-8');
        const globalTempPath = this.globalMetadataPath + '.tmp';
        await fs.writeFile(globalTempPath, indexData);
        await fs.rename(globalTempPath, this.globalMetadataPath);
        console.log('✅ Synced to global storage');
      }
    } catch (error) {
      console.warn('⚠️ Failed to sync to global storage:', error instanceof Error ? error.message : error);
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
            console.warn('⚠️ Could not read global chunk count');
          }
          
          console.log(`🔄 Syncing global embeddings to local storage (${globalChunks} chunks)...`);
          const indexData = await fs.readFile(this.globalMetadataPath, 'utf-8');
          const localTempPath = this.metadataPath + '.tmp';
          await fs.writeFile(localTempPath, indexData);
          await fs.rename(localTempPath, this.metadataPath);
          console.log(`✅ Synced ${globalChunks} chunks to local storage`);
        } else {
          console.log('📋 Local storage is up to date');
        }
      }
    } catch (error) {
      console.warn('⚠️ Failed to sync to local storage:', error instanceof Error ? error.message : error);
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
    
    // Note: No file hash storage needed - delta calculation uses chunk comparison
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
      console.warn('⚠️ Failed to clear index directory:', error instanceof Error ? error.message : error);
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
      console.warn('Could not load metadata:', error instanceof Error ? error.message : error);
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
      console.warn('Could not update metadata:', error instanceof Error ? error.message : error);
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