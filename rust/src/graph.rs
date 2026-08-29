//! Relationships + containment join. JS `src/graph.js` (+ `containmentJoin` from `src/struct.js`).
//! Spec §5.6 assigns `applyBudget` to struct/context (Job D), not this module.

use crate::chunker::{Chunk, Edge};
use crate::db::Db;
use rusqlite::params;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceScore {
    pub extracted: f64,
    pub inferred: f64,
    pub ambiguous: f64,
}

pub const CONFIDENCE_SCORE: ConfidenceScore = ConfidenceScore {
    extracted: 1.0,
    inferred: 0.7,
    ambiguous: 0.5,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipRow {
    pub source_chunk_id: String,
    pub target_chunk_id: String,
    pub rel_type: String,
    pub confidence: String,
    pub confidence_score: f64,
    pub confidence_reasoning: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedInline {
    pub confidence: String,
    pub confidence_score: f64,
    pub confidence_reasoning: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Neighbor {
    pub chunk_id: String,
    pub symbol_name: Option<String>,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub rel_type: String,
    pub confidence: String,
    pub confidence_score: f64,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dependent {
    pub chunk_id: String,
    pub symbol_name: Option<String>,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub hop: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainingChunk {
    pub chunk_id: String,
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub chunk_type: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub language: Option<String>,
}

pub fn build_import_map(edges: &[Edge]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for e in edges {
        if e.rel_type == "imports" {
            map.insert(e.raw_target.clone(), e.raw_target.clone());
        }
    }
    map
}

fn posix_dirname(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        None => ".".to_string(),
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    }
}

fn posix_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let absolute = path.starts_with('/');
    let trailing = path.len() > 1 && path.ends_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            // Absolute `..` always pops (no-op at root). Relative pops unless the
            // stack is empty or already trailing `..` (those stay as `..`).
            if absolute || stack.last().is_some_and(|s| *s != "..") {
                stack.pop();
            } else {
                stack.push("..");
            }
        } else {
            stack.push(part);
        }
    }
    let mut out = stack.join("/");
    if absolute {
        out = format!("/{out}");
        if out == "/" {
            return "/".to_string();
        }
    } else if out.is_empty() {
        out = ".".to_string();
    }
    if trailing && out != "/" && out != "." {
        out.push('/');
    }
    out
}

fn posix_join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        return posix_normalize(b);
    }
    if a.is_empty() || a == "." {
        return posix_normalize(b);
    }
    let joined = if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    };
    posix_normalize(&joined)
}

fn strip_last_ext(path: &str) -> &str {
    match path.rfind('.') {
        Some(dot) => {
            let after = &path[dot + 1..];
            if after.is_empty() || after.contains('/') {
                path
            } else {
                &path[..dot]
            }
        }
        None => path,
    }
}

fn imported_path_prefixes(file_path: &str, import_map: &HashMap<String, String>) -> Vec<String> {
    let dir = posix_dirname(file_path);
    import_map
        .keys()
        .map(|spec| {
            if spec.starts_with('.') {
                posix_join(&dir, spec)
            } else {
                spec.clone()
            }
        })
        .collect()
}

