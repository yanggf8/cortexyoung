//! Central `$CORT_CACHE_DIR/usage.db` recorder and `cort usage` aggregator.
//! Contract: docs/superpowers/plans/2026-08-28-rust-port.md §10.

use crate::errors::CortError;
use rusqlite::{params, Connection, ErrorCode, OpenFlags};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const USAGE_SCHEMA_VERSION: i64 = 2;
pub const RETENTION_DAYS: i64 = 90;
const UNUPLOADED_RETENTION_DAYS: i64 = 180;
pub const DEFAULT_USAGE_DAYS: i64 = 30;
const RECORD_BUSY_MS: u64 = 25;
const DAY_MS: i64 = 86_400_000;
const PRUNE_BATCH: i64 = 500;
const FIELD_CAP: usize = 256;
const SCHEMA_SQL: &str = include_str!("usage_schema.sql");
const NOTE: &str = "saved_bytes is raw body bytes omitted, not total-output savings; sources: \
read receipt-hits and ranged reads (file bytes left out of the context) — format choices and \
symbol-slice counterfactuals are deliberately not counted";
const LAST_PRUNE_KEY: &str = "LAST_PRUNE_DAY";
const VERSION_KEY: &str = "USAGE_SCHEMA_VERSION";
const MACHINE_KEY: &str = "MACHINE_ID";
const MACHINE_SOURCE_KEY: &str = "MACHINE_ID_SOURCE";
const PRUNED_UNUPLOADED_KEY: &str = "pruned_unuploaded_rows";
const LOGGED_PRUNED_KEY: &str = "logged_pruned_unuploaded_rows";
pub const INSTANCE_KEY: &str = "INSTANCE_ID";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KickCounters {
    pub instance_id: String,
    /// `MAX(id)` over `command_log` rows **excluding** `command = 'measure'`: how much genuine
    /// usage is waiting to be collected. `measure` rows never make a batch due on their own.
    pub max_command_id: i64,
    /// `MAX(id)` over **every** `command_log` row, `measure` included: the highest rowid that
    /// exists at all. A cursor is only a rewind when it is past *this*, because
    /// The command watermark is the highest local rowid already uploaded, including
    /// `measure` rows with it, so a cursor that is legitimately ahead of `max_command_id` is the
    /// normal state right after a batch, not evidence of a restored database.
    pub max_any_command_id: i64,
    /// `MAX(id)` over `index_health_log`. That table has no `measure` rows, so one max serves both
    /// the pending count and the rewind test.
    pub max_health_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthRow {
    pub id: i64,
    pub ts: i64,
    pub machine_id: String,
    pub machine_source: String,
    pub root: Option<String>,
    pub local_project_id: String,
    pub project_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub fresh: Option<String>,
    pub chunks: Option<i64>,
    pub relationships: Option<i64>,
    pub not_chunked: Option<i64>,
    pub real_gap: Option<i64>,
    pub fts: Option<String>,
}

const HEALTH_COLUMNS: &str = "id, ts, machine_id, machine_source, root, local_project_id, \
    project_id, status, error, fresh, chunks, relationships, not_chunked, real_gap, fts";

fn read_health_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HealthRow> {
    Ok(HealthRow {
        id: row.get(0)?,
        ts: row.get(1)?,
        machine_id: row.get(2)?,
        machine_source: row.get(3)?,
        root: row.get(4)?,
        local_project_id: row.get(5)?,
        project_id: row.get(6)?,
        status: row.get(7)?,
        error: row.get(8)?,
        fresh: row.get(9)?,
        chunks: row.get(10)?,
        relationships: row.get(11)?,
        not_chunked: row.get(12)?,
        real_gap: row.get(13)?,
        fts: row.get(14)?,
    })
}

/// Only collection workers read health rows, so they wait out a commit like `kick_counters_within`.
fn open_health_read(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(WORKER_BUSY).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Insert an observation; the database allocates its id.
pub fn insert_health_row_at(path: &Path, row: &HealthRow) -> Result<i64, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(RECORD_BUSY_MS))
        .map_err(|e| e.to_string())?;
    chmod_600(path);
    ensure_schema(&conn, path).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO index_health_log
         (ts, machine_id, machine_source, root, local_project_id, project_id, status,
          error, fresh, chunks, relationships, not_chunked, real_gap, fts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            row.ts,
            row.machine_id,
            row.machine_source,
            row.root,
            row.local_project_id,
            row.project_id,
            row.status,
            row.error,
            row.fresh,
            row.chunks,
            row.relationships,
            row.not_chunked,
            row.real_gap,
            row.fts,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Read this machine's newest observation for each local project.
///
/// Scoped to `machine_id` because a `usage.db` carried from another machine keeps that machine's
/// rows: borrowing them would skip this machine's own observation as "same state, same day", or
/// map a deleted index back to the other machine's root.
pub fn last_health_rows_at(
    path: &Path,
    machine_id: &str,
) -> Result<HashMap<String, HealthRow>, String> {
    let conn = open_health_read(path)?;
    let sql = format!(
        "SELECT {HEALTH_COLUMNS} FROM index_health_log
         WHERE id IN (SELECT MAX(id) FROM index_health_log WHERE machine_id = ?1
                      GROUP BY local_project_id)"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([machine_id], read_health_row)
        .map_err(|e| e.to_string())?;
    let mut latest = HashMap::new();
    for row in rows {
        let row = row.map_err(|e| e.to_string())?;
        latest.insert(row.local_project_id.clone(), row);
    }
    Ok(latest)
}

/// Page health observations by id, oldest first.
pub fn health_rows_after_at(
    path: &Path,
    after_id: i64,
    limit: usize,
    since_ms: i64,
) -> Result<Vec<HealthRow>, String> {
    let conn = open_health_read(path)?;
    let sql =
        format!("SELECT {HEALTH_COLUMNS} FROM index_health_log WHERE id > ?1 AND ts >= ?2 ORDER BY id LIMIT ?3");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![after_id, since_ms, i64::try_from(limit).unwrap_or(i64::MAX)],
            read_health_row,
        )
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// Return distinct projects touched since a command id and the last non-measure id read.
///
/// Both reads share one transaction: the caller stores the returned id as its cursor, so a prune
/// committing between them would advance the cursor past rows the second read never saw.
pub fn touched_project_ids_at(path: &Path, after_id: i64) -> Result<(Vec<String>, i64), String> {
    let mut conn = open_health_read(path)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let max_id: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(id), ?1) FROM command_log
             WHERE id > ?1 AND command <> 'measure'",
            [after_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut stmt = tx
        .prepare(
            "SELECT DISTINCT project_id FROM command_log
             WHERE id > ?1 AND id <= ?2 AND command <> 'measure'
               AND project_id IS NOT NULL AND project_id <> '_unknown'
             ORDER BY project_id",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map(params![after_id, max_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| e.to_string())?;
    Ok((ids, max_id))
}

/// Read the two watermarks without creating or migrating a database. This is the hook's read, so
/// it waits no longer than a record does; a busy database reads as unknown.
pub fn kick_counters_at(path: &Path) -> Option<KickCounters> {
    kick_counters_within(path, Duration::from_millis(RECORD_BUSY_MS))
}

/// How long a collection worker or a person's command waits on usage.db's write lock. usage.db is
/// a rollback-journal database, so every recorded command blocks readers for the length of its
/// commit; 25 ms lost that race on about four in ten background runs (2026-10-28).
pub const WORKER_BUSY: Duration = Duration::from_secs(2);

/// [`kick_counters_at`] with the caller's wait on the write lock.
pub fn kick_counters_within(path: &Path, busy: Duration) -> Option<KickCounters> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.busy_timeout(busy).ok()?;
    let objects: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN (
                'command_log', 'index_health_log', 'idx_command_log_not_measure',
                'idx_index_health_log_local')",
            [],
            |row| row.get(0),
        )
        .ok()?;
    if objects != 4 {
        return None;
    }
    let instance_id = meta(&conn, INSTANCE_KEY).ok()??;
    let max_command_id = conn
        .query_row(
            "SELECT COALESCE(MAX(id), 0) FROM command_log WHERE command <> 'measure'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    // No WHERE clause: headline rowid, so SQLite answers from the rowid table itself in O(1)
    // rather than walking the partial index.
    let max_any_command_id = conn
        .query_row("SELECT COALESCE(MAX(id), 0) FROM command_log", [], |row| {
            row.get(0)
        })
        .ok()?;
    let max_health_id = conn
        .query_row(
            "SELECT COALESCE(MAX(id), 0) FROM index_health_log",
            [],
            |row| row.get(0),
        )
        .ok()?;
    Some(KickCounters {
        instance_id,
        max_command_id,
        max_any_command_id,
        max_health_id,
    })
}

/// Which machine these rows were recorded on, as a hash rather than a name.
///
/// Numbers from this database get quoted in documents, and a document that silently averages two
/// machines is wrong in a way nobody can see afterwards. The id is derived, never stored as a
/// random value, so the same machine answers the same thing after the cache is deleted -- a
/// regenerated id would read as a second machine and reintroduce exactly the confusion this exists
/// to remove.
///
/// Hashed for the same reason `project_id` is: a hostname is a name for a person's laptop. The
/// source is recorded beside it because the sources are not equally trustworthy -- `/etc/machine-id`
/// is stable across renames, a hostname is not, and `unknown` means the answer is a guess and
/// should not be leaned on.
pub fn machine_id() -> &'static str {
    static ID: std::sync::OnceLock<(String, &'static str)> = std::sync::OnceLock::new();
    &ID.get_or_init(resolve_machine).0
}

pub fn machine_id_source() -> &'static str {
    static ID: std::sync::OnceLock<(String, &'static str)> = std::sync::OnceLock::new();
    ID.get_or_init(resolve_machine).1
}

fn resolve_machine() -> (String, &'static str) {
    use sha2::{Digest, Sha256};
    // `CORT_MACHINE_ID` first so a test can pin one, and so a user on a platform none of the
    // fallbacks fit can say what their machines are called rather than being told `unknown`.
    let (raw, source) = if let Ok(v) = std::env::var("CORT_MACHINE_ID") {
        if v.is_empty() {
            (String::new(), "unknown")
        } else {
            (v, "env")
        }
    } else if let Ok(v) = std::fs::read_to_string("/etc/machine-id") {
        (v.trim().to_string(), "etc-machine-id")
    } else if let Ok(v) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
        (v.trim().to_string(), "dbus-machine-id")
    } else if let Some(v) = std::env::var_os("HOSTNAME").and_then(|h| h.into_string().ok()) {
        (v, "hostname-env")
    } else {
        (String::new(), "unknown")
    };
    if raw.is_empty() {
        return ("unknown".to_string(), "unknown");
    }
    // Sixteen hex characters: enough that two machines colliding is not a thing that happens, short
    // enough to sit in a report line a person reads.
    let full = crate::db::hex_lower(&Sha256::digest(raw.as_bytes()));
    (full[..16].to_string(), source)
}

#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub now_ms: i64,
    pub project_id: Option<String>,
    pub command: String,
    pub args_summary: String,
    pub status: String,
    pub error_code: Option<String>,
    pub read_source: Option<String>,
    pub requested_content_mode: Option<String>,
    pub effective_content_mode: Option<String>,
    pub receipt_hit: Option<bool>,
    pub index_stale: Option<bool>,
    pub bytes_out: i64,
    pub saved_bytes: i64,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Resolve `$CORT_CACHE_DIR/usage.db` without panicking and without creating dirs.
pub fn usage_db_path() -> Option<PathBuf> {
    match std::env::var("CORT_CACHE_DIR") {
        Ok(dir) => Some(PathBuf::from(dir).join("usage.db")),
        Err(_) => {
            let home = std::env::var_os("HOME")?;
            Some(
                PathBuf::from(home)
                    .join(".cache")
                    .join("cortex-ng")
                    .join("usage.db"),
            )
        }
    }
}

pub fn empty_report(days: i64) -> Value {
    json!({
        "best_effort": true,
        "commands": {},
        "days": days,
        "hook_census": {},
        // No database to have been stamped, so nothing to disagree with -- but the machine is still
        // named, because a report that omits the field on some runs makes every consumer treat it
        // as optional and stop checking it.
        "machine": {
            "id": machine_id(),
            "source": machine_id_source(),
            "db_created_on": Value::Null,
            "db_created_on_source": Value::Null,
            "mixed": false,
        },
        "note": NOTE,
        "projects": {},
    })
}

pub fn parse_usage_days(raw: Option<&str>) -> Result<i64, CortError> {
    let Some(s) = raw else {
        return Ok(DEFAULT_USAGE_DAYS);
    };
    let ok = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let n: i64 = if ok { s.parse().unwrap_or(-1) } else { -1 };
    if !(1..=RETENTION_DAYS).contains(&n) {
        return Err(CortError::new(
            "invalid_usage_days",
            json!({ "provided": s, "allowed": "1..=90" }),
        ));
    }
    Ok(n)
}

pub fn args_summary(
    symbol: Option<&str>,
    path: Option<&str>,
    start: Option<i64>,
    end: Option<i64>,
) -> String {
    let mut map = Map::new();
    map.insert("v".into(), json!(1));
    if let Some(s) = symbol {
        let t = cap_str(s);
        if !t.is_empty() {
            map.insert("symbol".into(), json!(t));
        }
    }
    if let Some(p) = path.and_then(project_relative) {
        map.insert("path".into(), json!(cap_str(&p)));
    }
    if let Some(s) = start {
        map.insert("start".into(), json!(s));
    }
    if let Some(e) = end {
        map.insert("end".into(), json!(e));
    }
    Value::Object(map).to_string()
}

/// Absorbs every failure. Zero panic, zero retry, busy gives up within 25ms.
pub fn record_command(rec: &CommandRecord) {
    let Some(path) = usage_db_path() else {
        return;
    };
    record_command_at(&path, rec);
}

pub fn record_command_at(path: &Path, rec: &CommandRecord) {
    let _ = record_inner(path, rec);
}

fn record_inner(path: &Path, rec: &CommandRecord) -> Result<(), ()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|_| ())?;
        }
    }
    let conn = Connection::open(path).map_err(|_| ())?;
    let _ = conn.busy_timeout(Duration::from_millis(RECORD_BUSY_MS));
    chmod_600(path);
    ensure_schema(&conn, path).map_err(|_| ())?;
    let hit = rec.receipt_hit.map(i64::from);
    let stale = rec.index_stale.map(i64::from);
    conn.execute(
        "INSERT INTO command_log
            (ts, project_id, machine_id, machine_source, command, args_summary, status, error_code,
             read_source, requested_content_mode, effective_content_mode,
             receipt_hit, index_stale, bytes_out, saved_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            rec.now_ms,
            rec.project_id,
            machine_id(),
            machine_id_source(),
            rec.command,
            rec.args_summary,
            rec.status,
            rec.error_code,
            rec.read_source,
            rec.requested_content_mode,
            rec.effective_content_mode,
            hit,
            stale,
            rec.bytes_out,
            rec.saved_bytes,
        ],
    )
    .map_err(|_| ())?;
    prune_best_effort(&conn, rec.now_ms);
    Ok(())
}

