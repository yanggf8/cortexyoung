import { QueryRequest, QueryResponse, SearchResponse, CodeChunk, ContextPackage, ContextGroup, MultiHopConfig } from './types';
import { VectorStore } from './vector-store';
import { EmbeddingGenerator } from './embedder';
import { RelationshipTraversalEngine } from './relationship-traversal-engine';
import { RelationshipQuery, TraversalOptions, RelationshipType } from './relationship-types';

export class SemanticSearcher {
  private relationshipEngine?: RelationshipTraversalEngine;

  constructor(
    private vectorStore: VectorStore,
    private embedder: EmbeddingGenerator,
    private repositoryPath?: string
  ) {
    if (repositoryPath) {
      this.relationshipEngine = new RelationshipTraversalEngine(repositoryPath);
    }
  }

  async initializeRelationshipEngine(files: Map<string, string>): Promise<void> {
    if (this.relationshipEngine) {
      console.log('🔗 Initializing relationship engine with persistent storage...');
      await this.relationshipEngine.buildRelationshipGraph(files);
      console.log('✅ Relationship engine ready with persistent cache support');
    }
  }

  async search(query: QueryRequest): Promise<SearchResponse> {
    const startTime = Date.now();
    
    try {
      console.log(`🔍 Searching for: "${query.task}"`);
      
      // Check if we should use relationship-aware search
      if (this.relationshipEngine && query.multi_hop?.enabled) {
        return await this.relationshipAwareSearch(query, startTime);
      }
      
      // Fallback to traditional semantic search
      return await this.traditionalSemanticSearch(query, startTime);
      
    } catch (error) {
      console.error('Search failed:', error);
      
      return {
        status: 'error',
        chunks: [],
        context_package: {
          total_tokens: 0,
          groups: [],
          summary: 'Search failed',
          token_efficiency: 0,
          related_files: []
        },
        query_time_ms: Date.now() - startTime,
        total_chunks_considered: 0,
        metadata: {
          total_chunks_found: 0,
          query_time_ms: Date.now() - startTime,
          chunks_returned: 0,
          token_estimate: 0,
          efficiency_score: 0
        }
      };
    }
  }

  private async relationshipAwareSearch(query: QueryRequest, startTime: number): Promise<SearchResponse> {
    console.log('🔗 Using relationship-aware search');
    
    // Generate query embedding for initial search
    const queryEmbedding = await this.embedder.embed(query.task);
    
    // Perform initial semantic search to find starting points
    const initialCandidates = await this.vectorStore.similaritySearch(
      queryEmbedding,
      Math.min(query.max_chunks || 20, 10) // Limit initial candidates for traversal
    );
    
    console.log(`📊 Found ${initialCandidates.length} initial candidates`);
    
    if (initialCandidates.length === 0) {
      return this.createEmptyResponse(startTime);
    }

    // Build relationship query
    const relationshipQuery: RelationshipQuery = {
      baseQuery: query.task,
      focusSymbols: this.extractFocusSymbols(query.task),
      relationshipTypes: this.mapMultiHopToRelationshipTypes(query.multi_hop!),
      traversalOptions: this.buildTraversalOptions(query.multi_hop!),
      includeContext: true,
      contextRadius: 3
    };

    // Execute relationship traversal
    const relationshipResult = await this.relationshipEngine!.executeRelationshipQuery(relationshipQuery);
    
    console.log(`🔗 Found ${relationshipResult.relationshipPaths.length} relationship paths`);
    console.log(`📊 Generated ${relationshipResult.contextGroups.length} context groups`);

    // Combine initial candidates with relationship-discovered chunks
    const allChunkIds = new Set([
      ...initialCandidates.map(c => c.chunk_id),
      ...relationshipResult.primaryChunks,
      ...relationshipResult.relatedChunks
    ]);

    // Retrieve all chunks
    const allChunks = await this.retrieveChunksByIds(Array.from(allChunkIds));
    
    // Enhanced ranking with relationship information
    const rankedResults = this.rankWithRelationships(
      allChunks,
      relationshipResult,
      query,
      queryEmbedding
    );

    // Synthesize enhanced context package
    const contextPackage = this.synthesizeRelationshipContext(
      rankedResults,
      relationshipResult,
      query.context_mode || 'structured'
    );

    const queryTime = Date.now() - startTime;
    
    return {
      status: 'success',
      chunks: rankedResults.slice(0, query.max_chunks || 20),
      context_package: contextPackage,
      query_time_ms: queryTime,
      total_chunks_considered: allChunks.length,
      relationship_paths: relationshipResult.relationshipPaths.slice(0, 5), // Include top paths
      efficiency_score: relationshipResult.efficiencyScore,
      metadata: {
        total_chunks_found: allChunks.length,
        query_time_ms: queryTime,
        chunks_returned: rankedResults.slice(0, query.max_chunks || 20).length,
        token_estimate: contextPackage.total_tokens || 0,
        efficiency_score: relationshipResult.efficiencyScore
      }
    };
  }

