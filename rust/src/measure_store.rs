//! Compaction measurement rows, stored in a remote Turso database.
//!
//! Two implementations of one trait: [`SqliteStore`] is a plain rusqlite file (tests, and any
//! caller that wants the logic without a network), and [`RemoteStore`] uses Hrana HTTP.
//! Both use the same [`MEASURE_SCHEMA`] and SQL text.

use crate::memory_store::{self, Config};
use crate::network_timeout::{network_failure, REMOTE_PREFIX};
use crate::remote::{Cell, ErrorKind, RemoteDb, RemoteError, Stmt, Transport, UreqTransport};
use crate::telemetry::TelemetryEvent;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowCounts {
    pub tool_calls: i64,
    pub rereads: i64,
    pub repeated_commands: i64,
    pub transcript_lookups: i64,
    pub human_messages: i64,
    pub corrections: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionRow {
    pub machine_id: String,
    pub boundary_uuid: String,
    pub session_id: String,
    pub time_ms: i64,
    pub project_id: Option<String>,
    pub trigger: String,
    pub pre_tokens: Option<i64>,
    pub post_tokens: Option<i64>,
    pub pre: WindowCounts,
    pub post: WindowCounts,
    pub post_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineHost {
    pub machine_id: String,
    pub host: String,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRow {
    pub machine_id: String,
    pub harness: Option<String>,
    pub gate: Option<String>,
    pub gate_ms: Option<i64>,
    /// The row's `hook` outcome (`fired`, `no_payload`, ...); `None` when `args_summary` is
    /// not JSON.
    pub hook: Option<String>,
}

pub trait MeasureStore {
    /// Insert, or replace the row with the same `(machine_id, boundary_uuid)`.
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String>;
    /// Uploaded boundary owners for one machine, used to rebuild lost local progress.
    fn compaction_owners(&mut self, machine_id: &str) -> Result<Vec<(String, String)>, String>;
    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String>;
    fn gate_rows_since(&mut self, since_ms: i64) -> Result<Vec<GateRow>, String>;
    /// Record that `host` was seen for `machine_id`; the latest `seen_ms` wins.
    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String>;
    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String>;
    /// `INSERT OR IGNORE` each event; returns how many were new. One transaction per chunk.
    fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String>;
    /// Local rowid (the payload's `id`) -> event_id for what the store holds for this machine.
    fn remote_ids(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<HashMap<i64, String>, String>;
    /// How many rows the store holds for this machine and source, counted in SQL: unlike
    /// `remote_ids`, a row whose payload has no usable `id` still counts.
    fn event_count(&mut self, machine_id: &str, source: &str) -> Result<usize, String>;
    /// The stored payload of one event, for a mismatch report.
    fn remote_payload(
        &mut self,
        machine_id: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<String>, String>;
    fn event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Option<(i64, String)>, String>;
    fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String>;
    /// Names of indexes currently present in the database.
    fn index_names(&mut self) -> Result<Vec<String>, String>;
}

pub fn missing_indexes(present: &[String]) -> Vec<String> {
    MEASURE_SCHEMA
        .iter()
        .filter(|sql| sql.starts_with("CREATE INDEX ") || sql.starts_with("CREATE UNIQUE INDEX "))
        .filter_map(|sql| {
            let mut words = sql.split_whitespace().skip_while(|w| *w != "INDEX").skip(1);
            let first = words.next()?;
            if first == "IF" {
                words.nth(2)
            } else {
                Some(first)
            }
        })
        .filter(|name| !present.iter().any(|have| have == name))
        .map(str::to_owned)
        .collect()
}

/// Classify the safe display string emitted by a store operation.
pub fn remote_error_kind(message: &str) -> &'static str {
    let error = message.strip_prefix("remote read: ").unwrap_or(message);
    let error = error.strip_prefix(REMOTE_PREFIX).unwrap_or(error);
    if error.starts_with("network timeout") || error.starts_with("timeout") {
        "timeout"
    } else if error.starts_with("blocked") {
        "blocked"
    } else if error.starts_with("unauthorized") {
        "unauthorized"
    } else if error.starts_with("unreachable") {
        "unreachable"
    } else if error.starts_with("busy") {
        "busy"
    } else if error.starts_with("sql") || error.starts_with("constraint") {
        "sql"
    } else {
        "other"
    }
}

/// Every CREATE statement; the remote store runs them when its local version marker is missing.
pub const MEASURE_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS compactions (
        machine_id TEXT NOT NULL,
        boundary_uuid TEXT NOT NULL,
        session_id TEXT NOT NULL,
        time_ms INTEGER NOT NULL,
        project_id TEXT,
        trigger_kind TEXT NOT NULL,
        pre_tokens INTEGER,
        post_tokens INTEGER,
        pre_tool_calls INTEGER NOT NULL,
        pre_rereads INTEGER NOT NULL,
        pre_repeated_commands INTEGER NOT NULL,
        pre_transcript_lookups INTEGER NOT NULL,
        pre_human_messages INTEGER NOT NULL,
        pre_corrections INTEGER NOT NULL,
        post_tool_calls INTEGER NOT NULL,
        post_rereads INTEGER NOT NULL,
        post_repeated_commands INTEGER NOT NULL,
        post_transcript_lookups INTEGER NOT NULL,
        post_human_messages INTEGER NOT NULL,
        post_corrections INTEGER NOT NULL,
        post_complete INTEGER NOT NULL,
        PRIMARY KEY (machine_id, boundary_uuid)
    )",
    "CREATE INDEX IF NOT EXISTS compactions_time ON compactions (time_ms)",
    "CREATE TABLE IF NOT EXISTS machine_hosts (
        machine_id TEXT NOT NULL,
        host TEXT NOT NULL,
        last_seen_ms INTEGER NOT NULL,
        PRIMARY KEY (machine_id, host)
    )",
    // Usage counters imported from claudecat's retired CORT-AUDIT.md, one row per host and day.
    // Health rows of the same import are events (`cort_index_health`), not a table of their own.
    "CREATE TABLE IF NOT EXISTS imported_usage (
        day TEXT NOT NULL,
        host TEXT NOT NULL,
        commands_window INTEGER,
        core_window INTEGER,
        deep_window INTEGER,
        commands_7d INTEGER,
        deep_7d INTEGER,
        decline_top TEXT,
        window_days INTEGER NOT NULL,
        PRIMARY KEY (day, host)
    )",
    // The next two are claudecat's DDL verbatim (turso_memory.rs), so rows it already wrote fit.
    "CREATE TABLE IF NOT EXISTS telemetry_events (source TEXT NOT NULL, event_id TEXT NOT NULL, machine_id TEXT NOT NULL, machine_source TEXT NOT NULL, project_id TEXT, time_ms INTEGER, payload TEXT NOT NULL, PRIMARY KEY (source, event_id))",
    "CREATE TABLE IF NOT EXISTS telemetry_project_aliases (machine_id TEXT NOT NULL, local_project_id TEXT NOT NULL, shared_project_id TEXT NOT NULL, PRIMARY KEY (machine_id, local_project_id, shared_project_id))",
    "CREATE INDEX IF NOT EXISTS telemetry_events_source_time ON telemetry_events (source, time_ms)",
    "CREATE INDEX IF NOT EXISTS telemetry_events_source_machine_time ON telemetry_events (source, machine_id, time_ms)",
    // Legacy watermarks, read only when rebuilding lost local progress.
    "CREATE TABLE IF NOT EXISTS event_watermarks (
        machine_id TEXT NOT NULL,
        source TEXT NOT NULL,
        max_rowid INTEGER NOT NULL,
        row_fingerprint TEXT NOT NULL,
        PRIMARY KEY (machine_id, source)
    )",
];