pub fn query_usage(days: i64) -> Result<Value, CortError> {
    let path = usage_db_path().ok_or_else(|| {
        CortError::new(
            "usage_corrupt",
            json!({ "message": "usage db path unresolved" }),
        )
    })?;
    query_usage_at(&path, days, now_ms())
}

pub fn query_usage_at(path: &Path, days: i64, now_ms: i64) -> Result<Value, CortError> {
    if !path.exists() {
        return Ok(empty_report(days));
    }
    let conn = open_query(path)?;
    ensure_schema_readable(&conn)?;
    let cutoff = now_ms.saturating_sub(days.clamp(1, RETENTION_DAYS) * DAY_MS);
    let mut commands = Map::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT command,
                        SUM(CASE WHEN status = 'ok' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END),
                        SUM(bytes_out),
                        SUM(saved_bytes),
                        SUM(CASE WHEN receipt_hit = 1 THEN 1 ELSE 0 END),
                        SUM(CASE WHEN receipt_hit IS NOT NULL THEN 1 ELSE 0 END),
                        SUM(CASE WHEN index_stale IS NOT NULL THEN 1 ELSE 0 END),
                        SUM(CASE WHEN index_stale = 1 THEN 1 ELSE 0 END)
                   FROM command_log
                  WHERE ts >= ?1
                  GROUP BY command",
            )
            .map_err(map_query_err)?;
        let rows = stmt
            .query_map(params![cutoff], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, i64>(8)?,
                ))
            })
            .map_err(map_query_err)?;
        for row in rows {
            let (cmd, ok, error, bytes_out, saved, hits, auto, stale_eval, stale_true) =
                row.map_err(map_query_err)?;
            commands.insert(
                cmd,
                command_stats(Agg {
                    ok,
                    error,
                    bytes_out,
                    saved,
                    hits,
                    auto,
                    stale_eval,
                    stale_true,
                }),
            );
        }
    }
    let mut projects = Map::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT project_id,
                        SUM(CASE WHEN status = 'ok' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END),
                        SUM(bytes_out),
                        SUM(saved_bytes)
                   FROM command_log
                  WHERE ts >= ?1
                  GROUP BY project_id",
            )
            .map_err(map_query_err)?;
        let rows = stmt
            .query_map(params![cutoff], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(map_query_err)?;
        for row in rows {
            let (pid, ok, error, bytes_out, saved) = row.map_err(map_query_err)?;
            let key = match pid.as_deref() {
                None => "_global".to_string(),
                Some(s) => s.to_string(),
            };
            projects.insert(key, project_stats(ok, error, bytes_out, saved));
        }
    }
    // The census wraps each partition with `_total` so a reader can check the sum without
    // adding the buckets up -- the check a hand-assembled funnel skipped.
    let with_total = |part: Map<String, Value>| {
        let mut m = Map::new();
        let total: i64 = part.values().filter_map(|v| v.as_i64()).sum();
        m.insert("_total".into(), json!(total));
        for (k, v) in part {
            m.insert(k, v);
        }
        m
    };
    let suggest_census = with_total(hook_census_at(path, "hook-suggest", cutoff)?);
    let refresh_census = with_total(hook_census_at(path, "hook-refresh", cutoff)?);
    let mut recall_census = with_total(hook_census_at(path, "hook-recall", cutoff)?);
    recall_census.extend(recall_network_at(&conn, cutoff)?);
    Ok(json!({
        "best_effort": true,
        "commands": commands,
        "days": days,
        "hook_census": {
            "hook-suggest": suggest_census,
            "hook-refresh": refresh_census,
            "hook-recall": recall_census,
        },
        "machine": machine_block(&conn),
        "note": NOTE,
        "projects": projects,
    }))
}