  private async traditionalSemanticSearch(query: QueryRequest, startTime: number): Promise<SearchResponse> {
    console.log('📊 Using traditional semantic search');
    
    // Generate query embedding
    const queryEmbedding = await this.embedder.embed(query.task);
    
    // Perform initial semantic search
    const candidates = await this.vectorStore.similaritySearch(
      queryEmbedding,
      query.max_chunks || 20
    );
    
    console.log(`📊 Found ${candidates.length} candidate chunks`);
    
    // Apply legacy multi-hop expansion if needed
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
      status: 'success',
      chunks: rankedResults,
      context_package: contextPackage,
      query_time_ms: queryTime,
      total_chunks_considered: expandedCandidates.length,
      metadata: {
        total_chunks_found: expandedCandidates.length,
        query_time_ms: queryTime,
        chunks_returned: rankedResults.length,
        token_estimate: contextPackage.total_tokens || 0,
        efficiency_score: contextPackage.token_efficiency || 0.7
      }
    };
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

  // New methods for relationship-aware search

  private createEmptyResponse(startTime: number): SearchResponse {
    const queryTime = Date.now() - startTime;
    return {
      status: 'success',
      chunks: [],
      context_package: {
        total_tokens: 0,
        groups: [],
        summary: 'No relevant code found',
        token_efficiency: 0,
        related_files: []
      },
      query_time_ms: queryTime,
      total_chunks_considered: 0,
      metadata: {
        total_chunks_found: 0,
        query_time_ms: queryTime,
        chunks_returned: 0,
        token_estimate: 0,
        efficiency_score: 0
      }
    };
  }

  private extractFocusSymbols(query: string): string[] {
    // Extract potential function/class names from query
    const words = query.toLowerCase().split(/\s+/);
    const focusSymbols: string[] = [];

    // Look for patterns like "function X", "class Y", "method Z"
    for (let i = 0; i < words.length - 1; i++) {
      if (['function', 'class', 'method', 'component', 'service'].includes(words[i])) {
        focusSymbols.push(words[i + 1]);
      }
    }

    // Look for camelCase or PascalCase identifiers
    const identifierPattern = /\b[a-z][a-zA-Z0-9]*|[A-Z][a-zA-Z0-9]*/g;
    const identifiers = query.match(identifierPattern) || [];
    focusSymbols.push(...identifiers.filter(id => id.length > 2));

    return [...new Set(focusSymbols)]; // Remove duplicates
  }

  private mapMultiHopToRelationshipTypes(multiHop: MultiHopConfig): RelationshipType[] {
    const typeMap: Record<string, RelationshipType> = {
      'calls': 'calls',
      'imports': 'imports',
      'data_flow': 'data_flow',
      'co_change': 'depends_on' // Map co_change to depends_on for now
    };

    return multiHop.relationship_types.map(type => typeMap[type] || 'depends_on');
  }

  private buildTraversalOptions(multiHop: MultiHopConfig): TraversalOptions {
    return {
      maxDepth: multiHop.max_hops || 2,
      relationshipTypes: this.mapMultiHopToRelationshipTypes(multiHop),
      direction: 'both',
      minStrength: 0.3,
      minConfidence: 0.5,
      includeTransitive: true,
      pruneStrategy: 'strength',
      maxResults: 50
    };
  }

  private async retrieveChunksByIds(chunkIds: string[]): Promise<CodeChunk[]> {
    const chunks: CodeChunk[] = [];
    
    for (const chunkId of chunkIds) {
      try {
        const chunk = await this.vectorStore.getChunk(chunkId);
        if (chunk) {
          chunks.push(chunk);
        }
      } catch (error) {
        console.warn(`Failed to retrieve chunk ${chunkId}:`, error);
      }
    }
    
    return chunks;
  }

  private rankWithRelationships(
    chunks: CodeChunk[],
    relationshipResult: any,
    query: QueryRequest,
    queryEmbedding: number[]
  ): CodeChunk[] {
    // Enhanced ranking that combines semantic similarity with relationship strength
    return chunks
      .map(chunk => {
        let score = (chunk as any).similarity_score || 0;
        
        // Boost score if chunk is in a strong relationship path
        const chunkInPaths = relationshipResult.relationshipPaths.filter((path: any) =>
          path.symbols.some((symbolId: string) => symbolId.includes(chunk.chunk_id))
        );
        
        if (chunkInPaths.length > 0) {
          const avgPathStrength = chunkInPaths.reduce((sum: number, path: any) => 
            sum + path.totalStrength, 0) / chunkInPaths.length;
          score += avgPathStrength * 0.3; // 30% boost from relationship strength
        }
        
        // Boost score if chunk is in high-importance context group
        const chunkInGroups = relationshipResult.contextGroups.filter((group: any) =>
          group.chunkIds.includes(chunk.chunk_id)
        );
        
        if (chunkInGroups.length > 0) {
          const avgGroupImportance = chunkInGroups.reduce((sum: number, group: any) => 
            sum + group.importance, 0) / chunkInGroups.length;
          score += avgGroupImportance * 0.2; // 20% boost from group importance
        }
        
        return { ...chunk, enhanced_score: score };
      })
      .sort((a, b) => (b as any).enhanced_score - (a as any).enhanced_score)
      .slice(0, query.max_chunks || 20);
  }

