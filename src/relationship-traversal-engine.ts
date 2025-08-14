import {
  CodeSymbol,
  CodeRelationship,
  RelationshipGraph,
  RelationshipType,
  TraversalOptions,
  TraversalResult,
  RelationshipPath,
  PathType,
  TraversalStatistics,
  RelationshipQuery,
  RelationshipSearchResult,
  ContextGroup
} from './relationship-types';
import { CallGraphAnalyzer } from './call-graph-analyzer';
import { DependencyMapper } from './dependency-mapper';
import { DataFlowAnalyzer } from './data-flow-analyzer';
import { PersistentRelationshipStore } from './persistent-relationship-store';
import Parser from 'tree-sitter';
import JavaScript from 'tree-sitter-javascript';
import TypeScript from 'tree-sitter-typescript';
import * as path from 'path';

export class RelationshipTraversalEngine {
  private graph: RelationshipGraph;
  private callGraphAnalyzer: CallGraphAnalyzer;
  private dependencyMapper: DependencyMapper;
  private dataFlowAnalyzer: DataFlowAnalyzer;
  private jsParser: Parser;
  private tsParser: Parser;
  private repositoryPath: string;
  private persistentStore: PersistentRelationshipStore;

  constructor(repositoryPath: string) {
    this.repositoryPath = repositoryPath;
    this.callGraphAnalyzer = new CallGraphAnalyzer();
    this.dependencyMapper = new DependencyMapper(repositoryPath);
    this.dataFlowAnalyzer = new DataFlowAnalyzer();
    this.persistentStore = new PersistentRelationshipStore(repositoryPath);
    
    this.graph = {
      symbols: new Map(),
      relationships: new Map(),
      symbolsByFile: new Map(),
      symbolsByType: new Map(),
      relationshipsByType: new Map(),
      outgoingRelationships: new Map(),
      incomingRelationships: new Map()
    };
    
    // Initialize parsers
    this.jsParser = new Parser();
    this.jsParser.setLanguage(JavaScript);
    
    this.tsParser = new Parser();
    this.tsParser.setLanguage(TypeScript.typescript);
  }

  async buildRelationshipGraph(files: Map<string, string>): Promise<void> {
    console.log(`🔗 Building comprehensive relationship graph for ${files.size} files...`);
    const startTime = Date.now();

    // Initialize persistent store
    await this.persistentStore.initialize();

    // Try to load cached relationship graph (prefer global, fallback to local)
    let loadedFromCache = false;
    if (await this.persistentStore.globalRelationshipGraphExists()) {
      console.log('🌐 Loading relationship graph from global storage...');
      const cachedGraph = await this.persistentStore.loadPersistedRelationshipGraph(true);
      if (cachedGraph) {
        this.graph = cachedGraph;
        loadedFromCache = true;
        
        // Sync to local if needed
        await this.persistentStore.syncToLocal();
      }
    } else if (await this.persistentStore.relationshipGraphExists()) {
      console.log('📁 Loading relationship graph from local storage...');
      const cachedGraph = await this.persistentStore.loadPersistedRelationshipGraph(false);
      if (cachedGraph) {
        this.graph = cachedGraph;
        loadedFromCache = true;
        
        // Sync to global
        await this.persistentStore.syncToGlobal();
      }
    }

    // If no cache available or cache is invalid, build from scratch
    if (!loadedFromCache) {
      console.log('🔄 Building relationship graph from scratch...');
      
      // Phase 1: Build dependency map
      await this.dependencyMapper.buildDependencyMap(files);

      // Phase 2: Analyze individual files for call graphs and data flow
      const allSymbols: CodeSymbol[] = [];
      const allRelationships: CodeRelationship[] = [];

      for (const [filePath, content] of files) {
        try {
          // Call graph analysis
          const callAnalysis = await this.callGraphAnalyzer.analyzeFile(filePath, content);
          allSymbols.push(...callAnalysis.symbols);
          allRelationships.push(...callAnalysis.relationships);

          // Data flow analysis
          const tree = this.parseFile(content, filePath);
          if (tree) {
            const dataFlowAnalysis = this.dataFlowAnalyzer.analyzeDataFlow(tree, content, filePath);
            allRelationships.push(...dataFlowAnalysis.relationships);
          }

        } catch (error) {
          console.warn(`Failed to analyze relationships in ${filePath}:`, error);
        }
      }

      // Phase 3: Add dependency relationships
      const dependencyRelationships = this.dependencyMapper.generateDependencyRelationships();
      allRelationships.push(...dependencyRelationships);

      // Phase 4: Build graph structure
      this.populateGraph(allSymbols, allRelationships);

      // Phase 5: Build indices for fast lookup
      this.buildGraphIndices();

      // Phase 6: Save to persistent storage
      await this.persistentStore.savePersistedRelationshipGraph(this.graph);
    }

    const timeMs = Date.now() - startTime;
    const action = loadedFromCache ? 'loaded from cache' : 'built from scratch';
    console.log(`✅ Relationship graph ${action} in ${timeMs}ms: ${this.graph.symbols.size} symbols, ${this.graph.relationships.size} relationships`);
  }