/// Count `hook-suggest` rows in `[since_ms, ..)` by the outcome its `args_summary` records --
/// the harness facet of the fire rows, with [`hook_models_at`] as the model facet beside it.
///
/// Aggregating by `command` cannot answer the question the measurement asks -- every Bash tool
/// call the agent makes produces one `hook-suggest` row, so the command total is a measure of agent
/// activity, not of which tool fired. The outcome column splits the two shapes a row can have: the
/// judging hook wrote `hit*` when a suggestion was injected, which made this db a second source for
/// that period's injection count, while since the thin trigger (2026-10-25) a row is only
/// `no_payload` or `fired`. The legacy values are still read here because the local database keeps
/// 90 days (the remote history was discarded at the 2026-10-28 measurement restart).
///
/// Nothing in the product calls this today: the `usage` report reads [`hook_census_at`], and the
/// lens functions are exercised by the test suite. They stay because the rows they read stay.
///
/// Rows written before the outcome existed carry the bare summary `hook`; they are counted under
/// `legacy_unsplit` rather than folded into a silence, because "we do not know" is not "no".
///
/// `want_harness` is the second half of the same discipline. Every harness that wires this hook
/// calls one binary and writes to one database, and the mining compares these rows against *one*
/// harness's transcripts. A row from a different harness has no transcript counterpart on that
/// side, so folding it in inflates the numerator while every guard still reports green. Rows whose
/// harness does not match are counted under `other_harness`, and rows from before the field existed
/// under `unspecified` -- never attributed to whichever harness happened to be wired first, which
/// is true today and silently false the day it is not.
pub fn hook_outcomes_at(
    path: &Path,
    since_ms: i64,
    until_ms: Option<i64>,
    want_harness: Option<&str>,
) -> Result<Map<String, Value>, CortError> {
    outcomes_of_hook_at(path, "hook-suggest", since_ms, until_ms, want_harness)
}

