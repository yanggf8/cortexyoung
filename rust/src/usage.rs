//! Central `$CORT_CACHE_DIR/usage.db` recorder and `cort usage` aggregator.
//! Contract: docs/superpowers/plans/2026-08-28-rust-port.md §10.

use crate::errors::CortError;
use rusqlite::{params, Connection, ErrorCode, OpenFlags};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const USAGE_SCHEMA_VERSION: i64 = 1;
pub const RETENTION_DAYS: i64 = 90;
pub const DEFAULT_USAGE_DAYS: i64 = 30;
const RECORD_BUSY_MS: u64 = 25;
const DAY_MS: i64 = 86_400_000;
const PRUNE_BATCH: i64 = 500;
const FIELD_CAP: usize = 256;
const SCHEMA_SQL: &str = include_str!("usage_schema.sql");
const NOTE: &str = "saved_bytes is raw body bytes omitted, not total-output savings";
const LAST_PRUNE_KEY: &str = "LAST_PRUNE_DAY";
const VERSION_KEY: &str = "USAGE_SCHEMA_VERSION";

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

pub fn saved_bytes_for(source: Option<&str>, effective: Option<&str>, omitted_body_len: i64) -> i64 {
    if source == Some("store") && effective == Some("receipt") {
        omitted_body_len.max(0)
    } else {
        0
    }
}

pub fn omitted_body_len(content: &str, note_start: i64, start: i64, end: i64) -> i64 {
    let lines: Vec<&str> = content.split('\n').collect();
    let from = (start - note_start) as usize;
    let to = ((end - note_start + 1) as usize).min(lines.len());
    let from = from.min(to);
    lines[from..to].join("\n").len() as i64
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
    ensure_schema(&conn).map_err(|_| ())?;
    let hit = rec.receipt_hit.map(i64::from);
    let stale = rec.index_stale.map(i64::from);
    conn.execute(
        "INSERT INTO command_log
            (ts, project_id, command, args_summary, status, error_code,
             read_source, requested_content_mode, effective_content_mode,
             receipt_hit, index_stale, bytes_out, saved_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            rec.now_ms,
            rec.project_id,
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
    Ok(json!({
        "best_effort": true,
        "commands": commands,
        "days": days,
        "note": NOTE,
        "projects": projects,
    }))
}

pub fn render_usage_lean(payload: &Value) -> String {
    let days = payload.get("days").and_then(Value::as_i64).unwrap_or(0);
    let best = payload
        .get("best_effort")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let note = payload.get("note").and_then(Value::as_str).unwrap_or(NOTE);
    let mut lines = vec![
        format!("# usage days={days} best_effort={best}"),
        format!("# {note}"),
    ];
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

fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
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
        Some(_) => Err(rusqlite::Error::InvalidQuery),
    }
}

fn ensure_schema_readable(conn: &Connection) -> Result<(), CortError> {
    let found = meta(conn, VERSION_KEY).map_err(map_query_err)?;
    match found.as_deref() {
        Some(v) if v == USAGE_SCHEMA_VERSION.to_string() => Ok(()),
        None => {
            // Fresh file that record_command created always writes the version.
            // A readable DB without our meta is treated as corrupt for the query.
            conn.query_row(
                "SELECT COUNT(*) FROM command_log",
                [],
                |r| r.get::<_, i64>(0),
            )
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

fn prune_best_effort(conn: &Connection, now: i64) {
    let today = utc_day(now);
    let last = meta(conn, LAST_PRUNE_KEY).ok().flatten();
    if last.as_deref().is_some_and(|d| d >= today.as_str()) {
        return;
    }
    let cutoff = now.saturating_sub(RETENTION_DAYS * DAY_MS);
    loop {
        match conn.execute(
            "DELETE FROM command_log WHERE id IN (
                SELECT id FROM command_log WHERE ts < ?1 LIMIT ?2
             )",
            params![cutoff, PRUNE_BATCH],
        ) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(_) => return,
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
        Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked) => CortError::new(
            "usage_busy",
            json!({ "sqlite_code": "SQLITE_BUSY" }),
        ),
        Some(ErrorCode::DatabaseCorrupt) | Some(ErrorCode::NotADatabase) => CortError::new(
            "usage_corrupt",
            json!({ "sqlite_code": "SQLITE_CORRUPT" }),
        ),
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

fn cap_str(s: &str) -> String {
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
fn civil_from_unix_days(days: i64) -> (i32, u32, u32) {
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
