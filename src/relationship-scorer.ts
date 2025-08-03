import {
  CodeSymbol,
  CodeRelationship,
  RelationshipPath,
  RelationshipScorer,
  ScoringContext,
  RelationshipType
} from './relationship-types';

export class AdvancedRelationshipScorer implements RelationshipScorer {

  scoreRelationship(relationship: CodeRelationship, context: ScoringContext): number {
    let score = relationship.strength; // Base score from relationship strength

    // Apply type-specific scoring
    score *= this.getRelationshipTypeWeight(relationship.type);

    // Apply confidence weighting
    score *= relationship.confidence;

    // Apply frequency weighting if available
    const frequency = relationship.metadata?.frequency || 1;
    score *= Math.min(2.0, 1.0 + Math.log10(frequency) * 0.3);

    // Apply recency weighting for recent changes
    if (context.recentChanges) {
      const symbolFile = this.extractFileFromSymbolId(relationship.fromSymbol);
      const recentChange = context.recentChanges.get(symbolFile);
      if (recentChange) {
        const daysSinceChange = (Date.now() - recentChange.getTime()) / (1000 * 60 * 60 * 24);
        const recencyBoost = Math.exp(-daysSinceChange / 7) * 0.3; // Exponential decay over 7 days
        score *= (1.0 + recencyBoost);
      }
    }

    // Apply error frequency penalty/boost
    if (context.errorFrequency) {
      const symbolName = this.extractSymbolNameFromId(relationship.fromSymbol);
      const errorCount = context.errorFrequency.get(symbolName) || 0;
      if (errorCount > 0) {
        // Boost if this is error handling code (catches, throws)
        if (['catches', 'throws'].includes(relationship.type)) {
          score *= (1.0 + Math.min(0.5, errorCount * 0.1));
        } else {
          // Slight penalty for error-prone code in other contexts
          score *= Math.max(0.7, 1.0 - (errorCount * 0.05));
        }
      }
    }

    // Apply user feedback if available
    if (context.userFeedback) {
      const symbolName = this.extractSymbolNameFromId(relationship.fromSymbol);
      const feedback = context.userFeedback.get(symbolName) || 0;
      score *= (1.0 + feedback * 0.2); // ±20% based on user feedback
    }

    // Semantic similarity boost if query embedding is available
    if (context.queryEmbedding) {
      const semanticBoost = this.calculateSemanticBoost(relationship, context);
      score *= (1.0 + semanticBoost);
    }

    return Math.min(1.0, Math.max(0.0, score));
  }

  scoreSymbol(symbol: CodeSymbol, context: ScoringContext): number {
    let score = 0.5; // Base score

    // Type-specific scoring
    score += this.getSymbolTypeWeight(symbol.type);

    // Scope scoring (more specific scopes often more relevant)
    score += this.getScopeWeight(symbol.scope);

    // File recency scoring
    if (context.recentChanges) {
      const recentChange = context.recentChanges.get(symbol.filePath);
      if (recentChange) {
        const daysSinceChange = (Date.now() - recentChange.getTime()) / (1000 * 60 * 60 * 24);
        score += Math.exp(-daysSinceChange / 7) * 0.3;
      }
    }

    // Error frequency impact
    if (context.errorFrequency) {
      const errorCount = context.errorFrequency.get(symbol.name) || 0;
      if (errorCount > 0) {
        // Boost for error handling symbols
        if (symbol.name.toLowerCase().includes('error') || 
            symbol.name.toLowerCase().includes('catch') ||
            symbol.name.toLowerCase().includes('handle')) {
          score += Math.min(0.3, errorCount * 0.05);
        }
      }
    }

    // Focus area matching
    if (context.focusAreas) {
      const symbolText = `${symbol.name} ${symbol.filePath}`.toLowerCase();
      for (const focusArea of context.focusAreas) {
        if (symbolText.includes(focusArea.toLowerCase())) {
          score += 0.2;
          break;
        }
      }
    }

    // User feedback
    if (context.userFeedback) {
      const feedback = context.userFeedback.get(symbol.name) || 0;
      score += feedback * 0.3;
    }

    return Math.min(1.0, Math.max(0.0, score));
  }