pub fn resolve_targets(
    db: &Db,
    project_id: &str,
    file_path: &str,
    import_map: &HashMap<String, String>,
    symbol: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = db.prepare(
        "SELECT chunk_id, file_path FROM chunks
          WHERE project_id = ?1 AND symbol_name = ?2 ORDER BY chunk_id",
    )?;
    let all: Vec<(String, String)> = stmt
        .query_map(params![project_id, symbol], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if all.is_empty() {
        return Ok(Vec::new());
    }

    let same_file: Vec<String> = all
        .iter()
        .filter(|(_, fp)| fp == file_path)
        .map(|(id, _)| id.clone())
        .collect();
    if !same_file.is_empty() {
        return Ok(same_file);
    }

    let prefixes = imported_path_prefixes(file_path, import_map);
    let via_import: Vec<String> = all
        .iter()
        .filter(|(_, fp)| {
            let no_ext = strip_last_ext(fp);
            prefixes
                .iter()
                .any(|p| no_ext == p || no_ext.ends_with(&format!("/{p}")))
        })
        .map(|(id, _)| id.clone())
        .collect();
    if !via_import.is_empty() {
        return Ok(via_import);
    }

    Ok(all.into_iter().map(|(id, _)| id).collect())
}

pub fn relationship_rows_for_file(
    db: &Db,
    project_id: &str,
    file_path: &str,
    chunks: &[Chunk],
    edges: &[Edge],
) -> rusqlite::Result<Vec<RelationshipRow>> {
    let mut chunk_by_symbol: HashMap<String, String> = HashMap::new();
    for c in chunks {
        if let Some(name) = &c.symbol_name {
            chunk_by_symbol.insert(name.clone(), c.chunk_id.clone());
        }
    }
    relationship_rows_for_symbol_map(db, project_id, file_path, &chunk_by_symbol, edges)
}

/// Same resolution, but the caller owns the symbol→chunk map. The global rebuild loads that
/// map straight from `chunks` instead of materialising every chunk body.
pub fn relationship_rows_for_symbol_map(
    db: &Db,
    project_id: &str,
    file_path: &str,
    chunk_by_symbol: &HashMap<String, String>,
    edges: &[Edge],
) -> rusqlite::Result<Vec<RelationshipRow>> {
    let import_map = build_import_map(edges);
    let mut rows = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for e in edges {
        let Some(source_symbol) = &e.source_symbol else {
            continue;
        };
        let Some(source_chunk_id) = chunk_by_symbol.get(source_symbol) else {
            continue;
        };
        let targets: Vec<String> =
            resolve_targets(db, project_id, file_path, &import_map, &e.raw_target)?
                .into_iter()
                .filter(|id| id != source_chunk_id)
                .collect();
        if targets.is_empty() {
            continue;
        }
        let n = targets.len();
        let confidence = if n == 1 { "INFERRED" } else { "AMBIGUOUS" };
        let score = if n == 1 {
            CONFIDENCE_SCORE.inferred
        } else {
            CONFIDENCE_SCORE.ambiguous * (1.0 / n as f64)
        };
        let reasoning = if n == 1 {
            format!("resolved: {}", e.raw_target)
        } else {
            format!("ambiguous: {} ({n} candidates)", e.raw_target)
        };
        for target in targets {
            let key = format!("{source_chunk_id} {target} {}", e.rel_type);
            if !seen.insert(key) {
                continue;
            }
            rows.push(RelationshipRow {
                source_chunk_id: source_chunk_id.clone(),
                target_chunk_id: target,
                rel_type: e.rel_type.clone(),
                confidence: confidence.to_string(),
                confidence_score: score,
                confidence_reasoning: reasoning.clone(),
            });
        }
    }
    Ok(rows)
}

pub fn unresolved_inline(symbol: &str) -> UnresolvedInline {
    UnresolvedInline {
        confidence: "AMBIGUOUS".to_string(),
        confidence_score: CONFIDENCE_SCORE.ambiguous,
        confidence_reasoning: format!("unresolved: {symbol}"),
    }
}

pub fn get_neighbors(db: &Db, chunk_id: &str, limit: i64) -> rusqlite::Result<Vec<Neighbor>> {
    let mut stmt = db.prepare(
        "SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, c.end_line,
                r.rel_type, r.confidence, r.confidence_score, 'outgoing' AS direction
           FROM relationships r JOIN chunks c ON c.chunk_id = r.target_chunk_id
          WHERE r.source_chunk_id = ?1
         UNION ALL
         SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, c.end_line,
                r.rel_type, r.confidence, r.confidence_score, 'incoming' AS direction
           FROM relationships r JOIN chunks c ON c.chunk_id = r.source_chunk_id
          WHERE r.target_chunk_id = ?2
          ORDER BY confidence_score DESC, chunk_id
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![chunk_id, chunk_id, limit], |r| {
        Ok(Neighbor {
            chunk_id: r.get(0)?,
            symbol_name: r.get(1)?,
            file_path: r.get(2)?,
            start_line: r.get(3)?,
            end_line: r.get(4)?,
            rel_type: r.get(5)?,
            confidence: r.get(6)?,
            confidence_score: r.get(7)?,
            direction: r.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn get_transitive_dependents(
    db: &Db,
    chunk_id: &str,
    depth: i64,
) -> rusqlite::Result<Vec<Dependent>> {
    let mut stmt = db.prepare(
        "WITH RECURSIVE dependents(chunk_id, hop) AS (
           SELECT r.source_chunk_id, 1 FROM relationships r WHERE r.target_chunk_id = ?1
           UNION
           SELECT r.source_chunk_id, d.hop + 1
             FROM relationships r JOIN dependents d ON r.target_chunk_id = d.chunk_id
            WHERE d.hop < ?2
         )
         SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, c.end_line, MIN(d.hop) AS hop
           FROM dependents d JOIN chunks c ON c.chunk_id = d.chunk_id
          WHERE c.chunk_id != ?3
          GROUP BY c.chunk_id
          ORDER BY hop, c.chunk_id",
    )?;
    let rows = stmt.query_map(params![chunk_id, depth, chunk_id], |r| {
        Ok(Dependent {
            chunk_id: r.get(0)?,
            symbol_name: r.get(1)?,
            file_path: r.get(2)?,
            start_line: r.get(3)?,
            end_line: r.get(4)?,
            hop: r.get(5)?,
        })
    })?;
    rows.collect()
}

/// Smallest enclosing chunk (span ASC, then start_line DESC). Spec §5.5; JS lives on struct.
pub fn containment_join(
    db: &Db,
    project_id: &str,
    file_path: &str,
    start_line: i64,
    end_line: i64,
) -> rusqlite::Result<Option<ContainingChunk>> {
    let mut stmt = db.prepare(
        "SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, language
           FROM chunks
          WHERE project_id = ?1 AND file_path = ?2 AND start_line <= ?3 AND end_line >= ?4
          ORDER BY (end_line - start_line) ASC, start_line DESC
          LIMIT 1",
    )?;
    let mut rows = stmt.query(params![project_id, file_path, start_line, end_line])?;
    match rows.next()? {
        Some(r) => Ok(Some(ContainingChunk {
            chunk_id: r.get(0)?,
            file_path: r.get(1)?,
            symbol_name: r.get(2)?,
            chunk_type: r.get(3)?,
            start_line: r.get(4)?,
            end_line: r.get(5)?,
            language: r.get(6)?,
        })),
        None => Ok(None),
    }
}

const INSERT_REL: &str = "INSERT INTO relationships
  (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning)
  VALUES (?1, ?2, ?3, ?4, ?5, ?6)
  ON CONFLICT(source_chunk_id, target_chunk_id, rel_type) DO NOTHING";

pub fn insert_relationship(db: &Db, row: &RelationshipRow) -> rusqlite::Result<bool> {
    Ok(db.execute(
        INSERT_REL,
        params![
            row.source_chunk_id,
            row.target_chunk_id,
            row.rel_type,
            row.confidence,
            row.confidence_score,
            row.confidence_reasoning,
        ],
    )? > 0)
}

/// Rebuild the project's whole `relationships` table from persisted chunks + raw edges.
///
/// The graph is derived state: resolving one edge needs the *target* file's chunks, which a
/// per-file update cannot see. Recomputing every edge is what makes an incremental re-index of
/// a callee keep its callers' edges (audit F-01). Resolution is pure SQL over state already in
/// the database, so no ast-grep subprocess is involved and it stays cheap enough to run on
/// every index.
pub fn rebuild_relationships(db: &Db, project_id: &str) -> rusqlite::Result<i64> {
    let mut files: Vec<String> = {
        let mut stmt = db.prepare(
            "SELECT DISTINCT file_path FROM raw_edges WHERE project_id = ?1 ORDER BY file_path",
        )?;
        let rows = stmt.query_map(params![project_id], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    files.sort();
    files.dedup();

    db.execute(
        "DELETE FROM relationships WHERE source_chunk_id IN
           (SELECT chunk_id FROM chunks WHERE project_id = ?1)",
        params![project_id],
    )?;

    let mut count = 0i64;
    for file_path in &files {
        let mut chunk_by_symbol: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = db.prepare(
                "SELECT symbol_name, chunk_id FROM chunks
                  WHERE project_id = ?1 AND file_path = ?2 AND symbol_name IS NOT NULL
                  ORDER BY start_line, chunk_id",
            )?;
            let rows = stmt.query_map(params![project_id, file_path], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (symbol, chunk_id) = row?;
                chunk_by_symbol.insert(symbol, chunk_id);
            }
        }

        let mut edges: Vec<Edge> = Vec::new();
        {
            let mut stmt = db.prepare(
                "SELECT source_symbol, raw_target, rel_type, start_line FROM raw_edges
                  WHERE project_id = ?1 AND file_path = ?2
                  ORDER BY start_line, raw_target, rel_type",
            )?;
            let rows = stmt.query_map(params![project_id, file_path], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            for row in rows {
                let (source_symbol, raw_target, rel_type, start_line) = row?;
                edges.push(Edge {
                    rel_type,
                    source_symbol: if source_symbol.is_empty() {
                        None
                    } else {
                        Some(source_symbol)
                    },
                    raw_target,
                    start_line,
                });
            }
        }

        let rows =
            relationship_rows_for_symbol_map(db, project_id, file_path, &chunk_by_symbol, &edges)?;
        for row in rows {
            if insert_relationship(db, &row)? {
                count += 1;
            }
        }
    }
    Ok(count)
}
