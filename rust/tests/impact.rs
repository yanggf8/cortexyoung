//! D-26..D-34 — the Rust port kept the case ids (audit F-12).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::graph::internal_rust_path_target;
use cort::impact::{impact_command, DEFAULT_DEPTH};
use cort::indexer::full_index;
use std::fs;
use std::path::PathBuf;

const CHAIN: &[(&str, &str)] = &[
    ("src/d.ts", "export function d() { return 1; }\n"),
    (
        "src/c.ts",
        "import { d } from './d';\nexport function c() { return d(); }\n",
    ),
    (
        "src/b.ts",
        "import { c } from './c';\nexport function b() { return c(); }\n",
    ),
    (
        "src/a.ts",
        "import { b } from './b';\nexport function a() { return b(); }\n",
    ),
];

fn make_project(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-proj-")
        .tempdir()
        .unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, body).unwrap();
    }
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn indexed(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let (dir, root) = make_project(files);
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

/// D-26
#[test]
fn the_default_depth_is_3() {
    assert_eq!(DEFAULT_DEPTH, 3);
}

/// D-27
#[test]
fn dependents_are_returned_with_their_hop_distance() {
    let (_dir, root, db, project_id, bin) = indexed(CHAIN);
    let out = impact_command(&db, &bin, &root, &project_id, "d", DEFAULT_DEPTH).unwrap();
    let deps = out["dependents"].as_array().unwrap();
    let mut by_name = serde_json::Map::new();
    for d in deps {
        by_name.insert(
            d["symbol_name"].as_str().unwrap().to_string(),
            d["hop"].clone(),
        );
    }
    assert_eq!(
        serde_json::Value::Object(by_name),
        serde_json::json!({ "c": 1, "b": 2, "a": 3 })
    );
}

/// D-28
#[test]
fn depth_is_respected() {
    let (_dir, root, db, project_id, bin) = indexed(CHAIN);
    let out = impact_command(&db, &bin, &root, &project_id, "d", 1).unwrap();
    let names: Vec<&str> = out["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["symbol_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["c"]);
}

/// D-29
#[test]
fn a_symbol_with_no_dependents_returns_an_empty_list_not_an_error() {
    let (_dir, root, db, project_id, bin) = indexed(CHAIN);
    let out = impact_command(&db, &bin, &root, &project_id, "a", DEFAULT_DEPTH).unwrap();
    assert_eq!(out["dependents"], serde_json::json!([]));
    assert_eq!(out["seed_count"], 1);
}

/// D-30
#[test]
fn an_unknown_symbol_reports_zero_seeds_without_throwing() {
    let (_dir, root, db, project_id, bin) = indexed(CHAIN);
    let out = impact_command(&db, &bin, &root, &project_id, "nosuchsymbol", DEFAULT_DEPTH).unwrap();
    assert_eq!(out["seed_count"], 0);
    assert_eq!(out["dependents"], serde_json::json!([]));
}

/// D-31
#[test]
fn an_ambiguous_symbol_seeds_from_every_matching_chunk() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/a.ts", "export function dup() { return 1; }\n"),
        ("src/b.ts", "export function dup() { return 2; }\n"),
        ("src/c.ts", "export function caller() { return dup(); }\n"),
    ]);
    let out = impact_command(&db, &bin, &root, &project_id, "dup", DEFAULT_DEPTH).unwrap();
    assert_eq!(out["seed_count"], 2);
    assert!(out["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["symbol_name"] == "caller"));
}

/// D-32
#[test]
fn unresolved_references_are_inlined_on_the_fly_and_nothing_is_persisted() {
    let (_dir, root, db, project_id, bin) = indexed(&[(
        "src/only.ts",
        "export function solo() { return externalThing(1); }\n",
    )]);
    let out = impact_command(&db, &bin, &root, &project_id, "solo", DEFAULT_DEPTH).unwrap();
    assert_eq!(out["unresolved"].as_array().unwrap().len(), 1);
    assert_eq!(
        out["unresolved"][0]["confidence_reasoning"],
        "unresolved: externalThing"
    );
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

/// D-33
#[test]
fn the_packet_reports_index_staleness() {
    let (_dir, root, db, project_id, bin) = indexed(CHAIN);
    let out = impact_command(&db, &bin, &root, &project_id, "d", DEFAULT_DEPTH).unwrap();
    assert_eq!(out["index_is_stale"], false);
}

/// D-34
#[test]
fn symbol_accepts_a_comma_separated_batch_and_merges_dependents_at_min_hop() {
    let (_dir, root, db, project_id, bin) = indexed(CHAIN);
    let out = impact_command(&db, &bin, &root, &project_id, "d,c", DEFAULT_DEPTH).unwrap();
    assert_eq!(out["seed_count"], 2);
    let deps = out["dependents"].as_array().unwrap();
    let mut by_name = serde_json::Map::new();
    for d in deps {
        by_name.insert(
            d["symbol_name"].as_str().unwrap().to_string(),
            d["hop"].clone(),
        );
    }
    assert_eq!(
        serde_json::Value::Object(by_name),
        serde_json::json!({ "b": 1, "a": 2 }),
        "b is hop-1 from seed c, a is hop-2; seeds themselves are excluded"
    );
}

/// Root cause: `resolve_targets` matched `symbol_name` exactly, so `crate::def::my_func()` — the way
/// Rust code is normally written — could never attach. See
/// docs/2026-08-31-rust-qualified-call-resolution.md.
#[test]
fn an_internal_rust_path_call_resolves_to_the_bare_symbol() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/def.rs", "pub fn my_func() -> u32 { 7 }\n"),
        (
            "src/lib.rs",
            "pub mod def;\npub fn caller() -> u32 { crate::def::my_func() }\n",
        ),
    ]);
    let out = impact_command(&db, &bin, &root, &project_id, "my_func", 1).unwrap();
    let names: Vec<&str> = out["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["symbol_name"].as_str())
        .collect();
    assert_eq!(names, vec!["caller"], "{out}");
}

