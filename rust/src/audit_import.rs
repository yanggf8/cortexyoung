//! Import the table claudecat's retired daily audit used to write (`CORT-AUDIT.md`).
//!
//! The audit is gone, but its rows are the only history of index health and of the usage counters
//! before cort collected them itself. Each table row carries both: the health columns become
//! `cort_index_health` events (the same source live observations use) and the usage columns become
//! [`ImportedUsage`] rows keyed by `(day, host)`. Both halves are stamped `cort-audit-md` so a
//! report can say the numbers are imported rather than observed.
//!
//! Nothing here decides what a number means: the old audit's own 7-day gate and labels are kept as
//! text (`fresh` keeps its original label), and a cell the audit could not read is `None`, never 0.

use crate::measure::{days_from_civil, DAY_MS};
use crate::measure_store::MeasureStore;
use crate::telemetry::{health_event_id, sorted_compact_json, TelemetryEvent, SOURCE_HEALTH};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// How every imported row names itself in `machine_id` / `machine_source` / payload `origin`.
pub const IMPORTED_SOURCE: &str = "cort-audit-md";

/// The table's usage windows are all `/30d`; it never carried another length.
const WINDOW_DAYS: i64 = 30;

/// The usage columns of one row: command counts and the decline breakdown, as the audit printed
/// them. `None` means the audit could not read the cell, including where the real value was 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedUsage {
    pub day: String,
    pub host: String,
    pub commands_window: Option<i64>,
    pub core_window: Option<i64>,
    pub deep_window: Option<i64>,
    pub commands_7d: Option<i64>,
    pub deep_7d: Option<i64>,
    pub decline_top: Option<String>,
    pub window_days: i64,
}

impl ImportedUsage {
    pub fn to_json(&self) -> Value {
        json!({
            "day": self.day,
            "host": self.host,
            "commands_window": self.commands_window,
            "core_window": self.core_window,
            "deep_window": self.deep_window,
            "commands_7d": self.commands_7d,
            "deep_7d": self.deep_7d,
            "decline_top": self.decline_top,
            "window_days": self.window_days,
        })
    }
}

/// One row whose `(day, host)` the store already holds with different values. Nothing is
/// overwritten: an import never rewrites history another import already wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedUsageConflict {
    pub existing: ImportedUsage,
    pub incoming: ImportedUsage,
}

/// What [`MeasureStore::upsert_imported_usage`] did with a batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportedUsageOutcome {
    pub inserted: usize,
    pub unchanged: usize,
    pub conflicts: Vec<ImportedUsageConflict>,
}

/// One parsed table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    /// 1-based line number in the file it came from, for error messages and forensics.
    pub line: usize,
    pub day: String,
    pub host: String,
    pub root: String,
    /// `ok` or `no_index`; the audit's own label stays in `fresh`.
    pub status: String,
    /// The audit's label (`fresh`, `fresh?`, `STALE`, `STALE/refresh`, `STALE/rebuild`,
    /// `STALE/graph`), `None` for `no-index`.
    pub fresh: Option<String>,
    pub chunks: Option<i64>,
    pub relationships: Option<i64>,
    pub not_chunked: Option<i64>,
    pub real_gap: Option<i64>,
    /// `synced` or `drift=N`.
    pub fts: Option<String>,
    pub usage: ImportedUsage,
}

/// Parse every table row: a line starting with `| 20`, in the two shapes the file holds.
///
/// A 15-column row has `host` and `decline-top`; the single 13-column row (2026-09-06) has neither
/// and its host is recorded as `unknown`. Any other width, an unreadable number, or a `fresh` label
/// the audit never printed is an error naming the line -- a silently guessed value would become
/// history nobody can check.
pub fn parse_audit_md(md: &str) -> Result<Vec<AuditRow>, String> {
    let mut rows = Vec::new();
    for (index, line) in md.lines().enumerate() {
        let line_no = index + 1;
        if !line.starts_with("| 20") {
            continue;
        }
        let cells = cells_of(line);
        let row = match cells.len() {
            15 => full_row(line_no, &cells)?,
            13 => short_row(line_no, &cells)?,
            n => {
                return Err(format!(
                    "line {line_no}: expected 13 or 15 columns, got {n}"
                ))
            }
        };
        rows.push(row);
    }
    Ok(rows)
}

/// The trimmed cells of a table line, without the outer pipes.
fn cells_of(line: &str) -> Vec<&str> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

