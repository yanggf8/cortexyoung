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
  -- v4 (the recall line): the line that *names* the callee inside the source chunk, so confirming
  -- one edge costs a one-line read instead of re-reading the dependent. It is the earliest such
  -- line, because `relationships` is keyed by (source, target, rel_type): two calls from one
  -- function to one callee are one edge, and one line is all this column claims to be.
  call_site_line INTEGER,
  -- How the extractor saw the call, because the resolution policy differs per form: a `receiver`
  -- edge (`t.add()`) attaches only when the method name is unique project-wide, while `bare` and
  -- `scoped` keep the pre-v4 rules. Carrying the form is what makes an edge falsifiable -- a
  -- reader can check the claim without re-deriving it.
  call_form TEXT NOT NULL DEFAULT 'bare'
    CHECK(call_form IN ('bare','receiver','scoped')),
  confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
  confidence_reasoning TEXT,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);

-- F-01: `relationships` is derived state that spans files. Rebuilding it after a per-file
-- chunk update needs the unresolved matches persisted, because ON DELETE CASCADE takes an
-- edge with it as soon as the *target* chunk is replaced by a re-index of another file.
-- `source_symbol` is '' for file-level edges (imports), which must be distinct from NULL so
-- the primary key actually deduplicates them.
CREATE TABLE IF NOT EXISTS raw_edges (
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  source_symbol TEXT NOT NULL DEFAULT '',
  raw_target TEXT NOT NULL,
  rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
  -- v4: carried from the pack rule that matched (`edge:calls:receiver`), because the gate in
  -- `graph::resolve_edge_targets` has to know which form an unresolved name arrived as. `bare` is
  -- the pre-v4 shape, so rows an upgrade adds with DEFAULT 'bare' mean what they always meant.
  -- The primary key is deliberately unchanged. A bare `add()` and a receiver `t.add()` on one line do
  -- not collide, because a receiver edge stores its head (`t.add`) beside the method name; what is
  -- left is a true duplicate, and `indexer::replace_file_raw_edges` writes in a canonical order so
  -- the surviving row does not depend on the order ast-grep emitted records in.
  call_form TEXT NOT NULL DEFAULT 'bare'
    CHECK(call_form IN ('bare','receiver','scoped')),
  start_line INTEGER NOT NULL,
  PRIMARY KEY (project_id, file_path, rel_type, raw_target, source_symbol, start_line)
);
CREATE INDEX IF NOT EXISTS idx_raw_edges_file ON raw_edges(project_id, file_path);

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
