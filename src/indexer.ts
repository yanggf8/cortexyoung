import { CodeChunk, IndexRequest, IndexResponse, QueryRequest, QueryResponse, IEmbedder } from './types';
import { GitScanner } from './git-scanner';
import { SmartChunker } from './chunker';
import * as crypto from 'crypto';
import * as path from 'path';
import { EmbeddingGenerator } from './embedder';
import { VectorStore } from './vector-store';
import { PersistentVectorStore } from './persistent-vector-store';
import { SemanticSearcher } from './searcher';
import { StartupStageTracker } from './startup-stages';
import { ReindexAdvisor } from './reindex-advisor';
import { UnifiedStorageCoordinator } from './unified-storage-coordinator';
import { FastQEmbedder } from './fastq-embedder';
import { ProcessPoolEmbedder } from './process-pool-embedder';
import { CloudflareAIEmbedder } from './cloudflare-ai-embedder';
import { DependencyMapper } from './dependency-mapper';
import { EmbeddingStrategyManager } from './embedding-strategy';
import { log, warn, error } from './logging-utils';
import { EmbeddingBackupUtility } from './embedding-backup-utility';
import { MMRConfigManager, createMMRConfigFromEnvironment } from './mmr-config-manager';
import { SemanticWatcher } from './semantic-watcher';
import { ContextInvalidator } from './context-invalidator';
import * as os from 'os';

export class CodebaseIndexer {
  private gitScanner: GitScanner;
  private chunker: SmartChunker;
  private embedder: EmbeddingGenerator;
  private unifiedEmbedder?: IEmbedder; // Unified interface for all embedding providers
  private strategyManager: EmbeddingStrategyManager;
  private vectorStore: PersistentVectorStore;
  private searcher: SemanticSearcher;
  private dependencyMapper: DependencyMapper;
  private repositoryPath: string;
  private stageTracker?: StartupStageTracker;
  private reindexAdvisor: ReindexAdvisor;
  private storageCoordinator: UnifiedStorageCoordinator;
  private semanticWatcher?: SemanticWatcher;
  private contextInvalidator?: ContextInvalidator;

  constructor(repositoryPath: string, stageTracker?: StartupStageTracker) {
    this.repositoryPath = repositoryPath;
    this.gitScanner = new GitScanner(repositoryPath);
    this.chunker = new SmartChunker();
    this.embedder = new EmbeddingGenerator();
    this.strategyManager = new EmbeddingStrategyManager(repositoryPath);
    this.storageCoordinator = new UnifiedStorageCoordinator(repositoryPath);
    this.vectorStore = this.storageCoordinator.getVectorStore();
    
    // Initialize MMR configuration
    const mmrConfigManager = new MMRConfigManager(repositoryPath);
    const envOverrides = createMMRConfigFromEnvironment();
    let mmrConfig = undefined;
    
    if (envOverrides) {
      log('[Indexer] Applying MMR configuration overrides from environment');
      mmrConfig = envOverrides;
    }
    
    this.searcher = new SemanticSearcher(this.vectorStore, this.embedder, repositoryPath, mmrConfig);
    this.dependencyMapper = new DependencyMapper(repositoryPath);
    this.stageTracker = stageTracker;
    this.reindexAdvisor = new ReindexAdvisor(this.embedder);
  }

  /**
   * Clean up resources for unified embedder and strategy manager
   */
  async cleanup(reason: string = 'cleanup'): Promise<void> {
    if (this.unifiedEmbedder) {
      log(`🧹 Cleaning up unified embedder (${this.unifiedEmbedder.providerId}, reason: ${reason})...`);
      
      // Only ProcessPoolEmbedder requires shutdown
      if (this.unifiedEmbedder instanceof ProcessPoolEmbedder) {
        await this.unifiedEmbedder.shutdown(reason);
      }
      
      this.unifiedEmbedder = undefined;
    }

    if (this.strategyManager) {
      await this.strategyManager.cleanup();
    }
  }

