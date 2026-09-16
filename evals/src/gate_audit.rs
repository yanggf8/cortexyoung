//! The receiver gate's refusal census: how many receiver calls the index holds, how many the
//! gate attached, and — for every refusal — which of the gate's own refusal reasons produced
//! it. This is the measurement that decides whether type-directed dispatch (a receiver whose
//! static type is a trait) is ever worth proposing as an edge source
//! (`docs/2026-09-16-codegraph-lessons-plan.md`, item B): the refusals are where recall leaks,
//! and until now nobody has cut them by cause.
//!
//! Two rules shape the module. The *decision* is never copied: candidate cardinality comes from
//! `cort::graph::ReceiverIndex::candidates`, ownership from `symbol_owner`, binding from
//! `receiver_binds` — the same functions `graph::resolve_edge_targets` applies — so a census row
//! and an `impact` answer cannot disagree about the same edge, which is the only property that
//! makes the census describe the product rather than a copy of it. And the module is a sibling
//! of `recall.rs`, not an extension of it: `recall.rs`'s charter is the source-only
//! counterfactual that never links `cort`, while this module is index-side by construction (its
//! population is `raw_edges`, which only exists in the database).

use cort::chunker::bare_name;
use cort::db::{self, Db};
use cort::graph::{receiver_binds, receiver_shape, symbol_owner, ReceiverIndex};
use rusqlite::params;
use serde_json::{json, Value};
use std::path::Path;

/// What the gate's own primitives conclude about one receiver edge. The variants are the gate's
/// ordered refusal reasons — zero candidates, many candidates, one ownerless candidate, one
/// candidate refused at the binding step — in the order `resolve_edge_targets` and
/// `receiver_binds` encounter them.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Attached,
    ZeroCandidates,
    MultipleCandidates,
    OneOwnerless,
    BindingRefused { no_receiver_shape: bool },
}

/// The gate's own classification of one receiver target, for whatever row carries it. Reason
/// attribution follows the gate's own order — `receiver_binds` is called first, exactly as
/// production calls it, and only its refusal is attributed: shape (via the gate's shared
/// [`receiver_shape`] primitive), then ownership, then the name match. Nothing here re-derives
/// a predicate the gate owns.
pub fn classify(index: &ReceiverIndex, target: &str, enclosing: Option<&str>) -> Verdict {
    let candidates = index.candidates(bare_name(target));
    match candidates.len() {
        0 => Verdict::ZeroCandidates,
        n if n > 1 => Verdict::MultipleCandidates,
        _ => {
            let (_, symbol) = &candidates[0];
            if receiver_binds(target, enclosing, symbol) {
                return Verdict::Attached;
            }
            if receiver_shape(target).is_none() {
                // Refused on shape before any owner question exists — which is also why an
                // ownerless candidate can land here, and why the class is named for the refusal
                // step, not for the candidate's ownership.
                return Verdict::BindingRefused {
                    no_receiver_shape: true,
                };
            }
            if symbol_owner(symbol).is_none() {
                return Verdict::OneOwnerless;
            }
            Verdict::BindingRefused {
                no_receiver_shape: false,
            }
        }
    }
}

/// One raw edge as the census reads it. `file_path` and `start_line` come from `raw_edges`
/// itself, so every example is checkable by hand without re-reading a single source file.
#[derive(Debug, Clone)]
pub struct EdgeRow {
    pub file_path: String,
    pub source_symbol: String,
    pub raw_target: String,
    pub start_line: i64,
}

/// One refused call, carried with enough provenance to be read: the instruction for class 4 is
/// *read the rows*, and a row without `file:line` cannot be read.
#[derive(Debug, Clone)]
pub struct RefusalExample {
    pub file: String,
    pub line: i64,
    pub target: String,
}