const COMPACTION_COLUMNS: &str = "machine_id, boundary_uuid, session_id, time_ms, project_id, \
    trigger_kind, pre_tokens, post_tokens, \
    pre_tool_calls, pre_rereads, pre_repeated_commands, pre_transcript_lookups, \
    pre_human_messages, pre_corrections, \
    post_tool_calls, post_rereads, post_repeated_commands, post_transcript_lookups, \
    post_human_messages, post_corrections, post_complete";

const UPSERT_COMPACTION: &str = "INSERT OR REPLACE INTO compactions (machine_id, boundary_uuid, \
    session_id, time_ms, project_id, trigger_kind, pre_tokens, post_tokens, \
    pre_tool_calls, pre_rereads, pre_repeated_commands, pre_transcript_lookups, \
    pre_human_messages, pre_corrections, \
    post_tool_calls, post_rereads, post_repeated_commands, post_transcript_lookups, \
    post_human_messages, post_corrections, post_complete) \
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
const SELECT_COMPACTION_OWNERS: &str =
    "SELECT boundary_uuid, session_id FROM compactions WHERE machine_id = ?";
const UPSERT_HOST: &str = "INSERT INTO machine_hosts (machine_id, host, last_seen_ms) \
    VALUES (?, ?, ?) ON CONFLICT (machine_id, host) DO UPDATE SET \
    last_seen_ms = MAX(last_seen_ms, excluded.last_seen_ms)";
const SELECT_HOSTS: &str = "SELECT machine_id, host, last_seen_ms FROM machine_hosts";
const INSERT_EVENT: &str = "INSERT OR IGNORE INTO telemetry_events (source, event_id, machine_id, \
    machine_source, project_id, time_ms, payload) VALUES (?, ?, ?, ?, ?, ?, ?)";
const SELECT_REMOTE: &str =
    "SELECT event_id, CASE WHEN json_valid(payload) THEN json_extract(payload, '$.id') END \
     FROM telemetry_events WHERE source = ? AND machine_id = ?";
const COUNT_MACHINE_EVENTS: &str =
    "SELECT COUNT(*) FROM telemetry_events WHERE source = ? AND machine_id = ?";
const SELECT_PAYLOAD: &str =
    "SELECT payload FROM telemetry_events WHERE source = ? AND machine_id = ? AND event_id = ?";
const SELECT_WATERMARK: &str =
    "SELECT max_rowid, row_fingerprint FROM event_watermarks WHERE machine_id = ? AND source = ?";