fn full_row(line: usize, c: &[&str]) -> Result<AuditRow, String> {
    let day = day_of(line, c[0])?;
    let host = host_of(c[2]);
    let (status, fresh) = fresh_of(line, c[3])?;
    let fts = opt_text(c[8]);
    Ok(AuditRow {
        line,
        usage: ImportedUsage {
            day: day.clone(),
            host: host.clone(),
            commands_window: num(line, "commands_window", c[9])?,
            core_window: num(line, "core_window", c[10])?,
            deep_window: num(line, "deep_window", c[11])?,
            commands_7d: num(line, "commands_7d", c[12])?,
            deep_7d: num(line, "deep_7d", c[13])?,
            decline_top: opt_text(c[14]),
            window_days: WINDOW_DAYS,
        },
        day,
        host,
        root: root_of(c[1]),
        status,
        fresh,
        chunks: num(line, "chunks", c[4])?,
        relationships: num(line, "relationships", c[5])?,
        not_chunked: num(line, "not_chunked", c[6])?,
        real_gap: num(line, "real_gap", c[7])?,
        fts,
    })
}

/// The 2026-09-06 shape: no `host`, no `decline-top`.
fn short_row(line: usize, c: &[&str]) -> Result<AuditRow, String> {
    let day = day_of(line, c[0])?;
    let host = "unknown".to_string();
    let (status, fresh) = fresh_of(line, c[2])?;
    let fts = opt_text(c[7]);
    Ok(AuditRow {
        line,
        usage: ImportedUsage {
            day: day.clone(),
            host: host.clone(),
            commands_window: num(line, "commands_window", c[8])?,
            core_window: num(line, "core_window", c[9])?,
            deep_window: num(line, "deep_window", c[10])?,
            commands_7d: num(line, "commands_7d", c[11])?,
            deep_7d: num(line, "deep_7d", c[12])?,
            decline_top: None,
            window_days: WINDOW_DAYS,
        },
        day,
        host,
        root: root_of(c[1]),
        status,
        fresh,
        chunks: num(line, "chunks", c[3])?,
        relationships: num(line, "relationships", c[4])?,
        not_chunked: num(line, "not_chunked", c[5])?,
        real_gap: num(line, "real_gap", c[6])?,
        fts,
    })
}

/// The project cell is a backticked path; the backticks are markdown, not part of the root.
fn root_of(cell: &str) -> String {
    cell.trim_matches('`').to_string()
}

/// A cell the audit could not read. `?` and `-` are the two spellings it used, and an empty cell
/// means the column was not carried at all; none of them is a zero.
fn is_missing(cell: &str) -> bool {
    cell.is_empty() || cell == "?" || cell == "-"
}

fn opt_text(cell: &str) -> Option<String> {
    if is_missing(cell) {
        None
    } else {
        Some(cell.to_string())
    }
}

fn num(line: usize, column: &str, cell: &str) -> Result<Option<i64>, String> {
    if is_missing(cell) {
        return Ok(None);
    }
    cell.parse::<i64>()
        .map(Some)
        .map_err(|_| format!("line {line}: {column} is not an integer: {cell:?}"))
}

/// `no-index` is the status column's own value; every other label is a freshness verdict. `?` is
/// not one of them: a row whose freshness the audit could not read is an error, not a blank.
fn fresh_of(line: usize, cell: &str) -> Result<(String, Option<String>), String> {
    match cell {
        "no-index" => Ok(("no_index".to_string(), None)),
        "fresh" | "fresh?" | "STALE" | "STALE/refresh" | "STALE/rebuild" | "STALE/graph" => {
            Ok(("ok".to_string(), Some(cell.to_string())))
        }
        other => Err(format!("line {line}: unknown fresh label {other:?}")),
    }
}

/// `YYYY-MM-DD` only. The shape and the month/day ranges are checked; the calendar itself is not
/// (there is no date crate here, and an impossible 2026-02-30 would be caught by the audit long
/// before it reached this file).
fn day_of(line: usize, cell: &str) -> Result<String, String> {
    let bad = || format!("line {line}: date is not YYYY-MM-DD: {cell:?}");
    let shaped = {
        let mut parts = cell.split('-');
        matches!(
            (parts.next(), parts.next(), parts.next(), parts.next()),
            (Some(y), Some(m), Some(d), None)
                if y.len() == 4
                    && m.len() == 2
                    && d.len() == 2
                    && y.bytes().all(|b| b.is_ascii_digit())
                    && m.bytes().all(|b| b.is_ascii_digit())
                    && d.bytes().all(|b| b.is_ascii_digit())
        )
    };
    let in_range =
        civil_day(cell).is_some_and(|(_, m, d)| (1..=12).contains(&m) && (1..=31).contains(&d));
    if shaped && in_range {
        Ok(cell.to_string())
    } else {
        Err(bad())
    }
}