  /**
   * Creates a backup of current embedding data before destructive operations
   * Only backs up valid data (chunk count > 0, valid JSON)
   */
  private async createPreRebuildBackup(reason: string): Promise<void> {
    try {
      // Get the local storage directory from the vector store
      const localIndexDir = path.dirname((this.vectorStore as any).metadataPath);
      
      // Create validated backup
      const backupResult = await EmbeddingBackupUtility.createValidatedBackup(
        localIndexDir, 
        reason
      );
      
      if (backupResult.success) {
        log(`✅ Pre-rebuild backup created: ${backupResult.backupPath}`);
        log(`📊 Backed up ${backupResult.validationResult.chunkCount} chunks safely`);
      } else {
        log(`ℹ️  Backup skipped: ${backupResult.skipReason}`);
      }
    } catch (error) {
      warn(`⚠️  Backup creation failed: ${error instanceof Error ? error.message : 'Unknown error'}`);
      log('   Proceeding with rebuild (no valid data to lose)');
    }
  }

  async indexRepository(request: IndexRequest): Promise<IndexResponse> {
    const startTime = Date.now();
    
    try {
      // Start model loading early to overlap with storage initialization
      const modelLoadPromise = this.embedder.getModelInfo();
      
      // Initialize unified storage coordinator
      await this.storageCoordinator.initialize();
      
      // Ensure model is loaded before we need it
      await modelLoadPromise;
      
      // Check if we can load existing embeddings with valid data
      const hasExistingIndex = await this.vectorStore.hasValidIndex();
      
      if (hasExistingIndex && request.mode === 'incremental') {
        // Analyze index health for informational purposes only
        const recommendation = await this.reindexAdvisor.getReindexRecommendation(
          this.vectorStore, 
          this.repositoryPath
        );
        
        // Only override user's mode in extreme corruption cases that prevent incremental processing
        if (recommendation.forcedRebuildRequired) {
          log('🚨 CRITICAL CORRUPTION DETECTED: ' + recommendation.primaryReason);
          log('🔄 Cannot proceed with incremental mode due to severe index corruption');
          log('   Switching to full rebuild to recover from corruption...');
          
          // Create backup before clearing corrupted data (may still have valuable chunks)
          await this.createPreRebuildBackup('corruption-recovery');
          
          await this.vectorStore.clearIndex();
          return await this.performFullIndex({ ...request, mode: 'reindex' }, startTime);
        }
        
        // Provide informational feedback but respect user's incremental request
        if (recommendation.allRecommendations.length > 0) {
          log('💡 Index health analysis: ' + recommendation.primaryReason);
          log('   (proceeding with incremental mode as requested)');
        }
        
        log('📂 Loading existing embeddings and performing incremental update...');
        return await this.performIncrementalIndex(request, startTime);
      } else if (hasExistingIndex && (request.mode === 'full' || request.mode === 'reindex')) {
        const reason = request.mode === 'reindex' ? 'forced rebuild requested' : 'full mode specified';
        log(`🔄 Existing index found, but performing full reindex (${reason})...`);
        
        // Create backup before intentional full rebuild
        const backupReason = request.mode === 'reindex' ? 'manual-reindex' : 'manual-full-rebuild';
        await this.createPreRebuildBackup(backupReason);
        
        await this.vectorStore.clearIndex();
      } else {
        log('🆕 No existing index found, performing full index...');
      }
      
      return await this.performFullIndex(request, startTime);
    } catch (err) {
      error(`Indexing failed: ${err instanceof Error ? err.message : 'Unknown error'}`);
      return {
        status: 'error',
        chunks_processed: 0,
        time_taken_ms: Date.now() - startTime,
        error_message: err instanceof Error ? err.message : 'Unknown error'
      };
    }
  }

