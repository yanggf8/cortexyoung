//! Reverse dependents. JS `src/impact.js`.

use crate::chunker::{extract_file, ExtractFileArgs};
use crate::db::Db;
use crate::errors::CortError;
use crate::graph::{
    build_import_map, get_transitive_dependents, resolve_targets, unresolved_inline,
};
use crate::indexer::IndexError;
use crate::staleness::compute_stale;
use rusqlite::params_from_iter;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const DEFAULT_DEPTH: i64 = 3;

fn map_index(err: IndexError) -> CortError {
    match err {
        IndexError::Cort(c) => c,
        other => CortError::new("storage_busy", json!({ "message": other.to_string() })),
    }
}

fn map_sql(err: rusqlite::Error) -> CortError {
    CortError::new("storage_busy", json!({ "message": err.to_string() }))
}

pub fn impact_command(
    db: &Db,
    bin: &str,
    root: impl AsRef<Path>,
    project_id: &str,
    symbol: &str,
    depth: i64,
) -> Result<Value, CortError> {
    let root = root.as_ref();
    let names: Vec<&str> = symbol
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let seeds: Vec<(String, String, Option<String>, i64, i64)> = if names.is_empty() {
        Vec::new()
    } else {
        let placeholders = names
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT chunk_id, file_path, symbol_name, start_line, end_line
               FROM chunks WHERE project_id = ?1 AND symbol_name IN ({placeholders})
              ORDER BY file_path, start_line"
        );
        let mut stmt = db.prepare(&sql).map_err(map_sql)?;
        let rows = stmt
            .query_map(
                params_from_iter(
                    std::iter::once(&project_id as &dyn rusqlite::types::ToSql)
                        .chain(names.iter().map(|n| n as &dyn rusqlite::types::ToSql)),
                ),
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(map_sql)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_sql)?
    };

    let mut merged: HashMap<String, crate::graph::Dependent> = HashMap::new();
    for seed in &seeds {
        let deps = get_transitive_dependents(db, &seed.0, depth).map_err(map_sql)?;
        for dep in deps {
            match merged.get(&dep.chunk_id) {
                Some(prev) if prev.hop <= dep.hop => {}
                _ => {
                    merged.insert(dep.chunk_id.clone(), dep);
                }
            }
        }
    }
    let seed_ids: HashSet<&str> = seeds.iter().map(|s| s.0.as_str()).collect();
    let mut dependents: Vec<_> = merged
        .into_values()
        .filter(|d| !seed_ids.contains(d.chunk_id.as_str()))
        .collect();
    dependents.sort_by(|a, b| a.hop.cmp(&b.hop).then_with(|| a.chunk_id.cmp(&b.chunk_id)));

    let mut unresolved = Vec::new();
    let mut seen_symbols: HashSet<String> = HashSet::new();
    for seed in &seeds {
        let abs = root.join(&seed.1);
        if !abs.is_file() {
            continue;
        }
        let source = match std::fs::read_to_string(&abs) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let extracted = extract_file(ExtractFileArgs {
            bin,
            project_id,
            file_path: &seed.1,
            abs_path: abs.to_str().unwrap_or(""),
            source: &source,
            timeout_ms: None,
        })?;
        let import_map = build_import_map(&extracted.edges);
        for e in &extracted.edges {
            if e.source_symbol.as_deref() != seed.2.as_deref() {
                continue;
            }
            if !seen_symbols.insert(e.raw_target.clone()) {
                continue;
            }
            let targets = resolve_targets(db, project_id, &seed.1, &import_map, &e.raw_target)
                .map_err(map_sql)?;
            if targets.is_empty() {
                let u = unresolved_inline(&e.raw_target);
                unresolved.push(json!({
                    "symbol": e.raw_target,
                    "rel_type": e.rel_type,
                    "confidence": u.confidence,
                    "confidence_score": u.confidence_score,
                    "confidence_reasoning": u.confidence_reasoning,
                }));
            }
        }
    }

    let stale = compute_stale(db, bin, root, project_id).map_err(map_index)?;
    let seed_json: Vec<Value> = seeds
        .iter()
        .map(|s| {
            json!({
                "chunk_id": s.0,
                "file_path": s.1,
                "start_line": s.3,
            })
        })
        .collect();
    let dep_json: Vec<Value> = dependents
        .iter()
        .map(|d| {
            json!({
                "chunk_id": d.chunk_id,
                "symbol_name": d.symbol_name,
                "file_path": d.file_path,
                "start_line": d.start_line,
                "end_line": d.end_line,
                "hop": d.hop,
            })
        })
        .collect();
    Ok(json!({
        "symbol": symbol,
        "depth": depth,
        "seed_count": seeds.len(),
        "seeds": seed_json,
        "dependents": dep_json,
        "dependent_count": dep_json.len(),
        "unresolved": unresolved,
        "index_is_stale": stale.index_is_stale,
    }))
}