/// The census over one index's receiver edges. The classes are the gate's refusal reasons, so
/// they are disjoint by construction and sum to `refused`; `population` is the row count handed
/// in, and `attached + refused == population` is therefore a claim about coverage of the input,
/// not a restatement of the addition that produced it.
#[derive(Debug, Default)]
pub struct Census {
    pub population: usize,
    pub attached: usize,
    pub zero_candidates: usize,
    pub multiple_candidates: usize,
    pub one_ownerless: usize,
    /// One candidate, refused at the binding step — for a missing receiver shape or for a name
    /// that does not bind. `no_receiver_shape` counts the former beside the class, never merged
    /// into it: such a row is not a type-directed-dispatch candidate, and silently counting it
    /// would inflate the one number this module exists to read.
    pub binding_refused: usize,
    pub binding_refused_no_shape: usize,
    pub zero_examples: Vec<RefusalExample>,
    pub multiple_examples: Vec<RefusalExample>,
    pub ownerless_examples: Vec<RefusalExample>,
    pub binding_refused_examples: Vec<RefusalExample>,
}

fn push_example(out: &mut Vec<RefusalExample>, edge: &EdgeRow, cap: usize) {
    if out.len() < cap {
        out.push(RefusalExample {
            file: edge.file_path.clone(),
            line: edge.start_line,
            target: edge.raw_target.clone(),
        });
    }
}

/// Fold raw receiver edges through the gate's own primitives. Examples are capped per class so
/// a venue with ten thousand refusals still prints a report a person can hold.
pub fn census(index: &ReceiverIndex, edges: &[EdgeRow], examples_per_class: usize) -> Census {
    let mut c = Census::default();
    for edge in edges {
        match classify(index, &edge.raw_target, Some(&edge.source_symbol)) {
            Verdict::Attached => c.attached += 1,
            Verdict::ZeroCandidates => {
                c.zero_candidates += 1;
                push_example(&mut c.zero_examples, edge, examples_per_class);
            }
            Verdict::MultipleCandidates => {
                c.multiple_candidates += 1;
                push_example(&mut c.multiple_examples, edge, examples_per_class);
            }
            Verdict::OneOwnerless => {
                c.one_ownerless += 1;
                push_example(&mut c.ownerless_examples, edge, examples_per_class);
            }
            Verdict::BindingRefused { no_receiver_shape } => {
                c.binding_refused += 1;
                if no_receiver_shape {
                    c.binding_refused_no_shape += 1;
                }
                push_example(&mut c.binding_refused_examples, edge, examples_per_class);
            }
        }
    }
    // Set from the input, not from the buckets: the tests then assert bucket-sum == population
    // as a coverage claim, and a bucket that silently dropped rows would fail it.
    c.population = edges.len();
    c
}

fn examples_json(examples: &[RefusalExample]) -> Value {
    Value::Array(
        examples
            .iter()
            .map(|e| json!({ "file": e.file, "line": e.line, "target": e.target }))
            .collect(),
    )
}

