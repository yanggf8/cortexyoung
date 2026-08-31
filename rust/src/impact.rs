//! Reverse dependents.

use crate::chunker::{extract_file, ExtractFileArgs};
use crate::db::Db;
use crate::errors::CortError;
use crate::graph::{
    build_import_map, get_transitive_dependents, resolve_edge_targets, unresolved_inline,
    ReceiverIndex,
};
use crate::indexer::IndexError;
use crate::staleness::compute_stale;
use rusqlite::{params, params_from_iter};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub const DEFAULT_DEPTH: i64 = 3;

/// Every `calls` edge in the project that knows its call site, as source -> edges in
/// (line, form) order, each carrying the chunk it points at.
///
/// One pass over the edge table instead of an `IN (…)` lookup per dependent: the dependent list is
/// bounded by nothing (a hub symbol returns hundreds), and a per-dependent query with a hundred-item
/// parameter list is the sort of thing that turns into a `SQLITE_TOOBIGPARAMS` on someone else's
/// repository. The whole project's edges fit in memory at the sizes this tool indexes, and reading
/// them once is also what keeps `--depth 3` on a hub no more expensive than `--depth 1`.
/// One stored call site: `(line that names the callee, form it was extracted as, callee chunk)`.
type CallSite = (i64, String, String);

fn call_sites_by_source(
    db: &rusqlite::Connection,
    project_id: &str,
) -> Result<HashMap<String, Vec<CallSite>>, rusqlite::Error> {
    let mut stmt = db.prepare(
        "SELECT r.source_chunk_id, r.call_site_line, r.call_form, r.target_chunk_id
           FROM relationships r JOIN chunks c ON c.chunk_id = r.source_chunk_id
          WHERE r.rel_type = 'calls' AND r.call_site_line IS NOT NULL AND c.project_id = ?1
          ORDER BY r.source_chunk_id, r.call_site_line, r.call_form",
    )?;
    let rows = stmt.query_map(params![project_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut map: HashMap<String, Vec<CallSite>> = HashMap::new();
    for row in rows {
        let (source, line, form, target) = row?;
        map.entry(source).or_default().push((line, form, target));
    }
    Ok(map)
}

/// The call site that ties `chunk_id` to something nearer the seed: the earliest recorded line whose
/// target is a seed or a lower-hop dependent, and the form it was extracted as. `None` means this
/// dependent reached the seed through an `imports`/`exports` edge, or through an edge written before
/// schema v4 recorded lines -- it is rendered as a dash, never as a guessed line.
fn call_site_for(
    sites: &HashMap<String, Vec<CallSite>>,
    chunk_id: &str,
    parents: &HashSet<&str>,
) -> Option<(i64, String)> {
    let edges = sites.get(chunk_id)?;
    // Already in (line, form) order, so the first hit is the earliest call site into the parent set.
    edges
        .iter()
        .find(|(_, _, target)| parents.contains(target.as_str()))
        .map(|(line, form, _)| (*line, form.clone()))
}

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

    // The same gate the graph resolved with, so the "what could not be attached" list below cannot
    // disagree with the edges above it.
    let receiver_index = ReceiverIndex::build(db, project_id).map_err(map_sql)?;
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
            let targets =
                resolve_edge_targets(db, project_id, &seed.1, &import_map, e, &receiver_index)
                    .map_err(map_sql)?;
            if targets.is_empty() {
                let u = unresolved_inline(&e.raw_target);
                unresolved.push(json!({
                    "symbol": e.raw_target,
                    "rel_type": e.rel_type,
                    "call_form": e.call_form.as_str(),
                    "call_site_line": e.start_line,
                    "confidence": u.confidence,
                    "confidence_score": u.confidence_score,
                    "confidence_reasoning": u.confidence_reasoning,
                }));
            }
        }
    }

    let stale = compute_stale(db, bin, root, project_id).map_err(map_index)?;
    // Which line inside each dependent names the thing it calls. A dependent's parents are taken as
    // "the seeds plus everything nearer the seed than it is", because the recursion records the hop
    // a chunk entered at but not which edge carried it: the claim printed is then "this dependent
    // calls something it depends on, from this line", which is what a reader can actually check.
    let sites = call_sites_by_source(db, project_id).map_err(map_sql)?;
    let mut groups: BTreeMap<i64, Vec<&str>> = BTreeMap::new();
    for d in &dependents {
        groups.entry(d.hop).or_default().push(d.chunk_id.as_str());
    }
    let mut parents: HashSet<&str> = seeds.iter().map(|s| s.0.as_str()).collect();
    let mut call_sites: HashMap<&str, (i64, String)> = HashMap::new();
    for ids in groups.values() {
        for id in ids {
            if let Some(site) = call_site_for(&sites, id, &parents) {
                call_sites.insert(id, site);
            }
        }
        parents.extend(ids.iter().copied());
    }
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
            let site = call_sites.get(d.chunk_id.as_str());
            json!({
                "chunk_id": d.chunk_id,
                "symbol_name": d.symbol_name,
                "file_path": d.file_path,
                "start_line": d.start_line,
                "end_line": d.end_line,
                "hop": d.hop,
                // The line to read to confirm this dependent, and how the extractor saw that call.
                "call_site_line": site.map(|(line, _)| *line),
                "call_form": site.map(|(_, form)| form.as_str()),
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
