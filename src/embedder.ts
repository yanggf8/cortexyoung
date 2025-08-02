import { FlagEmbedding, EmbeddingModel } from 'fastembed';

export class EmbeddingGenerator {
  private embedder: FlagEmbedding | null = null;
  private isInitialized = false;
  private readonly modelName = EmbeddingModel.BGESmallENV15;

  constructor() {
    // Model will be initialized on first use
  }

  private async ensureInitialized(): Promise<void> {
    if (!this.isInitialized) {
      try {
        console.log(`🔄 Initializing BGE-small-en-v1.5 embedding model...`);
        this.embedder = await FlagEmbedding.init({
          model: this.modelName,
          maxLength: 512,
          cacheDir: './.fastembed_cache'
        });
        this.isInitialized = true;
        console.log('✅ Embedding model initialized successfully');
      } catch (error) {
        console.warn('⚠️  Failed to initialize embedding model, using mock embeddings:', error);
        this.embedder = null;
        this.isInitialized = true;
      }
    }
  }

  async embed(text: string): Promise<number[]> {
    await this.ensureInitialized();
    
    if (!this.embedder) {
      return this.generateMockEmbedding(text);
    }

    try {
      const embeddings = this.embedder.embed([text]);
      for await (const batch of embeddings) {
        return Array.from(batch[0]);
      }
      return this.generateMockEmbedding(text);
    } catch (error) {
      console.warn('Failed to generate embedding, using mock:', error);
      return this.generateMockEmbedding(text);
    }
  }

  async embedBatch(texts: string[]): Promise<number[][]> {
    await this.ensureInitialized();
    
    if (!this.embedder) {
      return texts.map(text => this.generateMockEmbedding(text));
    }

    try {
      const embeddings = this.embedder.embed(texts);
      const results: number[][] = [];
      for await (const batch of embeddings) {
        results.push(...batch.map(embedding => Array.from(embedding)));
      }
      return results;
    } catch (error) {
      console.warn('Failed to generate batch embeddings, using mock:', error);
      return texts.map(text => this.generateMockEmbedding(text));
    }
  }

  async getModelInfo(): Promise<{ name: string; dimension: number; isLoaded: boolean }> {
    await this.ensureInitialized();
    
    return {
      name: 'BGE-small-en-v1.5',
      dimension: 384, // BGE small model dimension
      isLoaded: this.embedder !== null
    };
  }

  private generateMockEmbedding(text: string): number[] {
    // Generate deterministic mock embedding based on text
    const dimension = 384; // Same as BGE small model
    const embedding = new Array(dimension);
    
    // Simple hash-based mock embedding
    let seed = this.hashString(text);
    for (let i = 0; i < dimension; i++) {
      seed = (seed * 9301 + 49297) % 233280;
      embedding[i] = (seed / 233280) * 2 - 1; // Normalize to [-1, 1]
    }
    
    return embedding;
  }

  private hashString(str: string): number {
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash;
    }
    return Math.abs(hash);
  }
}