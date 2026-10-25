//! Compaction measurement rows, stored in a Turso database of their own.
//!
//! Two implementations of one trait: [`SqliteStore`] is a plain rusqlite file (tests, and any
//! caller that wants the logic without a network), and the Turso store behind
//! [`with_turso_store`] is a sync replica that must only be opened through the `turso` crate.
//! Both run the same [`MEASURE_SCHEMA`] and the same SQL text.

use crate::audit_import::{ImportedUsage, ImportedUsageConflict, ImportedUsageOutcome};
use crate::memory_store::{self, Config};
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

pub trait MeasureStore {
    /// Insert, or replace the row with the same `(machine_id, boundary_uuid)`.
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String>;
    /// The session that already owns this boundary, if any.
    fn compaction_session(
        &mut self,
        machine_id: &str,
        boundary_uuid: &str,
    ) -> Result<Option<String>, String>;
    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String>;
    /// Record that `host` was seen for `machine_id`; the latest `seen_ms` wins.
    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String>;
    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String>;
    /// Insert each row whose `(day, host)` is absent. An existing row is never overwritten:
    /// identical -> unchanged, different -> returned as a conflict `(existing, incoming)`.
    fn upsert_imported_usage(
        &mut self,
        rows: &[ImportedUsage],
    ) -> Result<ImportedUsageOutcome, String>;
    /// Every imported usage row, ordered by `(day, host)`.
    fn imported_usage(&mut self) -> Result<Vec<ImportedUsage>, String>;
    /// `INSERT OR IGNORE` each event; returns how many were new. One transaction per chunk.
    fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String>;
    /// Local rowid (the payload's `id`) -> event_id for what the store holds for this machine.
    fn remote_ids(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<HashMap<i64, String>, String>;
    /// The stored payload of one event, for a mismatch report.
    fn remote_payload(
        &mut self,
        machine_id: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<String>, String>;
    /// Payloads of this machine's stored events whose `time_ms` lies in `[min_ms, max_ms]`.
    /// Payloads of this machine's stored events that have no `time_ms`.
    fn remote_payloads_without_time(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Vec<String>, String>;
    fn remote_payloads_in_range(
        &mut self,
        machine_id: &str,
        source: &str,
        min_ms: i64,
        max_ms: i64,
    ) -> Result<Vec<String>, String>;
    fn event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Option<(i64, String)>, String>;
    /// Replace the watermark: the highest local rowid uploaded and a fingerprint of that row's raw
    /// `ts` and `command`. The fingerprint lets the next run notice that `usage.db` was replaced
    /// under the same rowids; it ignores the column set, so a schema change does not trip it.
    fn set_event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
        max_rowid: i64,
        row_fingerprint: &str,
    ) -> Result<(), String>;
    fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String>;
    /// Every alias row as `(machine_id, local_project_id, shared_project_id)`.
    fn aliases(&mut self) -> Result<Vec<(String, String, String)>, String>;
    /// Up to `limit` events with `(source, event_id)` strictly after `after`, ascending (keyset
    /// pagination on the primary key). `rowid` is `None`: a stored row has no local rowid.
    fn events_page(
        &mut self,
        after: Option<(String, String)>,
        limit: usize,
    ) -> Result<Vec<TelemetryEvent>, String>;
    /// Every stored event whose `(source, event_id)` lies between `first` and `last` inclusive,
    /// in the same key order `events_page` uses; there may be rows the caller did not ask about.
    fn events_in_key_range(
        &mut self,
        first: &(String, String),
        last: &(String, String),
    ) -> Result<Vec<TelemetryEvent>, String>;
    /// `(source, machine_id, rows)` for every group, counted by the database.
    fn event_counts(&mut self) -> Result<Vec<(String, String, usize)>, String>;
    fn push(&mut self) -> Result<(), String>;
    fn pull(&mut self) -> Result<(), String>;
}

/// Every CREATE statement; both implementations run all of them on open.
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
    // Highest local rowid already uploaded, per machine and source: what keeps the hourly sync from
    // reading the whole remote table.
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
const SELECT_SESSION: &str =
    "SELECT session_id FROM compactions WHERE machine_id = ? AND boundary_uuid = ?";
const UPSERT_HOST: &str = "INSERT INTO machine_hosts (machine_id, host, last_seen_ms) \
    VALUES (?, ?, ?) ON CONFLICT (machine_id, host) DO UPDATE SET \
    last_seen_ms = MAX(last_seen_ms, excluded.last_seen_ms)";
const SELECT_HOSTS: &str = "SELECT machine_id, host, last_seen_ms FROM machine_hosts";
const INSERT_EVENT: &str = "INSERT OR IGNORE INTO telemetry_events (source, event_id, machine_id, \
    machine_source, project_id, time_ms, payload) VALUES (?, ?, ?, ?, ?, ?, ?)";
const SELECT_REMOTE: &str =
    "SELECT event_id, CASE WHEN json_valid(payload) THEN json_extract(payload, '$.id') END \
     FROM telemetry_events WHERE source = ? AND machine_id = ?";
const SELECT_RANGE: &str =
    "SELECT payload FROM telemetry_events WHERE source = ? AND machine_id = ? \
    AND time_ms >= ? AND time_ms <= ?";
const SELECT_NO_TIME: &str =
    "SELECT payload FROM telemetry_events WHERE source = ? AND machine_id = ? \
    AND time_ms IS NULL";
const SELECT_PAYLOAD: &str =
    "SELECT payload FROM telemetry_events WHERE source = ? AND machine_id = ? AND event_id = ?";
const SELECT_WATERMARK: &str =
    "SELECT max_rowid, row_fingerprint FROM event_watermarks WHERE machine_id = ? AND source = ?";
const UPSERT_WATERMARK: &str =
    "INSERT INTO event_watermarks (machine_id, source, max_rowid, row_fingerprint) \
    VALUES (?, ?, ?, ?) ON CONFLICT (machine_id, source) DO UPDATE SET \
    max_rowid = excluded.max_rowid, row_fingerprint = excluded.row_fingerprint";
const INSERT_ALIAS: &str = "INSERT OR IGNORE INTO telemetry_project_aliases \
    (machine_id, local_project_id, shared_project_id) VALUES (?, ?, ?)";
const SELECT_ALIASES: &str =
    "SELECT machine_id, local_project_id, shared_project_id FROM telemetry_project_aliases";
const SELECT_EVENTS_FIRST_PAGE: &str = "SELECT source, event_id, machine_id, machine_source, \
    project_id, time_ms, payload FROM telemetry_events ORDER BY source, event_id LIMIT ?";
const SELECT_EVENTS_AFTER: &str = "SELECT source, event_id, machine_id, machine_source, \
    project_id, time_ms, payload FROM telemetry_events \
    WHERE source > ? OR (source = ? AND event_id > ?) ORDER BY source, event_id LIMIT ?";
const SELECT_EVENTS_IN_RANGE: &str = "SELECT source, event_id, machine_id, machine_source, \
    project_id, time_ms, payload FROM telemetry_events \
    WHERE (source > ? OR (source = ? AND event_id >= ?)) \
    AND (source < ? OR (source = ? AND event_id <= ?)) ORDER BY source, event_id";
const COUNT_EVENTS: &str = "SELECT source, machine_id, COUNT(*) FROM telemetry_events \
    GROUP BY source, machine_id ORDER BY source, machine_id";
const SELECT_IMPORTED_USAGE: &str =
    "SELECT day, host, commands_window, core_window, deep_window, commands_7d, deep_7d, \
     decline_top, window_days FROM imported_usage ORDER BY day, host";
const INSERT_IMPORTED_USAGE: &str = "INSERT INTO imported_usage (day, host, commands_window, \
    core_window, deep_window, commands_7d, deep_7d, decline_top, window_days) \
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";
/// Events per transaction: a commit per row is minutes on a 130k-row first upload.
const EVENT_CHUNK: usize = 2000;

fn select_since() -> String {
    format!("SELECT {COMPACTION_COLUMNS} FROM compactions WHERE time_ms >= ? ORDER BY time_ms, machine_id, boundary_uuid")
}

/// A column value neutral between rusqlite and turso, so row <-> SQL mapping is written once.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Text(String),
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

fn alias_from_cells(c: &[Cell]) -> Result<(String, String, String), String> {
    Ok((text(&c[0])?, text(&c[1])?, text(&c[2])?))
}

fn imported_usage_cells(u: &ImportedUsage) -> Vec<Cell> {
    vec![
        Cell::Text(u.day.clone()),
        Cell::Text(u.host.clone()),
        opt_int(u.commands_window),
        opt_int(u.core_window),
        opt_int(u.deep_window),
        opt_int(u.commands_7d),
        opt_int(u.deep_7d),
        u.decline_top.clone().map_or(Cell::Null, Cell::Text),
        Cell::Int(u.window_days),
    ]
}

fn imported_usage_from_cells(c: &[Cell]) -> Result<ImportedUsage, String> {
    Ok(ImportedUsage {
        day: text(&c[0])?,
        host: text(&c[1])?,
        commands_window: opt_i(&c[2])?,
        core_window: opt_i(&c[3])?,
        deep_window: opt_i(&c[4])?,
        commands_7d: opt_i(&c[5])?,
        deep_7d: opt_i(&c[6])?,
        decline_top: match &c[7] {
            Cell::Null => None,
            other => Some(text(other)?),
        },
        window_days: int(&c[8])?,
    })
}

fn event_from_cells(c: &[Cell]) -> Result<TelemetryEvent, String> {
    Ok(TelemetryEvent {
        source: text(&c[0])?,
        event_id: text(&c[1])?,
        machine_id: text(&c[2])?,
        machine_source: text(&c[3])?,
        project_id: match &c[4] {
            Cell::Null => None,
            other => Some(text(other)?),
        },
        time_ms: opt_i(&c[5])?,
        payload: text(&c[6])?,
        rowid: None,
    })
}

fn events_page_query(after: Option<(String, String)>, limit: usize) -> (&'static str, Vec<Cell>) {
    let limit = Cell::Int(limit as i64);
    match after {
        None => (SELECT_EVENTS_FIRST_PAGE, vec![limit]),
        Some((source, event_id)) => (
            SELECT_EVENTS_AFTER,
            vec![
                Cell::Text(source.clone()),
                Cell::Text(source),
                Cell::Text(event_id),
                limit,
            ],
        ),
    }
}

fn range_key_params(first: &(String, String), last: &(String, String)) -> Vec<Cell> {
    let t = |s: &String| Cell::Text(s.clone());
    vec![
        t(&first.0),
        t(&first.0),
        t(&first.1),
        t(&last.0),
        t(&last.0),
        t(&last.1),
    ]
}

fn count_from_cells(c: &[Cell]) -> Result<(String, String, usize), String> {
    Ok((text(&c[0])?, text(&c[1])?, int(&c[2])? as usize))
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

fn range_params(machine_id: &str, source: &str, min_ms: i64, max_ms: i64) -> Vec<Cell> {
    vec![
        Cell::Text(source.into()),
        Cell::Text(machine_id.into()),
        Cell::Int(min_ms),
        Cell::Int(max_ms),
    ]
}

fn alias_cells(r: &(String, String, String)) -> Vec<Cell> {
    vec![
        Cell::Text(r.0.clone()),
        Cell::Text(r.1.clone()),
        Cell::Text(r.2.clone()),
    ]
}

/// A plain SQLite file implementing the store; `push`/`pull` have nothing to sync.
pub struct SqliteStore {
    conn: rusqlite::Connection,
    fail_push: bool,
    fail_pull: bool,
    pushes: usize,
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
            fail_push: false,
            fail_pull: false,
            pushes: 0,
        })
    }

