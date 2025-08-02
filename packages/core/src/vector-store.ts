import { CodeChunk } from '../../shared/src/index';

export class VectorStore {
  private chunks: Map<string, CodeChunk> = new Map();

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

  async getStats(): Promise<{ total_chunks: number }> {
    return { total_chunks: this.chunks.size };
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