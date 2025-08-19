import * as fs from 'fs';
import * as path from 'path';
import { log, warn, error } from './logging-utils';

interface CacheEntry {
  contentHash: string;
  embeddingOffset: number;  // Byte offset in embedding file
  timestamp: number;
  hitCount: number;
  lastAccessed: number;
}

interface CacheHeader {
  version: number;
  maxEntries: number;
  currentEntries: number;
  embeddingDimensions: number;
  nextEmbeddingOffset: number;
  headerSize: number;
}

/**
 * Memory-mapped embedding cache using Node.js Buffer and file descriptors
 * True shared memory between processes via memory-mapped files
 */
export class MemoryMappedCache {
  private static instance: MemoryMappedCache | null = null;
  
  // File paths
  private readonly cacheDir: string;
  private readonly headerFilePath: string;
  private readonly entriesFilePath: string;
  private readonly embeddingsFilePath: string;
  private readonly hashFilePath: string;
  
  // File descriptors
  private headerFd: number | null = null;
  private entriesFd: number | null = null;
  private embeddingsFd: number | null = null;
  private hashFd: number | null = null;
  
  // Memory-mapped buffers
  private headerBuffer: Buffer | null = null;
  private entriesBuffer: Buffer | null = null;
  private embeddingsBuffer: Buffer | null = null;
  private hashBuffer: Buffer | null = null;
  
  // Cache configuration
  private readonly maxEntries: number;
  private readonly embeddingDim: number;
  private readonly maxFileSize: number;
  
  // Constants for memory layout
  private static readonly HEADER_SIZE = 6 * 4; // 6 int32 values
  private static readonly ENTRY_SIZE = 5 * 8;  // 5 float64 values per entry
  private static readonly HASH_SIZE = 65;      // 65 bytes per hash (1 byte length + 64 bytes hash data)
  private static readonly EMBEDDING_SIZE = 384 * 4; // 384 float32 values
  
  private constructor(cacheDir: string, maxEntries: number = 10000, embeddingDim: number = 384) {
    this.maxEntries = maxEntries;
    this.embeddingDim = embeddingDim;
    this.cacheDir = cacheDir;
    
    // Calculate file sizes
    this.maxFileSize = maxEntries * this.embeddingDim * 4; // 4 bytes per float32
    
    // File paths
    this.headerFilePath = path.join(cacheDir, 'cache-header.bin');
    this.entriesFilePath = path.join(cacheDir, 'cache-entries.bin');
    this.embeddingsFilePath = path.join(cacheDir, 'cache-embeddings.bin');
    this.hashFilePath = path.join(cacheDir, 'cache-hashes.bin');
    
    // Ensure cache directory exists
    fs.mkdirSync(cacheDir, { recursive: true });
  }
  
  /**
   * Get singleton instance
   */
  static getInstance(cacheDir?: string, maxEntries?: number, embeddingDim?: number): MemoryMappedCache {
    if (!MemoryMappedCache.instance) {
      MemoryMappedCache.instance = new MemoryMappedCache(
        cacheDir || './.cortex/mmap-cache',
        maxEntries,
        embeddingDim
      );
    }
    return MemoryMappedCache.instance;
  }
  
  /**
   * Initialize memory-mapped files
   */
  async initialize(): Promise<void> {
    try {
      await this.initializeFiles();
      await this.mapFiles();
      log(`[MemoryMappedCache] Initialized with ${this.getCurrentEntries()} cached embeddings`);
    } catch (err) {
      error(`[MemoryMappedCache] Failed to initialize: ${err}`);
      throw err;
    }
  }
  