  private parseFile(content: string, filePath: string): Parser.Tree | null {
    try {
      const ext = path.extname(filePath).toLowerCase();
      
      // Select appropriate parser based on file extension
      let parser: Parser;
      if (ext === '.ts' || ext === '.tsx') {
        parser = this.tsParser;
      } else if (ext === '.js' || ext === '.jsx' || ext === '.mjs') {
        parser = this.jsParser;
      } else {
        // Unsupported file type
        return null;
      }
      
      // Parse the content
      const tree = parser.parse(content);
      
      // Verify parse was successful
      if (tree.rootNode.hasError) {
        console.warn(`⚠️ Parse errors in ${filePath}, partial AST analysis may be incomplete`);
      }
      
      return tree;
    } catch (error) {
      console.warn(`❌ Failed to parse ${filePath}:`, error instanceof Error ? error.message : error);
      return null;
    }
  }

  private populateGraph(symbols: CodeSymbol[], relationships: CodeRelationship[]): void {
    // Add symbols
    symbols.forEach(symbol => {
      this.graph.symbols.set(symbol.id, symbol);
    });

    // Add relationships
    relationships.forEach(relationship => {
      this.graph.relationships.set(relationship.id, relationship);
    });
  }

  private buildGraphIndices(): void {
    // Build symbol indices
    for (const symbol of this.graph.symbols.values()) {
      // Index by file
      if (!this.graph.symbolsByFile.has(symbol.filePath)) {
        this.graph.symbolsByFile.set(symbol.filePath, new Set());
      }
      this.graph.symbolsByFile.get(symbol.filePath)!.add(symbol.id);

      // Index by type
      if (!this.graph.symbolsByType.has(symbol.type)) {
        this.graph.symbolsByType.set(symbol.type, new Set());
      }
      this.graph.symbolsByType.get(symbol.type)!.add(symbol.id);
    }

    // Build relationship indices
    for (const relationship of this.graph.relationships.values()) {
      // Index by type
      if (!this.graph.relationshipsByType.has(relationship.type)) {
        this.graph.relationshipsByType.set(relationship.type, new Set());
      }
      this.graph.relationshipsByType.get(relationship.type)!.add(relationship.id);

      // Build adjacency lists
      const fromSymbol = relationship.fromSymbol;
      const toSymbol = relationship.toSymbol;

      // Outgoing relationships
      if (!this.graph.outgoingRelationships.has(fromSymbol)) {
        this.graph.outgoingRelationships.set(fromSymbol, new Set());
      }
      this.graph.outgoingRelationships.get(fromSymbol)!.add(relationship.id);

      // Incoming relationships
      if (!this.graph.incomingRelationships.has(toSymbol)) {
        this.graph.incomingRelationships.set(toSymbol, new Set());
      }
      this.graph.incomingRelationships.get(toSymbol)!.add(relationship.id);
    }
  }