  private synthesizeRelationshipContext(
    chunks: CodeChunk[],
    relationshipResult: any,
    mode: string
  ): ContextPackage {
    const groups: ContextGroup[] = [];
    
    // Convert relationship context groups to our format
    for (const relGroup of relationshipResult.contextGroups) {
      const groupChunks = chunks.filter(chunk => 
        relGroup.chunkIds.includes(chunk.chunk_id)
      );
      
      if (groupChunks.length > 0) {
        groups.push({
          title: relGroup.theme,
          description: this.generateRelationshipGroupDescription(relGroup),
          chunks: groupChunks,
          importance_score: relGroup.importance,
          relationship_paths: relGroup.relationships?.slice(0, 3) // Include top relationships
        });
      }
    }
    
    // Add traditional file-based groups for chunks not in relationship groups
    const remainingChunks = chunks.filter(chunk => 
      !relationshipResult.contextGroups.some((group: any) =>
        group.chunkIds.includes(chunk.chunk_id)
      )
    );
    
    if (remainingChunks.length > 0) {
      const fileGroups = this.groupChunksByFile(remainingChunks);
      groups.push(...fileGroups);
    }
    
    // Generate enhanced summary
    const summary = this.generateRelationshipSummary(chunks, relationshipResult);
    
    return {
      summary,
      groups: groups.sort((a, b) => b.importance_score - a.importance_score),
      related_files: [...new Set(chunks.map(c => c.file_path))],
      relationship_insights: this.generateRelationshipInsights(relationshipResult),
      total_tokens: relationshipResult.totalTokens || this.estimateTokens({ summary, groups } as ContextPackage),
      token_efficiency: relationshipResult.efficiencyScore || 0.7
    };
  }

  private generateRelationshipGroupDescription(relGroup: any): string {
    const symbolCount = relGroup.symbols?.length || 0;
    const relationshipCount = relGroup.relationships?.length || 0;
    
    return `${symbolCount} symbols connected by ${relationshipCount} relationships`;
  }

  private groupChunksByFile(chunks: CodeChunk[]): ContextGroup[] {
    const fileGroups = new Map<string, CodeChunk[]>();
    
    for (const chunk of chunks) {
      if (!fileGroups.has(chunk.file_path)) {
        fileGroups.set(chunk.file_path, []);
      }
      fileGroups.get(chunk.file_path)!.push(chunk);
    }
    
    return Array.from(fileGroups.entries()).map(([filePath, fileChunks]) => ({
      title: `File: ${filePath}`,
      description: `Additional code chunks from ${filePath}`,
      chunks: fileChunks,
      importance_score: this.calculateGroupImportance(fileChunks)
    }));
  }

  private generateRelationshipSummary(chunks: CodeChunk[], relationshipResult: any): string {
    const pathCount = relationshipResult.relationshipPaths?.length || 0;
    const groupCount = relationshipResult.contextGroups?.length || 0;
    const fileCount = new Set(chunks.map(c => c.file_path)).size;
    
    const parts = [
      `Found ${chunks.length} code chunks across ${fileCount} files`,
    ];
    
    if (pathCount > 0) {
      parts.push(`${pathCount} relationship paths discovered`);
    }
    
    if (groupCount > 0) {
      parts.push(`organized into ${groupCount} thematic groups`);
    }
    
    // Add efficiency insight
    const efficiency = Math.round((relationshipResult.efficiencyScore || 0) * 100);
    parts.push(`${efficiency}% context efficiency through relationship analysis`);
    
    return parts.join(', ') + '.';
  }

  private generateRelationshipInsights(relationshipResult: any): string[] {
    const insights: string[] = [];
    
    // Analyze strongest paths
    if (relationshipResult.relationshipPaths?.length > 0) {
      const strongestPath = relationshipResult.relationshipPaths[0];
      insights.push(`Strongest connection: ${strongestPath.description}`);
    }
    
    // Analyze group themes
    const themes = relationshipResult.contextGroups?.map((g: any) => g.theme) || [];
    if (themes.length > 0) {
      insights.push(`Key areas: ${themes.slice(0, 3).join(', ')}`);
    }
    
    // Token efficiency insight
    const efficiency = relationshipResult.efficiencyScore || 0;
    if (efficiency > 0.8) {
      insights.push('High relevance - relationship traversal found highly connected code');
    } else if (efficiency > 0.5) {
      insights.push('Moderate relevance - some code connected through relationships');
    } else {
      insights.push('Low connectivity - mostly independent code pieces');
    }
    
    return insights;
  }
}