import { CodeChunk } from './types';

export class VectorStore {
  protected chunks: Map<string, CodeChunk> = new Map();

  async upsertChunks(chunks: CodeChunk[]): Promise<void> {
    for (const chunk of chunks) {
      this.chunks.set(chunk.chunk_id, chunk);
    }
  }

  async similaritySearch(
    queryEmbedding: number[],
    limit: number = 20
  ): Promise<CodeChunk[]> {
    const results: Array<{ chunk: CodeChunk; similarity: number }> = [];

    for (const chunk of this.chunks.values()) {
      if (chunk.embedding.length === 0) continue;
      
      const similarity = this.cosineSimilarity(queryEmbedding, chunk.embedding);
      results.push({ chunk, similarity });
    }

    // Sort by similarity descending and take top results
    results.sort((a, b) => b.similarity - a.similarity);
    
    return results
      .slice(0, limit)
      .map(result => ({
        ...result.chunk,
        relevance_score: result.similarity
      }));
  }

  async getChunk(chunkId: string): Promise<CodeChunk | undefined> {
    return this.chunks.get(chunkId);
  }

  async deleteChunk(chunkId: string): Promise<void> {
    this.chunks.delete(chunkId);
  }

  async clear(): Promise<void> {
    this.chunks.clear();
  }

  async getStats(): Promise<{ total_chunks: number; [key: string]: any }> {
    return { total_chunks: this.chunks.size };
  }

  async findByRelationship(relationshipType: 'exports' | 'symbol_name', symbol: string): Promise<CodeChunk[]> {
    const results: CodeChunk[] = [];
    
    for (const chunk of this.chunks.values()) {
      if (relationshipType === 'exports') {
        if (chunk.relationships.exports.includes(symbol)) {
          results.push(chunk);
        }
      } else if (relationshipType === 'symbol_name') {
        if (chunk.symbol_name === symbol) {
          results.push(chunk);
        }
      }
    }
    
    return results;
  }

  async findByFilePath(filePath: string): Promise<CodeChunk[]> {
    const results: CodeChunk[] = [];
    
    for (const chunk of this.chunks.values()) {
      if (chunk.file_path === filePath) {
        results.push(chunk);
      }
    }
    
    return results;
  }

  /**
   * Get chunk by symbol name or chunk ID
   */
  async getChunkBySymbol(symbolId: string): Promise<CodeChunk | null> {
    // First try direct chunk ID lookup
    const chunkById = this.chunks.get(symbolId);
    if (chunkById) {
      return chunkById;
    }

    // Then try to find by symbol name
    for (const chunk of this.chunks.values()) {
      if (chunk.symbol_name === symbolId) {
        return chunk;
      }
    }

    return null;
  }

  /**
   * Get all chunks
   */
  getAllChunks(): CodeChunk[] {
    return Array.from(this.chunks.values());
  }

  /**
   * Get chunk count
   */
  getChunkCount(): number {
    return this.chunks.size;
  }

  private cosineSimilarity(a: number[], b: number[]): number {
    if (a.length !== b.length) return 0;

    let dotProduct = 0;
    let normA = 0;
    let normB = 0;

    for (let i = 0; i < a.length; i++) {
      dotProduct += a[i] * b[i];
      normA += a[i] * a[i];
      normB += b[i] * b[i];
    }

    if (normA === 0 || normB === 0) return 0;

    return dotProduct / (Math.sqrt(normA) * Math.sqrt(normB));
  }
}