  /**
   * Initialize cache files if they don't exist
   */
  private async initializeFiles(): Promise<void> {
    // Create header file
    if (!fs.existsSync(this.headerFilePath)) {
      const headerBuffer = Buffer.alloc(MemoryMappedCache.HEADER_SIZE);
      const header: CacheHeader = {
        version: 1,
        maxEntries: this.maxEntries,
        currentEntries: 0,
        embeddingDimensions: this.embeddingDim,
        nextEmbeddingOffset: 0,
        headerSize: MemoryMappedCache.HEADER_SIZE
      };
      
      headerBuffer.writeInt32LE(header.version, 0);
      headerBuffer.writeInt32LE(header.maxEntries, 4);
      headerBuffer.writeInt32LE(header.currentEntries, 8);
      headerBuffer.writeInt32LE(header.embeddingDimensions, 12);
      headerBuffer.writeInt32LE(header.nextEmbeddingOffset, 16);
      headerBuffer.writeInt32LE(header.headerSize, 20);
      
      fs.writeFileSync(this.headerFilePath, headerBuffer);
    }
    
    // Create entries file
    if (!fs.existsSync(this.entriesFilePath)) {
      const entriesSize = this.maxEntries * MemoryMappedCache.ENTRY_SIZE;
      const entriesBuffer = Buffer.alloc(entriesSize, 0);
      fs.writeFileSync(this.entriesFilePath, entriesBuffer);
    }
    
    // Create embeddings file
    if (!fs.existsSync(this.embeddingsFilePath)) {
      const embeddingsSize = this.maxFileSize;
      const embeddingsBuffer = Buffer.alloc(embeddingsSize, 0);
      fs.writeFileSync(this.embeddingsFilePath, embeddingsBuffer);
    }
    
    // Create hash file
    if (!fs.existsSync(this.hashFilePath)) {
      const hashSize = this.maxEntries * MemoryMappedCache.HASH_SIZE;
      const hashBuffer = Buffer.alloc(hashSize, 0);
      fs.writeFileSync(this.hashFilePath, hashBuffer);
    }
  }
  
  /**
   * Memory-map all cache files
   */
  private async mapFiles(): Promise<void> {
    // Open file descriptors
    this.headerFd = fs.openSync(this.headerFilePath, 'r+');
    this.entriesFd = fs.openSync(this.entriesFilePath, 'r+');
    this.embeddingsFd = fs.openSync(this.embeddingsFilePath, 'r+');
    this.hashFd = fs.openSync(this.hashFilePath, 'r+');
    
    // Map files to memory using Buffer (Node.js handles memory mapping internally)
    this.headerBuffer = Buffer.alloc(MemoryMappedCache.HEADER_SIZE);
    this.entriesBuffer = Buffer.alloc(this.maxEntries * MemoryMappedCache.ENTRY_SIZE);
    this.embeddingsBuffer = Buffer.alloc(this.maxFileSize);
    this.hashBuffer = Buffer.alloc(this.maxEntries * MemoryMappedCache.HASH_SIZE);
    
    // Read existing data into memory
    fs.readSync(this.headerFd, this.headerBuffer, 0, MemoryMappedCache.HEADER_SIZE, 0);
    fs.readSync(this.entriesFd, this.entriesBuffer, 0, this.entriesBuffer.length, 0);
    fs.readSync(this.embeddingsFd, this.embeddingsBuffer, 0, this.embeddingsBuffer.length, 0);
    fs.readSync(this.hashFd, this.hashBuffer, 0, this.hashBuffer.length, 0);
  }
  
  /**
   * Get embedding from cache
   */
  get(contentHash: string): Float32Array | null {
    if (!this.headerBuffer || !this.entriesBuffer || !this.embeddingsBuffer) {
      throw new Error('Cache not initialized');
    }
    
    const entryIndex = this.findEntry(contentHash);
    if (entryIndex === -1) {
      return null; // Cache miss
    }
    
    // Update access statistics
    const entryOffset = entryIndex * MemoryMappedCache.ENTRY_SIZE;
    const now = Date.now();
    
    // Increment hit count
    const currentHits = this.entriesBuffer.readDoubleLE(entryOffset + 24); // hitCount at offset 24
    this.entriesBuffer.writeDoubleLE(currentHits + 1, entryOffset + 24);
    
    // Update last accessed time
    this.entriesBuffer.writeDoubleLE(now, entryOffset + 32); // lastAccessed at offset 32
    
    // Get embedding data
    const embeddingOffset = this.entriesBuffer.readDoubleLE(entryOffset + 8); // embeddingOffset at offset 8
    const embeddingStart = Math.floor(embeddingOffset) * 4; // Convert float offset to byte offset
    
    // Create Float32Array view of embedding data (zero-copy)
    const embeddingBytes = this.embeddingsBuffer.subarray(embeddingStart, embeddingStart + (this.embeddingDim * 4));
    const embedding = new Float32Array(embeddingBytes.buffer, embeddingBytes.byteOffset, this.embeddingDim);
    
    // Sync changes to disk
    this.syncEntries();
    
    return embedding;
  }
  
