//! Relationships + containment join.
//! Spec §5.6 assigns `applyBudget` to struct/context (Job D), not this module.

use crate::chunker::{bare_name, CallForm, Chunk, Edge};
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
    /// The line inside the source chunk that names the callee: the one line to read in order to
    /// check this edge without re-reading the function. `None` only for rows written before schema
    /// v4, which a re-index replaces.
    pub call_site_line: Option<i64>,
    /// Which call shape attached this edge. A `receiver` row is a name match the gate proved unique
    /// project-wide; it is not a type-checked call edge, and the form is how a reader learns that.
    pub call_form: CallForm,
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

/// The bare name behind an *internal* Rust path call, if this is one.
///
/// Root cause of the `extracted_but_unresolved` rows a reviewer reproduced: `resolve_targets` matches
/// `symbol_name` exactly, while `crate::def::my_func()` is stored as a raw target with its
/// qualification and as a chunk named just `my_func`. The exact match therefore finds nothing and the
/// edge is silently dropped -- a Rust call written the way Rust modules are normally called was
/// invisible to the graph.
///
/// Only the three prefixes that *prove* the target is inside the project are rescued. A std or
/// dependency call (`Vec::new`, `formatter.formatToParts`) must stay unresolved and visible: stripping
/// those to `new` would invent an edge to whatever happens to share the name, and would also make the
/// `unresolved` rows that currently disclose the hole disappear.
pub fn internal_rust_path_target(symbol: &str) -> Option<&str> {
    let rest = symbol
        .strip_prefix("crate::")
        .or_else(|| symbol.strip_prefix("self::"))
        .or_else(|| symbol.strip_prefix("super::"))
        .or_else(|| symbol.strip_prefix("::"))?;
    let bare = rest.rsplit("::").next().unwrap_or("");
    (!bare.is_empty() && !bare.contains('(') && !bare.contains('<')).then_some(bare)
}