fn civil_day(day: &str) -> Option<(i64, i64, i64)> {
    let mut parts = day.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((y, m, d))
}

/// UTC midnight of the row's day. [`parse_audit_md`] validated the shape, so the fallback is
/// unreachable for a parsed row.
fn day_ms(day: &str) -> i64 {
    civil_day(day).map_or(0, |(y, m, d)| days_from_civil(y, m, d) * DAY_MS)
}

fn host_of(cell: &str) -> String {
    if is_missing(cell) {
        // The short 2026-09-06 row is the established case; a full row with an unreadable host is
        // treated the same way rather than inventing a host name.
        "unknown".to_string()
    } else {
        cell.to_string()
    }
}

/// The health half as an event of the live observation source.
///
/// Its payload carries exactly `index_health_log`'s columns (minus the local `id`) plus `origin`,
/// so a report can read imported and observed rows with one shape. The id leaves `ts` out, which
/// is what makes a re-import -- or two imports of the same row from two files -- one event.
pub fn audit_health_event(row: &AuditRow, project_id: &str) -> TelemetryEvent {
    let machine_id = format!("imported:{}", row.host);
    let time_ms = day_ms(&row.day);
    let with_ts = json!({
        "chunks": row.chunks,
        "error": Value::Null,
        "fresh": row.fresh.as_deref(),
        "fts": row.fts.as_deref(),
        "local_project_id": Value::Null,
        "machine_id": machine_id,
        "machine_source": IMPORTED_SOURCE,
        "not_chunked": row.not_chunked,
        "origin": IMPORTED_SOURCE,
        "project_id": project_id,
        "real_gap": row.real_gap,
        "relationships": row.relationships,
        "root": row.root,
        "status": row.status,
        "ts": time_ms,
    });
    let payload = sorted_compact_json(&with_ts);
    let mut without_ts = with_ts;
    if let Value::Object(map) = &mut without_ts {
        map.remove("ts");
    }
    TelemetryEvent {
        source: SOURCE_HEALTH.to_string(),
        event_id: health_event_id(
            &machine_id,
            &row.day,
            &row.root,
            &sorted_compact_json(&without_ts),
        ),
        machine_id,
        machine_source: IMPORTED_SOURCE.to_string(),
        project_id: Some(project_id.to_string()),
        time_ms: Some(time_ms),
        payload,
        rowid: None,
    }
}

/// Write both halves of `rows` into `store` and report what happened. The caller pushes: like
/// every other write command, the replica is written first, so a dead network loses nothing.
///
/// Health rows are `INSERT OR IGNORE` (their id is a content key); a usage row whose `(day, host)`
/// already exists is never replaced, and a differing one is reported as a conflict instead.
/// "Already exists" is asked of the replica, so it syncs first, push then pull like every other
/// sync: the push sends what an earlier run wrote but could not push (a re-run is how that is
/// repaired), and the pull lets the replica see rows another import already pushed instead of
/// inserting them again.
///
/// Run it on one machine. Two machines importing the same rows at the same moment both see them
/// absent and both insert; the second push then collides on the remote's keys, and nothing here
/// reconciles two diverged replicas. The import is a one-time retirement step on one machine.
pub fn import_audit(
    store: &mut dyn MeasureStore,
    rows: &[AuditRow],
    project_id: &str,
) -> Result<Value, String> {
    crate::measure::sync(store)?;
    let events: Vec<TelemetryEvent> = rows
        .iter()
        .map(|row| audit_health_event(row, project_id))
        .collect();
    let health_inserted = store.insert_events(&events)?;
    let usage: Vec<ImportedUsage> = rows.iter().map(|row| row.usage.clone()).collect();
    let outcome = store.upsert_imported_usage(&usage)?;
    let mut hosts: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        *hosts.entry(row.host.clone()).or_default() += 1;
    }
    let conflicts: Vec<Value> = outcome
        .conflicts
        .iter()
        .map(|c| {
            json!({
                "day": c.incoming.day,
                "host": c.incoming.host,
                "existing": c.existing.to_json(),
                "incoming": c.incoming.to_json(),
            })
        })
        .collect();
    Ok(json!({
        "rows": rows.len(),
        "health_inserted": health_inserted,
        "health_already_present": events.len().saturating_sub(health_inserted),
        "usage_inserted": outcome.inserted,
        "usage_unchanged": outcome.unchanged,
        "usage_conflicts": conflicts,
        "hosts": hosts,
    }))
}
