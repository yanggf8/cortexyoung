//! C1-20..C1-29 — tests/graph.test.js (frozen JS reference)
//!
//! JS graph tests go through `fullIndex` (Job C2). C1 seeds via `extract_file` +
//! `relationship_rows_for_file` so we do not call the C2 stub.

use cort::ast_grep::resolve_ast_grep_bin;
use cort::chunker::{extract_file, Chunk, ExtractFileArgs, ExtractResult};
use cort::db::{ensure_schema, open_db, project_id_for, Db};
use cort::graph::{
    build_import_map, get_neighbors, get_transitive_dependents, relationship_rows_for_file,
    resolve_targets, unresolved_inline, CONFIDENCE_SCORE,
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
                    (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(source_chunk_id, target_chunk_id, rel_type) DO NOTHING",
                params![
                    r.source_chunk_id,
                    r.target_chunk_id,
                    r.rel_type,
                    r.confidence,
                    r.confidence_score,
                    r.confidence_reasoning,
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
            source_symbol: None,
            raw_target: "./helper".into(),
            start_line: 1,
        },
        cort::chunker::Edge {
            rel_type: "calls".into(),
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
        source_symbol: None,
        raw_target: "./helper".into(),
        start_line: 1,
    }]);
    let ids = resolve_targets(&ix.db, &ix.project_id, "src/alpha.ts", &map, "dup").unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids[0].contains("src/helper.ts"));
}
