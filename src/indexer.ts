import { CodeChunk, IndexRequest, IndexResponse, QueryRequest, QueryResponse } from './types';
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
import * as os from 'os';

export class CodebaseIndexer {
  private gitScanner: GitScanner;
  private chunker: SmartChunker;
  private embedder: EmbeddingGenerator;
  private vectorStore: PersistentVectorStore;
  private searcher: SemanticSearcher;
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
    this.stageTracker = stageTracker;
    this.reindexAdvisor = new ReindexAdvisor(this.embedder);
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
        // Analyze if reindex is recommended
        const recommendation = await this.reindexAdvisor.getReindexRecommendation(
          this.vectorStore, 
          this.repositoryPath
        );
        
        if (recommendation.shouldReindex && recommendation.mode === 'reindex') {
          console.log('🤖 Automatic reindex recommended:', recommendation.primaryReason);
          console.log('🔄 Switching from incremental to full reindex...');
          await this.vectorStore.clearIndex();
          return await this.performFullIndex({ ...request, mode: 'reindex' }, startTime);
        } else if (recommendation.allRecommendations.length > 0) {
          console.log('💡 Index analysis:', recommendation.primaryReason);
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
    
    // Scan repository for all files
    const scanResult = await this.gitScanner.scanRepository('full'); // Get all files to compare
    console.log(`Found ${scanResult.totalFiles} total files`);
    
    // Calculate what has changed
    const delta = await this.vectorStore.calculateFileDelta(scanResult.files);
    const changedFiles = [...delta.fileChanges.added, ...delta.fileChanges.modified];
    
    console.log(`📊 Delta analysis: +${delta.fileChanges.added.length} ~${delta.fileChanges.modified.length} -${delta.fileChanges.deleted.length} files`);
    
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
    
    // Process only changed files
    const newChunks: CodeChunk[] = [];
    const fileChanges = await this.gitScanner.getFileChanges(changedFiles);
    
    for (const filePath of changedFiles) {
      try {
        const content = await this.gitScanner.readFile(filePath);
        const fileChange = fileChanges.find(fc => fc.filePath === filePath);
        const coChangeFiles = await this.gitScanner.getCoChangeFiles(filePath);
        
        const chunks = await this.chunker.chunkFile(filePath, content, fileChange, coChangeFiles);
        newChunks.push(...chunks);
      } catch (error) {
        console.warn(`Failed to process file ${filePath}:`, error);
      }
    }
    
    console.log(`Generated ${newChunks.length} new code chunks`);
    
    // Generate embeddings only for new/changed chunks
    if (newChunks.length > 0) {
      console.log('Generating embeddings for changed content...');
      const embeddedChunks = await this.generateEmbeddings(newChunks);
      delta.added.push(...embeddedChunks);
    }
    
    // Apply delta to vector store
    console.log('Applying changes to vector database...');
    await this.vectorStore.applyDelta(delta);
    
    // Save updated index
    await this.vectorStore.savePersistedIndex();
    
    // Initialize relationship engine with all current files (incremental mode will use cache if available)
    this.stageTracker?.startStage('relationship_analysis', 'Initializing relationship engine');
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
    this.stageTracker?.completeStage('relationship_analysis', `Relationship engine ready (cached: ${files.size} files)`);
    
    const timeTaken = Date.now() - startTime;
    console.log(`✅ Incremental indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: newChunks.length,
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
        console.warn(`Failed to process file ${filePath}:`, error);
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
    this.stageTracker?.startStage('relationship_analysis', 'Building relationship graph from files');
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
    this.stageTracker?.completeStage('relationship_analysis', `Initialized relationship engine with ${files.size} files`);
    
    const timeTaken = Date.now() - startTime;
    console.log(`Indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: embeddedChunks.length,
      time_taken_ms: timeTaken
    };
  }

  private async generateEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    // Start embedding generation stage
    this.stageTracker?.startStage('embedding_generation', `Processing ${chunks.length} chunks with fastq`);
    
    // Use fastq - battle-tested, simple, and efficient
    const fastqEmbedder = new FastQEmbedder();
    
    try {
      const embeddedChunks = await fastqEmbedder.processAllEmbeddings(chunks);
      
      this.stageTracker?.completeStage('embedding_generation', 
        `Generated embeddings for ${embeddedChunks.length} chunks using fastq`);
      
      return embeddedChunks;
      
    } finally {
      // Clean shutdown
      await fastqEmbedder.shutdown();
    }
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