  async traverseRelationships(
    startSymbolIds: string[],
    options: TraversalOptions
  ): Promise<TraversalResult> {
    const startTime = Date.now();
    const discoveredSymbols = new Map<string, CodeSymbol>();
    const traversedRelationships = new Map<string, CodeRelationship>();
    const allPaths: RelationshipPath[] = [];
    const visited = new Set<string>();

    console.log(`🚶 Traversing relationships from ${startSymbolIds.length} starting symbols...`);

    // Start traversal from each starting symbol
    for (const startSymbolId of startSymbolIds) {
      const startSymbol = this.graph.symbols.get(startSymbolId);
      if (!startSymbol) {
        console.warn(`Start symbol not found: ${startSymbolId}`);
        continue;
      }

      discoveredSymbols.set(startSymbolId, startSymbol);

      // Perform traversal
      const paths = await this.performTraversal(
        startSymbolId,
        options,
        visited,
        discoveredSymbols,
        traversedRelationships
      );

      allPaths.push(...paths);
    }

    // Generate statistics
    const statistics = this.generateTraversalStatistics(
      allPaths,
      discoveredSymbols,
      traversedRelationships,
      Date.now() - startTime
    );

    const result: TraversalResult = {
      startSymbol: this.graph.symbols.get(startSymbolIds[0])!,
      discoveredSymbols,
      traversedRelationships,
      paths: allPaths,
      statistics
    };

    console.log(`✅ Traversal completed: ${discoveredSymbols.size} symbols, ${traversedRelationships.size} relationships, ${allPaths.length} paths`);

    return result;
  }

  private async performTraversal(
    currentSymbolId: string,
    options: TraversalOptions,
    visited: Set<string>,
    discoveredSymbols: Map<string, CodeSymbol>,
    traversedRelationships: Map<string, CodeRelationship>,
    currentPath: string[] = [],
    currentDepth: number = 0
  ): Promise<RelationshipPath[]> {
    
    if (currentDepth >= options.maxDepth || visited.has(currentSymbolId)) {
      return [];
    }

    visited.add(currentSymbolId);
    const paths: RelationshipPath[] = [];

    // Get relationships from current symbol
    const outgoingRelIds = this.graph.outgoingRelationships.get(currentSymbolId) || new Set();
    const incomingRelIds = this.graph.incomingRelationships.get(currentSymbolId) || new Set();

    let relationshipsToTraverse: Set<string> = new Set();

    // Determine which relationships to follow based on direction
    if (options.direction === 'forward' || options.direction === 'both') {
      relationshipsToTraverse = new Set([...relationshipsToTraverse, ...outgoingRelIds]);
    }
    if (options.direction === 'backward' || options.direction === 'both') {
      relationshipsToTraverse = new Set([...relationshipsToTraverse, ...incomingRelIds]);
    }

    // Filter relationships by type and strength
    const filteredRelationships = Array.from(relationshipsToTraverse)
      .map(relId => this.graph.relationships.get(relId)!)
      .filter(rel => rel && this.shouldTraverseRelationship(rel, options));

    // Apply pruning strategy
    const prunedRelationships = this.applyPruningStrategy(filteredRelationships, options);

    // Traverse each valid relationship
    for (const relationship of prunedRelationships) {
      traversedRelationships.set(relationship.id, relationship);

      // Determine next symbol
      const nextSymbolId = relationship.fromSymbol === currentSymbolId 
        ? relationship.toSymbol 
        : relationship.fromSymbol;

      const nextSymbol = this.graph.symbols.get(nextSymbolId);
      if (!nextSymbol) continue;

      discoveredSymbols.set(nextSymbolId, nextSymbol);

      // Create path for this step
      const stepPath: RelationshipPath = {
        symbols: [...currentPath, currentSymbolId, nextSymbolId],
        relationships: [relationship.id],
        totalStrength: relationship.strength,
        pathType: this.determinePathType([relationship]),
        depth: currentDepth + 1,
        description: this.generatePathDescription([currentSymbolId, nextSymbolId], [relationship])
      };

      paths.push(stepPath);

      // Continue traversal if not at max depth
      if (currentDepth + 1 < options.maxDepth) {
        const subPaths = await this.performTraversal(
          nextSymbolId,
          options,
          new Set(visited), // Create new visited set for each branch
          discoveredSymbols,
          traversedRelationships,
          [...currentPath, currentSymbolId],
          currentDepth + 1
        );

        // Extend current path with sub-paths
        for (const subPath of subPaths) {
          const extendedPath: RelationshipPath = {
            symbols: [currentSymbolId, ...subPath.symbols],
            relationships: [relationship.id, ...subPath.relationships],
            totalStrength: (relationship.strength + subPath.totalStrength) / 2,
            pathType: this.determinePathType([relationship, ...subPath.relationships.map(id => this.graph.relationships.get(id)!)]),
            depth: currentDepth + subPath.depth + 1,
            description: this.generatePathDescription(
              [currentSymbolId, ...subPath.symbols],
              [relationship, ...subPath.relationships.map(id => this.graph.relationships.get(id)!)]
            )
          };

          paths.push(extendedPath);
        }
      }
    }

    visited.delete(currentSymbolId);
    return paths;
  }

