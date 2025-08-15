import { CodeChunk, QueryRequest } from './types';
import { RelationshipTraversalEngine } from './relationship-traversal-engine';
import { VectorStore } from './vector-store';
import { log, warn } from './logging-utils';

export interface DependencyChain {
  seedChunks: CodeChunk[];           // Original search results
  forwardDependencies: CodeChunk[];  // What these functions call/use
  backwardDependencies: CodeChunk[]; // What calls/uses these functions
  criticalDependencies: CodeChunk[]; // Must-have context (types, interfaces)
  contextualDependencies: CodeChunk[]; // Nice-to-have context
  totalTokens: number;
  completenessScore: number;         // 0-1, how complete the context is
  relationshipPaths: RelationshipPath[];
}

export interface DependencyOptions {
  maxTokens: number;                 // Token budget constraint
  maxDepth: number;                  // Maximum traversal depth
  includeCallers: boolean;           // Include backward dependencies
  includeTypes: boolean;             // Include type definitions
  includeCriticalOnly: boolean;      // Only include critical deps
  reserveBuffer: number;             // Token safety buffer (default: 0.2)
}

export interface RelationshipPath {
  fromChunk: string;                 // Source chunk ID
  toChunk: string;                   // Target chunk ID
  relationshipType: string;          // Type of relationship
  depth: number;                     // Traversal depth
  importance: number;                // 0-1 importance score
}

export interface ContextCompletenessScore {
  overall: number;                   // Overall completeness 0-1
  forwardCoverage: number;           // How well we covered what X calls
  backwardCoverage: number;          // How well we covered what calls X
  typeCoverage: number;              // How well we covered type definitions
  criticalGaps: string[];            // Important missing dependencies
}

export class SmartDependencyTraverser {
  private relationshipEngine: RelationshipTraversalEngine;
  private vectorStore: VectorStore;

  constructor(relationshipEngine: RelationshipTraversalEngine, vectorStore: VectorStore) {
    this.relationshipEngine = relationshipEngine;
    this.vectorStore = vectorStore;
  }

  /**
   * Find complete dependency chain for given search results
   * Optimizes for context window efficiency - includes maximum relevant context within token budget
   */
  async findDependencyChain(
    seedChunks: CodeChunk[],
    query: QueryRequest,
    options: DependencyOptions
  ): Promise<DependencyChain> {
    const startTime = Date.now();
    
    log(`[SmartDependencyTraverser] Finding dependency chain for ${seedChunks.length} seeds, maxTokens=${options.maxTokens}`);

    // Calculate seed token cost
    const seedTokens = this.estimateTokens(seedChunks);
    const availableTokens = options.maxTokens - seedTokens - (options.maxTokens * options.reserveBuffer);

    if (availableTokens <= 0) {
      log(`[SmartDependencyTraverser] No tokens available for dependencies after seeds (${seedTokens} tokens)`);
      return {
        seedChunks,
        forwardDependencies: [],
        backwardDependencies: [],
        criticalDependencies: [],
        contextualDependencies: [],
        totalTokens: seedTokens,
        completenessScore: 0.5, // Partial context
        relationshipPaths: []
      };
    }

    // Extract symbols from seed chunks for relationship traversal
    const seedSymbols = this.extractSymbolsFromChunks(seedChunks);
    
    // Find critical dependencies first (types, interfaces, direct calls)
    const criticalDeps = await this.findCriticalDependencies(seedSymbols, availableTokens);
    const criticalTokens = this.estimateTokens(criticalDeps);
    
    // Find forward dependencies (what these functions call)
    const forwardDeps = await this.findForwardDependencies(
      seedSymbols, 
      availableTokens - criticalTokens,
      options.maxDepth
    );
    const forwardTokens = this.estimateTokens(forwardDeps);
    
    // Find backward dependencies (what calls these functions) if enabled
    let backwardDeps: CodeChunk[] = [];
    if (options.includeCallers) {
      const remainingTokens = availableTokens - criticalTokens - forwardTokens;
      if (remainingTokens > 0) {
        backwardDeps = await this.findBackwardDependencies(
          seedSymbols,
          remainingTokens,
          options.maxDepth
        );
      }
    }

    // Find additional contextual dependencies if we have token budget
    const usedTokens = criticalTokens + forwardTokens + this.estimateTokens(backwardDeps);
    let contextualDeps: CodeChunk[] = [];
    
    if (!options.includeCriticalOnly && usedTokens < availableTokens) {
      const remainingTokens = availableTokens - usedTokens;
      contextualDeps = await this.findContextualDependencies(
        seedSymbols,
        [...criticalDeps, ...forwardDeps, ...backwardDeps],
        remainingTokens
      );
    }

    // Build relationship paths for visualization/understanding
    const relationshipPaths = await this.buildRelationshipPaths(
      seedSymbols,
      [...criticalDeps, ...forwardDeps, ...backwardDeps, ...contextualDeps]
    );

    // Calculate completeness score
    const completenessScore = this.calculateCompletenessScore(
      seedSymbols,
      criticalDeps,
      forwardDeps,
      backwardDeps,
      relationshipPaths
    );

    const totalTokens = seedTokens + criticalTokens + forwardTokens + 
                       this.estimateTokens(backwardDeps) + this.estimateTokens(contextualDeps);

    const processingTime = Date.now() - startTime;
    
    log(`[SmartDependencyTraverser] Dependency chain complete: critical=${criticalDeps.length} forward=${forwardDeps.length} backward=${backwardDeps.length} contextual=${contextualDeps.length} totalTokens=${totalTokens} completeness=${(completenessScore * 100).toFixed(1)}% time=${processingTime}ms`);

    return {
      seedChunks,
      forwardDependencies: forwardDeps,
      backwardDependencies: backwardDeps,
      criticalDependencies: criticalDeps,
      contextualDependencies: contextualDeps,
      totalTokens,
      completenessScore,
      relationshipPaths
    };
  }

