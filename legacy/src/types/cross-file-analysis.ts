/**
 * Cross-File Analysis Types
 * Defines the architecture for analyzing patterns across multiple interconnected files
 */

export interface FileNode {
  /** Absolute file path */
  path: string;
  /** Relative path from project root */
  relativePath: string;
  /** File content */
  content: string;
  /** AST representation */
  ast?: any;
  /** Last modified timestamp */
  lastModified: number;
  /** File type classification */
  fileType: FileType;
}

export interface ImportDeclaration {
  /** What is being imported */
  imported: string[];
  /** Where it's imported from */
  source: string;
  /** Resolved absolute path */
  resolvedPath?: string;
  /** Import type: default, named, namespace */
  importType: 'default' | 'named' | 'namespace' | 'mixed';
  /** Line number in file */
  lineNumber: number;
}

export interface ExportDeclaration {
  /** What is being exported */
  exported: string[];
  /** Export type: default, named */
  exportType: 'default' | 'named';
  /** Line number in file */
  lineNumber: number;
  /** Re-export source if applicable */
  source?: string;
}

export interface SymbolDefinition {
  /** Symbol name */
  name: string;
  /** Symbol type: function, class, variable, etc. */
  type: SymbolType;
  /** File where it's defined */
  definedIn: string;
  /** Line number of definition */
  lineNumber: number;
  /** AST node information */
  astNode?: any;
  /** Symbol usage context */
  context: SymbolContext;
}

export interface SymbolUsage {
  /** Symbol name */
  name: string;
  /** File where it's used */
  usedIn: string;
  /** Line number of usage */
  lineNumber: number;
  /** Usage context (call, assignment, etc.) */
  context: UsageContext;
  /** How it was imported */
  importedFrom?: string;
}

export interface DependencyGraph {
  /** All files in the project */
  files: Map<string, FileNode>;
  /** Import relationships */
  imports: Map<string, ImportDeclaration[]>;
  /** Export relationships */
  exports: Map<string, ExportDeclaration[]>;
  /** Symbol definitions across files */
  symbols: Map<string, SymbolDefinition[]>;
  /** Symbol usage tracking */
  usages: Map<string, SymbolUsage[]>;
  /** File dependency edges */
  dependencies: Map<string, string[]>;
  /** Reverse dependencies */
  dependents: Map<string, string[]>;
}

export interface ExecutionPath {
  /** Starting file/function */
  entry: string;
  /** Chain of function calls */
  callChain: ExecutionStep[];
  /** Files involved in this path */
  filesInvolved: string[];
  /** Pattern classification */
  patternType: CrossFilePatternType;
}

export interface ExecutionStep {
  /** File containing this step */
  file: string;
  /** Function/method name */
  function: string;
  /** Line number */
  lineNumber: number;
  /** Next step in chain */
  callsTo?: ExecutionStep[];
  /** Parameters passed */
  parameters?: string[];
}

export interface CrossFilePattern {
  /** Pattern identifier */
  id: string;
  /** Pattern type */
  type: CrossFilePatternType;
  /** Files involved */
  files: string[];
  /** Execution paths */
  paths: ExecutionPath[];
  /** Confidence score */
  confidence: number;
  /** Supporting evidence */
  evidence: CrossFileEvidence[];
  /** Pattern description */
  description: string;
}

export interface CrossFileEvidence {
  /** Evidence type */
  type: EvidenceType;
  /** File where evidence was found */
  file: string;
  /** Line number */
  lineNumber: number;
  /** Code snippet */
  snippet: string;
  /** Evidence strength */
  weight: number;
}

// Enums and Type Definitions

export type FileType = 
  | 'route' 
  | 'controller' 
  | 'middleware' 
  | 'service' 
  | 'model' 
  | 'utility' 
  | 'config' 
  | 'main'
  | 'test'
  | 'unknown';

export type SymbolType = 
  | 'function'
  | 'class'
  | 'variable'
  | 'constant'
  | 'interface'
  | 'type'
  | 'enum'
  | 'namespace';

export type SymbolContext = 
  | 'definition'
  | 'declaration'
  | 'implementation'
  | 'export'
  | 'import';

export type UsageContext = 
  | 'call'
  | 'assignment'
  | 'parameter'
  | 'return'
  | 'property_access'
  | 'method_call'
  | 'instantiation';

export type CrossFilePatternType = 
  | 'mvc_flow'           // Route -> Controller -> Service -> Model
  | 'middleware_chain'   // Auth -> Validation -> Handler
  | 'service_composition' // Service A -> Service B -> Service C
  | 'error_propagation'  // Error thrown in Service, caught in Controller
  | 'auth_flow'          // Auth middleware -> Route handler
  | 'api_response_flow'  // Controller -> Response formatter -> Client
  | 'dependency_injection' // Container -> Service -> Repository
  | 'event_handling'     // Event emitter -> Event handlers
  | 'plugin_architecture' // Core -> Plugin system
  | 'layered_architecture' // Presentation -> Business -> Data
  | 'mixed'              // Mixed patterns

export type EvidenceType = 
  | 'import_declaration'
  | 'function_call'
  | 'method_invocation'
  | 'variable_assignment'
  | 'parameter_passing'
  | 'return_statement'
  | 'error_handling'
  | 'middleware_usage'
  | 'route_definition'
  | 'config_reference';

// Analysis Results

export interface CrossFileAnalysisResult {
  /** Dependency graph */
  dependencyGraph: DependencyGraph;
  /** Detected patterns */
  patterns: CrossFilePattern[];
  /** Execution flows */
  executionFlows: ExecutionPath[];
  /** Architecture classification */
  architecture: ArchitectureType;
  /** Analysis metadata */
  metadata: AnalysisMetadata;
}

export interface AnalysisMetadata {
  /** When analysis was performed */
  timestamp: string;
  /** Files analyzed */
  filesAnalyzed: number;
  /** Total execution time */
  executionTimeMs: number;
  /** Analysis depth achieved */
  depthLevel: number;
  /** Confidence in results */
  overallConfidence: number;
}

export type ArchitectureType = 
  | 'monolithic'
  | 'layered'
  | 'mvc'
  | 'microservices'
  | 'component_based'
  | 'event_driven'
  | 'plugin_based'
  | 'mixed'
  | 'unknown';