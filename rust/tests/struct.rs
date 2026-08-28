//! D-13..D-25 — tests/struct.test.js (frozen JS reference)

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::errors::CortError;
use cort::indexer::full_index;
use cort::r#struct::{
    StructOptions, containment_join, preflight_pattern, run_pattern, struct_command, MAX_MALFORMED_RATIO,
    MAX_NEIGHBORS, UNBOUNDED_SCAN_FILE_LIMIT,
};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake_ag() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ast-grep")
}

fn with_var(key: &str, val: Option<&str>, f: impl FnOnce()) {
    let _g = env_guard();
    let prev = std::env::var(key).ok();
    // SAFETY: tests in this file take ENV_LOCK so no other thread mutates env.
    unsafe {
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

const SAMPLE: &[(&str, &str)] = &[
    (
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    ),
    (
        "src/alpha.ts",
        "import { helper } from './helper';\n\
export function alpha(a: number) { return helper(a) + 1; }\n\
export class Beta {\n\
  go() { return alpha(2); }\n\
}\n",
    ),
    (
        "node_modules/pkg/index.ts",
        "export function shouldBeIgnored() {}\n",
    ),
    ("README.md", "# not a source file\n"),
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

/// D-13
#[test]
fn constants_match_the_spec() {
    assert!((MAX_MALFORMED_RATIO - 0.10).abs() < f64::EPSILON);
    assert_eq!(MAX_NEIGHBORS, 3);
    assert_eq!(UNBOUNDED_SCAN_FILE_LIMIT, 2000);
}

/// D-14
#[test]
fn a_malformed_pattern_is_caught_by_the_pre_flight_not_by_the_exit_code() {
    let (_dir, root, _db, _id, bin) = indexed(SAMPLE);
    let err = preflight_pattern(&bin, "function (", "ts", &[root.to_string_lossy().into_owned()])
        .unwrap_err();
    assert_eq!(err.code, "parse_failed");
    assert_eq!(err.detail["pattern"], "function (");
    assert_eq!(err.detail["lang"], "ts");
}

/// D-15
#[test]
fn a_valid_pattern_passes_the_pre_flight() {
    let (_dir, root, _db, _id, bin) = indexed(SAMPLE);
    preflight_pattern(
        &bin,
        "helper($A)",
        "ts",
        &[root.to_string_lossy().into_owned()],
    )
    .unwrap();
}

/// D-16
#[test]
fn zero_matches_is_a_clean_empty_result_never_parse_failed() {
    let (_dir, root, _db, _id, bin) = indexed(SAMPLE);
    let r = run_pattern(
        &bin,
        "zzzNoSuchFunction($A)",
        "ts",
        &[root.to_string_lossy().into_owned()],
        None,
        false,
    )
    .unwrap();
    assert!(r.matches.is_empty());
    assert_eq!(r.malformed, 0);
}

/// D-17
#[test]
fn matches_are_returned_with_1_indexed_lines() {
    let (_dir, root, _db, _id, bin) = indexed(SAMPLE);
    let r = run_pattern(
        &bin,
        "helper($A)",
        "ts",
        &[root.to_string_lossy().into_owned()],
        None,
        false,
    )
    .unwrap();
    assert!(!r.matches.is_empty());
    assert!(r.matches.iter().all(|m| m.start_line >= 1));
    assert!(r.matches.iter().all(|m| !m.file.is_empty()));
}

/// D-18
#[test]
fn a_few_malformed_json_lines_are_skipped_and_counted() {
    let good = serde_json::json!({
        "text": "x",
        "file": "a.ts",
        "range": { "start": { "line": 0 }, "end": { "line": 0 } },
    });
    let mut stream = String::new();
    for _ in 0..19 {
        stream.push_str(&good.to_string());
        stream.push('\n');
    }
    stream.push_str("junk\n");
    let b64 = base64_encode(stream.as_bytes());
    with_var("FAKE_AG_MODE", Some(&format!("emit:{b64}")), || {
        let r = run_pattern(
            fake_ag().to_str().unwrap(),
            "x",
            "ts",
            &[".".to_string()],
            None,
            true,
        )
        .unwrap();
        assert_eq!(r.malformed, 1);
        assert_eq!(r.matches.len(), 19);
    });
}

/// D-19
#[test]
fn more_than_10_percent_malformed_aborts_this_query_only() {
    let good = serde_json::json!({
        "text": "x",
        "file": "a.ts",
        "range": { "start": { "line": 0 }, "end": { "line": 0 } },
    });
    let mut stream = String::new();
    for _ in 0..8 {
        stream.push_str(&good.to_string());
        stream.push('\n');
    }
    stream.push_str("junk\njunk\n");
    let b64 = base64_encode(stream.as_bytes());
    with_var("FAKE_AG_MODE", Some(&format!("emit:{b64}")), || {
        let err = run_pattern(
            fake_ag().to_str().unwrap(),
            "x",
            "ts",
            &[".".to_string()],
            None,
            true,
        )
        .unwrap_err();
        assert_eq!(err.code, "run_aborted_malformed");
        assert_eq!(err.detail["malformed"], 2);
        assert_eq!(err.detail["total"], 10);
    });
}

/// D-20
#[test]
fn containment_join_picks_the_smallest_chunk_that_contains_the_match() {
    let (_dir, _root, db, project_id, _bin) = indexed(SAMPLE);
    let hit = containment_join(&db, &project_id, "src/alpha.ts", 4, 4)
        .unwrap()
        .expect("hit");
    assert_eq!(
        hit.symbol_name.as_deref(),
        Some("go"),
        "the method must win over the enclosing class"
    );
}

/// D-21
#[test]
fn containment_join_returns_null_when_no_chunk_contains_the_match() {
    let (_dir, _root, db, project_id, _bin) = indexed(SAMPLE);
    assert!(containment_join(&db, &project_id, "src/alpha.ts", 9999, 9999)
        .unwrap()
        .is_none());
}

/// D-22
#[test]
fn struct_command_attaches_at_most_max_neighbors_neighbours_and_reports_staleness() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let out = struct_command(
        &db,
        &bin,
        &root,
        &project_id,
        "helper($A)",
        "ts",
        StructOptions {
            globs: Vec::new(),
            budget: 1500,
            file_limit: None,
        },
    )
    .unwrap();
    assert!(!out["matches"].as_array().unwrap().is_empty());
    let m = &out["matches"][0];
    assert_eq!(m["symbol_name"], "alpha");
    assert!(m["neighbors"].is_array());
    assert!(m["neighbors"].as_array().unwrap().len() <= MAX_NEIGHBORS as usize);
    assert!(out["index_is_stale"].is_boolean());
    assert!(out["truncated"].is_boolean());
}

/// D-23
#[test]
fn struct_command_surfaces_parse_failed_as_a_structured_error_and_runs_nothing() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let err = struct_command(
        &db,
        &bin,
        &root,
        &project_id,
        "function (",
        "ts",
        StructOptions {
            globs: Vec::new(),
            budget: 1500,
            file_limit: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "parse_failed");
    assert_eq!(err.to_json()["error"], "parse_failed");
    let _ = CortError::with_code("parse_failed");
}

/// D-24
#[test]
fn an_unglobbed_scan_of_a_large_project_is_refused_with_actionable_advice() {
    let files: Vec<(String, String)> = (0..12)
        .map(|i| {
            (
                format!("src/f{i}.ts"),
                format!("export function f{i}() {{ return {i}; }}\n"),
            )
        })
        .collect();
    let files_ref: Vec<(&str, &str)> = files.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (_dir, root, db, project_id, bin) = indexed(&files_ref);
    let err = struct_command(
        &db,
        &bin,
        &root,
        &project_id,
        "f0()",
        "ts",
        StructOptions {
            globs: Vec::new(),
            budget: 1500,
            file_limit: Some(10),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "scan_too_broad");
    assert_eq!(err.detail["indexed_files"], 12);
    assert_eq!(err.detail["limit"], 10);
    assert!(err.detail["hint"].as_str().unwrap().contains("-g"));
}

/// D-25
#[test]
fn the_same_scan_succeeds_once_a_glob_narrows_it() {
    let files: Vec<(String, String)> = (0..12)
        .map(|i| {
            (
                format!("src/f{i}.ts"),
                format!("export function f{i}() {{ return {i}; }}\n"),
            )
        })
        .collect();
    let files_ref: Vec<(&str, &str)> = files.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (_dir, root, db, project_id, bin) = indexed(&files_ref);
    let glob = root.join("src/f0.ts").to_string_lossy().into_owned();
    struct_command(
        &db,
        &bin,
        &root,
        &project_id,
        "f0()",
        "ts",
        StructOptions {
            globs: vec![glob.clone()],
            budget: 1500,
            file_limit: Some(10),
        },
    )
    .unwrap();
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}