  /**
   * Find critical dependencies: types, interfaces, direct calls that are essential for understanding
   */
  private async findCriticalDependencies(
    seedSymbols: string[],
    availableTokens: number
  ): Promise<CodeChunk[]> {
    const criticalChunks: CodeChunk[] = [];
    let usedTokens = 0;

    for (const symbolId of seedSymbols) {
      // Find type definitions and interfaces
      const typeDeps = await this.relationshipEngine.findRelatedByType(
        symbolId, 
        ['extends', 'implements', 'instantiates'], 
        1 // depth 1 only for critical
      );

      // Find direct function calls
      const callDeps = await this.relationshipEngine.findRelatedByType(
        symbolId,
        ['calls'],
        1
      );

      // Convert to chunks and check token budget
      const allCritical = [...typeDeps, ...callDeps];
      for (const dep of allCritical) {
        const chunk = await this.vectorStore.getChunkBySymbol(dep.id);
        if (chunk) {
          const chunkTokens = this.estimateTokens([chunk]);
          if (usedTokens + chunkTokens <= availableTokens) {
            criticalChunks.push(chunk);
            usedTokens += chunkTokens;
          }
        }
      }
    }

    return this.deduplicateChunks(criticalChunks);
  }

  /**
   * Find forward dependencies: what these functions call/use
   */
  private async findForwardDependencies(
    seedSymbols: string[],
    availableTokens: number,
    maxDepth: number
  ): Promise<CodeChunk[]> {
    const forwardChunks: CodeChunk[] = [];
    let usedTokens = 0;

    for (const symbolId of seedSymbols) {
      const deps = await this.relationshipEngine.findRelatedByType(
        symbolId,
        ['calls', 'accesses', 'assigns', 'imports'],
        maxDepth
      );

      // Convert to chunks (relationship strength filtering happens at relationship level)
      const sortedDeps = deps; // Dependencies are already pre-filtered by the traversal engine
      
      for (const dep of sortedDeps) {
        const chunk = await this.vectorStore.getChunkBySymbol(dep.id);
        if (chunk) {
          const chunkTokens = this.estimateTokens([chunk]);
          if (usedTokens + chunkTokens <= availableTokens) {
            forwardChunks.push(chunk);
            usedTokens += chunkTokens;
          } else {
            break; // Token budget exhausted
          }
        }
      }
    }

    return this.deduplicateChunks(forwardChunks);
  }

