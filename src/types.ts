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
}

export type ChunkType = 'function' | 'class' | 'method' | 'documentation' | 'config';

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
}

export type RelationshipType = 'calls' | 'imports' | 'data_flow' | 'co_change';
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
}

export interface IndexRequest {
  repository_path: string;
  mode: 'full' | 'incremental';
  since_commit?: string;
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