/// The same split for either hook event, told apart by the command that wrote the row.
///
/// Two events ship (`hook-suggest` before the matched tool, `hook-refresh` after an edit) and
/// they answer different questions -- one records a fire of the matched tool surface (every
/// `Bash` call, plus Kimi's `Grep`, search or not), the other is whether the index stayed level
/// with the tree -- so they are never summed. They do share one row shape (`hook_row` in
/// `main.rs`) and therefore this reader: a second copy of the harness-attribution rule is how one
/// of them would quietly start counting `unspecified` differently from the other.
pub fn outcomes_of_hook_at(
    path: &Path,
    command: &str,
    since_ms: i64,
    until_ms: Option<i64>,
    want_harness: Option<&str>,
) -> Result<Map<String, Value>, CortError> {
    let mut out: Map<String, Value> = Map::new();
    if !path.exists() {
        return Ok(out);
    }
    let conn = open_query(path)?;
    ensure_schema_readable(&conn)?;
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    {
        // `until_ms` closes the window on the reading side because the count is aggregated here:
        // a caller filtering afterwards would be filtering a sum it can no longer split.
        let mut stmt = conn
            .prepare(
                "SELECT args_summary FROM command_log
                  WHERE command = ?2 AND ts >= ?1 AND (?3 IS NULL OR ts <= ?3)",
            )
            .map_err(map_query_err)?;
        let rows = stmt
            .query_map(params![since_ms, command, until_ms], |r| {
                r.get::<_, String>(0)
            })
            .map_err(map_query_err)?;
        for row in rows {
            let raw = row.map_err(map_query_err)?;
            let parsed = serde_json::from_str::<Value>(&raw).ok();
            let outcome = parsed
                .as_ref()
                .and_then(|v| v.get("hook").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "legacy_unsplit".to_string());
            // An explicit `unspecified` (a v2 row whose hook was wired without `--harness`) and a
            // missing field (a v1 row from before the field existed) are the same claim: nobody
            // recorded where this came from. Both belong in `unspecified`, and neither is evidence
            // of a *different* harness.
            let harness = parsed
                .as_ref()
                .and_then(|v| v.get("harness").and_then(Value::as_str))
                .filter(|h| *h != "unspecified");
            let key = match (want_harness, harness) {
                // No filter asked for: report the outcomes as they are.
                (None, _) => outcome,
                // A row that predates outcome recording stays `legacy_unsplit`. It is unattributable
                // for a different reason than `unspecified`, and collapsing the two would hide which
                // of the two upgrades a stale row is waiting on.
                _ if outcome == "legacy_unsplit" => outcome,
                (Some(_), None) => "unspecified".to_string(),
                (Some(want), Some(got)) if want == got => outcome,
                (Some(_), Some(_)) => "other_harness".to_string(),
            };
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    for (k, v) in counts {
        out.insert(k, json!(v));
    }
    Ok(out)
}

/// Which model answered, *within* a harness -- a second lens on the same rows, never a replacement/// for the first.
///
/// The harness total is the primary number and stays whole: "how often did the hook intercept on
/// Claude Code" is a real question with a real answer, and it does not stop being one because more
/// than one model sat behind that harness. `hook_outcomes_at` therefore ignores `model` entirely
/// and its counts are unchanged by anything here -- a property
/// `a_model_breakdown_never_splits_the_harness_total` pins, because the tempting mistake is to
/// make every figure conditional on a dimension that only
/// some rows carry.
///
/// Three silences, kept apart because they mean different things:
/// * `unrecorded` -- a v1/v2 row, written before `model` existed. Not evidence of anything.
/// * `unreported` -- a v3 row whose payload named no model. The harness did not say.
/// * anything else -- what the harness called it, verbatim, never normalised into a family name.
pub fn hook_models_at(
    path: &Path,
    command: &str,
    since_ms: i64,
    want_harness: Option<&str>,
) -> Result<Map<String, Value>, CortError> {
    let mut out: Map<String, Value> = Map::new();
    if !path.exists() {
        return Ok(out);
    }
    let conn = open_query(path)?;
    ensure_schema_readable(&conn)?;
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT args_summary FROM command_log
                  WHERE command = ?2 AND ts >= ?1",
            )
            .map_err(map_query_err)?;
        let rows = stmt
            .query_map(params![since_ms, command], |r| r.get::<_, String>(0))
            .map_err(map_query_err)?;
        for row in rows {
            let raw = row.map_err(map_query_err)?;
            let Some(parsed) = serde_json::from_str::<Value>(&raw).ok() else {
                continue;
            };
            if let Some(want) = want_harness {
                let got = parsed.get("harness").and_then(Value::as_str);
                if got != Some(want) {
                    continue;
                }
            }
            let v = parsed.get("v").and_then(Value::as_i64).unwrap_or(1);
            let key = match parsed.get("model").and_then(Value::as_str) {
                Some(m) if !m.is_empty() => m.to_string(),
                _ if v < 3 => "unrecorded".to_string(),
                _ => "unreported".to_string(),
            };
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    for (k, v) in counts {
        out.insert(k, json!(v));
    }
    Ok(out)
}

/// The exhaustive, disjoint census of one hook's rows: every row lands in exactly one bucket and
/// the buckets sum to the fires.
///
/// This exists because a funnel assembled by hand over `args_summary` could not be closed: the
/// 2026-09-10 reconciliation found 87 rows no bucket claimed -- two real outcomes
/// (`no_payload`, `upgrade_stood_down`) that the hand query never listed -- and 1,032 rows from
/// one day whose summary was not JSON at all, invisible to every `LIKE` and every JSON reader
/// because they match nothing. The partition answers by construction:
///
/// * `status_error` -- the dispatch failed; whatever verdict the row carries is not evidence.
/// * `unparseable_summary` -- the summary is not JSON (the writer-bug shape). Counted, never
///   skipped: a row that vanishes from the reader is a row every funnel silently miscounts.
/// * `legacy_unsplit` -- valid JSON with no readable `hook`: a row from before outcomes existed.
///   Same name `hook_outcomes_at` uses, for the same reason.
/// * `no_shape/<decline>` -- suggest only, and only for legacy rows: v≤5 rows attribute the one
///   outcome whose reason lives one level down, per `crate::hook::SUGGEST_DECLINES`;
///   `no_shape/decline_absent` is the pre-tag shape, a deployment state rather than a tenth cause.
///   The thin trigger never writes `no_shape`, but the local 90-day retention still holds rows that do (the remote
///   history was discarded at the 2026-10-28 restart). A `no_shape` value on any other command is not that
///   command's shape to attribute, so it surfaces as `unknown/no_shape` instead.
/// * every `crate::hook::SUGGEST_OUTCOMES` (suggest) or `crate::hook::REFRESH_OUTCOMES` (refresh)
///   value -- verbatim; the vocabulary is the writing command's own, and the decline split below
///   is suggest's contract alone. For suggest that means the current vocabulary *plus*
///   `crate::hook::LEGACY_SUGGEST_OUTCOMES`: what is written now and what old rows say, so the
///   census stays closed across the thin trigger rather than reporting history as `unknown/`.
/// * `unknown/<value>` -- anything else, verbatim under a prefix that makes it impossible to
///   mistake for a known outcome. A vocabulary that grew without its constant grows here first.
///
/// No harness filter on purpose: the census answers "do the numbers sum", and a filtered
/// partition is not the same question. The harness lenses live on `hook_outcomes_at`.
pub fn hook_census_at(
    path: &Path,
    command: &str,
    since_ms: i64,
) -> Result<Map<String, Value>, CortError> {
    // suggest's known set is the current vocabulary plus the judging hook's, because both are real
    // rows in the same table; refresh has only ever had one vocabulary and needs no union.
    let suggest_known: Vec<&str> = crate::hook::SUGGEST_OUTCOMES
        .iter()
        .chain(crate::hook::LEGACY_SUGGEST_OUTCOMES.iter())
        .copied()
        .collect();
    let recall_known: Vec<&str> = crate::hook::RECALL_OUTCOMES
        .iter()
        .chain(crate::hook::LEGACY_RECALL_OUTCOMES.iter())
        .copied()
        .collect();
    let (known, splits_decline): (&[&str], bool) = if command == "hook-recall" {
        (&recall_known, false)
    } else if command == "hook-refresh" {
        (&crate::hook::REFRESH_OUTCOMES, false)
    } else {
        (&suggest_known, true)
    };
    let mut out: Map<String, Value> = Map::new();
    if !path.exists() {
        return Ok(out);
    }
    let conn = open_query(path)?;
    ensure_schema_readable(&conn)?;
    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT status, args_summary FROM command_log
                  WHERE command = ?2 AND ts >= ?1",
            )
            .map_err(map_query_err)?;
        let rows = stmt
            .query_map(params![since_ms, command], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(map_query_err)?;
        for row in rows {
            let (status, raw) = row.map_err(map_query_err)?;
            let key = if status != "ok" {
                "status_error".to_string()
            } else if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                match parsed.get("hook").and_then(Value::as_str) {
                    None => "legacy_unsplit".to_string(),
                    Some("no_shape") if splits_decline => format!(
                        "no_shape/{}",
                        parsed
                            .get("decline")
                            .and_then(Value::as_str)
                            .unwrap_or("decline_absent")
                    ),
                    Some(h) if known.contains(&h) => h.to_string(),
                    Some(h) => format!("unknown/{h}"),
                }
            } else {
                "unparseable_summary".to_string()
            };
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    for (k, v) in counts {
        out.insert(k, json!(v));
    }
    Ok(out)
}

/// Network diagnostics are separate objects beside the outcome buckets, so `_total` still
/// counts hook rows exactly once. Old rows without timings never become zero-ms samples.
fn recall_network_at(conn: &Connection, since_ms: i64) -> Result<Map<String, Value>, CortError> {
    let mut stmt = conn
        .prepare("SELECT args_summary FROM command_log WHERE command = 'hook-recall' AND ts >= ?1 AND status = 'ok'")
        .map_err(map_query_err)?;
    let rows = stmt
        .query_map(params![since_ms], |r| r.get::<_, String>(0))
        .map_err(map_query_err)?;
    let phases = ["dns", "connect", "tls", "response"];
    let mut stalls = std::collections::BTreeMap::<String, u64>::new();
    let mut samples: [Vec<u64>; 4] = std::array::from_fn(|_| Vec::new());
    for row in rows {
        let raw = row.map_err(map_query_err)?;
        let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if let Some(phase) = parsed["stall"]["phase"]
            .as_str()
            .filter(|p| phases.contains(p) || *p == "budget")
        {
            *stalls.entry(phase.to_string()).or_default() += 1;
        }
        for (phase, values) in phases.iter().zip(&mut samples) {
            if let Some(ms) = parsed["net"][format!("{phase}_ms")].as_u64() {
                values.push(ms);
            }
        }
    }
    let mut out = Map::new();
    if !stalls.is_empty() {
        out.insert("stall_phase".into(), json!(stalls));
    }
    let mut net = Map::new();
    for (phase, values) in phases.iter().zip(&mut samples) {
        if values.is_empty() {
            continue;
        }
        values.sort_unstable();
        // Nearest-rank percentiles, including the measured zero-ms samples.
        let percentile = |p: usize| values[(values.len() * p).div_ceil(100) - 1];
        net.insert(
            format!("{phase}_ms"),
            json!({"n":values.len(), "p50":percentile(50), "p95":percentile(95), "max":values.last()}),
        );
    }
    if !net.is_empty() {
        out.insert("net".into(), json!(net));
    }
    Ok(out)
}

pub fn render_usage_lean(payload: &Value) -> String {
    let days = payload.get("days").and_then(Value::as_i64).unwrap_or(0);
    let best = payload
        .get("best_effort")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let note = payload.get("note").and_then(Value::as_str).unwrap_or(NOTE);
    let m = payload.get("machine");
    let mstr = |k: &str| {
        m.and_then(|m| m.get(k))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    };
    let mixed = m
        .and_then(|m| m.get("mixed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut lines = vec![
        format!("# usage days={days} best_effort={best}"),
        format!("# machine={} source={}", mstr("id"), mstr("source")),
        format!("# {note}"),
    ];
    // Loud, and in the header rather than a footnote: this aggregate can span machines even
    // though new rows carry their own stamp, and legacy rows cannot be separated after the fact.
    if mixed {
        lines.push(format!(
            "# WARNING mixed_machines: database was created on {} and opened here; schema-v1 rows cannot be attributed individually, inspect schema-v2 row stamps before combining",
            mstr("db_created_on")
        ));
    }
    if let Some(cmds) = payload.get("commands").and_then(Value::as_object) {
        for (name, row) in cmds {
            lines.push(format!(
                "{name}\tok={} error={} bytes_out={} saved_bytes={} receipt_hit_rate={} stale={}/{}",
                as_i64(row, "ok"),
                as_i64(row, "error"),
                as_i64(row, "bytes_out"),
                as_i64(row, "saved_bytes"),
                rate_cell(row),
                as_i64(row, "stale_true"),
                as_i64(row, "stale_evaluated"),
            ));
        }
    }
    // One line per hook command that has rows: the four canaries a funnel needs before its
    // numbers can be trusted. `total` is what the buckets must sum to, `unknown` anything the
    // outcome vocabulary cannot name; the JSON report carries the full partition, this line only
    // has to make an unsumming funnel visible in lean output.
    if let Some(census) = payload.get("hook_census").and_then(Value::as_object) {
        for (name, part) in census {
            // A command with no rows in the window gets no line: "nothing fired" is the command
            // total's job to say, and a permanent zero line is one more thing to skim past.
            if part.get("_total").and_then(Value::as_i64).unwrap_or(0) == 0 {
                continue;
            }
            let at = |k: &str| part.get(k).and_then(Value::as_i64).unwrap_or(0);
            let unknown: i64 = part
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| k.starts_with("unknown/"))
                        .filter_map(|(_, v)| v.as_i64())
                        .sum()
                })
                .unwrap_or(0);
            lines.push(format!(
                "# hook_census {name} total={} status_error={} unparseable_summary={} legacy_unsplit={} unknown={unknown}",
                at("_total"),
                at("status_error"),
                at("unparseable_summary"),
                at("legacy_unsplit"),
            ));
            if let Some(stalls) = part.get("stall_phase") {
                lines.push(format!("# hook_network {name} stall_phase={stalls}"));
            }
            if let Some(net) = part.get("net").and_then(Value::as_object) {
                for (phase, stats) in net {
                    lines.push(format!(
                        "# hook_network {name} {phase} n={} p50={} p95={} max={}",
                        stats["n"], stats["p50"], stats["p95"], stats["max"],
                    ));
                }
            }
        }
    }
    lines.push("# projects".into());
    if let Some(projs) = payload.get("projects").and_then(Value::as_object) {
        for (name, row) in projs {
            lines.push(format!(
                "{name}\tok={} error={} bytes_out={} saved_bytes={}",
                as_i64(row, "ok"),
                as_i64(row, "error"),
                as_i64(row, "bytes_out"),
                as_i64(row, "saved_bytes"),
            ));
        }
    }
    format!("{}\n", lines.join("\n"))
}