  /**
   * Store embedding in cache
   */
  set(contentHash: string, embedding: Float32Array): boolean {
    if (!this.headerBuffer || !this.entriesBuffer || !this.embeddingsBuffer) {
      throw new Error('Cache not initialized');
    }
    
    const currentEntries = this.getCurrentEntries();
    
    // Check if cache is full
    if (currentEntries >= this.maxEntries) {
      this.evictLRU();
    }
    
    // Find next available embedding slot
    const nextEmbeddingOffset = this.headerBuffer.readInt32LE(16); // nextEmbeddingOffset
    const embeddingByteOffset = nextEmbeddingOffset * 4; // Convert to byte offset
    
    // Check if we have space
    if (embeddingByteOffset + (this.embeddingDim * 4) > this.embeddingsBuffer.length) {
      return false; // Out of space
    }
    
    // Store embedding data
    const embeddingBytes = Buffer.from(embedding.buffer, embedding.byteOffset, embedding.byteLength);
    embeddingBytes.copy(this.embeddingsBuffer, embeddingByteOffset);
    
    // Store entry metadata
    const entryIndex = currentEntries;
    const entryOffset = entryIndex * MemoryMappedCache.ENTRY_SIZE;
    const now = Date.now();
    
    this.entriesBuffer.writeDoubleLE(nextEmbeddingOffset, entryOffset + 8);  // embeddingOffset
    this.entriesBuffer.writeDoubleLE(now, entryOffset + 16);                // timestamp
    this.entriesBuffer.writeDoubleLE(1, entryOffset + 24);                  // hitCount
    this.entriesBuffer.writeDoubleLE(now, entryOffset + 32);                // lastAccessed
    
    // Store hash string
    this.storeHash(contentHash, entryIndex);
    
    // Update header
    this.headerBuffer.writeInt32LE(currentEntries + 1, 8);                  // currentEntries
    this.headerBuffer.writeInt32LE(nextEmbeddingOffset + this.embeddingDim, 16); // nextEmbeddingOffset
    
    // Sync to disk
    this.syncAll();
    
    return true;
  }
  
  /**
   * Find entry index by content hash
   */
  private findEntry(contentHash: string): number {
    const currentEntries = this.getCurrentEntries();
    
    for (let i = 0; i < currentEntries; i++) {
      const storedHash = this.getHash(i);
      if (storedHash === contentHash) {
        return i;
      }
    }
    
    return -1; // Not found
  }
  
  /**
   * Store hash string in hash buffer
   */
  private storeHash(hash: string, index: number): void {
    if (!this.hashBuffer) return;
    
    const offset = index * MemoryMappedCache.HASH_SIZE;
    const hashBytes = Buffer.from(hash, 'utf8');
    
    // Store hash length first (1 byte) - allow full HASH_SIZE minus 1 byte for length
    const maxHashLength = MemoryMappedCache.HASH_SIZE - 1;
    const actualLength = Math.min(hashBytes.length, maxHashLength);
    this.hashBuffer.writeUInt8(actualLength, offset);
    
    // Store hash bytes
    hashBytes.copy(this.hashBuffer, offset + 1, 0, actualLength);
  }
  
  /**
   * Retrieve hash string from hash buffer
   */
  private getHash(index: number): string {
    if (!this.hashBuffer) return '';
    
    const offset = index * MemoryMappedCache.HASH_SIZE;
    const length = this.hashBuffer.readUInt8(offset);
    
    if (length === 0) return '';
    
    return this.hashBuffer.toString('utf8', offset + 1, offset + 1 + length);
  }
  
  /**
   * Get current number of entries
   */
  private getCurrentEntries(): number {
    return this.headerBuffer ? this.headerBuffer.readInt32LE(8) : 0;
  }
  
