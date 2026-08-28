CREATE TABLE IF NOT EXISTS _usage_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS command_log (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  project_id TEXT,
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
