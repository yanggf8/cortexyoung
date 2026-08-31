//! C1-20..C1-29 — the Rust port kept the case ids (audit F-12).
//!
//! JS graph tests go through `fullIndex` (Job C2). C1 seeds via `extract_file` +
//! `relationship_rows_for_file` so we do not call the C2 stub.

use cort::ast_grep::resolve_ast_grep_bin;
use cort::chunker::{extract_file, CallForm, Chunk, Edge, ExtractFileArgs, ExtractResult};
use cort::db::{ensure_schema, open_db, project_id_for, Db};
use cort::graph::{
    build_import_map, get_neighbors, get_transitive_dependents, receiver_binds,
    relationship_rows_for_file, relationship_rows_for_symbol_map, resolve_targets,
    unresolved_inline, ReceiverIndex, CONFIDENCE_SCORE,
};
use rusqlite::params;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SAMPLE: &[(&str, &str)] = &[
    (
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    ),
    (
        "src/alpha.ts",
        concat!(
            "import { helper } from './helper';\n",
            "export function alpha(a: number) { return helper(a) + 1; }\n",
            "export class Beta {\n",
            "  go() { return alpha(2); }\n",
            "}\n",
        ),
    ),
    (
        "node_modules/pkg/index.ts",
        "export function shouldBeIgnored() {}\n",
    ),
    ("README.md", "# not a source file\n"),
];

fn is_indexable(rel: &str) -> bool {
    let ignore = [
        "node_modules",
        "dist",
        "build",
        ".git",
        "__pycache__",
        ".venv",
        "venv",
        "target",
        "coverage",
        ".next",
        ".cache",
    ];
    if rel.split('/').any(|p| ignore.contains(&p)) {
        return false;
    }
    matches!(
        Path::new(rel).extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "rs")
    )
}

struct Indexed {
    _dir: tempfile::TempDir,
    db: Db,
    project_id: String,
    relationship_count: usize,
}

fn write_project(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    for (rel, body) in files {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, body).unwrap();
    }
    (dir, root)
}

fn insert_chunk(db: &Db, c: &Chunk) {
    db.execute(
        "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
            start_line, end_line, content, content_hash, language, chunk_source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            c.chunk_id,
            c.project_id,
            c.file_path,
            c.symbol_name,
            c.chunk_type,
            c.start_line,
            c.end_line,
            c.content,
            c.content_hash,
            c.language,
            c.chunk_source,
        ],
    )
    .unwrap();
}

fn index_files(files: &[(&str, &str)]) -> Indexed {
    let (dir, root) = write_project(files);
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let root_s = root.to_str().unwrap();
    let project_id = project_id_for(root_s);
    db.execute(
        "INSERT INTO projects (project_id, name, path, extractor_version)
         VALUES (?1, 't', ?2, 'v')",
        params![project_id, root_s],
    )
    .unwrap();

    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    let mut extracted: Vec<(String, ExtractResult)> = Vec::new();
    for (rel, body) in files {
        if !is_indexable(rel) {
            continue;
        }
        let abs = root.join(rel);
        let abs_s = abs.to_str().unwrap().to_string();
        let result = extract_file(ExtractFileArgs {
            bin: &bin,
            project_id: &project_id,
            file_path: rel,
            abs_path: &abs_s,
            source: body,
            timeout_ms: None,
        })
        .unwrap();
        extracted.push((rel.to_string(), result));
    }
    for (_, result) in &extracted {
        for c in &result.chunks {
            insert_chunk(&db, c);
        }
    }
    let mut relationship_count = 0usize;
    for (rel, result) in &extracted {
        let rows = relationship_rows_for_file(&db, &project_id, rel, &result.chunks, &result.edges)
            .unwrap();
        for r in rows {
            db.execute(
                "INSERT INTO relationships
                    (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score,
                     confidence_reasoning, call_site_line, call_form)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(source_chunk_id, target_chunk_id, rel_type) DO NOTHING",
                params![
                    r.source_chunk_id,
                    r.target_chunk_id,
                    r.rel_type,
                    r.confidence,
                    r.confidence_score,
                    r.confidence_reasoning,
                    r.call_site_line,
                    r.call_form.as_str(),
                ],
            )
            .unwrap();
            relationship_count += 1;
        }
    }
    Indexed {
        _dir: dir,
        db,
        project_id,
        relationship_count,
    }
}

