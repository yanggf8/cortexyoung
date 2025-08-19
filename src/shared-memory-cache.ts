/**
 * SharedArrayBuffer-based embedding cache for true shared memory
 * across child processes with zero I/O overhead via IPC
 */

interface CacheEntry {
  contentHash: string;
  embeddingStart: number;  // Offset in embedding buffer
  embeddingLength: number; // Length of embedding (384 for BGE)
  timestamp: number;
  hitCount: number;
  lastAccessed: number;
}

interface CacheHeader {
  maxEntries: number;
  currentEntries: number;
  embeddingDimensions: number;
  nextEmbeddingOffset: number;
  totalMemorySize: number;
}

/**
 * High-performance shared memory cache using SharedArrayBuffer
 * Zero-copy embedding storage with atomic operations
 */
export class SharedMemoryCache {
  private static instance: SharedMemoryCache | null = null;
  
  // SharedArrayBuffers
  private headerBuffer: SharedArrayBuffer;
  private entriesBuffer: SharedArrayBuffer;
  private embeddingsBuffer: SharedArrayBuffer;
  private stringBuffer: SharedArrayBuffer;
  
  // Typed array views
  private headerView: Int32Array;
  private entriesView: Float64Array;
  private embeddingsView: Float32Array;
  private stringView: Uint8Array;
  
  // Cache configuration
  private readonly maxEntries: number;
  private readonly embeddingDim: number;
  private readonly maxStringBytes: number;
  
  // Memory layout constants
  private static readonly HEADER_SIZE = 5 * 4; // 5 int32 values
  private static readonly ENTRY_SIZE = 6 * 8;  // 6 float64 values per entry
  private static readonly EMBEDDING_DIM = 384; // BGE-small-en-v1.5 dimensions
  
  private constructor(maxEntries: number = 10000, embeddingDim: number = 384) {
    this.maxEntries = maxEntries;
    this.embeddingDim = embeddingDim;
    this.maxStringBytes = maxEntries * 64; // 64 bytes per hash string average
    
    // Calculate buffer sizes
    const headerSize = SharedMemoryCache.HEADER_SIZE;
    const entriesSize = maxEntries * SharedMemoryCache.ENTRY_SIZE;
    const embeddingsSize = maxEntries * embeddingDim * 4; // Float32 = 4 bytes
    const stringSize = this.maxStringBytes;
    
    // Create SharedArrayBuffers
    this.headerBuffer = new SharedArrayBuffer(headerSize);
    this.entriesBuffer = new SharedArrayBuffer(entriesSize);
    this.embeddingsBuffer = new SharedArrayBuffer(embeddingsSize);
    this.stringBuffer = new SharedArrayBuffer(stringSize);
    
    // Create typed array views
    this.headerView = new Int32Array(this.headerBuffer);
    this.entriesView = new Float64Array(this.entriesBuffer);
    this.embeddingsView = new Float32Array(this.embeddingsBuffer);
    this.stringView = new Uint8Array(this.stringBuffer);
    
    // Initialize header
    this.initializeHeader();
  }
  
  /**
   * Get singleton instance with shared memory
   */
  static getInstance(maxEntries?: number, embeddingDim?: number): SharedMemoryCache {
    if (!SharedMemoryCache.instance) {
      SharedMemoryCache.instance = new SharedMemoryCache(maxEntries, embeddingDim);
    }
    return SharedMemoryCache.instance;
  }
  
  /**
   * Create instance from existing SharedArrayBuffers (for worker threads)
   */
  static fromSharedBuffers(
    headerBuffer: SharedArrayBuffer,
    entriesBuffer: SharedArrayBuffer, 
    embeddingsBuffer: SharedArrayBuffer,
    stringBuffer: SharedArrayBuffer
  ): SharedMemoryCache {
    const instance = Object.create(SharedMemoryCache.prototype);
    
    instance.headerBuffer = headerBuffer;
    instance.entriesBuffer = entriesBuffer;
    instance.embeddingsBuffer = embeddingsBuffer;
    instance.stringBuffer = stringBuffer;
    
    instance.headerView = new Int32Array(headerBuffer);
    instance.entriesView = new Float64Array(entriesBuffer);
    instance.embeddingsView = new Float32Array(embeddingsBuffer);
    instance.stringView = new Uint8Array(stringBuffer);
    
    // Read configuration from header
    instance.maxEntries = instance.headerView[0];
    instance.embeddingDim = instance.headerView[2];
    instance.maxStringBytes = instance.maxEntries * 64;
    
    return instance;
  }
  
