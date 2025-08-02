import { QueryRequest, QueryResponse, CodeChunk, ContextPackage, ContextGroup, MultiHopConfig } from './types';
import { VectorStore } from './vector-store';
import { EmbeddingGenerator } from './embedder';

export class SemanticSearcher {
  constructor(
    private vectorStore: VectorStore,
    private embedder: EmbeddingGenerator
  ) {}

  async search(query: QueryRequest): Promise<QueryResponse> {
    const startTime = Date.now();
    
    try {
      console.log(`🔍 Searching for: "${query.task}"`);
      
      // Generate query embedding
      const queryEmbedding = await this.embedder.embed(query.task);
      
      // Perform initial semantic search
      const candidates = await this.vectorStore.similaritySearch(
        queryEmbedding,
        query.max_chunks || 20
      );
      
      console.log(`📊 Found ${candidates.length} candidate chunks`);
      
      // Apply multi-hop expansion if enabled
      let expandedCandidates = candidates;
      if (query.multi_hop?.enabled) {
        expandedCandidates = await this.expandWithRelationships(
          candidates,
          query.multi_hop
        );
        console.log(`🔗 Expanded to ${expandedCandidates.length} chunks via relationships`);
      }
      
      // Rank and filter results
      const rankedResults = this.rankResults(
        expandedCandidates,
        query
      );
      
      // Synthesize context package
      const contextPackage = this.synthesizeContext(
        rankedResults,
        query.context_mode || 'structured'
      );
      
      const queryTime = Date.now() - startTime;
      
      return {
        context_package: contextPackage,
        metadata: {
          total_chunks_found: candidates.length,
          query_time_ms: queryTime,
          chunks_returned: rankedResults.length,
          token_estimate: this.estimateTokens(contextPackage),
          efficiency_score: this.calculateEfficiency(contextPackage)
        }
      };
    } catch (error) {
      throw new Error(`Search failed: ${error instanceof Error ? error.message : 'Unknown error'}`);
    }
  }

  private async expandWithRelationships(
    candidates: CodeChunk[],
    multiHopConfig: MultiHopConfig
  ): Promise<CodeChunk[]> {
    const expanded = new Set(candidates);
    const processedFiles = new Set<string>();
    
    for (const chunk of candidates) {
      if (processedFiles.has(chunk.file_path)) continue;
      processedFiles.add(chunk.file_path);
      
      // Follow relationships based on configured types
      for (const relType of multiHopConfig.relationship_types) {
        switch (relType) {
          case 'imports':
            if (chunk.relationships.imports.length > 0) {
              const importedChunks = await this.findRelatedChunks(chunk.relationships.imports, 'exports');
              importedChunks.forEach(c => expanded.add(c));
            }
            break;
            
          case 'calls':
            if (chunk.relationships.calls.length > 0) {
              const calledChunks = await this.findRelatedChunks(chunk.relationships.calls, 'symbol_name');
              calledChunks.forEach(c => expanded.add(c));
            }
            break;
            
          case 'co_change':
            if (chunk.git_metadata?.co_change_files.length > 0) {
              const coChangeChunks = await this.findChunksByFiles(chunk.git_metadata.co_change_files);
              coChangeChunks.forEach(c => expanded.add(c));
            }
            break;
            
          case 'data_flow':
            if (chunk.relationships.data_flow.length > 0) {
              const dataFlowChunks = await this.findRelatedChunks(chunk.relationships.data_flow, 'symbol_name');
              dataFlowChunks.forEach(c => expanded.add(c));
            }
            break;
        }
      }
    }
    
    return Array.from(expanded);
  }

  private async findRelatedChunks(symbols: string[], relationshipType: 'exports' | 'symbol_name'): Promise<CodeChunk[]> {
    const related: CodeChunk[] = [];
    
    for (const symbol of symbols) {
      const chunks = await this.vectorStore.findByRelationship(relationshipType, symbol);
      related.push(...chunks);
    }
    
    return related;
  }