fn indexed(files: &[(&str, &str)]) -> Indexed {
    index_files(files)
}

/// C1-20
#[test]
fn confidence_constants_match_the_spec_exactly() {
    assert_eq!(CONFIDENCE_SCORE.extracted, 1.0);
    assert_eq!(CONFIDENCE_SCORE.inferred, 0.7);
    assert_eq!(CONFIDENCE_SCORE.ambiguous, 0.5);
}

/// C1-21
#[test]
fn a_single_hit_call_resolves_to_one_inferred_row() {
    let ix = indexed(SAMPLE);
    assert!(ix.relationship_count > 0);
    let row = ix
        .db
        .query_row(
            "SELECT r.rel_type, r.confidence, r.confidence_score FROM relationships r
             JOIN chunks s ON s.chunk_id = r.source_chunk_id
             JOIN chunks t ON t.chunk_id = r.target_chunk_id
             WHERE s.project_id = ?1 AND s.symbol_name = 'alpha' AND t.symbol_name = 'helper'",
            params![ix.project_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            },
        )
        .expect("alpha -> helper must be a stored relationship");
    assert_eq!(row.0, "calls");
    assert_eq!(row.1, "INFERRED");
    assert_eq!(row.2, 0.7);
}

/// C1-22
#[test]
fn an_ambiguous_call_writes_one_row_per_target_with_score_0_5_over_n() {
    let ix = indexed(&[
        ("src/a.ts", "export function dup() { return 1; }\n"),
        ("src/b.ts", "export function dup() { return 2; }\n"),
        ("src/c.ts", "export function caller() { return dup(); }\n"),
    ]);
    let mut stmt = ix
        .db
        .prepare(
            "SELECT r.confidence, r.confidence_score FROM relationships r
             JOIN chunks s ON s.chunk_id = r.source_chunk_id
             WHERE s.project_id = ?1 AND s.symbol_name = 'caller'",
        )
        .unwrap();
    let rows: Vec<(String, f64)> = stmt
        .query_map(params![ix.project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|(c, _)| c == "AMBIGUOUS"));
    assert!(
        rows.iter().all(|(_, s)| (s - 0.25).abs() < 1e-9),
        "expected 0.5 * 1/2"
    );
}

