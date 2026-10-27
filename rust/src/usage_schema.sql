CREATE TABLE IF NOT EXISTS _usage_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS command_log (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  project_id TEXT,
  machine_id TEXT NOT NULL DEFAULT 'legacy_unknown',
  machine_source TEXT NOT NULL DEFAULT 'legacy_no_stamp',
  command TEXT NOT NULL,
  args_summary TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('ok', 'error')),
  error_code TEXT,
  read_source TEXT,
  requested_content_mode TEXT,
  effective_content_mode TEXT,
  receipt_hit INTEGER,
  index_stale INTEGER,
  bytes_out INTEGER NOT NULL CHECK(bytes_out >= 0),
  saved_bytes INTEGER NOT NULL DEFAULT 0 CHECK(saved_bytes >= 0)
);
CREATE INDEX IF NOT EXISTS idx_command_log_ts ON command_log(ts);
CREATE INDEX IF NOT EXISTS idx_command_log_project_ts ON command_log(project_id, ts);
CREATE INDEX IF NOT EXISTS idx_command_log_not_measure ON command_log(id) WHERE command <> 'measure';
-- NOTE: command_log's column names are hashed into every cort_command event_id (src/telemetry.rs); changing them voids `cort measure verify`.

CREATE TABLE IF NOT EXISTS index_health_log (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  machine_id TEXT NOT NULL,
  machine_source TEXT NOT NULL,
  root TEXT,
  local_project_id TEXT NOT NULL,
  project_id TEXT,
  status TEXT NOT NULL CHECK(status IN ('ok', 'unreadable', 'missing_dir', 'no_index')),
  error TEXT,
  fresh TEXT,
  chunks INTEGER,
  relationships INTEGER,
  not_chunked INTEGER,
  real_gap INTEGER,
  fts TEXT
);
CREATE INDEX IF NOT EXISTS idx_index_health_log_local ON index_health_log(local_project_id, id);