  private shouldTraverseRelationship(relationship: CodeRelationship, options: TraversalOptions): boolean {
    // Check relationship type filter
    if (options.relationshipTypes.length > 0 && 
        !options.relationshipTypes.includes(relationship.type)) {
      return false;
    }

    // Check strength threshold
    if (relationship.strength < options.minStrength) {
      return false;
    }

    // Check confidence threshold
    if (relationship.confidence < options.minConfidence) {
      return false;
    }

    return true;
  }

  private applyPruningStrategy(
    relationships: CodeRelationship[],
    options: TraversalOptions
  ): CodeRelationship[] {
    switch (options.pruneStrategy) {
      case 'strength':
        return relationships
          .sort((a, b) => b.strength - a.strength)
          .slice(0, options.maxResults || relationships.length);

      case 'relevance':
        // Could implement relevance scoring based on query context
        return relationships
          .sort((a, b) => (b.strength * b.confidence) - (a.strength * a.confidence))
          .slice(0, options.maxResults || relationships.length);

      case 'frequency':
        return relationships
          .sort((a, b) => (b.metadata?.frequency || 0) - (a.metadata?.frequency || 0))
          .slice(0, options.maxResults || relationships.length);

      case 'none':
      default:
        return relationships.slice(0, options.maxResults || relationships.length);
    }
  }

  private determinePathType(relationships: CodeRelationship[]): PathType {
    if (relationships.length === 0) return 'mixed';

    const types = relationships.map(r => r.type);
    const uniqueTypes = new Set(types);

    // Check for specific patterns
    if (uniqueTypes.size === 1) {
      const type = types[0];
      if (type === 'calls') return 'call_chain';
      if (type === 'imports' || type === 'depends_on') return 'dependency_chain';
      if (type === 'data_flow') return 'data_flow_chain';
      if (type === 'throws' || type === 'catches') return 'error_propagation';
      if (type === 'extends' || type === 'implements') return 'inheritance_chain';
    }

    return 'mixed';
  }

  private generatePathDescription(symbolIds: string[], relationships: CodeRelationship[]): string {
    if (symbolIds.length < 2) return '';

    const symbols = symbolIds.map(id => this.graph.symbols.get(id)).filter(Boolean) as CodeSymbol[];
    
    if (symbols.length === 2) {
      const rel = relationships[0];
      const verb = this.getRelationshipVerb(rel.type);
      return `${symbols[0].name} ${verb} ${symbols[1].name}`;
    }

    const pathType = this.determinePathType(relationships);
    switch (pathType) {
      case 'call_chain':
        return `Call chain: ${symbols.map(s => s.name).join(' → ')}`;
      case 'dependency_chain':
        return `Dependency: ${symbols.map(s => s.name).join(' → ')}`;
      case 'data_flow_chain':
        return `Data flows: ${symbols.map(s => s.name).join(' → ')}`;
      case 'error_propagation':
        return `Error propagation: ${symbols.map(s => s.name).join(' → ')}`;
      default:
        return `${symbols[0].name} → ... → ${symbols[symbols.length - 1].name} (${symbols.length - 1} hops)`;
    }
  }