fn as_i64(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn rate_cell(row: &Value) -> String {
    match row.get("receipt_hit_rate") {
        Some(Value::Null) | None => "-".to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

struct Agg {
    ok: i64,
    error: i64,
    bytes_out: i64,
    saved: i64,
    hits: i64,
    auto: i64,
    stale_eval: i64,
    stale_true: i64,
}

fn command_stats(a: Agg) -> Value {
    let rate = if a.auto == 0 {
        Value::Null
    } else {
        json!(a.hits as f64 / a.auto as f64)
    };
    json!({
        "bytes_out": a.bytes_out,
        "error": a.error,
        "ok": a.ok,
        "receipt_hit_rate": rate,
        "saved_bytes": a.saved,
        "stale_evaluated": a.stale_eval,
        "stale_true": a.stale_true,
    })
}

fn project_stats(ok: i64, error: i64, bytes_out: i64, saved: i64) -> Value {
    json!({
        "bytes_out": bytes_out,
        "error": error,
        "ok": ok,
        "saved_bytes": saved,
    })
}

fn open_query(path: &Path) -> Result<Connection, CortError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(map_query_err)?;
    let _ = conn.busy_timeout(Duration::from_millis(RECORD_BUSY_MS));
    conn.query_row("SELECT 1 FROM sqlite_master LIMIT 1", [], |_| Ok(()))
        .map_err(map_query_err)?;
    Ok(conn)
}

fn ensure_schema(conn: &Connection, path: &Path) -> rusqlite::Result<()> {
    // One commit avoids the fresh-file cost of 52 fsyncs (450–700 ms measured on NUC11i5).
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        conn.execute_batch(SCHEMA_SQL)?;
        stamp_machine(conn)?;
        stamp_instance(conn, path)?;
        let found: Option<String> = meta(conn, VERSION_KEY)?;
        let expected = USAGE_SCHEMA_VERSION.to_string();
        match found.as_deref() {
            None => {
                conn.execute(
                    "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![VERSION_KEY, expected],
                )?;
                Ok(())
            }
            Some(v) if v == expected => Ok(()),
            Some("1") => {
                let columns = [
                    ("machine_id", "ALTER TABLE command_log ADD COLUMN machine_id TEXT NOT NULL DEFAULT 'legacy_unknown'"),
                    ("machine_source", "ALTER TABLE command_log ADD COLUMN machine_source TEXT NOT NULL DEFAULT 'legacy_no_stamp'"),
                ];
                for (name, ddl) in columns {
                    let present = conn.query_row(
                        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('command_log') WHERE name = ?1)",
                        [name],
                        |row| row.get::<_, bool>(0),
                    )?;
                    if !present {
                        conn.execute(ddl, [])?;
                    }
                }
                conn.execute(
                    "UPDATE _usage_meta SET value = ?1 WHERE key = ?2",
                    params![expected, VERSION_KEY],
                )?;
                Ok(())
            }
            Some(_) => Err(rusqlite::Error::InvalidQuery),
        }?;
        conn.execute_batch("COMMIT")
    })();
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
}

