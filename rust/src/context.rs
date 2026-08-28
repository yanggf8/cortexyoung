//! Context packets. JS `src/context.js`, seed selection superseded by
//! `docs/superpowers/plans/2026-08-28-codex-fix-proposal.md` §4.

use crate::budget::{apply_budget, estimate_tokens};
use crate::chunker::{canonical_owner, extract_file, ExtractFileArgs};
use crate::db::Db;
use crate::errors::CortError;
use crate::fts::keyword_search;
use crate::graph::{
    build_import_map, get_neighbors, resolve_targets, unresolved_inline, Neighbor,
};
use crate::indexer::IndexError;
use crate::staleness::compute_stale;
use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub const DEFAULT_BUDGET: usize = 1500;
pub const NEIGHBORS_PER_SEED: i64 = 8;
pub const CONTENT_HEAD_LINES: usize = 12;
const MAX_SEEDS: usize = 5;

fn map_index(err: IndexError) -> CortError {
    match err {
        IndexError::Cort(c) => c,
        other => CortError::new("storage_busy", json!({ "message": other.to_string() })),
    }
}

fn map_sql(err: rusqlite::Error) -> CortError {
    CortError::new("storage_busy", json!({ "message": err.to_string() }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolQuery {
    Qualified { owner: String, member: String },
    Unqualified(String),
}

fn last_colon_colon_outside_generics(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut last = None;
    let mut i = 0;
    while i + 1 < b.len() {
        match b[i] {
            b'<' => depth += 1,
            b'>'
                if depth > 0 => {
                    depth -= 1;
                }
            b':' if depth == 0 && b[i + 1] == b':' => {
                last = Some(i);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    last
}

pub fn parse_symbol_query(query: &str) -> SymbolQuery {
    if let Some(i) = last_colon_colon_outside_generics(query) {
        let owner = &query[..i];
        let member = &query[i + 2..];
        if !owner.is_empty() && !member.is_empty() {
            return SymbolQuery::Qualified {
                owner: owner.to_string(),
                member: member.to_string(),
            };
        }
    }
    SymbolQuery::Unqualified(query.to_string())
}

#[derive(Clone)]
struct SeedRow {
    chunk_id: String,
    file_path: String,
    symbol_name: Option<String>,
    chunk_type: Option<String>,
    start_line: i64,
    end_line: i64,
    content: String,
    #[allow(dead_code)]
    language: Option<String>,
}

fn exact_symbol_seeds(db: &Db, project_id: &str, symbol_name: &str) -> Result<Vec<SeedRow>, CortError> {
    let mut stmt = db
        .prepare(
            "SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, language
               FROM chunks WHERE project_id = ?1 AND symbol_name = ?2
              ORDER BY file_path, start_line",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map(params![project_id, symbol_name], |r| {
            Ok(SeedRow {
                chunk_id: r.get(0)?,
                file_path: r.get(1)?,
                symbol_name: r.get(2)?,
                chunk_type: r.get(3)?,
                start_line: r.get(4)?,
                end_line: r.get(5)?,
                content: r.get(6)?,
                language: r.get(7)?,
            })
        })
        .map_err(map_sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_sql)
}

fn sort_dedupe_then_limit(mut rows: Vec<SeedRow>) -> (Vec<SeedRow>, usize, bool) {
    rows.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.start_line.cmp(&b.start_line))
            .then(a.chunk_id.cmp(&b.chunk_id))
    });
    let mut seen = HashSet::new();
    rows.retain(|r| seen.insert(r.chunk_id.clone()));
    let total = rows.len();
    let truncated = total > MAX_SEEDS;
    rows.truncate(MAX_SEEDS);
    (rows, total, truncated)
}

#[derive(Clone, Serialize)]
struct NeighborOut {
    chunk_id: String,
    symbol_name: Option<String>,
    file_path: String,
    start_line: i64,
    end_line: i64,
    rel_type: String,
    confidence: String,
    confidence_score: f64,
    direction: String,
}

impl From<Neighbor> for NeighborOut {
    fn from(n: Neighbor) -> Self {
        Self {
            chunk_id: n.chunk_id,
            symbol_name: n.symbol_name,
            file_path: n.file_path,
            start_line: n.start_line,
            end_line: n.end_line,
            rel_type: n.rel_type,
            confidence: n.confidence,
            confidence_score: n.confidence_score,
            direction: n.direction,
        }
    }
}

#[derive(Clone, Serialize)]
struct UnresolvedOut {
    symbol: String,
    rel_type: String,
    confidence: String,
    confidence_score: f64,
    confidence_reasoning: String,
}

#[derive(Clone, Serialize)]
struct ContextSeed {
    chunk_id: String,
    file_path: String,
    symbol_name: Option<String>,
    chunk_type: Option<String>,
    start_line: i64,
    end_line: i64,
    content: String,
    content_truncated: bool,
    neighbors: Vec<NeighborOut>,
    unresolved: Vec<UnresolvedOut>,
}

fn unresolved_for(
    db: &Db,
    bin: &str,
    root: &Path,
    project_id: &str,
    seed: &SeedRow,
) -> Result<Vec<UnresolvedOut>, CortError> {
    let abs = root.join(&seed.file_path);
    if !abs.is_file() {
        return Ok(Vec::new());
    }
    let source = match std::fs::read_to_string(&abs) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    let extracted = extract_file(ExtractFileArgs {
        bin,
        project_id,
        file_path: &seed.file_path,
        abs_path: abs.to_str().unwrap_or(""),
        source: &source,
        timeout_ms: None,
    })?;
    let import_map = build_import_map(&extracted.edges);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for e in &extracted.edges {
        if e.source_symbol.as_deref() != seed.symbol_name.as_deref() {
            continue;
        }
        if !seen.insert(e.raw_target.clone()) {
            continue;
        }
        let targets =
            resolve_targets(db, project_id, &seed.file_path, &import_map, &e.raw_target)
                .map_err(map_sql)?;
        if targets.is_empty() {
            let u = unresolved_inline(&e.raw_target);
            out.push(UnresolvedOut {
                symbol: e.raw_target.clone(),
                rel_type: e.rel_type.clone(),
                confidence: u.confidence,
                confidence_score: u.confidence_score,
                confidence_reasoning: u.confidence_reasoning,
            });
        }
    }
    Ok(out)
}

fn packet_tokens(
    query: &str,
    resolution: &str,
    kept: &[ContextSeed],
    seed_count: usize,
    truncated_query: bool,
) -> usize {
    let resolution = if kept.is_empty() { "none" } else { resolution };
    let v = json!({
        "query": query,
        "resolution": resolution,
        "seeds": kept,
        "seed_count": seed_count,
        "truncated": true,
        "truncated_query": truncated_query,
        "index_is_stale": false,
    });
    estimate_tokens(&v.to_string())
}

/// Clippy's 7-arg ceiling is exceeded by the JS parity signature; the three
/// presentation-level knobs travel together so the command keeps its mirror shape.
#[derive(Debug, Clone, Copy)]
pub struct ContextOptions {
    pub budget: usize,
    pub include_ambiguous: bool,
    pub full_content: bool,
}

pub fn context_command(
    db: &Db,
    bin: &str,
    root: impl AsRef<Path>,
    project_id: &str,
    query: &str,
    opts: ContextOptions,
) -> Result<Value, CortError> {
    let ContextOptions { budget, include_ambiguous, full_content } = opts;
    let root = root.as_ref();
    let parsed = parse_symbol_query(query);

    let mut resolution = "exact_symbol";
    let mut truncated_query = false;
    let limit_truncated;
    let seed_count_before_limit;

    let seed_rows = match parsed {
        SymbolQuery::Qualified { owner, member } => {
            let symbol = format!("{}::{member}", canonical_owner(&owner));
            let all = exact_symbol_seeds(db, project_id, &symbol)?;
            let (kept, total, trunc) = sort_dedupe_then_limit(all);
            seed_count_before_limit = total;
            limit_truncated = trunc;
            if kept.is_empty() {
                resolution = "none";
            }
            kept
        }
        SymbolQuery::Unqualified(text) => {
            let all = exact_symbol_seeds(db, project_id, &text)?;
            if all.is_empty() {
                let fts = keyword_search(db, project_id, &text, MAX_SEEDS as i64)?;
                truncated_query = fts.truncated_query;
                resolution = if fts.rows.is_empty() { "none" } else { "fts" };
                let mapped: Vec<SeedRow> = fts
                    .rows
                    .into_iter()
                    .map(|r| SeedRow {
                        chunk_id: r.chunk_id,
                        file_path: r.file_path,
                        symbol_name: r.symbol_name,
                        chunk_type: r.chunk_type,
                        start_line: r.start_line,
                        end_line: r.end_line,
                        content: r.content,
                        language: r.language,
                    })
                    .collect();
                let (kept, total, trunc) = sort_dedupe_then_limit(mapped);
                seed_count_before_limit = total;
                limit_truncated = trunc;
                kept
            } else {
                let (kept, total, trunc) = sort_dedupe_then_limit(all);
                seed_count_before_limit = total;
                limit_truncated = trunc;
                kept
            }
        }
    };

    let mut seeds = Vec::new();
    for row in seed_rows {
        let neighbors = get_neighbors(db, &row.chunk_id, NEIGHBORS_PER_SEED)
            .map_err(map_sql)?
            .into_iter()
            .filter(|n| include_ambiguous || n.confidence != "AMBIGUOUS")
            .map(NeighborOut::from)
            .collect();
        let unresolved = unresolved_for(db, bin, root, project_id, &row)?;
        let lines: Vec<&str> = row.content.split('\n').collect();
        let content_truncated = !full_content && lines.len() > CONTENT_HEAD_LINES;
        let content = if content_truncated {
            format!("{}\n…", lines[..CONTENT_HEAD_LINES].join("\n"))
        } else {
            row.content.clone()
        };
        seeds.push(ContextSeed {
            chunk_id: row.chunk_id,
            file_path: row.file_path,
            symbol_name: row.symbol_name,
            chunk_type: row.chunk_type,
            start_line: row.start_line,
            end_line: row.end_line,
            content,
            content_truncated,
            neighbors,
            unresolved,
        });
    }

    let seed_count = seed_count_before_limit.max(seeds.len());
    let budgeted = apply_budget(seeds, budget, |s| serde_json::to_string(s).unwrap_or_default());
    let mut kept = budgeted.kept;
    let mut truncated = budgeted.truncated || limit_truncated;
    if !kept.is_empty()
        && packet_tokens(query, resolution, &kept, seed_count, truncated_query) as f64
            > budget as f64 * 1.15
    {
        let mut best = kept.clone();
        for keep_n in (0..NEIGHBORS_PER_SEED).rev() {
            let trimmed: Vec<ContextSeed> = kept
                .iter()
                .map(|s| {
                    let mut t = s.clone();
                    t.neighbors.truncate(keep_n as usize);
                    t
                })
                .collect();
            best = trimmed.clone();
            if packet_tokens(query, resolution, &trimmed, seed_count, truncated_query) as f64
                <= budget as f64 * 1.15
            {
                break;
            }
        }
        kept = best;
        truncated = true;
    }

    let stale = compute_stale(db, bin, root, project_id).map_err(map_index)?;
    let resolution = if kept.is_empty() { "none" } else { resolution };
    Ok(json!({
        "query": query,
        "resolution": resolution,
        "seeds": kept,
        "seed_count": seed_count,
        "truncated": truncated,
        "truncated_query": truncated_query,
        "index_is_stale": stale.index_is_stale,
    }))
}