#[test]
fn a_dependency_call_stays_unresolved_and_says_so_instead_of_becoming_an_edge() {
    // The other half of the fix. Stripping `Vec::new` to `new` would invent an edge to any chunk
    // named `new` and erase the row that admits the hole.
    let (_dir, root, db, project_id, bin) = indexed(&[
        (
            "src/new.rs",
            "pub fn new() -> u32 { let v = Vec::new(); 1 }\n",
        ),
        // The std call sits inside the seed itself: `impact`'s `unresolved` section reports what the
        // seed could not resolve on its way out, while calls *to* the seed that got dropped are the
        // coverage screen's `extracted_but_unresolved` layer. Two different questions.
        ("src/lib.rs", "pub fn other() -> u32 { new() }\n"),
    ]);
    let out = impact_command(&db, &bin, &root, &project_id, "new", 1).unwrap();
    let names: Vec<&str> = out["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["symbol_name"].as_str())
        .collect();
    assert_eq!(names, vec!["other"], "only the real project call: {out}");
    let unresolved: Vec<&str> = out["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["symbol"].as_str())
        .collect();
    assert!(
        unresolved.contains(&"Vec::new"),
        "the std call must stay visible as unresolved rather than become an invented edge: {out}"
    );
}

#[test]
fn only_the_three_internal_rust_prefixes_are_rescued() {
    assert_eq!(
        internal_rust_path_target("crate::def::my_func"),
        Some("my_func")
    );
    assert_eq!(internal_rust_path_target("self::helper"), Some("helper"));
    assert_eq!(internal_rust_path_target("super::shared::f"), Some("f"));
    assert_eq!(internal_rust_path_target("::kids::f"), Some("f"));
    for keep_unresolved in [
        "Vec::new",
        "formatter.formatToParts",
        "Tally::add",
        "my_func",
        "crate::",
        "crate::generic::<T>",
        "crate::weird(",
    ] {
        assert_eq!(
            internal_rust_path_target(keep_unresolved),
            None,
            "{keep_unresolved} must not be stripped"
        );
    }
}