  private async performIncrementalIndex(request: IndexRequest, startTime: number): Promise<IndexResponse> {
    log(`Starting incremental indexing of ${request.repository_path}`);
    
    log(`📊 Loaded ${this.vectorStore.getChunkCount()} existing chunks for incremental processing`);
    
    // Scan repository for all files
    const scanResult = await this.gitScanner.scanRepository('full'); // Get all files to compare
    log(`Found ${scanResult.totalFiles} total files`);
    
    // Calculate what has changed using chunk-based hashing for accurate delta detection
    const chunkHashCalculator = async (filePath: string): Promise<string> => {
      // Simple approach: just hash the file content directly
      // This avoids all chunking inconsistencies by comparing at the file level
      const content = await this.gitScanner.readFile(filePath);
      return crypto.createHash('sha256').update(content).digest('hex');
    };
    
    const delta = await this.vectorStore.calculateFileDelta(scanResult.files, chunkHashCalculator);
    const changedFiles = [...delta.fileChanges.added, ...delta.fileChanges.modified];
    
    log(`📊 Delta analysis: +${delta.fileChanges.added.length} ~${delta.fileChanges.modified.length} -${delta.fileChanges.deleted.length} files`);
    
    // Handle deleted files first (clean up their chunks from the index)
    let deletedFileChunks = 0;
    if (delta.fileChanges.deleted.length > 0) {
      log(`🗑️  Processing ${delta.fileChanges.deleted.length} deleted files...`);
      for (const deletedFile of delta.fileChanges.deleted) {
        const deletedChunks = this.vectorStore.getChunksByFile(deletedFile);
        delta.removed.push(...deletedChunks.map(c => c.chunk_id));
        deletedFileChunks += deletedChunks.length;
        log(`   Removed ${deletedChunks.length} chunks from deleted file: ${deletedFile}`);
      }
    }
    
    if (changedFiles.length === 0 && delta.fileChanges.deleted.length === 0) {
      log('✅ No changes detected, index is up to date');
      
      // Provide additional context for users who expect changes
      const indexStats = await this.vectorStore.getStats();
      log(`🔍 Index contains ${this.vectorStore.getChunkCount()} chunks from ${indexStats.totalFiles} files`);
      log(`🕒 Index last updated: ${indexStats.lastUpdated.toISOString()}`);
      log('💡 If you expect changes, try: npm run cache:clear-all && npm run startup');
      
      const timeTaken = Date.now() - startTime;
      return {
        status: 'success',
        chunks_processed: 0,
        time_taken_ms: timeTaken
      };
    }
    
    // Process added and modified files (deleted files already handled above)
    const chunksToEmbed: CodeChunk[] = [];
    const chunksToKeep: CodeChunk[] = [];
    const fileChanges = await this.gitScanner.getFileChanges(changedFiles);

    // Process each file change type independently
    log(`🔄 Processing file changes:`);
    if (delta.fileChanges.added.length > 0) {
      log(`   📁 Adding ${delta.fileChanges.added.length} new files`);
    }
    if (delta.fileChanges.modified.length > 0) {
      log(`   📝 Updating ${delta.fileChanges.modified.length} modified files`);
    }

    for (const filePath of changedFiles) {
      try {
        const content = await this.gitScanner.readFile(filePath);
        const fileChange = fileChanges.find(fc => fc.filePath === filePath);
        const coChangeFiles = await this.gitScanner.getCoChangeFiles(filePath);
        
        const newChunks = await this.chunker.chunkFile(filePath, content, fileChange, coChangeFiles);
        const oldChunks = this.vectorStore.getChunksByFile(filePath);

        // Store file content hash for delta detection
        const contentHash = crypto.createHash('sha256').update(content).digest('hex');
        if (this.vectorStore instanceof require('./persistent-vector-store').PersistentVectorStore) {
          (this.vectorStore as any).setFileHash(filePath, contentHash);
        }

        const { toAdd, toKeep, toRemove } = this.vectorStore.compareChunks(oldChunks, newChunks);
        
        chunksToEmbed.push(...toAdd);
        chunksToKeep.push(...toKeep);
        // Add the IDs of removed chunks to the main delta's removed list
        delta.removed.push(...toRemove.map(c => c.chunk_id));

      } catch (error) {
        if (error instanceof Error && error.message.includes('File not found')) {
          warn(`Skipping deleted file: ${filePath}`);
        } else {
          warn(`Failed to process file ${filePath}: ${error}`);
        }
      }
    }
    
    // CRITICAL FIX: Preserve chunks from unchanged files
    // The issue was that only chunks from changed files were being processed,
    // but chunks from unchanged files need to be explicitly preserved
    log('🔄 Preserving chunks from unchanged files...');
    const unchangedFiles = scanResult.files.filter(filePath => 
      !delta.fileChanges.added.includes(filePath) && 
      !delta.fileChanges.modified.includes(filePath) && 
      !delta.fileChanges.deleted.includes(filePath)
    );
    
    // Count chunks before adding unchanged files to get accurate breakdown
    const chunksFromModifiedFiles = chunksToKeep.length;
    
    let unchangedChunksCount = 0;
    for (const unchangedFile of unchangedFiles) {
      const existingChunks = this.vectorStore.getChunksByFile(unchangedFile);
      chunksToKeep.push(...existingChunks);
      unchangedChunksCount += existingChunks.length;
    }
    
    log(`💡 Processing summary by file change type:`);
    log(`  - NEW FILES: ${delta.fileChanges.added.length} files`);
    log(`  - MODIFIED FILES: ${delta.fileChanges.modified.length} files`);
    log(`  - DELETED FILES: ${delta.fileChanges.deleted.length} files (${deletedFileChunks} chunks removed)`);
    log(`  - UNCHANGED FILES: ${unchangedFiles.length} files (${unchangedChunksCount} chunks preserved)`);
    log(`  - CHUNKS TO EMBED: ${chunksToEmbed.length} (new or modified)`);
    log(`  - CHUNKS TO KEEP: ${chunksToKeep.length} total (${unchangedChunksCount} from unchanged files + ${chunksFromModifiedFiles} unchanged in modified files)`);
    log(`  - TOTAL CHUNKS REMOVED: ${delta.removed.length}`);
    
    // Chunk accounting validation
    const startingChunks = this.vectorStore.getAllChunks().length;
    const expectedFinalChunks = startingChunks - delta.removed.length + chunksToEmbed.length;
    log(`🔢 Chunk accounting: ${startingChunks} initial → ${expectedFinalChunks} expected final (${startingChunks} - ${delta.removed.length} removed + ${chunksToEmbed.length} new)`);

    // Start relationship building in parallel with embedding generation
    const relationshipPromise = this.buildRelationshipsForChangedFiles(scanResult.files, [...delta.fileChanges.added, ...delta.fileChanges.modified]);
    
    // Generate embeddings only for new/changed chunks
    let embeddedChunks: CodeChunk[] = [];
    if (chunksToEmbed.length > 0) {
      log('🚀 Generating embeddings for new/modified content...');
      embeddedChunks = await this.generateEmbeddings(chunksToEmbed);
      // These are the brand new or updated chunks
      delta.added = embeddedChunks;
    } else {
      delta.added = [];
    }
    
    // These are the chunks that were unchanged and whose embeddings we are preserving
    delta.updated = chunksToKeep;

    // Apply the fine-grained delta to the vector store
    log('💾 Applying changes to vector database...');
    await this.vectorStore.applyDelta(delta);
    
    // Save updated index and wait for relationship building to complete in parallel
    const [, relationshipCount] = await Promise.all([
      this.vectorStore.savePersistedIndex(),
      relationshipPromise
    ]);
    
    if (relationshipCount > 0) {
      log(`✅ Updated ${relationshipCount} dependency relationships`);
    }
    
    // Relationship engine initialization handled in stage 2.3
    
    const timeTaken = Date.now() - startTime;
    log(`✅ Incremental indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: delta.added.length,
      time_taken_ms: timeTaken
    };
  }

  private async performFullIndex(request: IndexRequest, startTime: number): Promise<IndexResponse> {
    log(`Starting full indexing of ${request.repository_path}`);
    
    // Scan repository for files
    // Map reindex mode to full for git scanner
    const scanMode = request.mode === 'reindex' ? 'full' : request.mode;
    const scanResult = await this.gitScanner.scanRepository(scanMode, request.since_commit);
    log(`Found ${scanResult.totalFiles} files to process`);
    
    // Get file changes metadata
    const fileChanges = await this.gitScanner.getFileChanges(scanResult.files);
    
    // Process files in parallel for better performance
    log(`🚀 Processing ${scanResult.files.length} files in parallel...`);
    
    const chunkingPromises = scanResult.files.map(async (filePath, index) => {
      try {
        const content = await this.gitScanner.readFile(filePath);
        const fileChange = fileChanges.find(fc => fc.filePath === filePath);
        const coChangeFiles = await this.gitScanner.getCoChangeFiles(filePath);
        
        const chunks = await this.chunker.chunkFile(filePath, content, fileChange, coChangeFiles);
        
        // Store file content hash for delta detection
        const contentHash = crypto.createHash('sha256').update(content).digest('hex');
        if (this.vectorStore instanceof require('./persistent-vector-store').PersistentVectorStore) {
          (this.vectorStore as any).setFileHash(filePath, contentHash);
        }
        
        // Progress reporting for parallel processing
        if ((index + 1) % 10 === 0) {
          log(`📊 Processed ${index + 1}/${scanResult.files.length} files`);
        }
        
        return chunks;
      } catch (error) {
        if (error instanceof Error && error.message.includes('File not found')) {
          warn(`Skipping deleted file: ${filePath}`);
        } else {
          warn(`Failed to process file ${filePath}: ${error}`);
        }
        return [];
      }
    });
    
    const chunkArrays = await Promise.all(chunkingPromises);
    const allChunks: CodeChunk[] = chunkArrays.flat();
    
    log(`Generated ${allChunks.length} code chunks`);
    
    // Generate embeddings in batches
    log('Generating embeddings...');
    const embeddedChunks = await this.generateEmbeddings(allChunks);
    
    // Store in vector database
    log('Storing chunks in vector database...');
    await this.vectorStore.upsertChunks(embeddedChunks);
    
    // Save persistent index with model information
    const modelInfo = await this.embedder.getModelInfo();
    await this.vectorStore.savePersistedIndex(modelInfo);
    
    // Relationship engine initialization handled in stage 2.3
    
    const timeTaken = Date.now() - startTime;
    log(`Indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: embeddedChunks.length,
      time_taken_ms: timeTaken
    };
  }