  private getRelationshipVerb(type: RelationshipType): string {
    const verbMap: Record<RelationshipType, string> = {
      calls: 'calls',
      imports: 'imports',
      exports: 'exports',
      extends: 'extends',
      implements: 'implements',
      instantiates: 'creates',
      accesses: 'accesses',
      assigns: 'assigns to',
      throws: 'throws',
      catches: 'catches',
      configures: 'configures',
      depends_on: 'depends on',
      data_flow: 'flows to'
    };

    return verbMap[type] || 'relates to';
  }

  private generateTraversalStatistics(
    paths: RelationshipPath[],
    symbols: Map<string, CodeSymbol>,
    relationships: Map<string, CodeRelationship>,
    traversalTimeMs: number
  ): TraversalStatistics {
    
    const relationshipTypeDistribution = new Map<RelationshipType, number>();
    let maxDepth = 0;
    let totalPathLength = 0;
    let strongestPath = paths[0];

    for (const path of paths) {
      maxDepth = Math.max(maxDepth, path.depth);
      totalPathLength += path.symbols.length;

      if (!strongestPath || path.totalStrength > strongestPath.totalStrength) {
        strongestPath = path;
      }

      // Count relationship types
      for (const relId of path.relationships) {
        const rel = relationships.get(relId);
        if (rel) {
          const count = relationshipTypeDistribution.get(rel.type) || 0;
          relationshipTypeDistribution.set(rel.type, count + 1);
        }
      }
    }

    return {
      totalSymbolsVisited: symbols.size,
      totalRelationshipsTraversed: relationships.size,
      maxDepthReached: maxDepth,
      relationshipTypeDistribution,
      averagePathLength: paths.length > 0 ? totalPathLength / paths.length : 0,
      strongestPath: strongestPath || {
        symbols: [],
        relationships: [],
        totalStrength: 0,
        pathType: 'mixed',
        depth: 0,
        description: ''
      },
      traversalTimeMs
    };
  }

  // Public query interface
  async executeRelationshipQuery(query: RelationshipQuery): Promise<RelationshipSearchResult> {
    console.log(`🔍 Executing relationship query: "${query.baseQuery}"`);

    // Find initial symbols based on base query
    const startingSymbols = await this.findSymbolsForQuery(query.baseQuery, query.focusSymbols);

    if (startingSymbols.length === 0) {
      return {
        primaryChunks: [],
        relatedChunks: [],
        relationshipPaths: [],
        contextGroups: [],
        totalTokens: 0,
        efficiencyScore: 0
      };
    }

    // Perform relationship traversal
    const traversalResult = await this.traverseRelationships(
      startingSymbols.map(s => s.id),
      query.traversalOptions
    );

    // Generate context groups
    const contextGroups = this.generateContextGroups(
      traversalResult,
      query.baseQuery,
      query.includeContext,
      query.contextRadius
    );

    // Extract chunk IDs
    const primaryChunks = startingSymbols
      .map(s => s.chunkId)
      .filter(Boolean) as string[];

    const relatedChunks = Array.from(traversalResult.discoveredSymbols.values())
      .map(s => s.chunkId)
      .filter((id): id is string => Boolean(id))
      .filter(id => !primaryChunks.includes(id));

    // Calculate efficiency metrics
    const totalTokens = this.estimateTokenCount(contextGroups);
    const efficiencyScore = this.calculateEfficiencyScore(
      traversalResult.paths,
      contextGroups,
      query.baseQuery
    );

    return {
      primaryChunks,
      relatedChunks,
      relationshipPaths: traversalResult.paths,
      contextGroups,
      totalTokens,
      efficiencyScore
    };
  }

