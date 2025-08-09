// Relationship graph types and interfaces for advanced code traversal

export interface CodeSymbol {
  id: string;                    // Unique identifier: "file:function:line"
  name: string;                  // Symbol name (function, class, variable)
  type: 'function' | 'class' | 'variable' | 'module' | 'method' | 'property';
  filePath: string;             // Source file path
  startLine: number;            // Starting line number
  endLine: number;              // Ending line number
  signature?: string;           // Function signature or type annotation
  scope: 'global' | 'local' | 'class' | 'module';
  chunkId?: string;             // Associated code chunk ID
}

export interface CodeRelationship {
  id: string;                   // Unique relationship ID
  fromSymbol: string;           // Source symbol ID
  toSymbol: string;             // Target symbol ID
  type: RelationshipType;
  strength: number;             // Relationship strength (0-1)
  confidence: number;           // Analysis confidence (0-1)
  metadata?: RelationshipMetadata;
}

export type RelationshipType = 
  | 'calls'                     // Function A calls Function B
  | 'imports'                   // Module A imports Module B
  | 'exports'                   // Module A exports Symbol B
  | 'extends'                   // Class A extends Class B
  | 'implements'                // Class A implements Interface B
  | 'instantiates'              // Function A creates instance of Class B
  | 'accesses'                  // Function A accesses Property B
  | 'assigns'                   // Function A assigns to Variable B
  | 'throws'                    // Function A throws Exception B
  | 'catches'                   // Function A catches Exception B
  | 'configures'                // Function A configures Module B
  | 'depends_on'                // Generic dependency relationship
  | 'data_flow';                // Variable flows from A to B

export interface RelationshipMetadata {
  // Call relationships
  callType?: 'direct' | 'indirect' | 'async' | 'callback';
  parameters?: string[];        // Parameter types passed
  returnType?: string;          // Return type received
  
  // Import relationships  
  importType?: 'default' | 'named' | 'namespace' | 'dynamic';
  importedSymbols?: string[];   // Specific symbols imported
  
  // Data flow relationships
  flowType?: 'assignment' | 'parameter' | 'return' | 'property' | 'parameter_passing' | 'return_value' | 'property_access' | 'array_access' | 'destructuring' | 'spread' | 'closure_capture' | 'callback_parameter' | 'promise_resolution' | 'conditional_flow';
  transformations?: string[];   // Data transformations applied
  
  // Error relationships
  errorType?: string;           // Exception type
  handlingStrategy?: 'try_catch' | 'promise_catch' | 'callback_error';
  
  // Configuration relationships
  configType?: 'environment' | 'runtime' | 'build_time';
  configPath?: string[];        // Configuration key path
  
  // General metadata
  frequency?: number;           // How often this relationship occurs
  conditions?: string[];        // Conditions under which relationship exists
  sourceLocation?: {            // Where in source this relationship is defined
    line: number;
    column: number;
  };
  queryEmbedding?: number[];    // Query embedding for similarity scoring
}

export interface RelationshipGraph {
  symbols: Map<string, CodeSymbol>;
  relationships: Map<string, CodeRelationship>;
  
  // Index for fast lookups
  symbolsByFile: Map<string, Set<string>>;
  symbolsByType: Map<string, Set<string>>;
  relationshipsByType: Map<RelationshipType, Set<string>>;
  
  // Adjacency lists for graph traversal
  outgoingRelationships: Map<string, Set<string>>;  // symbol -> relationships going out
  incomingRelationships: Map<string, Set<string>>;  // symbol -> relationships coming in
}

export interface TraversalOptions {
  maxDepth: number;             // Maximum traversal depth
  relationshipTypes: RelationshipType[];  // Types of relationships to follow
  direction: 'forward' | 'backward' | 'both';  // Traversal direction
  minStrength: number;          // Minimum relationship strength threshold
  minConfidence: number;        // Minimum confidence threshold
  includeTransitive: boolean;   // Include indirect relationships
  pruneStrategy: 'none' | 'strength' | 'relevance' | 'frequency';
  maxResults?: number;          // Limit number of results
}

export interface TraversalResult {
  startSymbol: CodeSymbol;
  discoveredSymbols: Map<string, CodeSymbol>;
  traversedRelationships: Map<string, CodeRelationship>;
  paths: RelationshipPath[];
  statistics: TraversalStatistics;
}