  scorePath(path: RelationshipPath, context: ScoringContext): number {
    if (path.symbols.length === 0) return 0;

    // Base score from path strength
    let score = path.totalStrength;

    // Apply path type weighting
    score *= this.getPathTypeWeight(path.pathType);

    // Apply depth penalty (shorter paths generally better)
    const depthPenalty = Math.pow(0.8, path.depth - 1);
    score *= depthPenalty;

    // Boost for certain path patterns
    if (this.isImportantPathPattern(path, context)) {
      score *= 1.3;
    }

    // Penalize overly complex paths
    if (path.depth > 4) {
      score *= 0.7;
    }

    // Apply semantic relevance if query is available
    if (context.originalQuery) {
      const semanticRelevance = this.calculatePathSemanticRelevance(path, context);
      score *= (1.0 + semanticRelevance * 0.3);
    }

    return Math.min(1.0, Math.max(0.0, score));
  }

  // Enhanced scoring methods for multi-hop analysis

  scoreMultiHopPath(
    path: RelationshipPath,
    hops: number,
    context: ScoringContext
  ): number {
    let score = this.scorePath(path, context);

    // Multi-hop specific adjustments
    
    // Reward meaningful multi-hop patterns
    if (hops >= 2) {
      const pathEfficiency = this.calculatePathEfficiency(path);
      score *= pathEfficiency;

      // Boost for error propagation chains
      if (path.pathType === 'error_propagation' && hops >= 2) {
        score *= 1.4;
      }

      // Boost for data flow chains
      if (path.pathType === 'data_flow_chain' && hops >= 2) {
        score *= 1.3;
      }

      // Boost for call chains that follow common patterns
      if (path.pathType === 'call_chain' && this.isCommonCallPattern(path)) {
        score *= 1.2;
      }
    }

    // Penalize excessive hops
    if (hops > 5) {
      score *= Math.pow(0.9, hops - 5);
    }

    // Reward complete chains (loops back to start or reaches important endpoint)
    if (this.isCompleteChain(path, context)) {
      score *= 1.2;
    }

    return score;
  }

  calculateRelationshipRelevance(
    relationships: CodeRelationship[],
    query: string,
    queryEmbedding?: number[]
  ): Map<string, number> {
    const relevanceMap = new Map<string, number>();

    const queryLower = query.toLowerCase();
    const queryTerms = queryLower.split(/\s+/).filter(term => term.length > 2);

    for (const relationship of relationships) {
      let relevance = relationship.strength * relationship.confidence;

      // Keyword matching boost
      const relText = this.getRelationshipTextRepresentation(relationship).toLowerCase();
      const matchingTerms = queryTerms.filter(term => relText.includes(term));
      if (matchingTerms.length > 0) {
        relevance *= (1.0 + (matchingTerms.length / queryTerms.length) * 0.5);
      }

      // Semantic embedding similarity if available
      if (queryEmbedding && relationship.metadata?.queryEmbedding) {
        const similarity = this.cosineSimilarity(queryEmbedding, relationship.metadata.queryEmbedding);
        relevance *= (1.0 + similarity * 0.3);
      }

      relevanceMap.set(relationship.id, relevance);
    }

    return relevanceMap;
  }

  rankRelationshipPaths(
    paths: RelationshipPath[],
    context: ScoringContext,
    maxResults: number = 20
  ): RelationshipPath[] {
    return paths
      .map(path => ({
        ...path,
        computedScore: this.scoreMultiHopPath(path, path.depth, context)
      }))
      .sort((a, b) => (b as any).computedScore - (a as any).computedScore)
      .slice(0, maxResults);
  }