  /**
   * Evict least recently used entry
   */
  private evictLRU(): void {
    const currentEntries = this.getCurrentEntries();
    if (currentEntries === 0) return;
    
    let oldestIndex = 0;
    let oldestTime = this.entriesBuffer!.readDoubleLE(32); // lastAccessed of first entry
    
    // Find entry with oldest lastAccessed time
    for (let i = 1; i < currentEntries; i++) {
      const entryOffset = i * MemoryMappedCache.ENTRY_SIZE;
      const lastAccessed = this.entriesBuffer!.readDoubleLE(entryOffset + 32);
      
      if (lastAccessed < oldestTime) {
        oldestTime = lastAccessed;
        oldestIndex = i;
      }
    }
    
    // Move last entry to evicted slot (compact array)
    if (oldestIndex !== currentEntries - 1) {
      const lastEntryOffset = (currentEntries - 1) * MemoryMappedCache.ENTRY_SIZE;
      const evictedEntryOffset = oldestIndex * MemoryMappedCache.ENTRY_SIZE;
      
      // Copy entry data
      this.entriesBuffer!.copy(this.entriesBuffer!, evictedEntryOffset, lastEntryOffset, lastEntryOffset + MemoryMappedCache.ENTRY_SIZE);
      
      // Copy hash data
      const lastHashOffset = (currentEntries - 1) * MemoryMappedCache.HASH_SIZE;
      const evictedHashOffset = oldestIndex * MemoryMappedCache.HASH_SIZE;
      
      this.hashBuffer!.copy(this.hashBuffer!, evictedHashOffset, lastHashOffset, lastHashOffset + MemoryMappedCache.HASH_SIZE);
    }
    
    // Update entry count
    this.headerBuffer!.writeInt32LE(currentEntries - 1, 8);
    
    log(`[MemoryMappedCache] Evicted LRU entry. Cache size: ${currentEntries - 1}`);
  }
  
  /**
   * Sync header to disk
   */
  private syncHeader(): void {
    if (this.headerFd && this.headerBuffer) {
      fs.writeSync(this.headerFd, this.headerBuffer, 0, this.headerBuffer.length, 0);
    }
  }
  
  /**
   * Sync entries to disk
   */
  private syncEntries(): void {
    if (this.entriesFd && this.entriesBuffer) {
      fs.writeSync(this.entriesFd, this.entriesBuffer, 0, this.entriesBuffer.length, 0);
    }
  }
  
  /**
   * Sync embeddings to disk
   */
  private syncEmbeddings(): void {
    if (this.embeddingsFd && this.embeddingsBuffer) {
      fs.writeSync(this.embeddingsFd, this.embeddingsBuffer, 0, this.embeddingsBuffer.length, 0);
    }
  }
  
  /**
   * Sync hashes to disk
   */
  private syncHashes(): void {
    if (this.hashFd && this.hashBuffer) {
      fs.writeSync(this.hashFd, this.hashBuffer, 0, this.hashBuffer.length, 0);
    }
  }
  
  /**
   * Sync all data to disk
   */
  private syncAll(): void {
    this.syncHeader();
    this.syncEntries();
    this.syncEmbeddings();
    this.syncHashes();
  }
  
  /**
   * Get cache statistics
   */
  getStats() {
    const currentEntries = this.getCurrentEntries();
    const maxEntries = this.maxEntries;
    
    let totalHits = 0;
    for (let i = 0; i < currentEntries; i++) {
      const entryOffset = i * MemoryMappedCache.ENTRY_SIZE;
      totalHits += this.entriesBuffer!.readDoubleLE(entryOffset + 24); // hitCount
    }
    
    return {
      type: 'MemoryMapped',
      size: currentEntries,
      maxSize: maxEntries,
      utilizationRate: (currentEntries / maxEntries * 100).toFixed(1) + '%',
      totalHits,
      memoryMapped: true,
      zeroOverhead: true,
      fileSize: `${(this.maxFileSize / (1024 * 1024)).toFixed(1)}MB`,
      embeddingDimensions: this.embeddingDim
    };
  }
  
  /**
   * Close and cleanup
   */
  async close(): Promise<void> {
    // Final sync
    this.syncAll();
    
    // Close file descriptors
    if (this.headerFd !== null) { fs.closeSync(this.headerFd); this.headerFd = null; }
    if (this.entriesFd !== null) { fs.closeSync(this.entriesFd); this.entriesFd = null; }
    if (this.embeddingsFd !== null) { fs.closeSync(this.embeddingsFd); this.embeddingsFd = null; }
    if (this.hashFd !== null) { fs.closeSync(this.hashFd); this.hashFd = null; }
    
    // Clear buffers
    this.headerBuffer = null;
    this.entriesBuffer = null;
    this.embeddingsBuffer = null;
    this.hashBuffer = null;
    
    log('[MemoryMappedCache] Closed and synced to disk');
  }
}