// Shared types and interfaces for Cortex V2.1

export interface CodeChunk {
  chunk_id: string;
  file_path: string;
  symbol_name?: string;
  chunk_type: ChunkType;
  start_line: number;
  end_line: number;
  content: string;
  content_hash: string;
  embedding: number[];
  relationships: CodeRelationships;
  git_metadata: GitMetadata;
  language_metadata: LanguageMetadata;
  usage_patterns: UsagePatterns;
  last_modified: string;
  relevance_score?: number;
  similarity_score?: number;
  function_name?: string;
}

export type ChunkType = 'function' | 'class' | 'method' | 'documentation' | 'config';

// Simple version compatibility
export const CORTEX_PROGRAM_VERSION = '2.1.0';
export const CORTEX_SCHEMA_VERSION = '1.0.0';  // We use schema v1, program v2.1

// Program 2.1 is compatible with schema 1.x
export const COMPATIBLE_SCHEMA_VERSIONS = ['1.0.0', '1.1.0'];

export interface SchemaInfo {
  version: string;
  compatible: boolean;
  requiresMigration: boolean;
  migrationPath?: string[];
}

export interface ModelInfo {
  name: string;
  version: string;
  hash?: string;
  dimension: number;
  isLoaded: boolean;
}

export interface CodeRelationships {
  calls: string[];
  called_by: string[];
  imports: string[];
  exports: string[];
  data_flow: string[];
}

export interface GitMetadata {
  last_modified_commit: string;
  commit_author: string;
  commit_message: string;
  commit_date: string;
  file_history_length: number;
  co_change_files: string[];
}

export interface LanguageMetadata {
  language: string;
  complexity_score: number;
  dependencies: string[];
  exports: string[];
}

export interface UsagePatterns {
  access_frequency: number;
  task_contexts: string[];
}

export interface QueryRequest {
  task: string;
  max_chunks?: number;
  file_filters?: string[];
  recency_weight?: number;
  include_tests?: boolean;
  multi_hop?: MultiHopConfig;
  context_mode?: ContextMode;
}

export interface MultiHopConfig {
  enabled: boolean;
  max_hops: number;
  relationship_types: RelationshipType[];
  hop_decay: number;
  focus_symbols?: string[];
  include_paths?: boolean;
  traversal_direction?: 'forward' | 'backward';
  min_strength?: number;
}

export type RelationshipType =
  | 'calls'
  | 'imports'
  | 'data_flow'
  | 'co_change'
  | 'throws'
  | 'extends'
  | 'implements';
export type ContextMode = 'minimal' | 'structured' | 'adaptive';

export interface QueryResponse {
  context_package: ContextPackage;
  metadata: QueryMetadata;
}

export interface SearchResponse {
  status?: 'success' | 'error';
  chunks?: CodeChunk[];
  context_package: ContextPackage;
  query_time_ms: number;
  total_chunks_considered: number;
  relationship_paths?: any[];
  efficiency_score?: number;
  metadata: QueryMetadata;
  context_chunks?: CodeChunk[];
}

export interface ContextPackage {
  summary: string;
  groups: ContextGroup[];
  related_files: string[];
  total_tokens?: number;
  token_efficiency?: number;
  relationship_insights?: any;
}

export interface ContextGroup {
  title: string;
  description: string;
  chunks: CodeChunk[];
  importance_score: number;
  relationship_paths?: any[];
}

export interface QueryMetadata {
  total_chunks_found: number;
  query_time_ms: number;
  chunks_returned: number;
  token_estimate: number;
  efficiency_score: number;
  relationship_paths?: any[];
  confidence_scores?: number[];
  mmr_metrics?: {
    critical_set_coverage: number;
    diversity_score: number;
    budget_utilization: number;
    selection_time_ms: number;
  };
}

export interface IndexRequest {
  repository_path: string;
  mode: 'full' | 'incremental' | 'reindex';
  since_commit?: string;
  force_rebuild?: boolean; // For reindex mode
}

export interface IndexResponse {
  status: 'success' | 'error';
  chunks_processed: number;
  time_taken_ms: number;
  error_message?: string;
}

// MCP Tool interfaces
export interface MCPToolRequest {
  method: string;
  params: any;
  id: string | number;
}

export interface MCPToolResponse {
  result?: any;
  error?: MCPError;
  id: string | number;
}

export interface MCPError {
  code: number;
  message: string;
  data?: any;
}

// IEmbedder interface for standardizing embedding providers
export interface IEmbedder {
  readonly providerId: string;
  readonly modelId: string; 
  readonly dimensions: number;
  readonly maxBatchSize: number;
  readonly normalization: "l2" | "none";
  
  embedBatch(texts: string[], options?: EmbedOptions): Promise<EmbeddingResult>;
  getHealth(): Promise<ProviderHealth>;
  getMetrics(): Promise<ProviderMetrics>;
}

export interface EmbedOptions {
  timeout?: number;
  retryCount?: number;
  priority?: "high" | "normal" | "low";
  requestId?: string;
}

export interface EmbeddingResult {
  embeddings: number[][];
  metadata: EmbeddingMetadata;
  performance: PerformanceStats;
}

export interface EmbeddingMetadata {
  providerId: string;
  modelId: string;
  batchSize: number;
  processedAt: number;
  requestId?: string;
}

export interface PerformanceStats {
  duration: number;
  memoryDelta: number;
  processId?: number;
  cacheHits?: number;
  retries?: number;
}

export interface ProviderHealth {
  status: "healthy" | "degraded" | "unhealthy";
  details: string;
  lastCheck: number;
  uptime?: number;
  errorRate?: number;
}

export interface ProviderMetrics {
  requestCount: number;
  avgDuration: number;
  errorRate: number;
  lastSuccess: number;
  totalEmbeddings: number;
  cacheHitRate?: number;
}

// Embedding Cache Types
export interface EmbeddingCacheEntry {
  embedding: number[];
  created_at: string;
  model_version: string;
  access_count: number;
  last_accessed: string;
  chunk_metadata: {
    file_path: string;
    symbol_name?: string;
    chunk_type: ChunkType;
  };
}

export interface EmbeddingCache {
  [contentHash: string]: EmbeddingCacheEntry;
}

export interface CacheStats {
  total_entries: number;
  cache_hits: number;
  cache_misses: number;
  hit_rate: number;
  last_cleanup: string;
  size_bytes: number;
}