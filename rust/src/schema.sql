CREATE TABLE IF NOT EXISTS _cortex_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  git_head TEXT,
  last_indexed_at INTEGER,
  extractor_version TEXT NOT NULL,
  created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS chunks (
  chunk_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT CHECK(chunk_type IN ('function','class','method','config','documentation','unparsed')),
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  language TEXT,
  chunk_source TEXT NOT NULL CHECK(chunk_source IN ('ast','unparsed')),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_chunks_project ON chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(project_id, file_path);
CREATE INDEX IF NOT EXISTS idx_chunks_symbol ON chunks(project_id, symbol_name);

CREATE TABLE IF NOT EXISTS file_state (
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  file_content_hash TEXT NOT NULL,
  updated_at TEXT DEFAULT (datetime('now')),
  PRIMARY KEY (project_id, file_path)
);

CREATE TABLE IF NOT EXISTS relationships (
  source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
  confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
  confidence_reasoning TEXT,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);

-- NOTE: spec §4 requires tokenize='unicode61 "remove_diacritics 1" "tokenchars ._$"' but bundled SQLite 3.49.2 (better-sqlite3 11.10.0) rejects any parameterized unicode61 (parse error in tokenize directive) — only bare 'unicode61' passes. Downgraded to bare; revert to spec string when CI SQLite supports it. See task-2-report.md.

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid,
  tokenize='unicode61'
);
CREATE TRIGGER IF NOT EXISTS chunks_fts_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;
CREATE TRIGGER IF NOT EXISTS chunks_fts_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
END;
CREATE TRIGGER IF NOT EXISTS chunks_fts_update AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;

CREATE TABLE IF NOT EXISTS reading_notes (
  reading_id INTEGER PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  start_line INTEGER NOT NULL CHECK(start_line >= 1),
  end_line INTEGER NOT NULL CHECK(end_line >= start_line),
  ends_at_eof INTEGER NOT NULL CHECK(ends_at_eof IN (0, 1)),
  content TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  source_mtime_ms REAL NOT NULL,
  source_size INTEGER NOT NULL,
  read_count INTEGER NOT NULL DEFAULT 1,
  first_read_at INTEGER NOT NULL,
  last_read_at INTEGER NOT NULL,
  UNIQUE(project_id, file_path, start_line, end_line)
);
CREATE INDEX IF NOT EXISTS idx_reading_notes_file
  ON reading_notes(project_id, file_path, start_line, end_line);

CREATE VIRTUAL TABLE IF NOT EXISTS reading_notes_fts USING fts5(
  content, file_path,
  content=reading_notes, content_rowid=reading_id,
  tokenize='unicode61'
);
CREATE TRIGGER IF NOT EXISTS reading_notes_fts_insert AFTER INSERT ON reading_notes BEGIN
  INSERT INTO reading_notes_fts(rowid, content, file_path)
    VALUES (NEW.reading_id, NEW.content, NEW.file_path);
END;
CREATE TRIGGER IF NOT EXISTS reading_notes_fts_delete AFTER DELETE ON reading_notes BEGIN
  INSERT INTO reading_notes_fts(reading_notes_fts, rowid, content, file_path)
    VALUES('delete', OLD.reading_id, OLD.content, OLD.file_path);
END;
CREATE TRIGGER IF NOT EXISTS reading_notes_fts_update AFTER UPDATE ON reading_notes BEGIN
  INSERT INTO reading_notes_fts(reading_notes_fts, rowid, content, file_path)
    VALUES('delete', OLD.reading_id, OLD.content, OLD.file_path);
  INSERT INTO reading_notes_fts(rowid, content, file_path)
    VALUES (NEW.reading_id, NEW.content, NEW.file_path);
END;