  private async findSymbolsForQuery(baseQuery: string, focusSymbols?: string[]): Promise<CodeSymbol[]> {
    const symbols: CodeSymbol[] = [];

    // If focus symbols are specified, use those
    if (focusSymbols && focusSymbols.length > 0) {
      for (const symbolName of focusSymbols) {
        for (const symbol of this.graph.symbols.values()) {
          if (symbol.name.includes(symbolName)) {
            symbols.push(symbol);
          }
        }
      }
      return symbols;
    }

    // Otherwise, use simple keyword matching (in real implementation, use semantic search)
    const queryLower = baseQuery.toLowerCase();
    const keywords = queryLower.split(/\s+/).filter(word => word.length > 2);

    for (const symbol of this.graph.symbols.values()) {
      const symbolText = `${symbol.name} ${symbol.filePath}`.toLowerCase();
      
      if (keywords.some(keyword => symbolText.includes(keyword))) {
        symbols.push(symbol);
      }
    }

    return symbols.slice(0, 10); // Limit to prevent excessive traversal
  }

  private generateContextGroups(
    traversalResult: TraversalResult,
    baseQuery: string,
    includeContext: boolean,
    contextRadius: number
  ): ContextGroup[] {
    const groups: ContextGroup[] = [];

    // Group by relationship path types
    const pathsByType = new Map<PathType, RelationshipPath[]>();
    
    for (const path of traversalResult.paths) {
      if (!pathsByType.has(path.pathType)) {
        pathsByType.set(path.pathType, []);
      }
      pathsByType.get(path.pathType)!.push(path);
    }

    // Create context groups
    for (const [pathType, paths] of pathsByType) {
      const symbolIds = new Set<string>();
      const relationshipIds = new Set<string>();

      // Collect symbols and relationships from paths
      for (const path of paths) {
        path.symbols.forEach(id => symbolIds.add(id));
        path.relationships.forEach(id => relationshipIds.add(id));
      }

      const symbols = Array.from(symbolIds)
        .map(id => this.graph.symbols.get(id))
        .filter(Boolean) as CodeSymbol[];

      const relationships = Array.from(relationshipIds)
        .map(id => this.graph.relationships.get(id))
        .filter(Boolean) as CodeRelationship[];

      const chunkIds = symbols
        .map(s => s.chunkId)
        .filter(Boolean) as string[];

      const group: ContextGroup = {
        theme: this.getThemeForPathType(pathType),
        symbols,
        relationships,
        chunkIds: [...new Set(chunkIds)], // Remove duplicates
        importance: this.calculateGroupImportance(paths, baseQuery),
        tokenCount: this.estimateGroupTokens(symbols, relationships, includeContext, contextRadius)
      };

      groups.push(group);
    }

    // Sort by importance
    return groups.sort((a, b) => b.importance - a.importance);
  }

  private getThemeForPathType(pathType: PathType): string {
    const themeMap: Record<PathType, string> = {
      call_chain: 'Function Calls',
      dependency_chain: 'Module Dependencies',
      data_flow_chain: 'Data Flow',
      error_propagation: 'Error Handling',
      configuration_chain: 'Configuration',
      inheritance_chain: 'Class Inheritance',
      mixed: 'Related Code'
    };

    return themeMap[pathType] || 'Related Code';
  }

  private calculateGroupImportance(paths: RelationshipPath[], baseQuery: string): number {
    let importance = 0;

    for (const path of paths) {
      // Base importance on path strength and depth
      importance += path.totalStrength * (1 / Math.max(path.depth, 1));
    }

    return importance / paths.length;
  }

  private estimateGroupTokens(
    symbols: CodeSymbol[],
    relationships: CodeRelationship[],
    includeContext: boolean,
    contextRadius: number
  ): number {
    let tokens = 0;

    // Estimate tokens for symbols (rough approximation)
    symbols.forEach(symbol => {
      const lines = symbol.endLine - symbol.startLine + 1;
      tokens += lines * 15; // Rough estimate: 15 tokens per line
      
      if (includeContext) {
        tokens += contextRadius * 2 * 15; // Context lines around symbol
      }
    });

    return tokens;
  }