const INSERT_ALIAS: &str = "INSERT OR IGNORE INTO telemetry_project_aliases \
    (machine_id, local_project_id, shared_project_id) VALUES (?, ?, ?)";
const SELECT_INDEX_NAMES: &str = "SELECT name FROM sqlite_master WHERE type='index'";
const SELECT_GATE_ROWS: &str = "WITH candidates AS (
    SELECT machine_id, json_extract(payload, '$.args_summary') AS args_summary
    FROM telemetry_events
    WHERE source = 'cort_command' AND time_ms >= ?
      AND json_extract(payload, '$.command') = 'hook-suggest'
), valid_args AS (
    SELECT machine_id,
      CASE WHEN json_valid(args_summary) THEN args_summary END AS args_summary
    FROM candidates
)
SELECT machine_id,
  CAST(json_extract(args_summary, '$.harness') AS TEXT),
  CAST(json_extract(args_summary, '$.gate') AS TEXT),
  CAST(json_extract(args_summary, '$.gate_ms') AS TEXT),
  CAST(json_extract(args_summary, '$.hook') AS TEXT)
FROM valid_args";
/// Events per transaction: a commit per row is minutes on a 130k-row first upload.
const EVENT_CHUNK: usize = 2000;

fn select_since() -> String {
    format!("SELECT {COMPACTION_COLUMNS} FROM compactions WHERE time_ms >= ? ORDER BY time_ms, machine_id, boundary_uuid")
}

fn opt_int(v: Option<i64>) -> Cell {
    v.map_or(Cell::Null, Cell::Int)
}

fn compaction_cells(r: &CompactionRow) -> Vec<Cell> {
    let mut cells = vec![
        Cell::Text(r.machine_id.clone()),
        Cell::Text(r.boundary_uuid.clone()),
        Cell::Text(r.session_id.clone()),
        Cell::Int(r.time_ms),
        r.project_id.clone().map_or(Cell::Null, Cell::Text),
        Cell::Text(r.trigger.clone()),
        opt_int(r.pre_tokens),
        opt_int(r.post_tokens),
    ];
    for w in [&r.pre, &r.post] {
        cells.extend(
            [
                w.tool_calls,
                w.rereads,
                w.repeated_commands,
                w.transcript_lookups,
                w.human_messages,
                w.corrections,
            ]
            .map(Cell::Int),
        );
    }
    cells.push(Cell::Int(i64::from(r.post_complete)));
    cells
}

fn int(c: &Cell) -> Result<i64, String> {
    match c {
        Cell::Int(i) => Ok(*i),
        other => Err(format!("expected integer column, got {other:?}")),
    }
}

fn opt_i(c: &Cell) -> Result<Option<i64>, String> {
    match c {
        Cell::Null => Ok(None),
        c => int(c).map(Some),
    }
}

fn text(c: &Cell) -> Result<String, String> {
    match c {
        Cell::Text(s) => Ok(s.clone()),
        other => Err(format!("expected text column, got {other:?}")),
    }
}

fn window(cells: &[Cell]) -> Result<WindowCounts, String> {
    Ok(WindowCounts {
        tool_calls: int(&cells[0])?,
        rereads: int(&cells[1])?,
        repeated_commands: int(&cells[2])?,
        transcript_lookups: int(&cells[3])?,
        human_messages: int(&cells[4])?,
        corrections: int(&cells[5])?,
    })
}

fn compaction_from_cells(c: &[Cell]) -> Result<CompactionRow, String> {
    if c.len() != 21 {
        return Err(format!(
            "compactions row has {} columns, expected 21",
            c.len()
        ));
    }
    Ok(CompactionRow {
        machine_id: text(&c[0])?,
        boundary_uuid: text(&c[1])?,
        session_id: text(&c[2])?,
        time_ms: int(&c[3])?,
        project_id: match &c[4] {
            Cell::Null => None,
            other => Some(text(other)?),
        },
        trigger: text(&c[5])?,
        pre_tokens: opt_i(&c[6])?,
        post_tokens: opt_i(&c[7])?,
        pre: window(&c[8..14])?,
        post: window(&c[14..20])?,
        post_complete: int(&c[20])? != 0,
    })
}

fn host_from_cells(c: &[Cell]) -> Result<MachineHost, String> {
    Ok(MachineHost {
        machine_id: text(&c[0])?,
        host: text(&c[1])?,
        last_seen_ms: int(&c[2])?,
    })
}

fn gate_from_cells(c: &[Cell]) -> Result<GateRow, String> {
    let optional_text = |cell: &Cell| match cell {
        Cell::Null => Ok(None),
        other => text(other).map(Some),
    };
    Ok(GateRow {
        machine_id: text(&c[0])?,
        harness: optional_text(&c[1])?,
        gate: optional_text(&c[2])?,
        gate_ms: optional_text(&c[3])?.and_then(|s| s.parse::<i64>().ok()),
        hook: optional_text(&c[4])?,
    })
}