/// Assemble the report from an open index. `venue_head` arrives already resolved (or
/// `"no-git"`), because the file-locating half of [`report`] is the only part that touches the
/// world outside the database and tests feed this half directly.
pub fn report_from(
    db: &Db,
    project_id: &str,
    venue: &str,
    venue_head: &str,
    examples_per_class: usize,
) -> Result<Value, String> {
    let index_head = db::indexed_head(db, project_id)
        .map_err(|e| format!("reading the index head: {e}"))?
        .unwrap_or_else(|| "no-git".to_string());
    let index = ReceiverIndex::build(db, project_id)
        .map_err(|e| format!("building the receiver index: {e}"))?;
    let mut stmt = db
        .prepare(
            "SELECT file_path, source_symbol, raw_target, start_line FROM raw_edges
             WHERE project_id = ?1 AND rel_type = 'calls' AND call_form = 'receiver'
             ORDER BY file_path, start_line",
        )
        .map_err(|e| format!("reading raw_edges: {e}"))?;
    let mut rows = stmt
        .query(params![project_id])
        .map_err(|e| format!("reading raw_edges: {e}"))?;
    let mut edges: Vec<EdgeRow> = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("reading raw_edges: {e}"))? {
        edges.push(EdgeRow {
            file_path: row.get(0).map_err(|e| format!("reading raw_edges: {e}"))?,
            source_symbol: row.get(1).map_err(|e| format!("reading raw_edges: {e}"))?,
            raw_target: row.get(2).map_err(|e| format!("reading raw_edges: {e}"))?,
            start_line: row.get(3).map_err(|e| format!("reading raw_edges: {e}"))?,
        });
    }
    let c = census(&index, &edges, examples_per_class);
    Ok(json!({
        "venue": venue,
        "venue_head": venue_head,
        "index_head": index_head,
        // The index stores the full 40-char head while `git rev-parse --short` yields the short
        // form, so equality is a prefix match; the first end-to-end run caught the string
        // comparison reporting a fresh index as disagreed.
        "heads_agree": venue_head != "no-git" && index_head.starts_with(venue_head),
        "method": "gate-audit-v1 (index-side census of raw_edges receiver calls, classified by cort::graph's own gate primitives)",
        // Said in the report because the number cannot say it alone: zero_candidates is the
        // std/dependency frontier by construction, and a reader who takes it for a recall leak
        // has been misled by the shape of the JSON rather than by a number.
        "reading": "zero_candidates counts calls whose method name the project never declares — std, dependencies, iterator adapters; it is the static-analysis frontier, not a recall leak. binding_refused holds the type-directed-dispatch candidates; read its examples.",
        "population": c.population,
        "attached": c.attached,
        "refused": c.zero_candidates + c.multiple_candidates + c.one_ownerless + c.binding_refused,
        "refused_classes": {
            "zero_candidates": {
                "count": c.zero_candidates, "examples": examples_json(&c.zero_examples),
            },
            "multiple_candidates": {
                "count": c.multiple_candidates, "examples": examples_json(&c.multiple_examples),
            },
            "one_ownerless": {
                "count": c.one_ownerless, "examples": examples_json(&c.ownerless_examples),
            },
            "binding_refused": {
                "count": c.binding_refused,
                "no_receiver_shape": c.binding_refused_no_shape,
                "examples": examples_json(&c.binding_refused_examples),
            },
        },
    }))
}

/// `main.rs`'s `venue_head` refuses a venue git cannot answer, which is right where the head
/// *labels* rows that must not be ambiguous; a census wants to describe what it read either way,
/// so this tolerant spelling names the missing git instead of failing the audit.
fn head_or_no_git(repo: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "no-git".to_string())
}

/// Locate the venue's index and run the census over it. The database is opened
/// `SQLITE_OPEN_READ_ONLY`: `db::open_db` creates directories, selects WAL and rewrites
/// permissions, and an audit that claims to never write must not lean on a caller's existence
/// check to make that true. The canonical path is used only to locate the project; the venue is
/// reported under the spelling the caller supplied, so a committed artifact can stay
/// repo-relative.
pub fn report(venue: &str, examples_per_class: usize) -> Result<Value, String> {
    let canonical = Path::new(venue)
        .canonicalize()
        .map_err(|e| format!("{venue}: {e}"))?;
    let root = canonical
        .to_str()
        .ok_or_else(|| format!("{venue}: path is not valid UTF-8"))?;
    let project_id = db::project_id_for(root);
    let db_path = db::db_path_for(root);
    if !db_path.exists() {
        return Err(format!(
            "--venue {venue}: no index for this directory\n  gate-audit reads the index and never builds one; run `cort index` first"
        ));
    }
    let db =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("{}: {e}", db_path.display()))?;
    db.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|e| format!("{}: {e}", db_path.display()))?;
    let present: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("{}: {e}", db_path.display()))?;
    if present == 0 {
        return Err(format!(
            "--venue {venue}: no indexed project row in {}",
            db_path.display()
        ));
    }
    report_from(
        &db,
        &project_id,
        venue,
        &head_or_no_git(&canonical),
        examples_per_class,
    )
}
