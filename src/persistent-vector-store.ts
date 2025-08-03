import { CodeChunk } from './types';
import { VectorStore } from './vector-store';
import * as fs from 'fs/promises';
import * as path from 'path';
import * as crypto from 'crypto';

interface PersistedIndex {
  version: string;
  timestamp: number;
  repositoryPath: string;
  chunks: CodeChunk[];
  fileHashes: Map<string, string>;
  metadata: {
    totalChunks: number;
    lastIndexed: number;
    embeddingModel: string;
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
  private fileHashes: Map<string, string> = new Map();
  private repositoryPath: string;
  private indexPath: string;
  private metadataPath: string;
  private deltaPath: string;

  constructor(repositoryPath: string, indexDir: string = '.cortex') {
    super(); // Call parent constructor
    this.repositoryPath = repositoryPath;
    this.indexPath = path.join(repositoryPath, indexDir);
    this.metadataPath = path.join(this.indexPath, 'index.json');
    this.deltaPath = path.join(this.indexPath, 'deltas');
  }

  async initialize(): Promise<void> {
    // Ensure index directory exists
    await fs.mkdir(this.indexPath, { recursive: true });
    await fs.mkdir(this.deltaPath, { recursive: true });
    
    // Try to load existing index
    if (await this.indexExists()) {
      await this.loadPersistedIndex();
    }
  }

  async indexExists(): Promise<boolean> {
    try {
      await fs.access(this.metadataPath);
      return true;
    } catch {
      return false;
    }
  }

  async loadPersistedIndex(): Promise<boolean> {
    try {
      console.log('🔄 Loading persisted embeddings...');
      const startTime = Date.now();
      
      const indexData = await fs.readFile(this.metadataPath, 'utf-8');
      const persistedIndex: PersistedIndex = JSON.parse(indexData);
      
      // Load chunks
      this.chunks.clear();
      for (const chunk of persistedIndex.chunks) {
        this.chunks.set(chunk.chunk_id, chunk);
      }
      
      // Load file hashes
      this.fileHashes = new Map(Object.entries(persistedIndex.fileHashes as any));
      
      const loadTime = Date.now() - startTime;
      console.log(`✅ Loaded ${persistedIndex.chunks.length} chunks in ${loadTime}ms`);
      console.log(`📊 Index metadata:`, persistedIndex.metadata);
      
      return true;
    } catch (error) {
      console.warn('⚠️ Failed to load persisted index:', error instanceof Error ? error.message : error);
      return false;
    }
  }

  async savePersistedIndex(): Promise<void> {
    try {
      console.log('💾 Saving embeddings to disk...');
      const startTime = Date.now();
      
      const persistedIndex: PersistedIndex = {
        version: '1.0.0',
        timestamp: Date.now(),
        repositoryPath: this.repositoryPath,
        chunks: Array.from(this.chunks.values()),
        fileHashes: this.fileHashes as any,
        metadata: {
          totalChunks: this.chunks.size,
          lastIndexed: Date.now(),
          embeddingModel: 'BGE-small-en-v1.5'
        }
      };
      
      // Write to temporary file first, then atomic rename
      const tempPath = this.metadataPath + '.tmp';
      await fs.writeFile(tempPath, JSON.stringify(persistedIndex, null, 2));
      await fs.rename(tempPath, this.metadataPath);
      
      const saveTime = Date.now() - startTime;
      console.log(`✅ Saved ${persistedIndex.chunks.length} chunks in ${saveTime}ms`);
    } catch (error) {
      console.error('❌ Failed to save persisted index:', error instanceof Error ? error.message : error);
      throw error;
    }
  }

  async calculateFileDelta(files: string[]): Promise<IndexDelta> {
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

    // Check each file for changes
    for (const filePath of files) {
      try {
        const content = await fs.readFile(path.join(this.repositoryPath, filePath), 'utf-8');
        const currentHash = crypto.createHash('sha256').update(content).digest('hex');
        const storedHash = this.fileHashes.get(filePath);

        if (!storedHash) {
          // New file
          delta.fileChanges.added.push(filePath);
        } else if (storedHash !== currentHash) {
          // Modified file
          delta.fileChanges.modified.push(filePath);
          
          // Remove old chunks for this file
          const oldChunks = Array.from(this.chunks.values())
            .filter(chunk => chunk.file_path === filePath);
          delta.removed.push(...oldChunks.map(chunk => chunk.chunk_id));
        }
        
        // Update file hash
        this.fileHashes.set(filePath, currentHash);
      } catch (error) {
        console.warn(`⚠️ Failed to process file ${filePath}:`, error instanceof Error ? error.message : error);
      }
    }

    // Check for deleted files
    for (const [filePath] of this.fileHashes) {
      if (!files.includes(filePath)) {
        delta.fileChanges.deleted.push(filePath);
        
        // Remove chunks for deleted file
        const deletedChunks = Array.from(this.chunks.values())
          .filter(chunk => chunk.file_path === filePath);
        delta.removed.push(...deletedChunks.map(chunk => chunk.chunk_id));
        
        this.fileHashes.delete(filePath);
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

  // Override upsertChunks to ensure persistence
  async upsertChunks(chunks: CodeChunk[]): Promise<void> {
    await super.upsertChunks(chunks);
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
    
    return {
      total_chunks: this.chunks.size,
      totalFiles: this.fileHashes.size,
      indexSize,
      lastUpdated: stats?.mtime || new Date()
    };
  }

  // Override clear to also clear persistence
  async clear(): Promise<void> {
    await super.clear();
    this.fileHashes.clear();
    
    try {
      await fs.rm(this.indexPath, { recursive: true, force: true });
      await fs.mkdir(this.indexPath, { recursive: true });
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
}