  private async findChunksByFiles(filePaths: string[]): Promise<CodeChunk[]> {
    const chunks: CodeChunk[] = [];
    
    for (const filePath of filePaths) {
      const fileChunks = await this.vectorStore.findByFilePath(filePath);
      chunks.push(...fileChunks);
    }
    
    return chunks;
  }

  private rankResults(chunks: CodeChunk[], query: QueryRequest): CodeChunk[] {
    // Sort by similarity score (assuming chunks have similarity scores from vector search)
    return chunks
      .sort((a, b) => (b as any).similarity_score - (a as any).similarity_score)
      .slice(0, query.max_chunks || 20);
  }

  private synthesizeContext(chunks: CodeChunk[], mode: string): ContextPackage {
    // Group chunks by file and type
    const fileGroups = new Map<string, CodeChunk[]>();
    const typeGroups = new Map<string, CodeChunk[]>();
    
    for (const chunk of chunks) {
      // Group by file
      if (!fileGroups.has(chunk.file_path)) {
        fileGroups.set(chunk.file_path, []);
      }
      fileGroups.get(chunk.file_path)!.push(chunk);
      
      // Group by chunk type
      if (!typeGroups.has(chunk.chunk_type)) {
        typeGroups.set(chunk.chunk_type, []);
      }
      typeGroups.get(chunk.chunk_type)!.push(chunk);
    }
    
    const groups: ContextGroup[] = [];
    
    // Create file-based groups
    for (const [filePath, fileChunks] of fileGroups) {
      groups.push({
        title: `File: ${filePath}`,
        description: `Code chunks from ${filePath}`,
        chunks: fileChunks,
        importance_score: this.calculateGroupImportance(fileChunks)
      });
    }
    
    // Create summary
    const summary = this.generateSummary(chunks, groups);
    
    return {
      summary,
      groups: groups.sort((a, b) => b.importance_score - a.importance_score),
      related_files: Array.from(fileGroups.keys())
    };
  }

  private calculateGroupImportance(chunks: CodeChunk[]): number {
    // Base importance on chunk count and complexity
    const complexitySum = chunks.reduce((sum, chunk) => sum + (chunk.language_metadata.complexity_score || 1), 0);
    const recencyBonus = chunks.some(chunk => 
      chunk.git_metadata?.commit_date && 
      Date.now() - new Date(chunk.git_metadata.commit_date).getTime() < 7 * 24 * 60 * 60 * 1000
    ) ? 0.2 : 0;
    
    return Math.min(1.0, (complexitySum / chunks.length / 10) + recencyBonus);
  }

  private generateSummary(chunks: CodeChunk[], groups: ContextGroup[]): string {
    const fileCount = new Set(chunks.map(c => c.file_path)).size;
    const functionCount = chunks.filter(c => c.chunk_type === 'function').length;
    const classCount = chunks.filter(c => c.chunk_type === 'class').length;
    
    const parts = [`Found ${chunks.length} relevant code chunks across ${fileCount} files`];
    
    if (functionCount > 0) {
      parts.push(`${functionCount} functions`);
    }
    if (classCount > 0) {
      parts.push(`${classCount} classes`);
    }
    
    // Add top file mentions
    const topFiles = groups.slice(0, 3).map(g => g.title.replace('File: ', ''));
    if (topFiles.length > 0) {
      parts.push(`Key files: ${topFiles.join(', ')}`);
    }
    
    return parts.join('. ') + '.';
  }

  private estimateTokens(contextPackage: ContextPackage): number {
    // Rough token estimation: ~4 characters per token
    let totalChars = contextPackage.summary.length;
    
    for (const group of contextPackage.groups) {
      totalChars += group.title.length + group.description.length;
      for (const chunk of group.chunks) {
        totalChars += chunk.content.length;
      }
    }
    
    return Math.ceil(totalChars / 4);
  }

  private calculateEfficiency(contextPackage: ContextPackage): number {
    // Efficiency = relevant_tokens / total_tokens
    // Higher importance scores suggest better relevance
    const avgImportance = contextPackage.groups.reduce((sum, g) => sum + g.importance_score, 0) / contextPackage.groups.length;
    return Math.min(0.95, Math.max(0.1, avgImportance));
  }
}