/// Name this database file once, so a replaced or restored `usage.db` can be told from the one a
/// cursor was taken against. Read first, like [`stamp_machine`]: an unconditional
/// `INSERT OR IGNORE` takes the write lock on every recorded row even when it ignores.
fn stamp_instance(conn: &Connection, path: &Path) -> rusqlite::Result<()> {
    use sha2::{Digest, Sha256};

    if meta(conn, INSTANCE_KEY)?.is_some() {
        return Ok(());
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let seed = format!("{nanos}\0{}\0{}", std::process::id(), path.display());
    let digest = crate::db::hex_lower(&Sha256::digest(seed.as_bytes()));
    conn.execute(
        "INSERT OR IGNORE INTO _usage_meta (key, value) VALUES (?1, ?2)",
        params![INSTANCE_KEY, &digest[..32]],
    )?;
    Ok(())
}

/// Write the machine down the first time, and never overwrite it.
///
/// Never overwriting is the whole point. A database carried to a second machine -- a synced cache
/// dir, a restored backup -- keeps naming the machine it was created on, so the two ids disagree
/// and `query_usage_at` can say the file holds more than one machine's rows. Rewriting the stamp
/// would make that file look native to whichever machine opened it last, which is the silent
/// version of the same problem.
///
/// A pre-existing database from before this key gets stamped on its next open. That is the one case
/// where the stamp can be wrong, and it is why the report reads `machine_id_source` rather than
/// treating the stamp as gospel.
fn stamp_machine(conn: &Connection) -> rusqlite::Result<()> {
    if meta(conn, MACHINE_KEY)?.is_none() {
        conn.execute(
            "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            params![MACHINE_KEY, machine_id()],
        )?;
        conn.execute(
            "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            params![MACHINE_SOURCE_KEY, machine_id_source()],
        )?;
    }
    Ok(())
}

/// What the report says about where these rows came from.
///
/// `mixed` warns that the database file's original stamp differs from the current machine.
/// Schema-v1 rows have no per-row attribution; schema-v2 rows do. The aggregate still covers
/// the whole file, so the warning remains necessary when interpreting old and new rows together.
fn machine_block(conn: &Connection) -> Value {
    let stamped = meta(conn, MACHINE_KEY).ok().flatten();
    let stamped_source = meta(conn, MACHINE_SOURCE_KEY).ok().flatten();
    let here = machine_id();
    json!({
        "id": here,
        "source": machine_id_source(),
        "db_created_on": stamped,
        "db_created_on_source": stamped_source,
        "mixed": stamped.as_deref().is_some_and(|s| s != here),
    })
}

fn ensure_schema_readable(conn: &Connection) -> Result<(), CortError> {
    let found = meta(conn, VERSION_KEY).map_err(map_query_err)?;
    match found.as_deref() {
        Some(v) if v == USAGE_SCHEMA_VERSION.to_string() => Ok(()),
        None => {
            // Fresh file that record_command created always writes the version.
            // A readable DB without our meta is treated as corrupt for the query.
            conn.query_row("SELECT COUNT(*) FROM command_log", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(map_query_err)?;
            Ok(())
        }
        Some(_) => Err(CortError::new(
            "usage_corrupt",
            json!({ "reason": "schema_version_mismatch" }),
        )),
    }
}

fn meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM _usage_meta WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

pub fn uploaded_floor_at(path: &Path, source: &str, rowid: i64) -> Result<(), String> {
    let key = match source {
        crate::telemetry::SOURCE_COMMAND => "uploaded_command_rowid",
        crate::telemetry::SOURCE_HEALTH => "uploaded_health_rowid",
        _ => return Err("unknown upload source".into()),
    };
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(RECORD_BUSY_MS))
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, rowid.max(0).to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn pruned_unuploaded_rows_at(path: &Path) -> Option<i64> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    meta(&conn, PRUNED_UNUPLOADED_KEY)
        .ok()
        .flatten()?
        .parse()
        .ok()
}

pub fn unlogged_pruned_rows_at(path: &Path) -> Option<(i64, i64)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let total = meta(&conn, PRUNED_UNUPLOADED_KEY)
        .ok()
        .flatten()?
        .parse::<i64>()
        .ok()?;
    let logged = meta(&conn, LOGGED_PRUNED_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    (total > logged).then_some((total - logged, total))
}

pub fn mark_pruned_rows_logged_at(path: &Path, total: i64) -> Result<(), String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(RECORD_BUSY_MS))
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![LOGGED_PRUNED_KEY, total.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Stored usage-schema version, read-only. `None` = file missing, unreadable, or key absent;
/// the CALLER maps missing-file to Absent and the rest to Unreadable (that distinction lives
/// in diagnosis, not here). The SQL stays where the table is defined, so upgrade.rs never
/// restates usage's layout.
pub fn read_schema_version(path: &Path) -> Option<String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    meta(&conn, VERSION_KEY).ok()?
}