  private async generateEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Use embedding strategy manager for intelligent strategy selection
    const embedderType = process.env.EMBEDDER_TYPE || 'local';
    
    if (embedderType === 'cloudflare') {
      // Fallback to CloudflareAI when explicitly requested
      if (!this.unifiedEmbedder) {
        this.unifiedEmbedder = new CloudflareAIEmbedder();
        log(`🤖 Using unified embedder: ${this.unifiedEmbedder.providerId}`);
      }
      
      try {
        const texts = chunks.map(chunk => this.createEmbeddingText(chunk));
        const result = await this.unifiedEmbedder.embedBatch(texts, { 
          requestId: `index_${Date.now()}`,
          priority: 'normal'
        });
        return chunks.map((chunk, i) => ({ ...chunk, embedding: result.embeddings[i] }));
      } finally {
        // CloudflareAI doesn't need shutdown
      }
    } else {
      // Use intelligent embedding strategy manager with streaming storage
      const config = EmbeddingStrategyManager.getConfigFromEnv();
      
      // For large chunk sets, implement streaming storage
      if (chunks.length > 100) {
        return await this.generateEmbeddingsWithStreaming(chunks, config);
      } else {
        const result = await this.strategyManager.generateEmbeddings(chunks, config);
        
        // Log strategy performance
        const perf = result.performance;
        log(`📊 Embedding Performance (${result.strategy}):`);
        log(`   Total time: ${perf.totalTime}ms`);
        log(`   Throughput: ${perf.chunksPerSecond.toFixed(2)} chunks/second`);
        log(`   Peak memory: ${perf.peakMemoryMB.toFixed(1)}MB`);
        
        if (perf.cacheStats) {
          log(`   Cache: ${perf.cacheStats.hits} hits, ${perf.cacheStats.misses} misses (${(perf.cacheStats.hitRate * 100).toFixed(1)}% hit rate)`);
        }
        
        return result.chunks;
      }
    }
  }

  /**
   * Generate embeddings with streaming storage for large datasets
   */
  private async generateEmbeddingsWithStreaming(chunks: CodeChunk[], config: any): Promise<CodeChunk[]> {
    const batchSize = 50; // Process in smaller batches for streaming
    const allEmbeddedChunks: CodeChunk[] = [];
    
    log(`🔄 Using streaming embedding generation for ${chunks.length} chunks (batch size: ${batchSize})`);
    
    for (let i = 0; i < chunks.length; i += batchSize) {
      const batch = chunks.slice(i, i + batchSize);
      const batchNum = Math.floor(i / batchSize) + 1;
      const totalBatches = Math.ceil(chunks.length / batchSize);
      
      log(`📦 Processing batch ${batchNum}/${totalBatches} (${batch.length} chunks)`);
      
      const result = await this.strategyManager.generateEmbeddings(batch, config);
      allEmbeddedChunks.push(...result.chunks);
      
      // Stream to storage immediately for this batch
      if (result.chunks.length > 0) {
        await this.vectorStore.upsertChunks(result.chunks);
        log(`💾 Streamed batch ${batchNum} to storage (${result.chunks.length} chunks)`);
      }
    }
    
    log(`✅ Streaming embedding generation completed: ${allEmbeddedChunks.length} total chunks`);
    return allEmbeddedChunks;
  }


  /**
   * Build relationships for changed files in parallel with embedding generation
   */
  private async buildRelationshipsForChangedFiles(allFiles: string[], modifiedFiles: string[]): Promise<number> {
    if (modifiedFiles.length === 0) {
      return 0;
    }
    
    log(`🔗 Updating relationships for ${modifiedFiles.length} changed files...`);
    
    // Read all files in parallel for relationship analysis
    const files = new Map<string, string>();
    const fileReadPromises = allFiles.map(async (filePath) => {
      try {
        const content = await this.gitScanner.readFile(filePath);
        files.set(filePath, content);
      } catch (error) {
        warn(`Failed to read file for relationships ${filePath}: ${error}`);
      }
    });
    
    await Promise.all(fileReadPromises);
    
    // Build dependency map and generate relationships
    await this.dependencyMapper.buildDependencyMap(files);
    const relationships = this.dependencyMapper.generateDependencyRelationships();
    
    return relationships.length;
  }

  private createEmbeddingText(chunk: CodeChunk): string {
    // Optimized embedding text generation - consistent across all embedding generation
    // Reduces verbose preprocessing for better performance and token efficiency
    const parts = [];
    
    // Add symbol name if available (most important for semantic search)
    if (chunk.symbol_name) {
      parts.push(chunk.symbol_name);
    }
    
    // Add chunk type for context
    parts.push(chunk.chunk_type);
    
    // Add main content
    parts.push(chunk.content);
    
    // Add limited import context (top 3 most relevant)
    if (chunk.relationships.imports.length > 0) {
      parts.push(chunk.relationships.imports.slice(0, 3).join(' '));
    }
    
    return parts.join(' ');
  }

  async getIndexStats(): Promise<{ total_chunks: number; last_indexed?: string }> {
    const stats = await this.vectorStore.getStats();
    return {
      total_chunks: stats.total_chunks,
      last_indexed: new Date().toISOString() // TODO: Store actual last indexed time
    };
  }

  // Helper method to clear the index
  async clearIndex(): Promise<void> {
    await this.vectorStore.clear();
  }

  // Search functionality
  async search(query: QueryRequest): Promise<QueryResponse> {
    return await this.searcher.search(query);
  }

  // Real-time file watching methods
  async enableRealTimeUpdates(): Promise<void> {
    if (this.semanticWatcher) return;
    
    log('[CodebaseIndexer] Enabling real-time updates...');
    
    this.contextInvalidator = new ContextInvalidator(this.vectorStore);
    this.semanticWatcher = new SemanticWatcher(this.repositoryPath, this);
    
    // Listen for incremental reindex triggers
    (process as any).on('cortex:triggerIncrementalReindex', () => {
      this.performIncrementalReindex();
    });
    
    await this.semanticWatcher.start();
    log('[CodebaseIndexer] Real-time updates enabled');
  }

  async disableRealTimeUpdates(): Promise<void> {
    if (this.semanticWatcher) {
      await this.semanticWatcher.stop();
      this.semanticWatcher = undefined;
    }
    this.contextInvalidator = undefined;
    log('[CodebaseIndexer] Real-time updates disabled');
  }

  async handleFileChange(filePath: string, changeType: string): Promise<void> {
    // Simple incremental update - reprocess just this file
    const relativePath = path.relative(this.repositoryPath, filePath);
    
    if (changeType === 'deleted') {
      await this.vectorStore.removeChunksForFile(relativePath);
      return;
    }
    
    try {
      // Read file content
      const fs = await import('fs/promises');
      const content = await fs.readFile(filePath, 'utf-8');
      
      // Reprocess the changed file - use relativePath for chunking
      const chunks = await this.chunker.chunkFile(relativePath, content);
      if (chunks.length > 0) {
        // Use strategy manager for real-time embedding generation with graceful degradation
        const config = EmbeddingStrategyManager.getConfigFromEnv();
        
        log(`[CodebaseIndexer] Generating embeddings for real-time update: ${chunks.length} chunks in ${relativePath}`);
        
        try {
          const result = await this.strategyManager.generateEmbeddings(chunks, config);
          
          // Store the updated chunks with embeddings
          await this.vectorStore.upsertChunks(result.chunks);
          
          // Also save to persistent storage for consistency
          await this.vectorStore.savePersistedIndex();
          
          log(`[CodebaseIndexer] Successfully updated ${result.chunks.length} chunks for ${relativePath}`);
          
        } catch (memoryError) {
          // Graceful degradation for real-time updates when memory is constrained
          if (memoryError instanceof Error && memoryError.message.includes('System memory too high')) {
            warn(`[CodebaseIndexer] Real-time embedding skipped due to memory pressure (${relativePath})`);
            warn(`[CodebaseIndexer] Chunks will be reprocessed when memory becomes available`);
            
            // Store chunks without embeddings for now (they'll be embedded during next full indexing)
            const chunksWithoutEmbeddings = chunks.map(chunk => ({
              ...chunk,
              embedding: [] // Empty embedding array indicates pending embedding
            }));
            
            await this.vectorStore.upsertChunks(chunksWithoutEmbeddings);
            await this.vectorStore.savePersistedIndex();
            
            log(`[CodebaseIndexer] Stored ${chunks.length} chunks without embeddings for ${relativePath} (memory-constrained mode)`);
          } else {
            // Re-throw non-memory related errors
            throw memoryError;
          }
        }
      }
    } catch (error) {
      warn(`[CodebaseIndexer] Failed to process file change for ${relativePath}:`, error instanceof Error ? error.message : error);
    }
  }

  private async performIncrementalReindex(): Promise<void> {
    if (!this.contextInvalidator) return;
    
    const invalidatedChunks = this.contextInvalidator.getInvalidatedChunks();
    if (invalidatedChunks.length === 0) return;
    
    log(`[CodebaseIndexer] Performing incremental reindex for ${invalidatedChunks.length} chunks`);
    
    // Simple approach: reindex files with invalidated chunks
    const filesToReindex = new Set<string>();
    for (const chunkId of invalidatedChunks) {
      const chunk = await this.vectorStore.getChunk(chunkId);
      if (chunk) filesToReindex.add(chunk.file_path);
    }
    
    for (const filePath of filesToReindex) {
      // filePath from chunks is already relative, so we need to join with repositoryPath
      const absolutePath = path.isAbsolute(filePath) ? filePath : path.join(this.repositoryPath, filePath);
      await this.handleFileChange(absolutePath, 'content');
    }
    
    this.contextInvalidator.clearInvalidatedChunks();
    log('[CodebaseIndexer] Incremental reindex completed');
  }

  getRealTimeStats(): { 
    isWatching: boolean; 
    invalidatedChunks: number; 
  } {
    return {
      isWatching: this.semanticWatcher?.isWatching() ?? false,
      invalidatedChunks: this.contextInvalidator?.getStats().invalidatedCount ?? 0
    };
  }
}