  /**
   * Get shared buffers for passing to worker threads
   */
  getSharedBuffers() {
    return {
      headerBuffer: this.headerBuffer,
      entriesBuffer: this.entriesBuffer,
      embeddingsBuffer: this.embeddingsBuffer,
      stringBuffer: this.stringBuffer
    };
  }
  
  /**
   * Initialize cache header with atomic operations
   */
  private initializeHeader(): void {
    Atomics.store(this.headerView, 0, this.maxEntries);        // maxEntries
    Atomics.store(this.headerView, 1, 0);                      // currentEntries
    Atomics.store(this.headerView, 2, this.embeddingDim);      // embeddingDimensions
    Atomics.store(this.headerView, 3, 0);                      // nextEmbeddingOffset
    Atomics.store(this.headerView, 4, 
      this.headerBuffer.byteLength + 
      this.entriesBuffer.byteLength + 
      this.embeddingsBuffer.byteLength + 
      this.stringBuffer.byteLength
    ); // totalMemorySize
  }
  
  /**
   * Get embedding from cache with zero-copy access
   */
  get(contentHash: string): Float32Array | null {
    const entryIndex = this.findEntry(contentHash);
    if (entryIndex === -1) {
      return null; // Cache miss
    }
    
    // Update access statistics atomically
    const now = Date.now();
    const entryOffset = entryIndex * 6; // 6 float64 values per entry
    
    // Increment hit count
    const currentHits = this.entriesView[entryOffset + 4];
    this.entriesView[entryOffset + 4] = currentHits + 1;
    
    // Update last accessed time
    this.entriesView[entryOffset + 5] = now;
    
    // Get embedding data (zero-copy view)
    const embeddingStart = Math.floor(this.entriesView[entryOffset + 1]);
    const embeddingLength = Math.floor(this.entriesView[entryOffset + 2]);
    
    return this.embeddingsView.subarray(embeddingStart, embeddingStart + embeddingLength);
  }
  
  /**
   * Store embedding in cache with atomic operations
   */
  set(contentHash: string, embedding: Float32Array): boolean {
    const currentEntries = Atomics.load(this.headerView, 1);
    
    // Check if cache is full
    if (currentEntries >= this.maxEntries) {
      this.evictLRU();
    }
    
    // Find next available embedding slot
    const embeddingStart = Atomics.load(this.headerView, 3);
    const embeddingEnd = embeddingStart + embedding.length;
    
    // Check if we have space for embedding data
    if (embeddingEnd > this.embeddingsView.length) {
      return false; // Out of embedding space
    }
    
    // Store embedding data
    this.embeddingsView.set(embedding, embeddingStart);
    
    // Store entry metadata
    const entryIndex = currentEntries;
    const entryOffset = entryIndex * 6;
    const now = Date.now();
    
    this.storeString(contentHash, entryIndex);
    this.entriesView[entryOffset + 1] = embeddingStart;        // embeddingStart
    this.entriesView[entryOffset + 2] = embedding.length;      // embeddingLength  
    this.entriesView[entryOffset + 3] = now;                   // timestamp
    this.entriesView[entryOffset + 4] = 1;                     // hitCount
    this.entriesView[entryOffset + 5] = now;                   // lastAccessed
    
    // Update header atomically
    Atomics.store(this.headerView, 1, currentEntries + 1);    // currentEntries
    Atomics.store(this.headerView, 3, embeddingEnd);          // nextEmbeddingOffset
    
    return true;
  }
  