  // Private helper methods

  private getRelationshipTypeWeight(type: RelationshipType): number {
    const weights: Record<RelationshipType, number> = {
      calls: 1.0,           // Direct function calls are very important
      data_flow: 0.9,       // Data flow relationships are crucial
      imports: 0.8,         // Import dependencies are important
      throws: 0.9,          // Error handling is important
      catches: 0.9,         // Error handling is important
      extends: 0.7,         // Inheritance relationships
      implements: 0.7,      // Interface implementations
      exports: 0.6,         // Export relationships
      instantiates: 0.8,    // Object creation
      accesses: 0.6,        // Property access
      assigns: 0.7,         // Variable assignments
      configures: 0.5,      // Configuration relationships
      depends_on: 0.6       // Generic dependencies
    };

    return weights[type] || 0.5;
  }

  private getSymbolTypeWeight(type: string): number {
    const weights: Record<string, number> = {
      function: 0.3,
      method: 0.3,
      class: 0.2,
      variable: 0.1,
      property: 0.1,
      module: 0.2
    };

    return weights[type] || 0.1;
  }

  private getScopeWeight(scope: string): number {
    const weights = {
      local: 0.1,    // Local scope often most relevant
      class: 0.08,   // Class scope is quite relevant
      module: 0.05,  // Module scope somewhat relevant
      global: 0.02   // Global scope less specific
    };

    return weights[scope as keyof typeof weights] || 0.05;
  }

  private getPathTypeWeight(pathType: string): number {
    const weights = {
      call_chain: 1.0,
      data_flow_chain: 0.95,
      error_propagation: 0.9,
      dependency_chain: 0.8,
      inheritance_chain: 0.7,
      mixed: 0.6
    };

    return weights[pathType as keyof typeof weights] || 0.5;
  }

  private calculateSemanticBoost(relationship: CodeRelationship, context: ScoringContext): number {
    if (!context.queryEmbedding) return 0;

    // This would calculate semantic similarity between relationship and query
    // For now, return a simple boost based on relationship type relevance
    const queryLower = context.originalQuery?.toLowerCase() || '';
    
    // Boost error-related relationships for error queries
    if (queryLower.includes('error') || queryLower.includes('exception')) {
      if (['throws', 'catches'].includes(relationship.type)) {
        return 0.3;
      }
    }

    // Boost call relationships for debugging queries
    if (queryLower.includes('debug') || queryLower.includes('trace')) {
      if (relationship.type === 'calls') {
        return 0.2;
      }
    }

    // Boost data flow for data-related queries
    if (queryLower.includes('data') || queryLower.includes('variable')) {
      if (relationship.type === 'data_flow') {
        return 0.25;
      }
    }

    return 0;
  }

  private calculatePathEfficiency(path: RelationshipPath): number {
    if (path.relationships.length === 0) return 1.0;

    // Calculate how "direct" the path is
    const avgStrength = path.totalStrength;
    const lengthPenalty = 1.0 / Math.sqrt(path.relationships.length);
    
    return avgStrength * lengthPenalty;
  }

  private isCommonCallPattern(path: RelationshipPath): boolean {
    if (path.relationships.length < 2) return false;

    // Check for common patterns like: controller -> service -> repository
    // or: handler -> validator -> processor
    // This is a simplified check - in practice would be more sophisticated
    return path.relationships.length >= 2 && path.relationships.length <= 4;
  }

  private isCompleteChain(path: RelationshipPath, context: ScoringContext): boolean {
    if (path.symbols.length < 3) return false;

    // Check if path forms a meaningful complete unit
    // For example: starts with controller, goes through business logic, ends at data layer
    const startSymbol = path.symbols[0];
    const endSymbol = path.symbols[path.symbols.length - 1];

    // Simple heuristic: complete chains often start and end in different files
    return startSymbol !== endSymbol && 
           this.extractFileFromSymbolId(startSymbol) !== this.extractFileFromSymbolId(endSymbol);
  }

