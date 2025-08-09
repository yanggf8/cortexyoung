import { CodeChunk, IndexRequest, IndexResponse, QueryRequest, QueryResponse, IEmbedder } from './types';
import { GitScanner } from './git-scanner';
import { SmartChunker } from './chunker';
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
import * as os from 'os';

export class CodebaseIndexer {
  private gitScanner: GitScanner;
  private chunker: SmartChunker;
  private embedder: EmbeddingGenerator;
  private unifiedEmbedder?: IEmbedder; // Unified interface for all embedding providers
  private vectorStore: PersistentVectorStore;
  private searcher: SemanticSearcher;
  private dependencyMapper: DependencyMapper;
  private repositoryPath: string;
  private stageTracker?: StartupStageTracker;
  private reindexAdvisor: ReindexAdvisor;
  private storageCoordinator: UnifiedStorageCoordinator;

  constructor(repositoryPath: string, stageTracker?: StartupStageTracker) {
    this.repositoryPath = repositoryPath;
    this.gitScanner = new GitScanner(repositoryPath);
    this.chunker = new SmartChunker();
    this.embedder = new EmbeddingGenerator();
    this.storageCoordinator = new UnifiedStorageCoordinator(repositoryPath);
    this.vectorStore = this.storageCoordinator.getVectorStore();
    this.searcher = new SemanticSearcher(this.vectorStore, this.embedder, repositoryPath);
    this.dependencyMapper = new DependencyMapper(repositoryPath);
    this.stageTracker = stageTracker;
    this.reindexAdvisor = new ReindexAdvisor(this.embedder);
  }

  /**
   * Clean up resources for unified embedder
   */
  async cleanup(reason: string = 'cleanup'): Promise<void> {
    if (this.unifiedEmbedder) {
      console.log(`🧹 Cleaning up unified embedder (${this.unifiedEmbedder.providerId}, reason: ${reason})...`);
      
      // Only ProcessPoolEmbedder requires shutdown
      if (this.unifiedEmbedder instanceof ProcessPoolEmbedder) {
        await this.unifiedEmbedder.shutdown(reason);
      }
      
      this.unifiedEmbedder = undefined;
    }
  }

  async indexRepository(request: IndexRequest): Promise<IndexResponse> {
    const startTime = Date.now();
    
    try {
      // Start model loading early to overlap with storage initialization
      const modelLoadPromise = this.embedder.getModelInfo();
      
      // Initialize unified storage coordinator
      this.stageTracker?.startStage('cache_check', 'Checking for existing embeddings and relationships');
      await this.storageCoordinator.initialize();
      this.stageTracker?.completeStage('cache_check');
      
      // Ensure model is loaded before we need it
      await modelLoadPromise;
      
      // Check if we can load existing embeddings
      const hasExistingIndex = await this.vectorStore.indexExists();
      
      if (hasExistingIndex && request.mode === 'incremental') {
        // Analyze index health for informational purposes only
        const recommendation = await this.reindexAdvisor.getReindexRecommendation(
          this.vectorStore, 
          this.repositoryPath
        );
        
        // Only override user's mode in extreme corruption cases that prevent incremental processing
        if (recommendation.forcedRebuildRequired) {
          console.log('🚨 CRITICAL CORRUPTION DETECTED:', recommendation.primaryReason);
          console.log('🔄 Cannot proceed with incremental mode due to severe index corruption');
          console.log('   Switching to full rebuild to recover from corruption...');
          await this.vectorStore.clearIndex();
          return await this.performFullIndex({ ...request, mode: 'reindex' }, startTime);
        }
        
        // Provide informational feedback but respect user's incremental request
        if (recommendation.allRecommendations.length > 0) {
          console.log('💡 Index health analysis:', recommendation.primaryReason);
          console.log('   (proceeding with incremental mode as requested)');
        }
        
        console.log('📂 Loading existing embeddings and performing incremental update...');
        return await this.performIncrementalIndex(request, startTime);
      } else if (hasExistingIndex && (request.mode === 'full' || request.mode === 'reindex')) {
        const reason = request.mode === 'reindex' ? 'forced rebuild requested' : 'full mode specified';
        console.log(`🔄 Existing index found, but performing full reindex (${reason})...`);
        await this.vectorStore.clearIndex();
      } else {
        console.log('🆕 No existing index found, performing full index...');
      }
      
      return await this.performFullIndex(request, startTime);
    } catch (error) {
      console.error('Indexing failed:', error);
      return {
        status: 'error',
        chunks_processed: 0,
        time_taken_ms: Date.now() - startTime,
        error_message: error instanceof Error ? error.message : 'Unknown error'
      };
    }
  }