  /**
   * Find backward dependencies: what calls/uses these functions
   */
  private async findBackwardDependencies(
    seedSymbols: string[],
    availableTokens: number,
    maxDepth: number
  ): Promise<CodeChunk[]> {
    const backwardChunks: CodeChunk[] = [];
    let usedTokens = 0;

    for (const symbolId of seedSymbols) {
      // Find what calls this symbol (reverse lookup)
      const callers = await this.relationshipEngine.findCallers(symbolId, maxDepth);
      
      // Callers are already ordered by the traversal engine
      const sortedCallers = callers;
      
      for (const caller of sortedCallers) {
        const chunk = await this.vectorStore.getChunkBySymbol(caller.id);
        if (chunk) {
          const chunkTokens = this.estimateTokens([chunk]);
          if (usedTokens + chunkTokens <= availableTokens) {
            backwardChunks.push(chunk);
            usedTokens += chunkTokens;
          } else {
            break;
          }
        }
      }
    }

    return this.deduplicateChunks(backwardChunks);
  }

  /**
   * Find contextual dependencies: co-change patterns, similar functionality
   */
  private async findContextualDependencies(
    seedSymbols: string[],
    existingDeps: CodeChunk[],
    availableTokens: number
  ): Promise<CodeChunk[]> {
    const contextualChunks: CodeChunk[] = [];
    let usedTokens = 0;
    const existingChunkIds = new Set(existingDeps.map(c => c.chunk_id));

    for (const symbolId of seedSymbols) {
      // Find co-change patterns and similar functionality
      const coChangeDeps = await this.relationshipEngine.findRelatedByType(
        symbolId,
        ['configures', 'depends_on', 'data_flow'],
        2
      );

      for (const dep of coChangeDeps) {
        const chunk = await this.vectorStore.getChunkBySymbol(dep.id);
        if (chunk && !existingChunkIds.has(chunk.chunk_id)) {
          const chunkTokens = this.estimateTokens([chunk]);
          if (usedTokens + chunkTokens <= availableTokens) {
            contextualChunks.push(chunk);
            usedTokens += chunkTokens;
            existingChunkIds.add(chunk.chunk_id);
          } else {
            break;
          }
        }
      }
    }

    return contextualChunks;
  }

  /**
   * Build relationship paths for visualization and understanding
   */
  private async buildRelationshipPaths(
    seedSymbols: string[],
    dependencyChunks: CodeChunk[]
  ): Promise<RelationshipPath[]> {
    const paths: RelationshipPath[] = [];
    
    for (const seedSymbol of seedSymbols) {
      for (const depChunk of dependencyChunks) {
        const relationship = await this.relationshipEngine.findDirectRelationship(
          seedSymbol,
          depChunk.symbol_name || depChunk.chunk_id
        );
        
        if (relationship) {
          paths.push({
            fromChunk: seedSymbol,
            toChunk: depChunk.chunk_id,
            relationshipType: relationship.type,
            depth: 1, // For now, calculate actual depth later
            importance: relationship.strength || 0.5
          });
        }
      }
    }

    return paths;
  }

  /**
   * Calculate how complete our dependency context is
   */
  private calculateCompletenessScore(
    seedSymbols: string[],
    criticalDeps: CodeChunk[],
    forwardDeps: CodeChunk[],
    backwardDeps: CodeChunk[],
    relationshipPaths: RelationshipPath[]
  ): number {
    // Simple heuristic - can be made more sophisticated
    const criticalScore = criticalDeps.length > 0 ? 0.4 : 0;
    const forwardScore = forwardDeps.length > 0 ? 0.3 : 0;
    const backwardScore = backwardDeps.length > 0 ? 0.2 : 0;
    const relationshipScore = relationshipPaths.length > 0 ? 0.1 : 0;
    
    return Math.min(1.0, criticalScore + forwardScore + backwardScore + relationshipScore);
  }

  /**
   * Extract symbol IDs from code chunks
   */
  private extractSymbolsFromChunks(chunks: CodeChunk[]): string[] {
    return chunks
      .map(chunk => chunk.symbol_name || chunk.chunk_id)
      .filter(symbol => symbol);
  }

  /**
   * Estimate token count for chunks (rough approximation)
   */
  private estimateTokens(chunks: CodeChunk[]): number {
    return chunks.reduce((total, chunk) => {
      // Rough estimate: 1 token per 4 characters
      return total + Math.ceil(chunk.content.length / 4);
    }, 0);
  }

  /**
   * Remove duplicate chunks based on chunk_id
   */
  private deduplicateChunks(chunks: CodeChunk[]): CodeChunk[] {
    const seen = new Set<string>();
    return chunks.filter(chunk => {
      if (seen.has(chunk.chunk_id)) {
        return false;
      }
      seen.add(chunk.chunk_id);
      return true;
    });
  }
}