/// C1-23
#[test]
fn a_call_with_no_resolvable_target_writes_no_row_at_all() {
    let ix = indexed(&[(
        "src/only.ts",
        "export function solo() { return externalThing(1); }\n",
    )]);
    let n: i64 = ix
        .db
        .query_row(
            "SELECT COUNT(*) FROM relationships r
             JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?1",
            params![ix.project_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "zero-target edges must never be persisted");
    let tables: i64 = ix
        .db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'unresolved_refs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

/// C1-24
#[test]
fn unresolved_inline_is_the_on_the_fly_shape_and_carries_no_chunk_id() {
    let u = unresolved_inline("externalThing");
    assert_eq!(u.confidence, "AMBIGUOUS");
    assert_eq!(u.confidence_score, 0.5);
    assert_eq!(u.confidence_reasoning, "unresolved: externalThing");
    // no target_chunk_id / chunk_id fields on the type
}

/// C1-25
#[test]
fn a_symbol_never_calls_itself() {
    let ix = indexed(&[(
        "src/rec.ts",
        "export function loop(n: number) { return n > 0 ? loop(n - 1) : 0; }\n",
    )]);
    let mut stmt = ix
        .db
        .prepare(
            "SELECT r.source_chunk_id, r.target_chunk_id FROM relationships r
             JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?1",
        )
        .unwrap();
    let self_edges: usize = stmt
        .query_map(params![ix.project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .filter(|(s, t)| s == t)
        .count();
    assert_eq!(self_edges, 0);
}

/// C1-26
#[test]
fn get_neighbors_returns_depth_1_edges_in_both_directions_capped() {
    let ix = indexed(SAMPLE);
    let helper: String = ix
        .db
        .query_row(
            "SELECT chunk_id FROM chunks WHERE project_id = ?1 AND symbol_name = 'helper'",
            params![ix.project_id],
            |r| r.get(0),
        )
        .unwrap();
    let n = get_neighbors(&ix.db, &helper, 3).unwrap();
    assert!(!n.is_empty());
    assert!(n.len() <= 3);
    assert!(n
        .iter()
        .any(|x| x.symbol_name.as_deref() == Some("alpha") && x.direction == "incoming"));
}

/// C1-27
#[test]
fn get_transitive_dependents_walks_the_reverse_edge_up_to_depth() {
    let ix = indexed(SAMPLE);
    let helper: String = ix
        .db
        .query_row(
            "SELECT chunk_id FROM chunks WHERE project_id = ?1 AND symbol_name = 'helper'",
            params![ix.project_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut deps: Vec<String> = get_transitive_dependents(&ix.db, &helper, 3)
        .unwrap()
        .into_iter()
        .map(|d| d.symbol_name.unwrap_or_default())
        .collect();
    deps.sort();
    assert_eq!(
        deps,
        ["alpha", "go"],
        "go -> alpha -> helper is a 2-hop reverse chain"
    );
}

/// C1-28
#[test]
fn build_import_map_keys_only_the_module_specifiers_of_import_edges() {
    let map = build_import_map(&[
        cort::chunker::Edge {
            rel_type: "imports".into(),
            call_form: CallForm::Bare,
            source_symbol: None,
            raw_target: "./helper".into(),
            start_line: 1,
        },
        cort::chunker::Edge {
            rel_type: "calls".into(),
            call_form: CallForm::Bare,
            source_symbol: Some("alpha".into()),
            raw_target: "helper".into(),
            start_line: 2,
        },
    ]);
    assert!(map.contains_key("./helper"));
    assert_eq!(map.len(), 1);
}

/// C1-29
#[test]
fn resolve_targets_prefers_files_reachable_through_the_import_map() {
    let ix = indexed(&[
        ("src/helper.ts", "export function dup() { return 1; }\n"),
        ("src/far.ts", "export function dup() { return 2; }\n"),
        (
            "src/alpha.ts",
            "import { dup } from './helper';\nexport function alpha() { return dup(); }\n",
        ),
    ]);
    let map: HashMap<String, String> = build_import_map(&[cort::chunker::Edge {
        rel_type: "imports".into(),
        call_form: CallForm::Bare,
        source_symbol: None,
        raw_target: "./helper".into(),
        start_line: 1,
    }]);
    let ids = resolve_targets(&ix.db, &ix.project_id, "src/alpha.ts", &map, "dup").unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids[0].contains("src/helper.ts"));
}

// ---------------------------------------------------------------------------
// schema v4: the receiver gate and the call-site line.
//
// The measurement that set the policy (docs/2026-08-31-coverage-external-review.md): of 5,522
// receiver call sites in this repo, 5,330 name a symbol the project never declares and 32 name more
// than one. Attaching every receiver call by name would have bought ~160 real edges and ~100 wrong
// ones; attaching them only when the name is unique buys the 160.
// ---------------------------------------------------------------------------

const RECEIVER_FIXTURE: &[(&str, &str)] = &[
    (
        "src/lib.rs",
        concat!(
            "pub struct T;\n",
            "impl T { pub fn take(&self) -> u32 { 1 } }\n",
        ),
    ),
    (
        "src/use.rs",
        concat!(
            "use crate::lib::T;\n",
            "fn one(t: &T) -> u32 { t.take() }\n",
            "fn two(t: &T) -> u32 { t.take() }\n",
        ),
    ),
];

/// A receiver edge as v4's pack really emits it: the call *head* (`t.take`), not the bare name, because
/// the receiver is the evidence the binding rules use.
fn receiver_edge(source: &str, head: &str, line: i64) -> Edge {
    Edge {
        rel_type: "calls".into(),
        call_form: CallForm::Receiver,
        source_symbol: Some(source.into()),
        raw_target: head.into(),
        start_line: line,
    }
}

fn rows_for(
    ix: &Indexed,
    file: &str,
    edges: &[Edge],
    symbols: &[(&str, &str)],
) -> Vec<cort::graph::RelationshipRow> {
    let map: HashMap<String, String> = symbols
        .iter()
        .map(|(name, id)| (name.to_string(), id.to_string()))
        .collect();
    relationship_rows_for_symbol_map(&ix.db, &ix.project_id, file, &map, edges).unwrap()
}

fn chunk_ids_named(ix: &Indexed, name: &str) -> Vec<(String, String)> {
    let mut stmt = ix
        .db
        .prepare(
            "SELECT chunk_id, symbol_name FROM chunks WHERE symbol_name = ?1 ORDER BY chunk_id",
        )
        .unwrap();
    stmt.query_map(params![name], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn a_receiver_call_attaches_when_its_method_name_belongs_to_exactly_one_symbol() {
    let ix = indexed(RECEIVER_FIXTURE);
    // Pin the precondition the gate is supposed to be reasoning about, so a change in how symbols
    // are named cannot quietly turn this test into "assert nothing happened".
    let take = chunk_ids_named(&ix, "T::take");
    assert_eq!(take.len(), 1, "{take:?}");
    let index = ReceiverIndex::build(&ix.db, &ix.project_id).unwrap();
    assert_eq!(
        index.candidates("take").len(),
        1,
        "`take` must be unique project-wide for this test to mean anything"
    );
    let rows = rows_for(
        &ix,
        "src/use.rs",
        &[receiver_edge("one", "t.take", 2)],
        &[("one", "p:src/use.rs:2")],
    );
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].target_chunk_id, take[0].0);
    assert_eq!(rows[0].confidence, "INFERRED");
    assert_eq!(rows[0].call_site_line, Some(2));
    assert_eq!(rows[0].call_form, CallForm::Receiver);
    assert!(
        rows[0].confidence_reasoning.contains("not type-checked"),
        "the reasoning has to say which check was made: {:?}",
        rows[0].confidence_reasoning
    );
}

#[test]
fn a_receiver_call_is_not_attached_when_two_symbols_answer_to_the_name() {
    let ix = indexed(&[
        (
            "src/lib.rs",
            concat!(
                "pub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
                "pub struct U;\nimpl U { pub fn take(&self) -> u32 { 2 } }\n",
            ),
        ),
        ("src/use.rs", "fn one(t: &T) -> u32 { t.take() }\n"),
    ]);
    let rows = rows_for(
        &ix,
        "src/use.rs",
        &[receiver_edge("one", "t.take", 1)],
        &[("one", "p:src/use.rs:1")],
    );
    assert!(
        rows.is_empty(),
        "two candidates named `take` is a guess, not an edge: {rows:?}"
    );
}

#[test]
fn a_receiver_call_into_std_attaches_nothing_and_is_invisible_to_the_gate() {
    let ix = indexed(&[(
        "src/lib.rs",
        "pub fn v() -> u32 { let mut s = String::new(); s.push('a'); s.len() }\n",
    )]);
    let rows = rows_for(
        &ix,
        "src/lib.rs",
        &[
            receiver_edge("v", "s.push", 1),
            receiver_edge("v", "s.len", 1),
        ],
        &[("v", "p:src/lib.rs:1")],
    );
    assert!(
        rows.is_empty(),
        "no project symbol named `push`/`len`: {rows:?}"
    );
}

#[test]
fn the_gate_does_not_touch_the_recall_bare_names_already_had() {
    // `getCurrentTimeET`-style recall: a bare call whose name has two candidates has always attached
    // as AMBIGUOUS, and the recorded eval labels depend on it. Tightening that would be a recall
    // regression dressed up as a precision fix, so it is pinned here rather than trusted.
    let ix = indexed(&[
        ("src/a.ts", "export function dup() { return 1; }\n"),
        ("src/b.ts", "export function dup() { return 2; }\n"),
        ("src/c.ts", "export function caller() { return dup(); }\n"),
    ]);
    let bare = Edge {
        rel_type: "calls".into(),
        call_form: CallForm::Bare,
        source_symbol: Some("caller".into()),
        raw_target: "dup".into(),
        start_line: 1,
    };
    let rows = rows_for(&ix, "src/c.ts", &[bare], &[("caller", "p:src/c.ts:1")]);
    assert_eq!(
        rows.len(),
        2,
        "both `dup` candidates, as before v4: {rows:?}"
    );
    assert!(rows.iter().all(|r| r.confidence == "AMBIGUOUS"));
    assert_eq!(rows[0].call_form, CallForm::Bare);
}

#[test]
fn receiver_candidates_are_counted_by_the_last_segment_of_a_qualified_name() {
    let ix = indexed(&[(
        "src/lib.rs",
        concat!(
            "pub struct A;\nimpl A { pub fn run(&self) -> u32 { 1 } }\n",
            "pub struct B;\nimpl B { pub fn run(&self) -> u32 { 2 } }\n",
            "pub fn other() -> u32 { 3 }\n",
        ),
    )]);
    let index = ReceiverIndex::build(&ix.db, &ix.project_id).unwrap();
    assert_eq!(index.candidates("run").len(), 2, "A::run and B::run");
    assert_eq!(
        index.candidates("other").len(),
        1,
        "a free function is indexed by its name..."
    );
    assert!(index.candidates("missing").is_empty());
    // ...but being indexed is not the same as being bindable: `x.other()` cannot call a free
    // function, so rule 1 refuses it even though it is the only `other` in the project.
    let rows = rows_for(
        &ix,
        "src/lib.rs",
        &[
            receiver_edge("other", "a.run", 5),
            receiver_edge("other", "s.other", 5),
        ],
        &[("other", "p:src/lib.rs:5")],
    );
    assert!(
        rows.is_empty(),
        "ambiguous `run` refused, ownerless `other` refused: {rows:?}"
    );
    // Uniqueness is checked first, so nothing about the receiver can rescue an ambiguous name.
    let also_refused = rows_for(
        &ix,
        "src/lib.rs",
        &[receiver_edge("other", "a.run", 5)],
        &[("other", "p:src/lib.rs:5")],
    );
    assert!(also_refused.is_empty(), "{also_refused:?}");
}

#[test]
fn a_relationship_keeps_the_earliest_call_site_when_one_function_calls_twice() {
    let ix = indexed(RECEIVER_FIXTURE);
    let take = chunk_ids_named(&ix, "T::take")[0].0.clone();
    let rows = rows_for(
        &ix,
        "src/use.rs",
        &[
            receiver_edge("one", "t.take", 4),
            receiver_edge("one", "t.take", 2),
        ],
        &[("one", "p:src/use.rs:2")],
    );
    assert_eq!(
        rows.len(),
        1,
        "`relationships` is keyed by (source, target, rel_type): {rows:?}"
    );
    assert_eq!(
        rows[0].call_site_line,
        Some(2),
        "the row has to point at the first call, not whichever edge came last"
    );
    assert_eq!(rows[0].target_chunk_id, take);
}

#[test]
fn edges_are_walked_in_source_order_so_the_reported_line_does_not_depend_on_subprocess_output() {
    let ix = indexed(RECEIVER_FIXTURE);
    let forward = rows_for(
        &ix,
        "src/use.rs",
        &[
            receiver_edge("one", "t.take", 2),
            receiver_edge("one", "t.take", 4),
        ],
        &[("one", "p:src/use.rs:2")],
    );
    let backward = rows_for(
        &ix,
        "src/use.rs",
        &[
            receiver_edge("one", "t.take", 4),
            receiver_edge("one", "t.take", 2),
        ],
        &[("one", "p:src/use.rs:2")],
    );
    assert_eq!(forward, backward, "same rows, any input order");
}

// ---------------------------------------------------------------------------
// `receiver_binds`: the three rules, each on the shape that motivated it.
// ---------------------------------------------------------------------------

#[test]
fn a_receiver_call_never_binds_to_an_ownerless_symbol() {
    // The measured false edge: `code: status.code().unwrap_or(0)` in a test helper, where the only
    // symbol named `code` in the project was a free function in another file. `x.m()` cannot bind to
    // a free function, so uniqueness alone was never enough.
    assert!(!receiver_binds("status.code", Some("run_cort"), "code"));
    assert!(!receiver_binds("items.chain", Some("main"), "chain"));
}

#[test]
fn self_calls_bind_to_the_enclosing_impl_and_to_nothing_else() {
    assert!(receiver_binds(
        "self.matches",
        Some("FailFs::metadata"),
        "FailFs::matches"
    ));
    assert!(
        !receiver_binds("self.matches", Some("FailFs::metadata"), "Other::matches"),
        "a different owner is a different type"
    );
    assert!(
        !receiver_binds("self.matches", Some("cause_of"), "FailFs::matches"),
        "`self` outside an impl proves nothing"
    );
    // `self.field.load()` is a call on the *field's* type, not on Self -- the receiver is no longer
    // `self`, so it goes through the name rules and `written` does not look like `BatchRead`.
    assert!(!receiver_binds(
        "self.written.load",
        Some("CountingFs::open_reads"),
        "BatchRead::load"
    ));
    // `e` is an io::Error, not a FailFs: the name rules refuse it even though the method name is
    // unique and the enclosing symbol happens to share it.
    assert!(!receiver_binds(
        "e.kind",
        Some("FailFs::kind"),
        "FailFs::kind"
    ));
}

#[test]
fn a_receiver_binds_when_its_name_is_the_owner_s_name_in_any_rust_shape() {
    for (head, enclosing, candidate) in [
        ("t.take", "one", "T::take"),        // equal, one letter
        ("tally.add", "scan", "Tally::add"), // variable named for its type
        (
            "e.call_form.insertion_rank",
            "x",
            "CallForm::insertion_rank",
        ), // field named for its type
        (
            "index.candidates",
            "resolve_edge_targets",
            "ReceiverIndex::candidates",
        ), // suffix
        ("db.query_row", "list_projects", "Db::query_row"),
    ] {
        assert!(
            receiver_binds(head, Some(enclosing), candidate),
            "{head} -> {candidate}"
        );
    }
}

#[test]
fn a_receiver_that_does_not_look_like_the_owner_is_refused_even_when_the_name_is_unique() {
    for (head, candidate) in [
        ("e.kind", "FailFs::kind"),
        ("before.matches", "FailFs::matches"),
        ("io.kind", "FailFs::kind"),
        ("self.error.kind", "FailFs::kind"),
        ("s.get", "Store::get"),
        // Lost when the binding rules went in, recorded here as the counterweight to the precision
        // claim: `b` really is a `BatchRead` and `err` really is a `CortError`, but neither variable
        // name carries any trace of the type. The alternative is to let `e.kind()` attach to whatever
        // in the project owns a unique `kind` -- and a phantom caller cannot be argued out of the way
        // by a reader who has already been told the enumeration is complete.
        ("b.problem", "BatchRead::problem"),
        ("err.to_json", "CortError::to_json"),
    ] {
        assert!(
            !receiver_binds(head, Some("some_function"), candidate),
            "{head} -> {candidate} is a std, unrelated, or lost call"
        );
    }
    assert!(
        !receiver_binds("take", Some("go"), "T::take"),
        "no receiver"
    );
    assert!(!receiver_binds("", None, "T::take"), "nothing to bind");
}

#[test]
fn module_segments_strip_src_and_mod_components() {
    let cases = [
        ("rust/src/graph.rs", vec!["rust", "graph"]),
        ("src/a/mod.rs", vec!["a"]),
        ("lib.rs", vec!["lib"]),
        ("main.rs", vec!["main"]),
        (
            "crates/x/src/deep/nested.rs",
            vec!["crates", "x", "deep", "nested"],
        ),
    ];
    for (path, expected) in cases {
        let want: Vec<String> = expected.into_iter().map(str::to_string).collect();
        assert_eq!(cort::graph::module_segments(path), want, "path {path}");
    }
}

#[test]
fn expand_use_path_fans_out_brace_groups_and_drops_crate() {
    let expand = cort::graph::expand_use_path;
    assert_eq!(
        expand("crate::graph::rebuild_relationships"),
        [vec!["graph", "rebuild_relationships"]]
    );
    assert_eq!(
        expand("crate::chunker::{Chunk, Edge}"),
        [vec!["chunker", "Chunk"], vec!["chunker", "Edge"]]
    );
    assert_eq!(
        expand("std::fmt::{self, Display}"),
        [vec!["std", "fmt"], vec!["std", "fmt", "Display"]]
    );
    assert_eq!(expand("super::inner::thing"), [vec!["inner", "thing"]]);
    // File-relative specifiers are JS import paths, not module paths.
    assert!(expand("./helper").is_empty());
}

#[test]
fn a_qualified_rust_call_resolves_through_the_module_path_suffix() {
    let ix = indexed(&[
        ("src/a.rs", "pub fn value(x: u8) -> u8 { x }\n"),
        ("src/b.rs", "pub fn value(x: u8) -> u8 { x + 1 }\n"),
        ("src/main.rs", "fn go() { crate::a::value(1); }\n"),
    ]);
    let map: HashMap<String, String> = HashMap::new();
    let ids = resolve_targets(
        &ix.db,
        &ix.project_id,
        "src/main.rs",
        &map,
        "crate::a::value",
    )
    .unwrap();
    assert_eq!(
        ids.len(),
        1,
        "qualified call must pick the matching module, got {ids:?}"
    );
    assert!(ids[0].contains("src/a.rs"));
}

#[test]
fn a_use_path_disambiguates_a_bare_rust_call_between_modules() {
    let ix = indexed(&[
        ("src/a.rs", "pub fn value(x: u8) -> u8 { x }\n"),
        ("src/b.rs", "pub fn value(x: u8) -> u8 { x + 1 }\n"),
        (
            "src/main.rs",
            "use crate::a::value;\nfn go() { value(1); }\n",
        ),
    ]);
    let map: HashMap<String, String> = build_import_map(&[cort::chunker::Edge {
        rel_type: "imports".into(),
        call_form: CallForm::Bare,
        source_symbol: None,
        raw_target: "crate::a::value".into(),
        start_line: 1,
    }]);
    let ids = resolve_targets(&ix.db, &ix.project_id, "src/main.rs", &map, "value").unwrap();
    assert_eq!(ids.len(), 1, "the use path must pick module a, got {ids:?}");
    assert!(ids[0].contains("src/a.rs"));
}

#[test]
fn a_qualified_call_matching_no_module_stays_unresolved() {
    let ix = indexed(&[
        ("src/a.rs", "pub fn value(x: u8) -> u8 { x }\n"),
        ("src/main.rs", "fn go() { crate::nope::value(1); }\n"),
    ]);
    let map: HashMap<String, String> = HashMap::new();
    let ids = resolve_targets(
        &ix.db,
        &ix.project_id,
        "src/main.rs",
        &map,
        "crate::nope::value",
    )
    .unwrap();
    assert!(
        ids.is_empty(),
        "no module `nope` exists; the call must not resolve"
    );
}

/// The hole the module-suffix rule cannot see, pinned rather than argued away.
///
/// `use std::fs;` + `fs::write(..)` is a dependency call, but the only evidence the suffix rule has
/// is "some project module is named `fs`" -- and a project that ships `src/fs.rs` matches that
/// exactly. Telling `std::fs` from a local `fs` needs the crate's own name or `mod` declarations,
/// which is the undecided half of the "B" question (`docs/2026-08-31-coverage-external-review.md`).
/// If a future change makes this resolve to nothing, the README limitation that describes it has to
/// be deleted in the same commit -- a pinned wrong behaviour is a contract, a documented-away one is
/// a lie waiting to happen.
#[test]
fn a_std_module_qualifier_that_matches_a_local_module_file_still_attaches() {
    let ix = indexed(&[
        ("src/fs.rs", "pub fn write(path: &str) -> u32 { 1 }\n"),
        (
            "src/main.rs",
            "use std::fs;\nfn go() -> u32 { fs::write(\"x\") }\n",
        ),
    ]);
    let map = build_import_map(&[cort::chunker::Edge {
        rel_type: "imports".into(),
        call_form: CallForm::Bare,
        source_symbol: None,
        raw_target: "std::fs".into(),
        start_line: 1,
    }]);
    let ids = resolve_targets(&ix.db, &ix.project_id, "src/main.rs", &map, "fs::write").unwrap();
    assert_eq!(
        ids.len(),
        1,
        "current behaviour: the local `fs` module shadows the std one, {ids:?}"
    );
    assert!(ids[0].contains("src/fs.rs"), "{ids:?}");
}
