//! FTS sanitizer + keyword search.

use crate::db::Db;
use crate::errors::CortError;
use rusqlite::params;
use serde_json::json;

pub const MAX_OR_TERMS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedFtsQuery {
    pub query: String,
    pub truncated_query: bool,
}

pub fn sanitize_fts_query(raw: &str) -> Result<SanitizedFtsQuery, CortError> {
    // Spec §1.5: String(raw ?? '').trim().split(/\s+/).filter(t => t.length > 0)
    // Keep trim() even though split_whitespace already skips leading/trailing WS.
    #[allow(clippy::trim_split_whitespace)]
    let terms: Vec<&str> = raw
        .trim()
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return Err(CortError::new("empty_query", json!({ "raw": raw })));
    }
    let truncated_query = terms.len() > MAX_OR_TERMS;
    let kept = &terms[..MAX_OR_TERMS.min(terms.len())];
    let quoted: Vec<String> = kept
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    Ok(SanitizedFtsQuery {
        query: quoted.join(" OR "),
        truncated_query,
    })
}

#[derive(Debug, Clone)]
pub struct KeywordHit {
    pub chunk_id: String,
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub chunk_type: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub content: String,
    pub language: Option<String>,
    pub chunk_source: String,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct KeywordSearchResult {
    pub rows: Vec<KeywordHit>,
    pub truncated_query: bool,
}

pub fn keyword_search(
    db: &Db,
    project_id: &str,
    raw: &str,
    limit: i64,
) -> Result<KeywordSearchResult, CortError> {
    let sanitized = sanitize_fts_query(raw)?;
    let result = db.prepare(
        "SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type, c.start_line, c.end_line,
                c.content, c.language, c.chunk_source, bm25(chunks_fts) AS score
           FROM chunks_fts
           JOIN chunks c ON c.rowid = chunks_fts.rowid
          WHERE chunks_fts MATCH ?1 AND c.project_id = ?2
          ORDER BY score
          LIMIT ?3",
    );
    let rows = result.and_then(|mut stmt| {
        let mapped = stmt.query_map(params![sanitized.query, project_id, limit], |row| {
            Ok(KeywordHit {
                chunk_id: row.get(0)?,
                file_path: row.get(1)?,
                symbol_name: row.get(2)?,
                chunk_type: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                content: row.get(6)?,
                language: row.get(7)?,
                chunk_source: row.get(8)?,
                score: row.get(9)?,
            })
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()
    });
    match rows {
        Ok(rows) => Ok(KeywordSearchResult {
            rows,
            truncated_query: sanitized.truncated_query,
        }),
        Err(err) => Err(CortError::new(
            "fts_query_failed",
            json!({
                "query": sanitized.query,
                "message": err.to_string(),
            }),
        )),
    }
}