  /**
   * Find entry index by content hash
   */
  private findEntry(contentHash: string): number {
    const currentEntries = Atomics.load(this.headerView, 1);
    
    for (let i = 0; i < currentEntries; i++) {
      const storedHash = this.getString(i);
      if (storedHash === contentHash) {
        return i;
      }
    }
    
    return -1; // Not found
  }
  
  /**
   * Store string in shared string buffer
   */
  private storeString(str: string, index: number): void {
    const encoder = new TextEncoder();
    const bytes = encoder.encode(str);
    const maxBytesPerString = 64;
    const startOffset = index * maxBytesPerString;
    
    // Store string length first
    this.stringView[startOffset] = Math.min(bytes.length, maxBytesPerString - 1);
    
    // Store string bytes
    const actualLength = Math.min(bytes.length, maxBytesPerString - 1);
    this.stringView.set(bytes.subarray(0, actualLength), startOffset + 1);
  }
  
  /**
   * Retrieve string from shared string buffer
   */
  private getString(index: number): string {
    const maxBytesPerString = 64;
    const startOffset = index * maxBytesPerString;
    
    // Get string length
    const length = this.stringView[startOffset];
    
    // Get string bytes
    const bytes = this.stringView.subarray(startOffset + 1, startOffset + 1 + length);
    
    const decoder = new TextDecoder();
    return decoder.decode(bytes);
  }
  
  /**
   * Evict least recently used entry
   */
  private evictLRU(): void {
    const currentEntries = Atomics.load(this.headerView, 1);
    if (currentEntries === 0) return;
    
    let oldestIndex = 0;
    let oldestTime = this.entriesView[5]; // lastAccessed of first entry
    
    // Find entry with oldest lastAccessed time
    for (let i = 1; i < currentEntries; i++) {
      const entryOffset = i * 6;
      const lastAccessed = this.entriesView[entryOffset + 5];
      
      if (lastAccessed < oldestTime) {
        oldestTime = lastAccessed;
        oldestIndex = i;
      }
    }
    
    // Move last entry to evicted slot (compact array)
    if (oldestIndex !== currentEntries - 1) {
      const lastEntryOffset = (currentEntries - 1) * 6;
      const evictedEntryOffset = oldestIndex * 6;
      
      // Copy entry data
      for (let j = 0; j < 6; j++) {
        this.entriesView[evictedEntryOffset + j] = this.entriesView[lastEntryOffset + j];
      }
      
      // Copy string data
      const maxBytesPerString = 64;
      const lastStringOffset = (currentEntries - 1) * maxBytesPerString;
      const evictedStringOffset = oldestIndex * maxBytesPerString;
      
      this.stringView.set(
        this.stringView.subarray(lastStringOffset, lastStringOffset + maxBytesPerString),
        evictedStringOffset
      );
    }
    
    // Update entry count
    Atomics.store(this.headerView, 1, currentEntries - 1);
  }
  
  /**
   * Get cache statistics
   */
  getStats() {
    const currentEntries = Atomics.load(this.headerView, 1);
    const maxEntries = Atomics.load(this.headerView, 0);
    const totalMemory = Atomics.load(this.headerView, 4);
    
    let totalHits = 0;
    for (let i = 0; i < currentEntries; i++) {
      const entryOffset = i * 6;
      totalHits += this.entriesView[entryOffset + 4]; // hitCount
    }
    
    return {
      type: 'SharedArrayBuffer',
      size: currentEntries,
      maxSize: maxEntries,
      utilizationRate: (currentEntries / maxEntries * 100).toFixed(1) + '%',
      totalHits,
      memoryUsed: `${(totalMemory / (1024 * 1024)).toFixed(1)}MB`,
      embeddingDimensions: this.embeddingDim,
      isShared: true,
      zeroOverhead: true
    };
  }
  
  /**
   * Clear all cache entries
   */
  clear(): void {
    Atomics.store(this.headerView, 1, 0); // currentEntries = 0
    Atomics.store(this.headerView, 3, 0); // nextEmbeddingOffset = 0
  }
}