/// Deletes one batch of rows never uploaded and adds them to the loss counter in one transaction:
/// a deleted row cannot be counted later, so the two commit together or not at all.
fn prune_unuploaded_batch(
    conn: &Connection,
    sql: &str,
    age: i64,
    floor: i64,
) -> rusqlite::Result<usize> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let batch = (|| {
        let n = conn.execute(sql, params![age, floor, PRUNE_BATCH])?;
        if n > 0 {
            conn.execute(
                "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET
                     value = CAST(CAST(value AS INTEGER) + CAST(excluded.value AS INTEGER) AS TEXT)",
                params![PRUNED_UNUPLOADED_KEY, n.to_string()],
            )?;
        }
        conn.execute_batch("COMMIT")?;
        Ok(n)
    })();
    if batch.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    batch
}

fn prune_best_effort(conn: &Connection, now: i64) {
    let today = utc_day(now);
    let last = meta(conn, LAST_PRUNE_KEY).ok().flatten();
    if last.as_deref().is_some_and(|d| d >= today.as_str()) {
        return;
    }
    let cutoff = now.saturating_sub(RETENTION_DAYS * DAY_MS);
    let hard_cutoff = now.saturating_sub(UNUPLOADED_RETENTION_DAYS * DAY_MS);
    for (table, floor_key) in [
        ("command_log", "uploaded_command_rowid"),
        ("index_health_log", "uploaded_health_rowid"),
    ] {
        let floor = meta(conn, floor_key)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        for (age, relation, count_loss) in [(cutoff, "<=", false), (hard_cutoff, ">", true)] {
            let sql = format!(
                "DELETE FROM {table} WHERE id IN (
                SELECT id FROM {table} WHERE ts < ?1 AND id {relation} ?2 LIMIT ?3)"
            );
            loop {
                let deleted = if count_loss {
                    prune_unuploaded_batch(conn, &sql, age, floor)
                } else {
                    conn.execute(&sql, params![age, floor, PRUNE_BATCH])
                };
                match deleted {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        }
    }
    let _ = conn.execute(
        "INSERT INTO _usage_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![LAST_PRUNE_KEY, today],
    );
}

fn map_query_err(e: rusqlite::Error) -> CortError {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked) => {
            CortError::new("usage_busy", json!({ "sqlite_code": "SQLITE_BUSY" }))
        }
        Some(ErrorCode::DatabaseCorrupt) | Some(ErrorCode::NotADatabase) => {
            CortError::new("usage_corrupt", json!({ "sqlite_code": "SQLITE_CORRUPT" }))
        }
        _ => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("busy") || msg.contains("locked") {
                CortError::new("usage_busy", json!({ "message": e.to_string() }))
            } else {
                CortError::new("usage_corrupt", json!({ "message": e.to_string() }))
            }
        }
    }
}