  private async performIncrementalIndex(request: IndexRequest, startTime: number): Promise<IndexResponse> {
    console.log(`Starting incremental indexing of ${request.repository_path}`);
    
    // Validate vector store state and attempt recovery for incremental indexing
    const chunkCount = this.vectorStore.getChunkCount();
    const fileHashCount = this.vectorStore.getFileHashCount();
    
    console.log(`📊 Vector store state validation:`);
    console.log(`   - Chunks in memory: ${chunkCount}`);
    console.log(`   - File hashes loaded: ${fileHashCount}`);
    
    if (chunkCount > 0 && fileHashCount === 0) {
      console.warn('⚠️ WARNING: Chunks exist but no file hashes loaded');
      console.log('   - Attempting to rebuild file hash cache from existing chunks...');
      
      try {
        // Try to recover file hashes from existing chunk data
        const chunks = await this.vectorStore.getAllChunks();
        const fileHashes = new Map<string, string>();
        
        // Reconstruct file hashes by reading current files and matching with chunks
        const uniqueFiles = [...new Set(chunks.map(c => c.file_path))];
        for (const filePath of uniqueFiles) {
          try {
            const content = await this.gitScanner.readFile(filePath);
            // Simple hash calculation using crypto
            const crypto = require('crypto');
            const hash = crypto.createHash('sha256').update(content).digest('hex');
            fileHashes.set(filePath, hash);
          } catch (error) {
            // File might be deleted, will be handled in delta calculation
            console.log(`     File ${filePath} no longer exists (will be cleaned up)`);
          }
        }
        
        // Update vector store with recovered hashes by accessing the private property directly
        // This is a recovery operation for corrupted state
        (this.vectorStore as any).fileHashes = fileHashes;
        console.log(`   ✅ Recovered ${fileHashes.size} file hashes, proceeding with incremental mode`);
        
      } catch (error) {
        console.error('   ❌ Failed to recover file hash cache:', error);
        console.log('   🔄 Falling back to full rebuild due to unrecoverable state');
        await this.vectorStore.clearIndex();
        return await this.performFullIndex({ ...request, mode: 'reindex' }, startTime);
      }
    }
    
    // Scan repository for all files
    const scanResult = await this.gitScanner.scanRepository('full'); // Get all files to compare
    console.log(`Found ${scanResult.totalFiles} total files`);
    
    // Calculate what has changed
    const delta = await this.vectorStore.calculateFileDelta(scanResult.files);
    const changedFiles = [...delta.fileChanges.added, ...delta.fileChanges.modified];
    
    console.log(`📊 Delta analysis: +${delta.fileChanges.added.length} ~${delta.fileChanges.modified.length} -${delta.fileChanges.deleted.length} files`);
    
    // Handle deleted files first (clean up their chunks from the index)
    if (delta.fileChanges.deleted.length > 0) {
      console.log(`🗑️  Processing ${delta.fileChanges.deleted.length} deleted files...`);
      for (const deletedFile of delta.fileChanges.deleted) {
        const deletedChunks = this.vectorStore.getChunksByFile(deletedFile);
        delta.removed.push(...deletedChunks.map(c => c.chunk_id));
        console.log(`   Removed ${deletedChunks.length} chunks from deleted file: ${deletedFile}`);
      }
    }
    
    if (changedFiles.length === 0 && delta.fileChanges.deleted.length === 0) {
      console.log('✅ No changes detected, index is up to date');
      
      // Still need to initialize relationship engine (will use cache if available)
      this.stageTracker?.startStage('relationship_analysis', 'Loading relationship graph from cache');
      const files = new Map<string, string>();
      for (const filePath of scanResult.files) {
        try {
          const content = await this.gitScanner.readFile(filePath);
          files.set(filePath, content);
        } catch (error) {
          console.warn(`Failed to read file for relationships ${filePath}:`, error);
        }
      }
      
      await this.searcher.initializeRelationshipEngine(files);
      this.stageTracker?.completeStage('relationship_analysis', `Relationship engine loaded from cache (${files.size} files)`);
      
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
    console.log(`🔄 Processing file changes:`);
    if (delta.fileChanges.added.length > 0) {
      console.log(`   📁 Adding ${delta.fileChanges.added.length} new files`);
    }
    if (delta.fileChanges.modified.length > 0) {
      console.log(`   📝 Updating ${delta.fileChanges.modified.length} modified files`);
    }

    for (const filePath of changedFiles) {
      try {
        const content = await this.gitScanner.readFile(filePath);
        const fileChange = fileChanges.find(fc => fc.filePath === filePath);
        const coChangeFiles = await this.gitScanner.getCoChangeFiles(filePath);
        
        const newChunks = await this.chunker.chunkFile(filePath, content, fileChange, coChangeFiles);
        const oldChunks = this.vectorStore.getChunksByFile(filePath);

        const { toAdd, toKeep, toRemove } = this.vectorStore.compareChunks(oldChunks, newChunks);
        
        chunksToEmbed.push(...toAdd);
        chunksToKeep.push(...toKeep);
        // Add the IDs of removed chunks to the main delta's removed list
        delta.removed.push(...toRemove.map(c => c.chunk_id));

      } catch (error) {
        if (error instanceof Error && error.message.includes('File not found')) {
          console.warn(`Skipping deleted file: ${filePath}`);
        } else {
          console.warn(`Failed to process file ${filePath}:`, error);
        }
      }
    }
    
    console.log(`💡 Processing summary by file change type:`);
    console.log(`  - NEW FILES: ${delta.fileChanges.added.length} files`);
    console.log(`  - MODIFIED FILES: ${delta.fileChanges.modified.length} files`);
    console.log(`  - DELETED FILES: ${delta.fileChanges.deleted.length} files (${delta.removed.length - (chunksToEmbed.length > 0 ? delta.removed.filter(id => !chunksToEmbed.some(c => c.chunk_id === id)).length : delta.removed.length)} chunks removed)`);
    console.log(`  - CHUNKS TO EMBED: ${chunksToEmbed.length} (new or modified)`);
    console.log(`  - CHUNKS TO KEEP: ${chunksToKeep.length} (unchanged, cache hit)`);
    console.log(`  - TOTAL CHUNKS REMOVED: ${delta.removed.length}`);

    // Generate embeddings only for new/changed chunks
    if (chunksToEmbed.length > 0) {
      console.log('🚀 Generating embeddings for new/modified content...');
      const embeddedChunks = await this.generateEmbeddings(chunksToEmbed);
      // These are the brand new or updated chunks
      delta.added = embeddedChunks;
    } else {
      delta.added = [];
    }
    
    // These are the chunks that were unchanged and whose embeddings we are preserving
    delta.updated = chunksToKeep;

    // Apply the fine-grained delta to the vector store
    console.log('💾 Applying changes to vector database...');
    await this.vectorStore.applyDelta(delta);
    
    // Save updated index
    await this.vectorStore.savePersistedIndex();
    
    // Update relationship engine with incremental changes
    this.stageTracker?.startStage('relationship_analysis', 'Updating relationship graph with changes');
    const files = new Map<string, string>();
    for (const filePath of scanResult.files) {
      try {
        const content = await this.gitScanner.readFile(filePath);
        files.set(filePath, content);
      } catch (error) {
        console.warn(`Failed to read file for relationships ${filePath}:`, error);
      }
    }
    
    // Update dependency relationships for changed files  
    const modifiedFiles = [...delta.fileChanges.added, ...delta.fileChanges.modified];
    if (modifiedFiles.length > 0) {
      console.log(`🔗 Updating relationships for ${modifiedFiles.length} changed files...`);
      await this.dependencyMapper.buildDependencyMap(files);
      const relationships = this.dependencyMapper.generateDependencyRelationships();
      
      console.log(`✅ Updated ${relationships.length} dependency relationships`);
    }
    
    // Initialize searcher's relationship engine with updated relationships
    await this.searcher.initializeRelationshipEngine(files);
    this.stageTracker?.completeStage('relationship_analysis', `Updated relationships (${files.size} files processed)`);
    
    const timeTaken = Date.now() - startTime;
    console.log(`✅ Incremental indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: delta.added.length,
      time_taken_ms: timeTaken
    };
  }

  private async performFullIndex(request: IndexRequest, startTime: number): Promise<IndexResponse> {
    console.log(`Starting full indexing of ${request.repository_path}`);
    
    // Scan repository for files
    // Map reindex mode to full for git scanner
    const scanMode = request.mode === 'reindex' ? 'full' : request.mode;
    const scanResult = await this.gitScanner.scanRepository(scanMode, request.since_commit);
    console.log(`Found ${scanResult.totalFiles} files to process`);
    
    // Get file changes metadata
    const fileChanges = await this.gitScanner.getFileChanges(scanResult.files);
    
    // Process files in parallel for better performance
    console.log(`🚀 Processing ${scanResult.files.length} files in parallel...`);
    
    const chunkingPromises = scanResult.files.map(async (filePath, index) => {
      try {
        const content = await this.gitScanner.readFile(filePath);
        const fileChange = fileChanges.find(fc => fc.filePath === filePath);
        const coChangeFiles = await this.gitScanner.getCoChangeFiles(filePath);
        
        const chunks = await this.chunker.chunkFile(filePath, content, fileChange, coChangeFiles);
        
        // Progress reporting for parallel processing
        if ((index + 1) % 10 === 0) {
          console.log(`📊 Processed ${index + 1}/${scanResult.files.length} files`);
        }
        
        return chunks;
      } catch (error) {
        if (error instanceof Error && error.message.includes('File not found')) {
          console.warn(`Skipping deleted file: ${filePath}`);
        } else {
          console.warn(`Failed to process file ${filePath}:`, error);
        }
        return [];
      }
    });
    
    const chunkArrays = await Promise.all(chunkingPromises);
    const allChunks: CodeChunk[] = chunkArrays.flat();
    
    console.log(`Generated ${allChunks.length} code chunks`);
    
    // Generate embeddings in batches
    console.log('Generating embeddings...');
    const embeddedChunks = await this.generateEmbeddings(allChunks);
    
    // Store in vector database
    console.log('Storing chunks in vector database...');
    await this.vectorStore.upsertChunks(embeddedChunks);
    
    // Save persistent index with model information
    const modelInfo = await this.embedder.getModelInfo();
    await this.vectorStore.savePersistedIndex(modelInfo);
    
    // Initialize relationship engine with all files
    this.stageTracker?.startStage('relationship_analysis', 'Building comprehensive relationship graph');
    const files = new Map<string, string>();
    for (const filePath of scanResult.files) {
      try {
        const content = await this.gitScanner.readFile(filePath);
        files.set(filePath, content);
      } catch (error) {
        console.warn(`Failed to read file for relationships ${filePath}:`, error);
      }
    }
    
    // Build comprehensive dependency relationships
    console.log(`🔗 Analyzing dependencies for ${files.size} files...`);
    await this.dependencyMapper.buildDependencyMap(files);
    const relationships = this.dependencyMapper.generateDependencyRelationships();
    
    console.log(`✅ Built ${relationships.length} dependency relationships`);
    
    // Initialize searcher's relationship engine with the built relationships
    await this.searcher.initializeRelationshipEngine(files);
    
    this.stageTracker?.completeStage('relationship_analysis', `Built ${relationships.length} dependency relationships`);
    
    const timeTaken = Date.now() - startTime;
    console.log(`Indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: embeddedChunks.length,
      time_taken_ms: timeTaken
    };
  }

  private async generateEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    const embedderType = process.env.EMBEDDER_TYPE || 'local';
    this.stageTracker?.startStage('embedding_generation', `Processing ${chunks.length} chunks with ${embedderType} embedder`);

    let embeddedChunks: CodeChunk[];

    // Initialize unified embedder based on type
    if (!this.unifiedEmbedder) {
      this.unifiedEmbedder = embedderType === 'cloudflare' 
        ? new CloudflareAIEmbedder()
        : new ProcessPoolEmbedder();
      
      console.log(`🤖 Using unified embedder: ${this.unifiedEmbedder.providerId}`);
    }
    
    try {
      if (embedderType === 'cloudflare') {
        // Use IEmbedder interface for Cloudflare
        const texts = chunks.map(chunk => this.createEmbeddingText(chunk));
        const result = await this.unifiedEmbedder.embedBatch(texts, { 
          requestId: `index_${Date.now()}`,
          priority: 'normal'
        });
        embeddedChunks = chunks.map((chunk, i) => ({ ...chunk, embedding: result.embeddings[i] }));
        this.stageTracker?.completeStage('embedding_generation', `Generated embeddings for ${chunks.length} chunks using ${this.unifiedEmbedder.providerId}`);
      } else {
        // Use ProcessPoolEmbedder's existing method for backward compatibility during transition
        const processPoolEmbedder = this.unifiedEmbedder as ProcessPoolEmbedder;
        embeddedChunks = await processPoolEmbedder.processAllEmbeddings(chunks);
        this.stageTracker?.completeStage('embedding_generation', `Generated embeddings for ${chunks.length} chunks using ${this.unifiedEmbedder.providerId}`);
      }
    } finally {
      // Only shutdown ProcessPoolEmbedder after use
      if (this.unifiedEmbedder instanceof ProcessPoolEmbedder) {
        await this.unifiedEmbedder.shutdown();
        this.unifiedEmbedder = undefined;
      }
    }

    return embeddedChunks;
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
}