export interface RelationshipPath {
  symbols: string[];            // Ordered list of symbol IDs in path
  relationships: string[];      // Ordered list of relationship IDs
  totalStrength: number;        // Combined strength of all relationships
  pathType: PathType;
  depth: number;
  description: string;          // Human-readable path description
}

export type PathType = 
  | 'call_chain'                // A -> calls -> B -> calls -> C
  | 'dependency_chain'          // A -> imports -> B -> imports -> C  
  | 'data_flow_chain'           // Variable flows A -> B -> C
  | 'error_propagation'         // Error thrown A -> caught B -> rethrown C
  | 'configuration_chain'       // Config A -> affects B -> configures C
  | 'inheritance_chain'         // A -> extends -> B -> extends -> C
  | 'mixed';                    // Combination of relationship types

export interface TraversalStatistics {
  totalSymbolsVisited: number;
  totalRelationshipsTraversed: number;
  maxDepthReached: number;
  relationshipTypeDistribution: Map<RelationshipType, number>;
  averagePathLength: number;
  strongestPath: RelationshipPath;
  traversalTimeMs: number;
}

// Query interface for relationship-aware searches
export interface RelationshipQuery {
  baseQuery: string;            // Original semantic query
  focusSymbols?: string[];      // Specific symbols to focus on
  relationshipTypes?: RelationshipType[];
  traversalOptions: TraversalOptions;
  includeContext: boolean;      // Include related code context
  contextRadius: number;        // Lines of context around each symbol
}

export interface RelationshipSearchResult {
  primaryChunks: string[];      // Main chunks matching the query
  relatedChunks: string[];      // Chunks discovered through relationships
  relationshipPaths: RelationshipPath[];
  contextGroups: ContextGroup[];
  totalTokens: number;
  efficiencyScore: number;      // Ratio of relevant/total tokens
}

export interface ContextGroup {
  theme: string;                // "Authentication", "Error Handling", etc.
  symbols: CodeSymbol[];
  relationships: CodeRelationship[];
  chunkIds: string[];
  importance: number;           // Relevance to original query
  tokenCount: number;
}

// Analysis interfaces for building the relationship graph
export interface RelationshipAnalyzer {
  analyzeFile(filePath: string, content: string): Promise<FileAnalysisResult>;
  buildRelationshipGraph(files: Map<string, string>): Promise<RelationshipGraph>;
  updateGraph(graph: RelationshipGraph, changedFiles: string[]): Promise<RelationshipGraph>;
}

export interface FileAnalysisResult {
  symbols: CodeSymbol[];
  relationships: CodeRelationship[];
  imports: ImportDeclaration[];
  exports: ExportDeclaration[];
  errors: AnalysisError[];
}

export interface ImportDeclaration {
  source: string;               // Module being imported
  importedSymbols: string[];    // Specific symbols imported
  importType: 'default' | 'named' | 'namespace' | 'dynamic';
  line: number;
  isTypeOnly?: boolean;         // TypeScript type-only imports
}

export interface ExportDeclaration {
  exportedSymbol: string;
  exportType: 'default' | 'named' | 'namespace';
  line: number;
  isReExport?: boolean;         // Re-export from another module
  originalSource?: string;      // Source module if re-export
}

export interface AnalysisError {
  type: 'parse_error' | 'symbol_resolution' | 'type_inference';
  message: string;
  line?: number;
  column?: number;
  severity: 'error' | 'warning' | 'info';
}

// Scoring and ranking interfaces
export interface RelationshipScorer {
  scoreRelationship(relationship: CodeRelationship, context: ScoringContext): number;
  scoreSymbol(symbol: CodeSymbol, context: ScoringContext): number;
  scorePath(path: RelationshipPath, context: ScoringContext): number;
}

export interface ScoringContext {
  originalQuery: string;
  queryEmbedding?: number[];
  focusAreas?: string[];
  recentChanges?: Map<string, Date>;
  errorFrequency?: Map<string, number>;
  userFeedback?: Map<string, number>;
}

// Performance and caching interfaces  
export interface GraphCache {
  getCachedGraph(repositoryPath: string): Promise<RelationshipGraph | null>;
  saveGraph(repositoryPath: string, graph: RelationshipGraph): Promise<void>;
  invalidateFile(filePath: string): Promise<void>;
  getGraphMetadata(repositoryPath: string): Promise<GraphMetadata | null>;
}

export interface GraphMetadata {
  lastUpdated: Date;
  version: string;
  fileHashes: Map<string, string>;
  symbolCount: number;
  relationshipCount: number;
  analysisTimeMs: number;
}