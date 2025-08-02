import { CodeChunk, IndexRequest, IndexResponse, QueryRequest, QueryResponse } from './types';
import { GitScanner } from './git-scanner';
import { SmartChunker } from './chunker';
import { EmbeddingGenerator } from './embedder';
import { VectorStore } from './vector-store';
import { SemanticSearcher } from './searcher';

export class CodebaseIndexer {
  private gitScanner: GitScanner;
  private chunker: SmartChunker;
  private embedder: EmbeddingGenerator;
  private vectorStore: VectorStore;
  private searcher: SemanticSearcher;

  constructor(repositoryPath: string) {
    this.gitScanner = new GitScanner(repositoryPath);
    this.chunker = new SmartChunker();
    this.embedder = new EmbeddingGenerator();
    this.vectorStore = new VectorStore();
    this.searcher = new SemanticSearcher(this.vectorStore, this.embedder);
  }

  async indexRepository(request: IndexRequest): Promise<IndexResponse> {
    const startTime = Date.now();
    
    try {
      console.log(`Starting ${request.mode} indexing of ${request.repository_path}`);
      
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
      
      const timeTaken = Date.now() - startTime;
      console.log(`Indexing completed in ${timeTaken}ms`);
      
      return {
        status: 'success',
        chunks_processed: embeddedChunks.length,
        time_taken_ms: timeTaken
      };
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