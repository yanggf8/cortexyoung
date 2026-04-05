-- Cortex V5.0 Turso Schema
-- Applied once during `cortex init` via Worker's /admin/init-schema route

-- Chunks: source text + metadata
-- chunk_id format: project_id:file_path:start_line (globally unique)
CREATE TABLE IF NOT EXISTS chunks (
  chunk_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT,        -- function, class, method, config, documentation
  start_line INTEGER,
  end_line INTEGER,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  language TEXT,
  embedding F32_BLOB(384),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_chunks_project ON chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(project_id, file_path);
CREATE INDEX IF NOT EXISTS idx_chunks_language ON chunks(project_id, language);

-- Auto-update updated_at
CREATE TRIGGER IF NOT EXISTS chunks_updated_at
  AFTER UPDATE ON chunks
  FOR EACH ROW
BEGIN
  UPDATE chunks SET updated_at = datetime('now') WHERE chunk_id = NEW.chunk_id;
END;

-- Relationships: edges between chunks
CREATE TABLE IF NOT EXISTS relationships (
  source_chunk_id TEXT NOT NULL,
  target_chunk_id TEXT NOT NULL,
  rel_type TEXT NOT NULL,  -- calls, called_by, imports, exports, data_flow
  FOREIGN KEY (source_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  FOREIGN KEY (target_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_type_source ON relationships(rel_type, source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_type_target ON relationships(rel_type, target_chunk_id);

-- Projects: metadata
CREATE TABLE IF NOT EXISTS projects (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT,
  last_indexed TEXT,
  created_at TEXT DEFAULT (datetime('now'))
);

-- API keys (replaces Cloudflare KV — simpler, one data store)
CREATE TABLE IF NOT EXISTS api_keys (
  api_key TEXT PRIMARY KEY,
  created_at TEXT DEFAULT (datetime('now')),
  active INTEGER DEFAULT 1
);

-- FTS5 for keyword search fallback
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid
);

-- FTS5 sync triggers
CREATE TRIGGER IF NOT EXISTS chunks_fts_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES ('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_update AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES ('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;

-- Vector index for semantic search (DiskANN)
CREATE INDEX IF NOT EXISTS idx_chunks_embedding ON chunks(libsql_vector_idx(embedding));
