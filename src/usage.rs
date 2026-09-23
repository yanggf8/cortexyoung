//! Local-only navigation telemetry.
//! Raw queries are deliberately never stored; the data exists to improve routing and ranking.
use crate::data_dir;
use crate::navigate::NavigateResult;
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsageReport {
    pub window_days: u32,
    pub events: i64,
    pub cort_events: i64,
    pub symbol_hit_events: i64,
    pub document_hit_events: i64,
    pub no_hit_events: i64,
    pub top_documents: BTreeMap<String, i64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn db_path() -> PathBuf {
    data_dir::data_dir().join("usage.db")
}

fn open() -> rusqlite::Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS navigation_log (
            ts INTEGER NOT NULL,
            project_id TEXT NOT NULL,
            mode TEXT NOT NULL,
            query_shape TEXT NOT NULL,
            query_hash TEXT NOT NULL,
            symbol_hits INTEGER NOT NULL,
            document_hits INTEGER NOT NULL,
            file_hits INTEGER NOT NULL,
            cort_hits INTEGER NOT NULL,
            route_kind TEXT NOT NULL,
            primary_document TEXT,
            primary_heading TEXT
        );
        CREATE INDEX IF NOT EXISTS navigation_log_ts ON navigation_log(ts);
        CREATE INDEX IF NOT EXISTS navigation_log_document ON navigation_log(primary_document);",
    )?;
    Ok(conn)
}

fn query_shape(query: &str) -> &'static str {
    let tokens = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .count();
    if query.contains('/') || query.contains('.') {
        "path_like"
    } else if query.contains('?') || query.split_whitespace().count() >= 5 {
        "question_like"
    } else if tokens <= 1 {
        "one_token"
    } else {
        "multi_token"
    }
}

fn query_hash(query: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(query.as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn route_kind(result: &NavigateResult) -> &'static str {
    match (result.symbols.is_empty(), result.documents.is_empty()) {
        (false, false) => "mixed",
        (false, true) => "symbol",
        (true, false) => "document",
        (true, true) => "none",
    }
}

pub fn record_navigation(
    root: &Path,
    query: &str,
    used_cort: bool,
    cort_hits: usize,
    result: &NavigateResult,
) {
    if std::env::var_os("CLAUDECAT_NO_USAGE").is_some() {
        return;
    }
    let Ok(conn) = open() else { return };
    let primary = result.documents.first();
    let _ = conn.execute(
        "INSERT INTO navigation_log
         (ts, project_id, mode, query_shape, query_hash, symbol_hits, document_hits,
          file_hits, cort_hits, route_kind, primary_document, primary_heading)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            now_ms(),
            crate::cort::project_id(
                &std::fs::canonicalize(root)
                    .unwrap_or_else(|_| root.to_path_buf())
                    .to_string_lossy()
            ),
            if used_cort { "cort" } else { "tree" },
            query_shape(query),
            query_hash(query),
            result.symbols.len() as i64,
            result.documents.len() as i64,
            result.files.len() as i64,
            cort_hits as i64,
            route_kind(result),
            primary.map(|d| d.path.as_str()),
            primary.map(|d| d.heading_path.join(" > ")),
        ],
    );
}

pub fn report(days: u32) -> Option<UsageReport> {
    let conn = open().ok()?;
    let since = now_ms().saturating_sub(days as i64 * 24 * 60 * 60 * 1000);
    let mut out = UsageReport {
        window_days: days,
        ..Default::default()
    };
    out.events = conn
        .query_row(
            "SELECT COUNT(*) FROM navigation_log WHERE ts >= ?1",
            [since],
            |row| row.get(0),
        )
        .ok()?;
    out.cort_events = count_where(&conn, since, "mode = 'cort'");
    out.symbol_hit_events = count_where(&conn, since, "symbol_hits > 0");
    out.document_hit_events = count_where(&conn, since, "document_hits > 0");
    out.no_hit_events = count_where(&conn, since, "route_kind = 'none'");
    if let Ok(mut stmt) = conn.prepare(
        "SELECT primary_document, COUNT(*) FROM navigation_log
         WHERE ts >= ?1 AND primary_document IS NOT NULL
         GROUP BY primary_document ORDER BY COUNT(*) DESC, primary_document LIMIT 20",
    ) {
        if let Ok(rows) = stmt.query_map([since], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for row in rows.flatten() {
                out.top_documents.insert(row.0, row.1);
            }
        }
    }
    Some(out)
}

fn count_where(conn: &Connection, since: i64, predicate: &str) -> i64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM navigation_log WHERE ts >= ?1 AND {predicate}"),
        [since],
        |row| row.get(0),
    )
    .unwrap_or(0)
}