fn event_cells(e: &TelemetryEvent) -> Vec<Cell> {
    vec![
        Cell::Text(e.source.clone()),
        Cell::Text(e.event_id.clone()),
        Cell::Text(e.machine_id.clone()),
        Cell::Text(e.machine_source.clone()),
        e.project_id.clone().map_or(Cell::Null, Cell::Text),
        opt_int(e.time_ms),
        Cell::Text(e.payload.clone()),
    ]
}

/// `(event_id, id)` rows, `id` being the payload's `id` key (NULL when absent) -> id -> event_id.
fn remote_map(rows: &[Vec<Cell>]) -> Result<HashMap<i64, String>, String> {
    let mut out = HashMap::new();
    for r in rows {
        if let Some(id) = opt_i(&r[1])? {
            out.insert(id, text(&r[0])?);
        }
    }
    Ok(out)
}

/// COMMIT failed: try to roll back so the connection is not left inside a transaction, then
/// return the COMMIT error (the rollback's own error adds nothing).
fn commit_or_rollback(commit: Result<(), String>, rollback: impl FnOnce()) -> Result<(), String> {
    if commit.is_err() {
        rollback();
    }
    commit
}

fn alias_cells(r: &(String, String, String)) -> Vec<Cell> {
    vec![
        Cell::Text(r.0.clone()),
        Cell::Text(r.1.clone()),
        Cell::Text(r.2.clone()),
    ]
}

/// A plain SQLite file implementing the store.
pub struct SqliteStore {
    conn: rusqlite::Connection,
    fail_write: bool,
}

fn sql_err(e: rusqlite::Error) -> String {
    e.to_string()
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path).map_err(sql_err)?;
        for stmt in MEASURE_SCHEMA {
            conn.execute(stmt, []).map_err(sql_err)?;
        }
        Ok(Self {
            conn,
            fail_write: false,
        })
    }

    /// Test hook: reject writes while leaving stored rows intact.
    pub fn fail_write(&mut self, fail: bool) {
        self.fail_write = fail;
    }

    fn query(&self, sql: &str, params: Vec<Cell>, cols: usize) -> Result<Vec<Vec<Cell>>, String> {
        use rusqlite::types::{Value, ValueRef};
        let mut stmt = self.conn.prepare(sql).map_err(sql_err)?;
        let params: Vec<Value> = params.into_iter().map(to_sqlite).collect();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params))
            .map_err(sql_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(sql_err)? {
            let mut cells = Vec::with_capacity(cols);
            for i in 0..cols {
                cells.push(match row.get_ref(i).map_err(sql_err)? {
                    ValueRef::Null => Cell::Null,
                    ValueRef::Integer(i) => Cell::Int(i),
                    ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
                    other => return Err(format!("unexpected column type {other:?}")),
                });
            }
            out.push(cells);
        }
        Ok(out)
    }

    fn exec(&self, sql: &str, params: Vec<Cell>) -> Result<(), String> {
        if self.fail_write {
            return Err("write failed (fail_write set)".into());
        }
        let params: Vec<rusqlite::types::Value> = params.into_iter().map(to_sqlite).collect();
        self.conn
            .execute(sql, rusqlite::params_from_iter(params))
            .map_err(sql_err)?;
        Ok(())
    }
}

fn to_sqlite(c: Cell) -> rusqlite::types::Value {
    match c {
        Cell::Null => rusqlite::types::Value::Null,
        Cell::Int(i) => rusqlite::types::Value::Integer(i),
        Cell::Text(s) => rusqlite::types::Value::Text(s),
    }
}

impl MeasureStore for SqliteStore {
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String> {
        self.exec(UPSERT_COMPACTION, compaction_cells(row))
    }

    fn compaction_owners(&mut self, machine_id: &str) -> Result<Vec<(String, String)>, String> {
        self.query(
            SELECT_COMPACTION_OWNERS,
            vec![Cell::Text(machine_id.into())],
            2,
        )?
        .iter()
        .map(|r| Ok((text(&r[0])?, text(&r[1])?)))
        .collect()
    }

    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String> {
        self.query(&select_since(), vec![Cell::Int(since_ms)], 21)?
            .iter()
            .map(|r| compaction_from_cells(r))
            .collect()
    }