fn chunks_named(db: &Db, project_id: &str, name: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = db.prepare(
        "SELECT chunk_id, file_path FROM chunks
          WHERE project_id = ?1 AND symbol_name = ?2 ORDER BY chunk_id",
    )?;
    let rows = stmt
        .query_map(params![project_id, name], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Project-wide lookup from a call's *method* name to every chunk that declares a symbol ending in
/// that name: `add` -> the chunks for `add`, `Tally::add`, `Store::add`.
///
/// Built once per project (or once per call, for the single-file helpers) by scanning `chunks` in
/// `chunk_id` order, so the candidate set -- and therefore the gate's verdict -- is reproducible.
/// A `LIKE '%:' || name` query per edge would have cost a full scan of `chunks` per call site, which
/// on the measured venue is five thousand scans over the same table.
/// One entry per candidate: the chunk to attach, and the symbol name it was found under (the owner
/// half of the binding rules needs the name, not just the id).
pub type ReceiverCandidate = (String, String);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReceiverIndex {
    by_name: HashMap<String, Vec<ReceiverCandidate>>,
}

impl ReceiverIndex {
    pub fn build(db: &Db, project_id: &str) -> rusqlite::Result<Self> {
        let mut stmt = db.prepare(
            "SELECT symbol_name, chunk_id FROM chunks
              WHERE project_id = ?1 AND symbol_name IS NOT NULL ORDER BY chunk_id",
        )?;
        let rows = stmt.query_map(params![project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut by_name: HashMap<String, Vec<ReceiverCandidate>> = HashMap::new();
        for row in rows {
            let (symbol, chunk_id) = row?;
            by_name
                .entry(bare_name(&symbol).to_string())
                .or_default()
                .push((chunk_id, symbol));
        }
        Ok(Self { by_name })
    }

    /// Chunks declaring a symbol whose last segment is `name`, in `chunk_id` order.
    pub fn candidates(&self, name: &str) -> &[ReceiverCandidate] {
        self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// `Tally::add` -> `Tally`; `add` -> None. Rust chunks for `impl` and trait methods carry their type
/// as an owner (`chunker::compose_symbol_name`), which is what makes this question answerable at all.
pub fn symbol_owner(symbol: &str) -> Option<&str> {
    symbol.rsplit_once("::").map(|(owner, _)| owner)
}

/// Letters and digits only, lowercased: `CallForm` and `call_form` are the same name when one is a
/// type and the other is its variable, which is the entire Rust convention for receivers.
fn norm_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Can `x.method()` be a call to this symbol?
///
/// Two structural facts, then one heuristic, in that order:
///
/// 1. `x.m()` binds to a **method**. A project-wide free function named `m` is not a candidate no
///    matter how unique its name is, so a candidate with no owner is refused. (`status.code()`
///    attaching to a test helper called `code` was the single most common false edge in the first
///    measurement of this gate.)
/// 2. `self.m()` inside `Owner::f` binds to `Owner::m`, subject to `Deref`, which name resolution
///    cannot see. This is the one case with a real scoping rule behind it, so it is checked against
///    the *enclosing symbol* rather than against a name guess.
/// 3. Otherwise the receiver's last segment has to look like the owner's name: equal (so `t`/`T`
///    works), or one a prefix or suffix of the other with at least three characters of overlap
///    (`e.call_form.insertion_rank` -> `CallForm`, `index.candidates` -> `ReceiverIndex`,
///    `err.to_json` -> `CortError`).
///
/// Rule 3 is a heuristic and only ever *refuses*: when it is wrong the cost is a missing edge, which
/// the coverage screen reports as a gap, instead of a phantom caller an agent cannot argue away.
/// Measured on this repo, 4,833 receiver call sites produced 4,833 raw edges; uniqueness alone
/// attached **25 of them, 13 true and 12 false** (48% precision -- `e.kind()` onto a test fixture's
/// `FailFs::kind`, `status.code()` onto a helper called `code`, `.chain()` onto a function called
/// `chain`). These rules attach **9, all 9 true**, refusing four that were real (`b.problem()` onto
/// `BatchRead::problem` x3, `err.to_json()` onto `CortError::to_json`) because a one-letter variable
/// carries no trace of its type. `--depth 3` on this repo therefore got +9 edges and +0 phantoms.
pub fn receiver_binds(head: &str, enclosing: Option<&str>, candidate: &str) -> bool {
    let Some(dot) = head.rfind(['.', ':']) else {
        return false; // no receiver at all: not a receiver-shaped call
    };
    let receiver = bare_name(&head[..dot]);
    let Some(owner) = symbol_owner(candidate) else {
        return false; // rule 1
    };
    if receiver.eq_ignore_ascii_case("self") || receiver == "Self" {
        return symbol_owner(enclosing.unwrap_or(""))
            .is_some_and(|en| norm_name(en) == norm_name(owner));
    }
    let (recv, own) = (norm_name(receiver), norm_name(owner));
    if recv.is_empty() || own.is_empty() {
        return false;
    }
    if recv == own {
        return true;
    }
    recv.len() >= 3
        && own.len() >= 3
        && (recv.starts_with(&own)
            || own.starts_with(&recv)
            || recv.ends_with(&own)
            || own.ends_with(&recv))
}

/// Resolve one edge to the chunks it names, applying the policy that belongs to its form.
///
/// The receiver gate is the reason the form is carried at all. A measured 5,522 receiver call sites
/// in this repo: 96.5% name a symbol the project never declares (std, dependency, iterator
/// adapters), and of the residue that is unique by name, more than a third were still wrong because
/// the *receiver* belonged to some other type. So a receiver edge needs the unique name *and*
/// [`receiver_binds`]. Where either refuses, nothing is attached and nothing is hidden -- coverage's
/// `extracted_but_unresolved` layer reads `raw_edges` and reports the site as a gap.
///
/// The gate is deliberately *not* applied to `bare` or `scoped`: a multi-candidate bare call has
/// always attached as `AMBIGUOUS`, and recorded eval labels (cct's `getCurrentTimeET`, seeds=2)
/// depend on that recall. Turning it off for those forms would trade a measured behaviour for an
/// unmeasured improvement.
pub fn resolve_edge_targets(
    db: &Db,
    project_id: &str,
    file_path: &str,
    import_map: &HashMap<String, String>,
    edge: &Edge,
    index: &ReceiverIndex,
) -> rusqlite::Result<Vec<String>> {
    if edge.call_form == CallForm::Receiver {
        let candidates = index.candidates(bare_name(&edge.raw_target));
        if candidates.len() != 1 {
            return Ok(Vec::new());
        }
        let (chunk_id, symbol) = &candidates[0];
        return Ok(
            if receiver_binds(&edge.raw_target, edge.source_symbol.as_deref(), symbol) {
                vec![chunk_id.clone()]
            } else {
                Vec::new()
            },
        );
    }
    resolve_targets(db, project_id, file_path, import_map, &edge.raw_target)
}

pub fn resolve_targets(
    db: &Db,
    project_id: &str,
    file_path: &str,
    import_map: &HashMap<String, String>,
    symbol: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut all = chunks_named(db, project_id, symbol)?;
    if all.is_empty() {
        if let Some(bare) = internal_rust_path_target(symbol) {
            all = chunks_named(db, project_id, bare)?;
        }
    }
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
    let index = ReceiverIndex::build(db, project_id)?;
    relationship_rows_for_symbol_map_with_index(
        db,
        project_id,
        file_path,
        chunk_by_symbol,
        edges,
        &index,
    )
}

/// As [`relationship_rows_for_symbol_map`], with a receiver index the caller already built --
/// `rebuild_relationships` walks every file in the project and would otherwise rebuild the same
/// lookup once per file.
pub fn relationship_rows_for_symbol_map_with_index(
    db: &Db,
    project_id: &str,
    file_path: &str,
    chunk_by_symbol: &HashMap<String, String>,
    edges: &[Edge],
    index: &ReceiverIndex,
) -> rusqlite::Result<Vec<RelationshipRow>> {
    let import_map = build_import_map(edges);
    let mut rows = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // Walk the file in source order, not in subprocess order: `relationships` is keyed by
    // (source, target, rel_type), so the first edge to arrive is the one whose line becomes the
    // reported call site. Source order makes that the *earliest* call site instead of an accident.
    let mut ordered: Vec<&Edge> = edges.iter().collect();
    ordered.sort_by_key(|e| {
        (
            e.start_line,
            e.raw_target.clone(),
            e.rel_type.clone(),
            e.call_form.insertion_rank(),
        )
    });
    for e in ordered {
        let Some(source_symbol) = &e.source_symbol else {
            continue;
        };
        let Some(source_chunk_id) = chunk_by_symbol.get(source_symbol) else {
            continue;
        };
        let targets: Vec<String> =
            resolve_edge_targets(db, project_id, file_path, &import_map, e, index)?
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
        let reasoning = match (n, e.call_form) {
            // The receiver gate only lets a unique name through, but "unique name" is not "same
            // type": say which of the two was checked.
            (1, CallForm::Receiver) => format!(
                "resolved receiver: {} (unique method name; owner inferred from the receiver, not type-checked)",
                e.raw_target
            ),
            (1, _) => format!("resolved: {}", e.raw_target),
            _ => format!("ambiguous: {} ({n} candidates)", e.raw_target),
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
                call_site_line: Some(e.start_line),
                call_form: e.call_form,
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
  (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning,
   call_site_line, call_form)
  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
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
            row.call_site_line,
            row.call_form.as_str(),
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

    // The receiver gate asks "how many symbols in this project answer to that name", which is a
    // project-wide question -- so it is answered once, here, rather than per file.
    let index = ReceiverIndex::build(db, project_id)?;

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
                "SELECT source_symbol, raw_target, rel_type, start_line, call_form FROM raw_edges
                  WHERE project_id = ?1 AND file_path = ?2
                  ORDER BY start_line, raw_target, rel_type",
            )?;
            let rows = stmt.query_map(params![project_id, file_path], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?;
            for row in rows {
                let (source_symbol, raw_target, rel_type, start_line, call_form) = row?;
                // A form this build does not know is a row from a newer extractor. Drop it rather
                // than resolve it under `bare`, which is the looser policy: a dropped edge keeps
                // showing up in the coverage screen, a mislabelled one stops showing up anywhere.
                let Some(call_form) = CallForm::parse(&call_form) else {
                    continue;
                };
                edges.push(Edge {
                    rel_type,
                    call_form,
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

        let rows = relationship_rows_for_symbol_map_with_index(
            db,
            project_id,
            file_path,
            &chunk_by_symbol,
            &edges,
            &index,
        )?;
        for row in rows {
            if insert_relationship(db, &row)? {
                count += 1;
            }
        }
    }
    Ok(count)
}