  private estimateTokenCount(contextGroups: ContextGroup[]): number {
    return contextGroups.reduce((total, group) => total + group.tokenCount, 0);
  }

  private calculateEfficiencyScore(
    paths: RelationshipPath[],
    contextGroups: ContextGroup[],
    baseQuery: string
  ): number {
    if (paths.length === 0) return 0;

    // Calculate relevance vs token cost
    const averagePathStrength = paths.reduce((sum, path) => sum + path.totalStrength, 0) / paths.length;
    const totalTokens = this.estimateTokenCount(contextGroups);
    
    // Efficiency = relevance / token_cost (normalized)
    const efficiency = averagePathStrength / Math.max(totalTokens / 1000, 1);
    
    return Math.min(efficiency, 1.0);
  }

  // Public getters
  getGraph(): RelationshipGraph {
    return this.graph;
  }

  getSymbolsByFile(filePath: string): CodeSymbol[] {
    const symbolIds = this.graph.symbolsByFile.get(filePath) || new Set();
    return Array.from(symbolIds).map(id => this.graph.symbols.get(id)!).filter(Boolean);
  }

  getRelationshipsByType(type: RelationshipType): CodeRelationship[] {
    const relationshipIds = this.graph.relationshipsByType.get(type) || new Set();
    return Array.from(relationshipIds).map(id => this.graph.relationships.get(id)!).filter(Boolean);
  }

  getRelatedSymbols(symbolId: string, direction: 'incoming' | 'outgoing' | 'both' = 'both'): CodeSymbol[] {
    const relatedIds = new Set<string>();

    if (direction === 'outgoing' || direction === 'both') {
      const outgoing = this.graph.outgoingRelationships.get(symbolId) || new Set();
      for (const relId of outgoing) {
        const rel = this.graph.relationships.get(relId);
        if (rel) relatedIds.add(rel.toSymbol);
      }
    }

    if (direction === 'incoming' || direction === 'both') {
      const incoming = this.graph.incomingRelationships.get(symbolId) || new Set();
      for (const relId of incoming) {
        const rel = this.graph.relationships.get(relId);
        if (rel) relatedIds.add(rel.fromSymbol);
      }
    }

    return Array.from(relatedIds).map(id => this.graph.symbols.get(id)!).filter(Boolean);
  }

  getGraphStatistics() {
    return {
      totalSymbols: this.graph.symbols.size,
      totalRelationships: this.graph.relationships.size,
      symbolTypeDistribution: this.getSymbolTypeDistribution(),
      relationshipTypeDistribution: this.getRelationshipTypeDistribution(),
      averageSymbolConnections: this.calculateAverageConnections(),
      dependencyStats: this.dependencyMapper.getModuleStats()
    };
  }

  private getSymbolTypeDistribution(): Map<string, number> {
    const distribution = new Map<string, number>();
    
    for (const symbol of this.graph.symbols.values()) {
      const count = distribution.get(symbol.type) || 0;
      distribution.set(symbol.type, count + 1);
    }
    
    return distribution;
  }

  private getRelationshipTypeDistribution(): Map<RelationshipType, number> {
    const distribution = new Map<RelationshipType, number>();
    
    for (const relationship of this.graph.relationships.values()) {
      const count = distribution.get(relationship.type) || 0;
      distribution.set(relationship.type, count + 1);
    }
    
    return distribution;
  }

  private calculateAverageConnections(): number {
    let totalConnections = 0;
    
    for (const symbolId of this.graph.symbols.keys()) {
      const outgoing = this.graph.outgoingRelationships.get(symbolId)?.size || 0;
      const incoming = this.graph.incomingRelationships.get(symbolId)?.size || 0;
      totalConnections += outgoing + incoming;
    }
    
    return this.graph.symbols.size > 0 ? totalConnections / this.graph.symbols.size : 0;
  }
}