    fn gate_rows_since(&mut self, since_ms: i64) -> Result<Vec<GateRow>, String> {
        self.query(SELECT_GATE_ROWS, vec![Cell::Int(since_ms)], 5)?
            .iter()
            .map(|r| gate_from_cells(r))
            .collect()
    }

    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String> {
        self.exec(
            UPSERT_HOST,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(host.into()),
                Cell::Int(seen_ms),
            ],
        )
    }

    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String> {
        self.query(SELECT_HOSTS, vec![], 3)?
            .iter()
            .map(|r| host_from_cells(r))
            .collect()
    }

    fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String> {
        if self.fail_write && !events.is_empty() {
            return Err("write failed (fail_write set)".into());
        }
        let mut inserted = 0usize;
        for chunk in events.chunks(EVENT_CHUNK) {
            self.conn.execute_batch("BEGIN").map_err(sql_err)?;
            for e in chunk {
                let params: Vec<rusqlite::types::Value> =
                    event_cells(e).into_iter().map(to_sqlite).collect();
                match self
                    .conn
                    .execute(INSERT_EVENT, rusqlite::params_from_iter(params))
                {
                    Ok(n) => inserted += n,
                    Err(err) => {
                        let _ = self.conn.execute_batch("ROLLBACK");
                        return Err(sql_err(err));
                    }
                }
            }
            let commit = self.conn.execute_batch("COMMIT").map_err(sql_err);
            commit_or_rollback(commit, || {
                let _ = self.conn.execute_batch("ROLLBACK");
            })?;
        }
        Ok(inserted)
    }

    fn event_count(&mut self, machine_id: &str, source: &str) -> Result<usize, String> {
        let rows = self.query(
            COUNT_MACHINE_EVENTS,
            vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
            1,
        )?;
        match rows.first().map(|row| &row[0]) {
            Some(Cell::Int(n)) => usize::try_from(*n).map_err(|_| "negative event count".into()),
            _ => Err("invalid event count".into()),
        }
    }

    fn remote_ids(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<HashMap<i64, String>, String> {
        let rows = self.query(
            SELECT_REMOTE,
            vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
            2,
        )?;
        remote_map(&rows)
    }

    fn remote_payload(
        &mut self,
        machine_id: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.query(
            SELECT_PAYLOAD,
            vec![
                Cell::Text(source.into()),
                Cell::Text(machine_id.into()),
                Cell::Text(event_id.into()),
            ],
            1,
        )?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    fn event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Option<(i64, String)>, String> {
        let rows = self.query(
            SELECT_WATERMARK,
            vec![Cell::Text(machine_id.into()), Cell::Text(source.into())],
            2,
        )?;
        rows.first()
            .map(|r| Ok((int(&r[0])?, text(&r[1])?)))
            .transpose()
    }

    fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String> {
        for r in rows {
            self.exec(INSERT_ALIAS, alias_cells(r))?;
        }
        Ok(())
    }

    fn index_names(&mut self) -> Result<Vec<String>, String> {
        self.query(SELECT_INDEX_NAMES, vec![], 1)?
            .iter()
            .map(|row| match &row[0] {
                Cell::Text(name) => Ok(name.clone()),
                _ => Err("invalid index name".into()),
            })
            .collect()
    }
}

const SUBDIR: &str = "turso-measure";
const MEASURE_SCHEMA_VERSION: u32 = 2;

pub(crate) fn config_path() -> PathBuf {
    crate::data_dir::data_dir().join("turso-measure.json")
}

/// Whether `cort measure configure` has been run on this machine (the config file exists).
pub fn is_configured() -> bool {
    config_path().exists()
}

/// Store the measurement database's URL and token, private to this user.
pub fn configure(url: &str, token: &str) -> Result<PathBuf, String> {
    let config = memory_store::validated_config(url, token)?;
    memory_store::write_private_config(&config_path(), &config, "Turso measure config")
}

fn config() -> Result<Config, String> {
    let measure = memory_store::read_config(
        &config_path(),
        "Turso measure config",
        "Turso measurement is not configured; run `cort measure configure --url URL --token-stdin`",
    )?;
    let memory_path = memory_store::config_path();
    // A missing, unreadable, or malformed memory config cannot stop measurement. Only a
    // readable config can establish that these two stores name the same database.
    if let Ok(memory) = memory_store::read_config(&memory_path, "Turso memory config", "missing") {
        let identity = database_identity(&measure.url);
        if database_identity(&memory.url) == identity {
            return Err(format!(
                "the measure config names the memory database ({identity}); measurement needs its own database"
            ));
        }
    }
    Ok(measure)
}

fn with_config_lock<T>(action: impl FnOnce(Config) -> Result<T, String>) -> Result<T, String> {
    with_lock_for(config()?, action)
}

/// Like [`with_config_lock`] for a `Config` the caller already read, so a check made on that value
/// and the lock and open that follow cannot see two different files.
fn with_lock_for<T>(
    config: Config,
    action: impl FnOnce(Config) -> Result<T, String>,
) -> Result<T, String> {
    let _lock = memory_store::lock_in(SUBDIR, &config, "measure.lock")?;
    action(config)
}

/// Hold the exclusive `measure.lock` for the configured database while `action` runs.
pub fn with_measure_lock<T>(action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    with_config_lock(|_| action())
}

#[derive(Debug)]
pub enum OpenError {
    Busy,
    Config(String),
    /// Opening the remote database ran out of its network limit.
    Network(String),
    Other(String),
}

/// Open the configured remote database while holding `measure.lock` for the returned store's lifetime,
/// bounding each network operation by the configured `CORT_COLLECT_NET_TIMEOUT_SECS`.
pub fn open_measure_store_within(wait: Duration) -> Result<Box<dyn MeasureStore>, OpenError> {
    open_measure_store_within_timeout(wait, default_net_timeout())
}

/// [`open_measure_store_within`] with the caller's network limit, so a collect run bounds the
/// schema setup by the same value the run's own configuration names.
pub fn open_measure_store_within_timeout(
    wait: Duration,
    net_timeout: Duration,
) -> Result<Box<dyn MeasureStore>, OpenError> {
    let config = config().map_err(OpenError::Config)?;
    let lock =
        memory_store::lock_in_within(SUBDIR, &config, "measure.lock", wait).map_err(|error| {
            match error {
                memory_store::LockWait::TimedOut => OpenError::Busy,
                memory_store::LockWait::Failed(message) => OpenError::Other(message),
            }
        })?;
    let mut store = RemoteStore::open(&config, net_timeout).map_err(open_error)?;
    store._lock = Some(lock);
    Ok(Box::new(store))
}

/// The network bound for paths outside a collect run (such as `report`):
/// the same configured default for every HTTP request.
fn default_net_timeout() -> Duration {
    Duration::from_millis(crate::collect::CollectConfig::from_env().net_timeout_ms)
}

/// A schema request that timed out or found the remote unavailable is a network failure; other
/// open failures stay `Other`.
fn open_error(message: String) -> OpenError {
    if network_failure(&message) {
        OpenError::Network(message)
    } else {
        OpenError::Other(message)
    }
}

/// Direct Hrana store for the measurement database.
pub struct RemoteStore {
    db: RemoteDb,
    net_timeout: Duration,
    _lock: Option<memory_store::StoreLock>,
}

impl RemoteStore {
    fn connect(
        config: &Config,
        net_timeout: Duration,
        transport: Box<dyn Transport>,
    ) -> Result<Self, String> {
        let db = RemoteDb::new(&config.url, &config.token, transport).map_err(|e| e.to_string())?;
        Ok(Self {
            db,
            net_timeout,
            _lock: None,
        })
    }

    fn open(config: &Config, net_timeout: Duration) -> Result<Self, String> {
        let marker = memory_store::lock_dir(SUBDIR, config)
            .join(format!("schema-v{MEASURE_SCHEMA_VERSION}.ok"));
        Self::open_with_marker(
            config,
            net_timeout,
            Box::new(UreqTransport {
                timeout: net_timeout,
            }),
            &marker,
        )
    }

    fn open_with_marker(
        config: &Config,
        net_timeout: Duration,
        transport: Box<dyn Transport>,
        marker: &Path,
    ) -> Result<Self, String> {
        let store = Self::connect(config, net_timeout, transport)?;
        if !marker.exists() {
            store.initialize_schema()?;
            if let Some(parent) = marker.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let temporary = marker.with_extension("tmp");
            fs::write(&temporary, b"ok\n").map_err(|e| e.to_string())?;
            fs::rename(&temporary, marker).map_err(|e| e.to_string())?;
        }
        Ok(store)
    }

    fn open_read_only(config: &Config, net_timeout: Duration) -> Result<Self, String> {
        Self::open_read_only_with_transport(
            config,
            net_timeout,
            Box::new(UreqTransport {
                timeout: net_timeout,
            }),
        )
    }

    fn open_read_only_with_transport(
        config: &Config,
        net_timeout: Duration,
        transport: Box<dyn Transport>,
    ) -> Result<Self, String> {
        Self::connect(config, net_timeout, transport)
    }

    /// Test seam for exercising the same SQL through an injected Hrana transport.
    #[doc(hidden)]
    pub fn with_transport(
        url: &str,
        token: &str,
        transport: Box<dyn Transport>,
        create_schema: bool,
    ) -> Result<Self, String> {
        let config = Config {
            url: url.into(),
            token: token.into(),
        };
        if create_schema {
            let store = Self::connect(&config, default_net_timeout(), transport)?;
            store.initialize_schema()?;
            Ok(store)
        } else {
            Self::open_read_only_with_transport(&config, default_net_timeout(), transport)
        }
    }

    fn initialize_schema(&self) -> Result<(), String> {
        for statement in MEASURE_SCHEMA {
            self.exec(statement, vec![])?;
        }
        Ok(())
    }

    fn error(&self, error: RemoteError) -> String {
        if error.kind == ErrorKind::Timeout {
            format!(
                "network timeout after {}s ({error})",
                self.net_timeout.as_secs()
            )
        } else if matches!(
            error.kind,
            ErrorKind::Blocked | ErrorKind::Unauthorized | ErrorKind::Unreachable | ErrorKind::Busy
        ) {
            format!("{REMOTE_PREFIX}{error}")
        } else {
            error.to_string()
        }
    }

    fn exec(&self, sql: &str, params: Vec<Cell>) -> Result<(), String> {
        self.db.execute(sql, params).map_err(|e| self.error(e))?;
        Ok(())
    }

    fn query(&self, sql: &str, params: Vec<Cell>, cols: usize) -> Result<Vec<Vec<Cell>>, String> {
        let rows = self
            .db
            .execute(sql, params)
            .map_err(|e| self.error(e))?
            .rows;
        if rows.iter().any(|row| row.len() != cols) {
            return Err(format!("expected {cols} columns in remote row"));
        }
        Ok(rows)
    }
}

impl MeasureStore for RemoteStore {
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String> {
        self.exec(UPSERT_COMPACTION, compaction_cells(row))
    }

    fn compaction_owners(&mut self, machine_id: &str) -> Result<Vec<(String, String)>, String> {
        self.query(
            SELECT_COMPACTION_OWNERS,
            vec![Cell::Text(machine_id.into())],
            2,
        )?
        .iter()
        .map(|r| Ok((text(&r[0])?, text(&r[1])?)))
        .collect()
    }

    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String> {
        self.query(&select_since(), vec![Cell::Int(since_ms)], 21)?
            .iter()
            .map(|r| compaction_from_cells(r))
            .collect()
    }

    fn gate_rows_since(&mut self, since_ms: i64) -> Result<Vec<GateRow>, String> {
        self.query(SELECT_GATE_ROWS, vec![Cell::Int(since_ms)], 5)?
            .iter()
            .map(|r| gate_from_cells(r))
            .collect()
    }

    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String> {
        self.exec(
            UPSERT_HOST,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(host.into()),
                Cell::Int(seen_ms),
            ],
        )
    }

    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String> {
        self.query(SELECT_HOSTS, vec![], 3)?
            .iter()
            .map(|r| host_from_cells(r))
            .collect()
    }

    fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String> {
        let mut inserted = 0usize;
        for chunk in events.chunks(EVENT_CHUNK) {
            let stmts: Vec<_> = chunk
                .iter()
                .map(|event| Stmt::new(INSERT_EVENT, event_cells(event)))
                .collect();
            let results = self
                .db
                .transaction(&stmts)
                .map_err(|error| self.error(error))?;
            inserted += results
                .iter()
                .map(|result| result.affected_row_count as usize)
                .sum::<usize>();
        }
        Ok(inserted)
    }

    fn event_count(&mut self, machine_id: &str, source: &str) -> Result<usize, String> {
        let rows = self.query(
            COUNT_MACHINE_EVENTS,
            vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
            1,
        )?;
        match rows.first().map(|row| &row[0]) {
            Some(Cell::Int(n)) => usize::try_from(*n).map_err(|_| "negative event count".into()),
            _ => Err("invalid event count".into()),
        }
    }

    fn remote_ids(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<HashMap<i64, String>, String> {
        let rows = self.query(
            SELECT_REMOTE,
            vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
            2,
        )?;
        remote_map(&rows)
    }

    fn remote_payload(
        &mut self,
        machine_id: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.query(
            SELECT_PAYLOAD,
            vec![
                Cell::Text(source.into()),
                Cell::Text(machine_id.into()),
                Cell::Text(event_id.into()),
            ],
            1,
        )?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    fn event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Option<(i64, String)>, String> {
        let rows = self.query(
            SELECT_WATERMARK,
            vec![Cell::Text(machine_id.into()), Cell::Text(source.into())],
            2,
        )?;
        rows.first()
            .map(|r| Ok((int(&r[0])?, text(&r[1])?)))
            .transpose()
    }

    fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String> {
        for r in rows {
            self.exec(INSERT_ALIAS, alias_cells(r))?;
        }
        Ok(())
    }

    fn index_names(&mut self) -> Result<Vec<String>, String> {
        self.query(SELECT_INDEX_NAMES, vec![], 1)?
            .iter()
            .map(|row| match &row[0] {
                Cell::Text(name) => Ok(name.clone()),
                _ => Err("invalid index name".into()),
            })
            .collect()
    }
}

/// The one remote read `measure status` makes: read-only, so it never creates the schema it is
/// checking for, and without `measure.lock`, because a read needs no exclusion and a status must
/// not wait behind a running collection. The network bound is at most `STATUS_NET_TIMEOUT`.
pub fn read_measure_store<T>(
    f: impl FnOnce(&mut dyn MeasureStore) -> Result<T, String>,
) -> Result<T, String> {
    let config = config()?;
    let mut store =
        RemoteStore::open_read_only(&config, default_net_timeout().min(STATUS_NET_TIMEOUT))?;
    f(&mut store)
}

const STATUS_NET_TIMEOUT: Duration = Duration::from_secs(10);

pub fn with_turso_store<T>(
    f: impl FnOnce(&mut dyn MeasureStore) -> Result<T, String>,
) -> Result<T, String> {
    with_config_lock(|config| {
        let mut store = RemoteStore::open(&config, default_net_timeout())?;
        f(&mut store)
    })
}

/// The database a Turso URL names: its host, lower-cased, without scheme, userinfo, port, path,
/// query, fragment or trailing slash. Ports are dropped deliberately: URLs differing only by
/// port are treated as one database, the safe direction for the memory database guard. This
/// identity also tracks the database in notes-uploaded.json. A bracketed IPv6 host keeps its
/// brackets.
pub fn database_identity(url: &str) -> String {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port
            .find(']')
            .map_or(host_port, |end| &host_port[..=end])
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_marker_follows_successful_ddl_and_skips_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("schema-v2.ok");
        let config = Config {
            url: "libsql://test.invalid".into(),
            token: "test".into(),
        };
        let failing = crate::remote::SqliteTransport::new().unwrap();
        failing.fail_next_status(500);
        assert!(RemoteStore::open_with_marker(
            &config,
            Duration::from_secs(1),
            Box::new(failing),
            &marker
        )
        .is_err());
        assert!(!marker.exists());
        let working = crate::remote::SqliteTransport::new().unwrap();
        let mut store = RemoteStore::open_with_marker(
            &config,
            Duration::from_secs(1),
            Box::new(working),
            &marker,
        )
        .unwrap();
        assert!(marker.exists());
        assert!(store.event_count("machine", "test").is_ok());
        let fresh = crate::remote::SqliteTransport::new().unwrap();
        let mut skipped = RemoteStore::open_with_marker(
            &config,
            Duration::from_secs(1),
            Box::new(fresh),
            &marker,
        )
        .unwrap();
        assert!(skipped.event_count("machine", "test").is_err());
    }

    #[test]
    fn unavailable_remote_opens_as_network() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            url: "libsql://test.invalid".into(),
            token: "test".into(),
        };
        for status in [403, 401, 500, 503] {
            let failing = crate::remote::SqliteTransport::new().unwrap();
            failing.fail_next_status(status);
            let error = RemoteStore::open_with_marker(
                &config,
                Duration::from_secs(1),
                Box::new(failing),
                &dir.path().join("new/schema.ok"),
            )
            .err()
            .unwrap();
            assert!(
                matches!(open_error(error), OpenError::Network(_)),
                "status {status}"
            );
        }
        // The quota block (403 with a BLOCKED code) and a busy remote are network failures too.
        for code in ["BLOCKED", "SQLITE_BUSY"] {
            let failing = crate::remote::SqliteTransport::new().unwrap();
            failing.fail_next_code(code);
            let error = RemoteStore::open_with_marker(
                &config,
                Duration::from_secs(1),
                Box::new(failing),
                &dir.path().join("new/schema.ok"),
            )
            .err()
            .unwrap();
            assert!(
                matches!(open_error(error), OpenError::Network(_)),
                "code {code}"
            );
        }
        let failing = crate::remote::SqliteTransport::new().unwrap();
        failing.fail_next_status(418);
        let error = RemoteStore::open_with_marker(
            &config,
            Duration::from_secs(1),
            Box::new(failing),
            &dir.path().join("new/schema.ok"),
        )
        .err()
        .unwrap();
        assert!(matches!(open_error(error), OpenError::Other(_)));
        let working = crate::remote::SqliteTransport::new().unwrap();
        RemoteStore::open_with_marker(
            &config,
            Duration::from_secs(1),
            Box::new(working),
            &dir.path().join("new/schema.ok"),
        )
        .unwrap();
        assert!(dir.path().join("new/schema.ok").exists());
    }

    #[test]
    fn growing_event_queries_use_indexes() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for ddl in MEASURE_SCHEMA {
            conn.execute_batch(ddl).unwrap();
        }
        for (name, sql, args) in [
            ("SELECT_REMOTE", SELECT_REMOTE, 2),
            ("SELECT_GATE_ROWS", SELECT_GATE_ROWS, 1),
        ] {
            let explain = format!("EXPLAIN QUERY PLAN {sql}");
            let mut statement = conn.prepare(&explain).unwrap();
            let params = vec![rusqlite::types::Value::Null; args];
            let lines: Vec<String> = statement
                .query_map(rusqlite::params_from_iter(params), |row| row.get(3))
                .unwrap()
                .map(|result| result.unwrap())
                .collect();
            for line in &lines {
                println!("{name}: {line}");
                assert!(
                    !line.contains("SCAN telemetry_events") || line.contains("USING"),
                    "unbounded event scan: {name}: {line}"
                );
            }
        }
    }

    #[test]
    fn commit_failure_rolls_back_and_returns_the_commit_error() {
        let mut rolled_back = false;
        let r = commit_or_rollback(Err("commit boom".into()), || rolled_back = true);
        assert_eq!(r, Err("commit boom".to_string()));
        assert!(rolled_back);
        let mut untouched = true;
        assert_eq!(commit_or_rollback(Ok(()), || untouched = false), Ok(()));
        assert!(untouched);
    }

    /// URL spellings for the same host produce the same database identity.
    #[test]
    fn database_identity_ignores_scheme_case_slash_and_path() {
        assert_eq!(
            database_identity("libsql://x.turso.io"),
            database_identity("turso://X.turso.io/")
        );
        assert_eq!(
            database_identity("https://x.turso.io/v2/pipeline?a=1"),
            database_identity("libsql://x.TURSO.io")
        );
        assert_ne!(
            database_identity("libsql://x.turso.io"),
            database_identity("libsql://y.turso.io")
        );
        assert_eq!(
            database_identity(" turso://Db-Org.turso.io/ "),
            "db-org.turso.io"
        );
        assert_eq!(
            database_identity("https://x.turso.io"),
            database_identity("https://x.turso.io:443")
        );
        assert_eq!(
            database_identity("https://u:p@x.turso.io:8080/p"),
            database_identity("libsql://X.turso.io")
        );
        assert_eq!(database_identity("https://[::1]:8080/p"), "[::1]");
        assert_eq!(
            database_identity("http://[::1]:1"),
            database_identity("http://[::1]:2/x")
        );
        assert_ne!(
            database_identity("http://[::1]"),
            database_identity("http://[::2]")
        );
    }
}
