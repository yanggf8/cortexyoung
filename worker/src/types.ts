// Shared types for Cortex V5.0 Worker

export interface Env {
  TURSO_URL: string;
  TURSO_AUTH_TOKEN: string;
  SETUP_TOKEN: string;
  ENVIRONMENT: string;
}

// Chunk payload from CLI
export interface ChunkPayload {
  chunk_id: string;
  project_id: string;
  file_path: string;
  symbol_name?: string;
  chunk_type?: string;
  start_line: number;
  end_line: number;
  content: string;
  content_hash: string;
  language?: string;
  embedding: number[];
}

// Relationship payload from CLI
export interface RelationshipPayload {
  source_chunk_id: string;
  target_chunk_id: string;
  rel_type: string;
}

// Search request
export interface SearchRequest {
  vector: number[];
  project_id: string;
  top_k?: number;
  offset?: number;
}

// Keyword search request
export interface KeywordSearchRequest {
  query: string;
  project_id: string;
  top_k?: number;
  offset?: number;
}

// Relationship query
export interface RelationshipRequest {
  symbol: string;
  project_id: string;
  depth?: number;
  rel_types?: string[];
}

// Batch index request
export interface BatchIndexRequest {
  project_id: string;
  project_name?: string;
  project_path?: string;
  chunks: ChunkPayload[];
  relationships: RelationshipPayload[];
}

// Delete file request
export interface DeleteFileRequest {
  project_id: string;
  file_path: string;
}

// API response shapes
export interface SearchResponse {
  chunks: Array<{
    chunk_id: string;
    file_path: string;
    symbol_name?: string;
    chunk_type?: string;
    start_line: number;
    end_line: number;
    content: string;
    language?: string;
    score: number;
  }>;
  total_matches: number;
  has_more: boolean;
}

export interface RelationshipResponse {
  nodes: Array<{
    chunk_id: string;
    file_path: string;
    symbol_name?: string;
    chunk_type?: string;
  }>;
  edges: Array<{
    source: string;
    target: string;
    rel_type: string;
  }>;
}

export interface ProjectStatus {
  project_id: string;
  name: string;
  chunk_count: number;
  relationship_count: number;
  last_indexed: string | null;
  languages: string[];
}