fn chmod_600(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    let _ = path;
}

pub fn cap_str(s: &str) -> String {
    if s.len() <= FIELD_CAP {
        return s.to_string();
    }
    let mut end = FIELD_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn project_relative(p: &str) -> Option<String> {
    if p.is_empty() {
        return None;
    }
    let path = Path::new(p);
    if path.is_absolute() || p.starts_with('~') {
        return None;
    }
    if p.split(['/', '\\']).any(|c| c == "..") {
        return None;
    }
    Some(p.replace('\\', "/"))
}

fn utc_day(ms: i64) -> String {
    let (y, m, d) = civil_from_unix_days(ms.div_euclid(DAY_MS));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant civil_from_days; `days` is days since 1970-01-01 UTC.
pub(crate) fn civil_from_unix_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 36524);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_schema_creation_and_unknown_version_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.db");
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn, &path).unwrap();
        drop(conn);
        let conn = Connection::open(&path).unwrap();
        let named = "SELECT count(*) FROM sqlite_master WHERE name IN \
             ('_usage_meta', 'command_log', 'index_health_log', 'idx_command_log_ts', \
              'idx_command_log_project_ts', 'idx_command_log_not_measure', \
              'idx_index_health_log_local')";
        let objects: i64 = conn.query_row(named, [], |row| row.get(0)).unwrap();
        assert_eq!(objects, 7);
        assert_eq!(
            meta(&conn, VERSION_KEY).unwrap(),
            Some(USAGE_SCHEMA_VERSION.to_string())
        );
        for key in [MACHINE_KEY, MACHINE_SOURCE_KEY, INSTANCE_KEY] {
            assert!(meta(&conn, key).unwrap().is_some());
        }

        let path = dir.path().join("unknown.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE _usage_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        conn.execute("INSERT INTO _usage_meta VALUES (?1, '999')", [VERSION_KEY])
            .unwrap();
        assert!(matches!(
            ensure_schema(&conn, &path),
            Err(rusqlite::Error::InvalidQuery)
        ));
        drop(conn);
        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM _usage_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(meta(&conn, VERSION_KEY).unwrap().as_deref(), Some("999"));
        let ours = "SELECT count(*) FROM sqlite_master \
             WHERE type IN ('table', 'index') AND name NOT LIKE 'sqlite_%'";
        let count: i64 = conn.query_row(ours, [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
}
