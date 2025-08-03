import { CodeChunk, IndexRequest, IndexResponse, QueryRequest, QueryResponse } from './types';
import { GitScanner } from './git-scanner';
import { SmartChunker } from './chunker';
import { EmbeddingGenerator } from './embedder';
import { VectorStore } from './vector-store';
import { PersistentVectorStore } from './persistent-vector-store';
import { SemanticSearcher } from './searcher';

export class CodebaseIndexer {
  private gitScanner: GitScanner;
  private chunker: SmartChunker;
  private embedder: EmbeddingGenerator;
  private vectorStore: PersistentVectorStore;
  private searcher: SemanticSearcher;
  private repositoryPath: string;

  constructor(repositoryPath: string) {
    this.repositoryPath = repositoryPath;
    this.gitScanner = new GitScanner(repositoryPath);
    this.chunker = new SmartChunker();
    this.embedder = new EmbeddingGenerator();
    this.vectorStore = new PersistentVectorStore(repositoryPath);
    this.searcher = new SemanticSearcher(this.vectorStore, this.embedder);
  }

  async indexRepository(request: IndexRequest): Promise<IndexResponse> {
    const startTime = Date.now();
    
    try {
      // Initialize persistent vector store
      await this.vectorStore.initialize();
      
      // Check if we can load existing embeddings
      const hasExistingIndex = await this.vectorStore.indexExists();
      
      if (hasExistingIndex && request.mode === 'incremental') {
        console.log('📂 Loading existing embeddings and performing incremental update...');
        return await this.performIncrementalIndex(request, startTime);
      } else if (hasExistingIndex && request.mode === 'full') {
        console.log('🔄 Existing index found, but performing full reindex...');
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
    const scanResult = await this.gitScanner.scanRepository(request.mode, request.since_commit);
    console.log(`Found ${scanResult.totalFiles} files to process`);
    
    // Get file changes metadata
    const fileChanges = await this.gitScanner.getFileChanges(scanResult.files);
    
    // Process files in chunks
    const allChunks: CodeChunk[] = [];
    let processedFiles = 0;
    
    for (const filePath of scanResult.files) {
      try {
        const content = await this.gitScanner.readFile(filePath);
        const fileChange = fileChanges.find(fc => fc.filePath === filePath);
        const coChangeFiles = await this.gitScanner.getCoChangeFiles(filePath);
        
        const chunks = await this.chunker.chunkFile(filePath, content, fileChange, coChangeFiles);
        allChunks.push(...chunks);
        
        processedFiles++;
        if (processedFiles % 10 === 0) {
          console.log(`Processed ${processedFiles}/${scanResult.files.length} files`);
        }
      } catch (error) {
        console.warn(`Failed to process file ${filePath}:`, error);
      }
    }
    
    console.log(`Generated ${allChunks.length} code chunks`);
    
    // Generate embeddings in batches
    console.log('Generating embeddings...');
    const embeddedChunks = await this.generateEmbeddings(allChunks);
    
    // Store in vector database
    console.log('Storing chunks in vector database...');
    await this.vectorStore.upsertChunks(embeddedChunks);
    
    // Save persistent index
    await this.vectorStore.savePersistedIndex();
    
    const timeTaken = Date.now() - startTime;
    console.log(`Indexing completed in ${timeTaken}ms`);
    
    return {
      status: 'success',
      chunks_processed: embeddedChunks.length,
      time_taken_ms: timeTaken
    };
  }

  private async generateEmbeddings(chunks: CodeChunk[]): Promise<CodeChunk[]> {
    const batchSize = 50; // Process embeddings in smaller batches
    const embeddedChunks: CodeChunk[] = [];
    
    for (let i = 0; i < chunks.length; i += batchSize) {
      const batch = chunks.slice(i, i + batchSize);
      const texts = batch.map(chunk => this.createEmbeddingText(chunk));
      
      try {
        const embeddings = await this.embedder.embedBatch(texts);
        
        for (let j = 0; j < batch.length; j++) {
          embeddedChunks.push({
            ...batch[j],
            embedding: embeddings[j] || []
          });
        }
        
        console.log(`Generated embeddings for batch ${Math.floor(i / batchSize) + 1}/${Math.ceil(chunks.length / batchSize)}`);
      } catch (error) {
        console.warn(`Failed to generate embeddings for batch starting at ${i}:`, error);
        // Add chunks without embeddings
        embeddedChunks.push(...batch);
      }
    }
    
    return embeddedChunks;
  }

  private createEmbeddingText(chunk: CodeChunk): string {
    // Create rich text for embedding that includes context
    const parts = [];
    
    // Add file path context
    parts.push(`File: ${chunk.file_path}`);
    
    // Add symbol name if available
    if (chunk.symbol_name) {
      parts.push(`Symbol: ${chunk.symbol_name}`);
    }
    
    // Add chunk type
    parts.push(`Type: ${chunk.chunk_type}`);
    
    // Add language context
    parts.push(`Language: ${chunk.language_metadata.language}`);
    
    // Add the actual content
    parts.push(`Content: ${chunk.content}`);
    
    // Add import/export information
    if (chunk.relationships.imports.length > 0) {
      parts.push(`Imports: ${chunk.relationships.imports.join(', ')}`);
    }
    
    if (chunk.relationships.exports.length > 0) {
      parts.push(`Exports: ${chunk.relationships.exports.join(', ')}`);
    }
    
    return parts.join('\n');
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