  private isImportantPathPattern(path: RelationshipPath, context: ScoringContext): boolean {
    const queryLower = context.originalQuery?.toLowerCase() || '';

    // Check for patterns that match the query intent
    if (queryLower.includes('error') && path.pathType === 'error_propagation') {
      return true;
    }

    if (queryLower.includes('data') && path.pathType === 'data_flow_chain') {
      return true;
    }

    if (queryLower.includes('call') && path.pathType === 'call_chain') {
      return true;
    }

    return false;
  }

  private calculatePathSemanticRelevance(path: RelationshipPath, context: ScoringContext): number {
    const query = context.originalQuery?.toLowerCase() || '';
    let relevance = 0;

    // Check path description against query
    if (path.description.toLowerCase().includes(query)) {
      relevance += 0.5;
    }

    // Check if path symbols match query terms
    const queryTerms = query.split(/\s+/).filter(term => term.length > 2);
    const symbolNames = path.symbols.map(id => this.extractSymbolNameFromId(id)).join(' ').toLowerCase();
    
    const matchingTerms = queryTerms.filter(term => symbolNames.includes(term));
    relevance += (matchingTerms.length / queryTerms.length) * 0.3;

    return Math.min(0.5, relevance);
  }

  private getRelationshipTextRepresentation(relationship: CodeRelationship): string {
    const fromName = this.extractSymbolNameFromId(relationship.fromSymbol);
    const toName = this.extractSymbolNameFromId(relationship.toSymbol);
    return `${fromName} ${relationship.type} ${toName}`;
  }

  private extractFileFromSymbolId(symbolId: string): string {
    // Symbol ID format: "file:symbol:line"
    return symbolId.split(':')[0] || '';
  }

  private extractSymbolNameFromId(symbolId: string): string {
    // Symbol ID format: "file:symbol:line"
    const parts = symbolId.split(':');
    return parts[1] || parts[0] || '';
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

  // Public utility methods for external use

  getTopScoredRelationships(
    relationships: CodeRelationship[],
    context: ScoringContext,
    limit: number = 10
  ): CodeRelationship[] {
    return relationships
      .map(rel => ({
        ...rel,
        computedScore: this.scoreRelationship(rel, context)
      }))
      .sort((a, b) => (b as any).computedScore - (a as any).computedScore)
      .slice(0, limit);
  }

  getTopScoredSymbols(
    symbols: CodeSymbol[],
    context: ScoringContext,
    limit: number = 10
  ): CodeSymbol[] {
    return symbols
      .map(symbol => ({
        ...symbol,
        computedScore: this.scoreSymbol(symbol, context)
      }))
      .sort((a, b) => (b as any).computedScore - (a as any).computedScore)
      .slice(0, limit);
  }

  calculateOverallRelevance(
    symbols: CodeSymbol[],
    relationships: CodeRelationship[],
    paths: RelationshipPath[],
    context: ScoringContext
  ): number {
    if (symbols.length === 0 && relationships.length === 0 && paths.length === 0) {
      return 0;
    }

    const avgSymbolScore = symbols.length > 0 
      ? symbols.reduce((sum, s) => sum + this.scoreSymbol(s, context), 0) / symbols.length 
      : 0;

    const avgRelationshipScore = relationships.length > 0
      ? relationships.reduce((sum, r) => sum + this.scoreRelationship(r, context), 0) / relationships.length
      : 0;

    const avgPathScore = paths.length > 0
      ? paths.reduce((sum, p) => sum + this.scorePath(p, context), 0) / paths.length
      : 0;

    // Weighted combination
    const weights = { symbols: 0.3, relationships: 0.4, paths: 0.3 };
    return (avgSymbolScore * weights.symbols + 
            avgRelationshipScore * weights.relationships + 
            avgPathScore * weights.paths);
  }
}