    /// Test hook: make `push` return an error while keeping local data.
    pub fn fail_push(&mut self, fail: bool) {
        self.fail_push = fail;
    }

    /// Test hook: make `pull` return an error.
    pub fn fail_pull(&mut self, fail: bool) {
        self.fail_pull = fail;
    }

    /// Test hook: how many times `push` was called (failed or not).
    pub fn push_calls(&self) -> usize {
        self.pushes
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

    fn compaction_session(
        &mut self,
        machine_id: &str,
        boundary_uuid: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.query(
            SELECT_SESSION,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(boundary_uuid.into()),
            ],
            1,
        )?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String> {
        self.query(&select_since(), vec![Cell::Int(since_ms)], 21)?
            .iter()
            .map(|r| compaction_from_cells(r))
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

    fn upsert_imported_usage(
        &mut self,
        rows: &[ImportedUsage],
    ) -> Result<ImportedUsageOutcome, String> {
        let mut known: HashMap<(String, String), ImportedUsage> = self
            .query(SELECT_IMPORTED_USAGE, vec![], 9)?
            .iter()
            .map(|r| imported_usage_from_cells(r))
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .map(|u| ((u.day.clone(), u.host.clone()), u))
            .collect();
        let mut outcome = ImportedUsageOutcome::default();
        for row in rows {
            let key = (row.day.clone(), row.host.clone());
            if let Some(existing) = known.get(&key) {
                if existing == row {
                    outcome.unchanged += 1;
                } else {
                    outcome.conflicts.push(ImportedUsageConflict {
                        existing: existing.clone(),
                        incoming: row.clone(),
                    });
                }
                continue;
            }
            self.exec(INSERT_IMPORTED_USAGE, imported_usage_cells(row))?;
            known.insert(key, row.clone());
            outcome.inserted += 1;
        }
        Ok(outcome)
    }

    fn imported_usage(&mut self) -> Result<Vec<ImportedUsage>, String> {
        self.query(SELECT_IMPORTED_USAGE, vec![], 9)?
            .iter()
            .map(|r| imported_usage_from_cells(r))
            .collect()
    }

    fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String> {
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

    /// Payloads of this machine's stored events whose `time_ms` lies in `[min_ms, max_ms]`.
    /// Payloads of this machine's stored events that have no `time_ms`.
    fn remote_payloads_without_time(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Vec<String>, String> {
        self.query(
            SELECT_NO_TIME,
            vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
            1,
        )?
        .iter()
        .map(|r| text(&r[0]))
        .collect()
    }

    fn remote_payloads_in_range(
        &mut self,
        machine_id: &str,
        source: &str,
        min_ms: i64,
        max_ms: i64,
    ) -> Result<Vec<String>, String> {
        self.query(
            SELECT_RANGE,
            range_params(machine_id, source, min_ms, max_ms),
            1,
        )?
        .iter()
        .map(|r| text(&r[0]))
        .collect()
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

    fn set_event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
        max_rowid: i64,
        row_fingerprint: &str,
    ) -> Result<(), String> {
        self.exec(
            UPSERT_WATERMARK,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(source.into()),
                Cell::Int(max_rowid),
                Cell::Text(row_fingerprint.into()),
            ],
        )
    }

    fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String> {
        for r in rows {
            self.exec(INSERT_ALIAS, alias_cells(r))?;
        }
        Ok(())
    }

    fn aliases(&mut self) -> Result<Vec<(String, String, String)>, String> {
        self.query(SELECT_ALIASES, vec![], 3)?
            .iter()
            .map(|r| alias_from_cells(r))
            .collect()
    }

    fn events_page(
        &mut self,
        after: Option<(String, String)>,
        limit: usize,
    ) -> Result<Vec<TelemetryEvent>, String> {
        let (sql, params) = events_page_query(after, limit);
        self.query(sql, params, 7)?
            .iter()
            .map(|r| event_from_cells(r))
            .collect()
    }

    fn events_in_key_range(
        &mut self,
        first: &(String, String),
        last: &(String, String),
    ) -> Result<Vec<TelemetryEvent>, String> {
        self.query(SELECT_EVENTS_IN_RANGE, range_key_params(first, last), 7)?
            .iter()
            .map(|r| event_from_cells(r))
            .collect()
    }

    fn event_counts(&mut self) -> Result<Vec<(String, String, usize)>, String> {
        self.query(COUNT_EVENTS, vec![], 3)?
            .iter()
            .map(|r| count_from_cells(r))
            .collect()
    }

    fn push(&mut self) -> Result<(), String> {
        self.pushes += 1;
        if self.fail_push {
            return Err("push failed (fail_push set)".into());
        }
        Ok(())
    }

    fn pull(&mut self) -> Result<(), String> {
        if self.fail_pull {
            return Err("pull failed (fail_pull set)".into());
        }
        Ok(())
    }
}

const SUBDIR: &str = "turso-measure";

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
    memory_store::read_config(
        &config_path(),
        "Turso measure config",
        "Turso measurement is not configured; run `cort measure configure --url URL --token-stdin`",
    )
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
    Other(String),
}

/// Open the configured replica while holding `measure.lock` for the returned store's lifetime.
pub fn open_measure_store_within(wait: Duration) -> Result<Box<dyn MeasureStore>, OpenError> {
    let config = config().map_err(OpenError::Other)?;
    let lock =
        memory_store::lock_in_within(SUBDIR, &config, "measure.lock", wait).map_err(|error| {
            match error {
                memory_store::LockWait::TimedOut => OpenError::Busy,
                memory_store::LockWait::Failed(message) => OpenError::Other(message),
            }
        })?;
    let mut store = TursoStore::open(&config).map_err(OpenError::Other)?;
    store._lock = Some(lock);
    Ok(Box::new(store))
}

/// The Turso sync replica as a [`MeasureStore`]. `push`/`pull` talk to the network.
struct TursoStore {
    rt: tokio::runtime::Runtime,
    db: turso::sync::Database,
    conn: turso::Connection,
    // Last field drops last, after the connection and runtime have finished using the replica.
    _lock: Option<fs::File>,
}

fn tv(c: Cell) -> turso::Value {
    match c {
        Cell::Null => turso::Value::Null,
        Cell::Int(i) => turso::Value::Integer(i),
        Cell::Text(s) => turso::Value::Text(s),
    }
}

fn te(e: turso::Error) -> String {
    e.to_string()
}

async fn turso_exec(conn: &turso::Connection, sql: &str, params: Vec<Cell>) -> Result<(), String> {
    conn.execute(sql, turso::params_from_iter(params.into_iter().map(tv)))
        .await
        .map_err(te)?;
    Ok(())
}

async fn turso_query(
    conn: &turso::Connection,
    sql: &str,
    params: Vec<Cell>,
    cols: usize,
) -> Result<Vec<Vec<Cell>>, String> {
    let mut rows = conn
        .query(sql, turso::params_from_iter(params.into_iter().map(tv)))
        .await
        .map_err(te)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(te)? {
        let mut cells = Vec::with_capacity(cols);
        for i in 0..cols {
            cells.push(match row.get_value(i).map_err(te)? {
                turso::Value::Null => Cell::Null,
                turso::Value::Integer(i) => Cell::Int(i),
                turso::Value::Text(t) => Cell::Text(t),
                other => return Err(format!("unexpected column type {other:?}")),
            });
        }
        out.push(cells);
    }
    Ok(out)
}

async fn turso_insert_events(
    conn: &turso::Connection,
    events: &[TelemetryEvent],
) -> Result<usize, String> {
    let mut inserted = 0usize;
    for chunk in events.chunks(EVENT_CHUNK) {
        conn.execute("BEGIN", ()).await.map_err(te)?;
        for e in chunk {
            let params = turso::params_from_iter(event_cells(e).into_iter().map(tv));
            match conn.execute(INSERT_EVENT, params).await {
                Ok(n) => inserted += n as usize,
                Err(err) => {
                    let _ = conn.execute("ROLLBACK", ()).await;
                    return Err(te(err));
                }
            }
        }
        if let Err(e) = conn.execute("COMMIT", ()).await {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(te(e));
        }
    }
    Ok(inserted)
}

impl TursoStore {
    fn open(config: &Config) -> Result<Self, String> {
        Self::open_replica(SUBDIR, config, "measure.db", true)
    }

    /// `create_schema = false` opens a replica of somebody else's database without issuing a
    /// single statement of ours against it.
    fn open_replica(
        subdir: &str,
        config: &Config,
        file: &str,
        create_schema: bool,
    ) -> Result<Self, String> {
        let rt = memory_store::runtime()?;
        let (db, conn) = rt.block_on(async {
            let db = memory_store::open_replica_in(subdir, config, file).await?;
            let conn = db.connect().await.map_err(te)?;
            if create_schema {
                for stmt in MEASURE_SCHEMA {
                    conn.execute(stmt, ()).await.map_err(te)?;
                }
            }
            Ok::<_, String>((db, conn))
        })?;
        Ok(Self {
            rt,
            db,
            conn,
            _lock: None,
        })
    }
}

impl MeasureStore for TursoStore {
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String> {
        self.rt.block_on(turso_exec(
            &self.conn,
            UPSERT_COMPACTION,
            compaction_cells(row),
        ))
    }

    fn compaction_session(
        &mut self,
        machine_id: &str,
        boundary_uuid: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.rt.block_on(turso_query(
            &self.conn,
            SELECT_SESSION,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(boundary_uuid.into()),
            ],
            1,
        ))?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String> {
        self.rt
            .block_on(turso_query(
                &self.conn,
                &select_since(),
                vec![Cell::Int(since_ms)],
                21,
            ))?
            .iter()
            .map(|r| compaction_from_cells(r))
            .collect()
    }

    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String> {
        self.rt.block_on(turso_exec(
            &self.conn,
            UPSERT_HOST,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(host.into()),
                Cell::Int(seen_ms),
            ],
        ))
    }

    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String> {
        self.rt
            .block_on(turso_query(&self.conn, SELECT_HOSTS, vec![], 3))?
            .iter()
            .map(|r| host_from_cells(r))
            .collect()
    }

    fn upsert_imported_usage(
        &mut self,
        rows: &[ImportedUsage],
    ) -> Result<ImportedUsageOutcome, String> {
        let known_rows =
            self.rt
                .block_on(turso_query(&self.conn, SELECT_IMPORTED_USAGE, vec![], 9))?;
        let mut known: HashMap<(String, String), ImportedUsage> = known_rows
            .iter()
            .map(|r| imported_usage_from_cells(r))
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .map(|u| ((u.day.clone(), u.host.clone()), u))
            .collect();
        let mut outcome = ImportedUsageOutcome::default();
        for row in rows {
            let key = (row.day.clone(), row.host.clone());
            if let Some(existing) = known.get(&key) {
                if existing == row {
                    outcome.unchanged += 1;
                } else {
                    outcome.conflicts.push(ImportedUsageConflict {
                        existing: existing.clone(),
                        incoming: row.clone(),
                    });
                }
                continue;
            }
            self.rt.block_on(turso_exec(
                &self.conn,
                INSERT_IMPORTED_USAGE,
                imported_usage_cells(row),
            ))?;
            known.insert(key, row.clone());
            outcome.inserted += 1;
        }
        Ok(outcome)
    }

    fn imported_usage(&mut self) -> Result<Vec<ImportedUsage>, String> {
        self.rt
            .block_on(turso_query(&self.conn, SELECT_IMPORTED_USAGE, vec![], 9))?
            .iter()
            .map(|r| imported_usage_from_cells(r))
            .collect()
    }

    fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String> {
        self.rt.block_on(turso_insert_events(&self.conn, events))
    }

    fn remote_ids(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<HashMap<i64, String>, String> {
        let rows = self.rt.block_on(turso_query(
            &self.conn,
            SELECT_REMOTE,
            vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
            2,
        ))?;
        remote_map(&rows)
    }

    fn remote_payload(
        &mut self,
        machine_id: &str,
        source: &str,
        event_id: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.rt.block_on(turso_query(
            &self.conn,
            SELECT_PAYLOAD,
            vec![
                Cell::Text(source.into()),
                Cell::Text(machine_id.into()),
                Cell::Text(event_id.into()),
            ],
            1,
        ))?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    /// Payloads of this machine's stored events whose `time_ms` lies in `[min_ms, max_ms]`.
    /// Payloads of this machine's stored events that have no `time_ms`.
    fn remote_payloads_without_time(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Vec<String>, String> {
        self.rt
            .block_on(turso_query(
                &self.conn,
                SELECT_NO_TIME,
                vec![Cell::Text(source.into()), Cell::Text(machine_id.into())],
                1,
            ))?
            .iter()
            .map(|r| text(&r[0]))
            .collect()
    }

    fn remote_payloads_in_range(
        &mut self,
        machine_id: &str,
        source: &str,
        min_ms: i64,
        max_ms: i64,
    ) -> Result<Vec<String>, String> {
        self.rt
            .block_on(turso_query(
                &self.conn,
                SELECT_RANGE,
                range_params(machine_id, source, min_ms, max_ms),
                1,
            ))?
            .iter()
            .map(|r| text(&r[0]))
            .collect()
    }

    fn event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Option<(i64, String)>, String> {
        let rows = self.rt.block_on(turso_query(
            &self.conn,
            SELECT_WATERMARK,
            vec![Cell::Text(machine_id.into()), Cell::Text(source.into())],
            2,
        ))?;
        rows.first()
            .map(|r| Ok((int(&r[0])?, text(&r[1])?)))
            .transpose()
    }

    fn set_event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
        max_rowid: i64,
        row_fingerprint: &str,
    ) -> Result<(), String> {
        self.rt.block_on(turso_exec(
            &self.conn,
            UPSERT_WATERMARK,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(source.into()),
                Cell::Int(max_rowid),
                Cell::Text(row_fingerprint.into()),
            ],
        ))
    }

    fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String> {
        for r in rows {
            self.rt
                .block_on(turso_exec(&self.conn, INSERT_ALIAS, alias_cells(r)))?;
        }
        Ok(())
    }

    fn aliases(&mut self) -> Result<Vec<(String, String, String)>, String> {
        self.rt
            .block_on(turso_query(&self.conn, SELECT_ALIASES, vec![], 3))?
            .iter()
            .map(|r| alias_from_cells(r))
            .collect()
    }

    fn events_page(
        &mut self,
        after: Option<(String, String)>,
        limit: usize,
    ) -> Result<Vec<TelemetryEvent>, String> {
        let (sql, params) = events_page_query(after, limit);
        self.rt
            .block_on(turso_query(&self.conn, sql, params, 7))?
            .iter()
            .map(|r| event_from_cells(r))
            .collect()
    }

    fn events_in_key_range(
        &mut self,
        first: &(String, String),
        last: &(String, String),
    ) -> Result<Vec<TelemetryEvent>, String> {
        self.rt
            .block_on(turso_query(
                &self.conn,
                SELECT_EVENTS_IN_RANGE,
                range_key_params(first, last),
                7,
            ))?
            .iter()
            .map(|r| event_from_cells(r))
            .collect()
    }

    fn event_counts(&mut self) -> Result<Vec<(String, String, usize)>, String> {
        self.rt
            .block_on(turso_query(&self.conn, COUNT_EVENTS, vec![], 3))?
            .iter()
            .map(|r| count_from_cells(r))
            .collect()
    }

    fn push(&mut self) -> Result<(), String> {
        self.rt.block_on(self.db.push()).map_err(te)
    }

    fn pull(&mut self) -> Result<(), String> {
        self.rt.block_on(self.db.pull()).map(|_| ()).map_err(te)
    }
}

/// Open the configured replica under `measure.lock` and run `f` against it.
pub fn with_turso_store<T>(
    f: impl FnOnce(&mut dyn MeasureStore) -> Result<T, String>,
) -> Result<T, String> {
    with_config_lock(|config| {
        let mut store = TursoStore::open(&config)?;
        f(&mut store)
    })
}

/// The database a Turso URL names: its host, lower-cased, without scheme, userinfo, port, path,
/// query, fragment or trailing slash. `libsql://x.turso.io` and `turso://X.turso.io/` are the same
/// database, and so are `https://x.turso.io` and `https://x.turso.io:443`. The port is dropped on
/// purpose: two databases that differ only by port are refused as one, which is the safe direction
/// for a guard whose job is to stop a copy writing into its own source. A bracketed IPv6 host
/// keeps its brackets.
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

/// Whether two configured URLs name the same database.
pub fn same_database(a: &str, b: &str) -> bool {
    database_identity(a) == database_identity(b)
}

/// Run `f(source, destination)` where `source` is a replica of the `cort memory` database
/// (`harnessuse`) kept in its own `legacy-telemetry.db`, and `destination` is the measure store.
/// The source has its own lock, so a copy does not block `cort memory`; neither `memory.db` nor
/// the source's schema is touched. Both configs are read before any file or lock is created.
pub fn with_legacy_and_measure_stores<T>(
    f: impl FnOnce(&mut dyn MeasureStore, &mut dyn MeasureStore) -> Result<T, String>,
) -> Result<T, String> {
    let memory_config = memory_store::load_config()?;
    let measure_config = config()?;
    if same_database(&memory_config.url, &measure_config.url) {
        return Err(format!(
            "the memory config and the measure config name the same database ({}); copying a \
             database into itself would write to the source, so nothing was opened",
            database_identity(&memory_config.url)
        ));
    }
    with_lock_for(measure_config, |config| {
        let _legacy_lock = memory_store::lock_in(
            memory_store::SUBDIR,
            &memory_config,
            "legacy-telemetry.lock",
        )?;
        let mut to = TursoStore::open(&config)?;
        let mut from = TursoStore::open_replica(
            memory_store::SUBDIR,
            &memory_config,
            "legacy-telemetry.db",
            false,
        )?;
        f(&mut from, &mut to)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The copy's paging and counting SQL must also run on Turso's engine.
    #[test]
    fn database_identity_ignores_scheme_case_slash_and_path() {
        assert!(same_database("libsql://x.turso.io", "turso://X.turso.io/"));
        assert!(same_database(
            "https://x.turso.io/v2/pipeline?a=1",
            "libsql://x.TURSO.io"
        ));
        assert!(!same_database("libsql://x.turso.io", "libsql://y.turso.io"));
        assert_eq!(
            database_identity(" turso://Db-Org.turso.io/ "),
            "db-org.turso.io"
        );
        assert!(same_database(
            "https://x.turso.io",
            "https://x.turso.io:443"
        ));
        assert!(same_database(
            "https://u:p@x.turso.io:8080/p",
            "libsql://X.turso.io"
        ));
        assert_eq!(database_identity("https://[::1]:8080/p"), "[::1]");
        assert!(same_database("http://[::1]:1", "http://[::1]:2/x"));
        assert!(!same_database("http://[::1]", "http://[::2]"));
    }

    #[test]
    fn copy_sql_runs_on_turso_local() {
        let rt = memory_store::runtime().unwrap();
        rt.block_on(async {
            let db = turso::Builder::new_local(":memory:").build().await.unwrap();
            let conn = db.connect().unwrap();
            for stmt in MEASURE_SCHEMA {
                conn.execute(stmt, ()).await.unwrap();
            }
            let mk = |source: &str, id: &str| TelemetryEvent {
                source: source.into(),
                event_id: id.into(),
                machine_id: "m".into(),
                machine_source: "env".into(),
                project_id: None,
                time_ms: None,
                payload: "{}".into(),
                rowid: None,
            };
            let events = [mk("a", "2"), mk("a", "1"), mk("b", "1")];
            assert_eq!(turso_insert_events(&conn, &events).await.unwrap(), 3);
            let (sql, params) = events_page_query(Some(("a".into(), "1".into())), 5);
            let rows = turso_query(&conn, sql, params, 7).await.unwrap();
            let got: Vec<_> = rows.iter().map(|r| event_from_cells(r).unwrap()).collect();
            assert_eq!(got, vec![mk("a", "2"), mk("b", "1")]);
            let range = turso_query(
                &conn,
                SELECT_EVENTS_IN_RANGE,
                range_key_params(&("a".into(), "2".into()), &("b".into(), "1".into())),
                7,
            )
            .await
            .unwrap();
            let range: Vec<_> = range.iter().map(|r| event_from_cells(r).unwrap()).collect();
            assert_eq!(range, vec![mk("a", "2"), mk("b", "1")]);
            let counts = turso_query(&conn, COUNT_EVENTS, vec![], 3).await.unwrap();
            let counts: Vec<_> = counts
                .iter()
                .map(|r| count_from_cells(r).unwrap())
                .collect();
            assert_eq!(
                counts,
                vec![
                    ("a".to_string(), "m".to_string(), 2),
                    ("b".to_string(), "m".to_string(), 1)
                ]
            );
        });
    }

    /// The shared SQL text must run on Turso's engine too, not only on SQLite.
    #[test]
    fn shared_sql_runs_on_turso_local() {
        let rt = memory_store::runtime().unwrap();
        rt.block_on(async {
            let db = turso::Builder::new_local(":memory:").build().await.unwrap();
            let conn = db.connect().unwrap();
            for stmt in MEASURE_SCHEMA {
                conn.execute(stmt, ()).await.unwrap();
            }
            let mk = |session: &str, t: i64| CompactionRow {
                machine_id: "m".into(),
                boundary_uuid: "b".into(),
                session_id: session.into(),
                time_ms: t,
                project_id: None,
                trigger: "auto".into(),
                pre_tokens: Some(5),
                post_tokens: None,
                pre: WindowCounts::default(),
                post: WindowCounts {
                    tool_calls: 3,
                    ..Default::default()
                },
                post_complete: true,
            };
            turso_exec(&conn, UPSERT_COMPACTION, compaction_cells(&mk("s1", 1)))
                .await
                .unwrap();
            turso_exec(&conn, UPSERT_COMPACTION, compaction_cells(&mk("s2", 2)))
                .await
                .unwrap();
            let rows = turso_query(&conn, &select_since(), vec![Cell::Int(0)], 21)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(compaction_from_cells(&rows[0]).unwrap(), mk("s2", 2));
            for seen in [500, 900, 700] {
                let p = vec![
                    Cell::Text("m".into()),
                    Cell::Text("h".into()),
                    Cell::Int(seen),
                ];
                turso_exec(&conn, UPSERT_HOST, p).await.unwrap();
            }
            let hosts = turso_query(&conn, SELECT_HOSTS, vec![], 3).await.unwrap();
            assert_eq!(host_from_cells(&hosts[0]).unwrap().last_seen_ms, 900);
            // Events: chunked transaction, INSERT OR IGNORE counting, watermark that never lowers.
            let ev = |id: &str, rowid: i64| TelemetryEvent {
                source: "cort_command".into(),
                event_id: id.into(),
                machine_id: "m".into(),
                machine_source: "env".into(),
                project_id: None,
                time_ms: Some(1),
                payload: format!("{{\"id\":{rowid}}}"),
                rowid: Some(rowid),
            };
            let batch = vec![ev("a", 1), ev("b", 2)];
            assert_eq!(turso_insert_events(&conn, &batch).await.unwrap(), 2);
            let mut junk = ev("j", 4);
            junk.payload = "not json".into(); // json_valid must keep it from failing the query
            let again = vec![ev("b", 2), ev("c", 3), junk];
            assert_eq!(turso_insert_events(&conn, &again).await.unwrap(), 2);
            let rows = turso_query(
                &conn,
                SELECT_REMOTE,
                vec![Cell::Text("cort_command".into()), Cell::Text("m".into())],
                2,
            )
            .await
            .unwrap();
            assert_eq!(remote_map(&rows).unwrap().len(), 3);
            let ranged = turso_query(
                &conn,
                SELECT_RANGE,
                range_params("m", "cort_command", 0, 5),
                1,
            )
            .await
            .unwrap();
            assert_eq!(ranged.len(), 4);
            let outside = turso_query(
                &conn,
                SELECT_RANGE,
                range_params("m", "cort_command", 2, 5),
                1,
            )
            .await
            .unwrap();
            assert!(outside.is_empty());
            let mut notime = ev("n", 5);
            notime.time_ms = None;
            turso_insert_events(&conn, &[notime]).await.unwrap();
            let none_rows = turso_query(
                &conn,
                SELECT_NO_TIME,
                vec![Cell::Text("cort_command".into()), Cell::Text("m".into())],
                1,
            )
            .await
            .unwrap();
            assert_eq!(none_rows.len(), 1);
            for mark in [10, 4] {
                let p = vec![
                    Cell::Text("m".into()),
                    Cell::Text("cort_command".into()),
                    Cell::Int(mark),
                    Cell::Text(format!("id{mark}")),
                ];
                turso_exec(&conn, UPSERT_WATERMARK, p).await.unwrap();
            }
            let w = turso_query(
                &conn,
                SELECT_WATERMARK,
                vec![Cell::Text("m".into()), Cell::Text("cort_command".into())],
                2,
            )
            .await
            .unwrap();
            // Replaced, not raised: a recreated usage.db restarts its rowids.
            assert_eq!(int(&w[0][0]).unwrap(), 4);
            assert_eq!(text(&w[0][1]).unwrap(), "id4");
            let alias = ("m".to_string(), "l".to_string(), "s".to_string());
            for _ in 0..2 {
                turso_exec(&conn, INSERT_ALIAS, alias_cells(&alias))
                    .await
                    .unwrap();
            }
        });
    }
}
