//! D-1..D-12 — tests/context.test.js (frozen JS reference)
//! Plus proposal §4 owner-disambiguation regressions.

use cort::ast_grep::{exec_ast_grep, resolve_ast_grep_bin, ExecOpts};
use cort::budget::estimate_tokens;
use cort::chunker::{
    canonical_owner, compose_symbol_name, extract_file, ComposeError, ExtractFileArgs,
};
use cort::context::{
    context_command, parse_symbol_query, ContextOptions, SymbolQuery, CONTENT_HEAD_LINES,
    DEFAULT_BUDGET, NEIGHBORS_PER_SEED,
};
use cort::db::{ensure_schema, get_meta, open_db, project_id_for, set_meta};
use cort::incremental::incremental_index;
use cort::indexer::full_index;
use cort::pack::{extractor_version, pack_files, sgconfig};
use cort::render::{render, Format};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake_ag() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ast-grep")
}

fn with_vars(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
    let _g = env_guard();
    let prev: Vec<(String, Option<String>)> = pairs
        .iter()
        .map(|(k, _)| ((*k).to_string(), std::env::var(k).ok()))
        .collect();
    unsafe {
        for (k, val) in pairs {
            match val {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        for (k, old) in prev {
            match old {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
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

const OLD_RUST_YML: &str =
    "id: cort-rust-chunk-function\nlanguage: Rust\nseverity: hint\nmessage: chunk:function\nrule: { kind: function_item, has: { field: name, pattern: $NAME } }\n";

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

fn git_init(root: &Path) {
    assert!(Command::new("git")
        .args(["init"])
        .current_dir(root)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .status()
        .unwrap()
        .success());
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.email=cort@test",
                "-c",
                "user.name=cort",
                "commit",
                "-m",
                "init"
            ])
            .env("GIT_AUTHOR_NAME", "cort")
            .env("GIT_AUTHOR_EMAIL", "cort@test")
            .env("GIT_COMMITTER_NAME", "cort")
            .env("GIT_COMMITTER_EMAIL", "cort@test")
            .current_dir(root)
            .status()
            .unwrap()
            .success(),
        "git commit failed"
    );
}

/// D-1
#[test]
fn the_default_budget_is_1500_tokens() {
    assert_eq!(DEFAULT_BUDGET, 1500);
    assert_eq!(NEIGHBORS_PER_SEED, 8);
    assert_eq!(CONTENT_HEAD_LINES, 12);
}

/// D-2
#[test]
fn an_exact_symbol_name_resolves_without_touching_fts() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "helper",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    assert_eq!(out["resolution"], "exact_symbol");
    assert_eq!(out["seeds"].as_array().unwrap().len(), 1);
    assert_eq!(out["seeds"][0]["symbol_name"], "helper");
}

/// D-3
#[test]
fn a_non_symbol_query_falls_back_to_fts() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "return",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    assert_eq!(out["resolution"], "fts");
    assert!(!out["seeds"].as_array().unwrap().is_empty());
}

/// D-4
#[test]
fn seeds_carry_depth_1_neighbours() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "helper",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    let names: Vec<&str> = out["seeds"][0]["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["symbol_name"].as_str())
        .collect();
    assert!(names.contains(&"alpha"), "{names:?}");
}

/// D-5
#[test]
fn ambiguous_neighbours_are_dropped_unless_explicitly_requested() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/a.ts", "export function dup() { return 1; }\n"),
        ("src/b.ts", "export function dup() { return 2; }\n"),
        ("src/c.ts", "export function caller() { return dup(); }\n"),
    ]);
    let strict = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "caller",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    let amb: Vec<_> = strict["seeds"][0]["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["confidence"] == "AMBIGUOUS")
        .collect();
    assert!(amb.is_empty());
    let loose = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "caller",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: true,
            full_content: false,
        },
    )
    .unwrap();
    let amb_loose: Vec<_> = loose["seeds"][0]["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["confidence"] == "AMBIGUOUS")
        .collect();
    assert!(!amb_loose.is_empty());
}

/// D-6
#[test]
fn an_unresolvable_reference_is_inlined_on_the_fly_and_never_persisted() {
    let (_dir, root, db, project_id, bin) = indexed(&[(
        "src/only.ts",
        "export function solo() { return externalThing(1); }\n",
    )]);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "solo",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    let u = out["seeds"][0]["unresolved"].as_array().unwrap();
    assert_eq!(u.len(), 1);
    assert_eq!(u[0]["confidence_reasoning"], "unresolved: externalThing");
    assert_eq!(u[0]["confidence_score"], 0.5);
    assert!(u[0].get("target_chunk_id").is_none());
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

/// D-7
#[test]
fn the_emitted_json_actually_fits_the_budget_and_reports_truncation() {
    let mut files: Vec<(String, String)> = vec![(
        "src/hub.ts".into(),
        "export function hub() { return 1; }\n".into(),
    )];
    for i in 0..40 {
        files.push((
            format!("src/c{i}.ts"),
            format!(
                "import {{ hub }} from './hub';\nexport function caller{i}() {{ return hub(); }}\n"
            ),
        ));
    }
    let files_ref: Vec<(&str, &str)> = files
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let (_dir, root, db, project_id, bin) = indexed(&files_ref);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "hub",
        ContextOptions {
            budget: 400,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    let rendered = serde_json::to_string(&out).unwrap();
    assert!(
        estimate_tokens(&rendered) as f64 <= 400.0 * 1.15,
        "the budget is measured on real output, with only packet overhead allowed on top"
    );
    assert_eq!(out["truncated"], true);
}

/// D-8
#[test]
fn an_unknown_query_returns_an_empty_packet_rather_than_throwing() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "nothingmatchesthis",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    assert_eq!(out["seeds"], json!([]));
    assert_eq!(out["resolution"], "none");
}

/// D-9
#[test]
fn context_never_invokes_struct() {
    let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/context.rs"));
    assert!(
        !src.contains("r#struct") && !src.contains("crate::struct"),
        "stage 3 must not depend on stage 2"
    );
}

/// D-10
#[test]
fn seed_content_is_truncated_by_default_and_restorable_with_full_content() {
    let mut lines = vec!["export function bigThing() {".to_string()];
    for i in 0..40 {
        lines.push(format!("  // filler line {i}"));
    }
    lines.push("}".to_string());
    lines.push(String::new());
    let body = lines.join("\n");
    let (_dir, root, db, project_id, bin) = indexed(&[("src/big.ts", body.as_str())]);
    let trimmed = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "bigThing",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    assert_eq!(trimmed["seeds"][0]["content_truncated"], true);
    let line_count = trimmed["seeds"][0]["content"]
        .as_str()
        .unwrap()
        .split('\n')
        .count();
    assert!(
        line_count <= 13,
        "expected at most 12 kept lines + marker, got {line_count}"
    );
    assert!(trimmed["seeds"][0]["content"]
        .as_str()
        .unwrap()
        .ends_with('…'));

    let full = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "bigThing",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: true,
        },
    )
    .unwrap();
    assert_eq!(full["seeds"][0]["content_truncated"], false);
    let full_c = full["seeds"][0]["content"].as_str().unwrap();
    assert!(
        full_c.contains("// filler line 39"),
        "full content must reach the last line"
    );
    assert!(full_c.trim_end().ends_with('}'));
    assert!(full_c.len() > trimmed["seeds"][0]["content"].as_str().unwrap().len());
}

/// D-11
#[test]
fn short_content_is_untouched_and_not_flagged() {
    let (_dir, root, db, project_id, bin) = indexed(SAMPLE);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "helper",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    assert_eq!(out["seeds"][0]["content_truncated"], false);
    assert!(!out["seeds"][0]["content"].as_str().unwrap().ends_with('…'));
}

/// D-12
#[test]
fn a_rust_symbol_returns_only_its_function_body_not_the_rest_of_a_large_file() {
    let body = [
        "fn wanted() -> i32 {",
        "    1",
        "}",
        "",
        "fn unrelated_secret() -> i32 {",
        "    999",
        "}",
        "",
    ]
    .join("\n");
    let (_dir, root, db, project_id, bin) = indexed(&[("src/main.rs", body.as_str())]);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "wanted",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: true,
        },
    )
    .unwrap();
    assert_eq!(out["resolution"], "exact_symbol");
    assert_eq!(out["seeds"].as_array().unwrap().len(), 1);
    assert_eq!(out["seeds"][0]["content"], "fn wanted() -> i32 {\n    1\n}");
    assert!(!out["seeds"][0]["content"]
        .as_str()
        .unwrap()
        .contains("unrelated_secret"));
}

/// Proposal §4 TDD-2: canonical_owner / compose_symbol_name.
#[test]
fn canonical_owner_strips_per_segment_generics_and_normalizes_whitespace() {
    assert_eq!(canonical_owner("Ledger"), "Ledger");
    assert_eq!(
        canonical_owner("crate::ledger::Ledger"),
        "crate::ledger::Ledger"
    );
    assert_eq!(
        canonical_owner("crate::ledger::Ledger<T>"),
        "crate::ledger::Ledger"
    );
    assert_eq!(canonical_owner("Ledger<T, U>"), "Ledger");
    assert_eq!(
        canonical_owner("  crate :: ledger :: Ledger < T >  "),
        "crate::ledger::Ledger"
    );
    assert_eq!(canonical_owner("Vec<HashMap<K, V>>"), "Vec");
    assert_eq!(canonical_owner("Foo<Bar<T>>::Inner<U>"), "Foo::Inner");
    assert_eq!(canonical_owner("(A, B)"), "(A, B)");
    assert_eq!(canonical_owner("( A , B )"), "( A , B )");
    assert_eq!(
        compose_symbol_name("method", Some("run"), Some("Ledger<T>"), Some("Rust")).unwrap(),
        Some("Ledger::run".into())
    );
    assert_eq!(
        compose_symbol_name("function", Some("main"), None, Some("Rust")).unwrap(),
        Some("main".into())
    );
    assert_eq!(
        compose_symbol_name("method", Some("run"), None, Some("Rust")).unwrap_err(),
        ComposeError::MethodMissingOwner
    );
    assert_eq!(
        compose_symbol_name("method", Some("go"), None, Some("TypeScript")).unwrap(),
        Some("go".into())
    );
}

fn parse_scan(abs: &Path) -> Vec<serde_json::Value> {
    let bin = resolve_ast_grep_bin().expect("ast-grep");
    let sg = sgconfig();
    let r = exec_ast_grep(
        &bin,
        &[
            "scan",
            "--json=stream",
            "--config",
            sg.to_str().unwrap(),
            abs.to_str().unwrap(),
        ],
        ExecOpts::default(),
    )
    .unwrap();
    r.stdout
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Proposal §4 TDD-1: free NAME only; impl/trait OWNER+NAME; no duplicate hits.
#[test]
fn rust_pack_rules_are_mutually_exclusive_and_capture_owner() {
    let body = [
        "fn main() {}",
        "struct Worker;",
        "impl Worker {",
        "    fn run(&self) {}",
        "}",
        "trait Runner {",
        "    fn go(&self) { 1 }",
        "    fn declared(&self);",
        "}",
        "impl Runner for Worker {",
        "    fn go(&self) { 2 }",
        "}",
        "",
    ]
    .join("\n");
    let (_dir, abs) = {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lib.rs");
        fs::write(&p, &body).unwrap();
        (dir, p)
    };
    let recs = parse_scan(&abs);
    let chunks: Vec<_> = recs
        .iter()
        .filter(|r| r["message"].as_str().unwrap_or("").starts_with("chunk:"))
        .collect();
    let main = chunks
        .iter()
        .find(|r| r["metaVariables"]["single"]["NAME"]["text"] == "main")
        .expect("main");
    assert_eq!(main["message"], "chunk:function");
    assert!(main["metaVariables"]["single"].get("OWNER").is_none());

    let run = chunks
        .iter()
        .find(|r| r["metaVariables"]["single"]["NAME"]["text"] == "run")
        .expect("run");
    assert_eq!(run["message"], "chunk:method");
    assert_eq!(run["metaVariables"]["single"]["OWNER"]["text"], "Worker");

    let trait_go = chunks
        .iter()
        .filter(|r| r["metaVariables"]["single"]["NAME"]["text"] == "go")
        .collect::<Vec<_>>();
    assert_eq!(trait_go.len(), 2, "trait default + trait impl, no dupes");
    let owners: Vec<&str> = trait_go
        .iter()
        .map(|r| {
            r["metaVariables"]["single"]["OWNER"]["text"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert!(owners.contains(&"Runner"), "{owners:?}");
    assert!(owners.contains(&"Worker"), "{owners:?}");
    assert!(!chunks
        .iter()
        .any(|r| { r["metaVariables"]["single"]["NAME"]["text"] == "declared" }));
}

/// Proposal §4 TDD-3: six impls named run → A::run..F::run, no bare run method.
#[test]
fn six_impls_all_named_run_store_qualified_names() {
    let mut body = String::new();
    for name in ["A", "B", "C", "D", "E", "F"] {
        body.push_str(&format!(
            "struct {name};\nimpl {name} {{ fn run() {{}} }}\n"
        ));
    }
    let (_dir, _root, db, _id, _bin) = indexed(&[("src/lib.rs", body.as_str())]);
    let names: Vec<String> = db
        .prepare("SELECT symbol_name FROM chunks WHERE chunk_type = 'method' ORDER BY symbol_name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["A::run", "B::run", "C::run", "D::run", "E::run", "F::run"]
    );
    let bare: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE symbol_name = 'run'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bare, 0);
}

/// Proposal §4 TDD-4: seed hard limit 5, querying the sixth is still exact.
#[test]
fn qualified_query_for_the_sixth_run_is_still_exact() {
    let mut body = String::new();
    for name in ["A", "B", "C", "D", "E", "F"] {
        body.push_str(&format!(
            "struct {name};\nimpl {name} {{ fn run() {{}} }}\n"
        ));
    }
    let (_dir, root, db, project_id, bin) = indexed(&[("src/lib.rs", body.as_str())]);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "F::run",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: true,
        },
    )
    .unwrap();
    assert_eq!(out["resolution"], "exact_symbol");
    assert_eq!(out["seeds"].as_array().unwrap().len(), 1);
    assert_eq!(out["seeds"][0]["symbol_name"], "F::run");
    assert_eq!(out["seed_count"], 1);
}

/// Proposal §4 TDD-5: trait default method body via --content full.
#[test]
fn trait_default_method_body_via_content_full() {
    let body = [
        "trait Worker {",
        "    fn run(&self) {",
        "        1",
        "    }",
        "}",
        "",
    ]
    .join("\n");
    let (_dir, root, db, project_id, bin) = indexed(&[("src/lib.rs", body.as_str())]);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "Worker::run",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: true,
        },
    )
    .unwrap();
    assert_eq!(out["resolution"], "exact_symbol");
    assert_eq!(out["seeds"][0]["chunk_type"], "method");
    assert_eq!(out["seeds"][0]["symbol_name"], "Worker::run");
    let content = out["seeds"][0]["content"].as_str().unwrap();
    assert!(content.contains("fn run"));
    assert!(content.contains('1'));
}

/// Proposal §4 TDD-6: trait-impl collision keeps both, stable file_path,start_line order.
#[test]
fn trait_impl_collision_keeps_both_type_run_in_stable_order() {
    let body = [
        "struct Type;",
        "trait T1 { fn run(&self); }",
        "trait T2 { fn run(&self); }",
        "impl T1 for Type { fn run(&self) {} }",
        "impl T2 for Type { fn run(&self) {} }",
        "",
    ]
    .join("\n");
    let (_dir, root, db, project_id, bin) = indexed(&[("src/lib.rs", body.as_str())]);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "Type::run",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: true,
        },
    )
    .unwrap();
    assert_eq!(out["resolution"], "exact_symbol");
    assert_eq!(out["seed_count"], 2);
    let seeds = out["seeds"].as_array().unwrap();
    assert_eq!(seeds.len(), 2);
    assert_eq!(seeds[0]["symbol_name"], "Type::run");
    assert_eq!(seeds[1]["symbol_name"], "Type::run");
    let a = (
        seeds[0]["file_path"].as_str().unwrap(),
        seeds[0]["start_line"].as_i64().unwrap(),
    );
    let b = (
        seeds[1]["file_path"].as_str().unwrap(),
        seeds[1]["start_line"].as_i64().unwrap(),
    );
    assert!(a <= b, "stable sort by file_path,start_line: {a:?} {b:?}");
}

/// Proposal §4 TDD-7: Type::method json/lean field-by-field; nonexistent qualified is none without FTS.
#[test]
fn type_method_json_and_lean_and_nonexistent_qualified_is_none_without_fts() {
    let body = [
        "struct Ledger;",
        "impl Ledger {",
        "    fn run(&self) {}",
        "}",
        "fn rummage() { let _ = \"run-adjacent\"; }",
        "",
    ]
    .join("\n");
    let (_dir, root, db, project_id, bin) = indexed(&[("src/lib.rs", body.as_str())]);
    let out = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "Ledger::run",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: true,
        },
    )
    .unwrap();
    assert_eq!(out["query"], "Ledger::run");
    assert_eq!(out["resolution"], "exact_symbol");
    assert!(out["seeds"].is_array());
    assert_eq!(out["seed_count"], 1);
    assert!(out["truncated"].is_boolean());
    assert_eq!(out["truncated_query"], false);
    assert!(out["index_is_stale"].is_boolean());
    let seed = &out["seeds"][0];
    for key in [
        "chunk_id",
        "file_path",
        "symbol_name",
        "chunk_type",
        "start_line",
        "end_line",
        "content",
        "content_truncated",
        "neighbors",
        "unresolved",
    ] {
        assert!(seed.get(key).is_some(), "missing {key}");
    }
    assert_eq!(seed["symbol_name"], "Ledger::run");
    assert_eq!(seed["chunk_type"], "method");

    let lean = render(Some("context"), Format::Lean, &out);
    assert!(
        lean.starts_with("# context Ledger::run resolution=exact_symbol seeds=1"),
        "{lean}"
    );
    assert!(lean.contains("Ledger::run\tmethod"), "{lean}");

    let none = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "Ledger::nope",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    assert_eq!(none["resolution"], "none");
    assert_eq!(none["seeds"], json!([]));
    assert_eq!(none["truncated_query"], false);
}

/// Unqualified still uses exact then FTS (proposal §4).
#[test]
fn parse_symbol_query_splits_on_the_last_colon_colon_outside_generics() {
    match parse_symbol_query("Ledger::new") {
        SymbolQuery::Qualified { owner, member } => {
            assert_eq!(owner, "Ledger");
            assert_eq!(member, "new");
        }
        other => panic!("{other:?}"),
    }
    match parse_symbol_query("crate::ledger::Ledger::run") {
        SymbolQuery::Qualified { owner, member } => {
            assert_eq!(owner, "crate::ledger::Ledger");
            assert_eq!(member, "run");
        }
        other => panic!("{other:?}"),
    }
    match parse_symbol_query("Vec<HashMap<K, V>>::new") {
        SymbolQuery::Qualified { owner, member } => {
            assert_eq!(owner, "Vec<HashMap<K, V>>");
            assert_eq!(member, "new");
        }
        other => panic!("{other:?}"),
    }
    match parse_symbol_query("helper") {
        SymbolQuery::Unqualified(t) => assert_eq!(t, "helper"),
        other => panic!("{other:?}"),
    }
}

/// Method record without OWNER fails closed — no bare NAME.
#[test]
fn method_record_without_owner_is_malformed_extraction() {
    let rec = serde_json::json!({
        "message": "chunk:method",
        "text": "fn run() {}",
        "language": "Rust",
        "file": "lib.rs",
        "range": { "start": { "line": 0 }, "end": { "line": 0 } },
        "metaVariables": { "single": { "NAME": { "text": "run" } } },
    });
    let stream = format!("{rec}\n");
    let b64 = {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes = stream.as_bytes();
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
    };
    let fake = fake_ag();
    let mode = format!("emit:{b64}");
    with_vars(&[("FAKE_AG_MODE", Some(mode.as_str()))], || {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join("lib.rs");
        fs::write(&abs, "fn run() {}\n").unwrap();
        let out = extract_file(ExtractFileArgs {
            bin: fake.to_str().unwrap(),
            project_id: "p",
            file_path: "lib.rs",
            abs_path: abs.to_str().unwrap(),
            source: "fn run() {}\n",
            timeout_ms: Some(5_000),
        })
        .unwrap();
        assert!(!out
            .chunks
            .iter()
            .any(|c| c.symbol_name.as_deref() == Some("run")));
        assert!(out.malformed >= 1);
    });
}

fn hash_pack_with_rust_yml(rust_yml: &[u8]) -> String {
    let mut h = Sha256::new();
    for f in pack_files() {
        if f.ends_with("rust.yml") {
            h.update(rust_yml);
        } else {
            h.update(fs::read(&f).unwrap());
        }
    }
    format!("{:x}", h.finalize())
}

/// Proposal §4 TDD-8 / deliverable 6: rust.yml change moves pack hash; old index requires full rebuild.
#[test]
fn rust_yml_change_moves_the_pack_hash_and_old_index_requires_full_rebuild() {
    let current = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src/pack/rules/rust.yml"),
    )
    .unwrap();
    assert_ne!(current.as_slice(), OLD_RUST_YML.as_bytes());
    let new_hash = hash_pack_with_rust_yml(&current);
    let old_hash = hash_pack_with_rust_yml(OLD_RUST_YML.as_bytes());
    assert_ne!(new_hash, old_hash);
    assert_eq!(new_hash, extractor_version());
    assert!(String::from_utf8_lossy(&current).contains("cort-rust-chunk-impl-method"));
    assert!(String::from_utf8_lossy(&current).contains("stopBy: end"));

    let (dir, root) = make_project(&[("src/main.rs", "fn a() {}\n")]);
    git_init(&root);
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let bin = resolve_ast_grep_bin().unwrap();
    full_index(&mut db, &bin, &root).unwrap();
    set_meta(
        &db,
        "extractor_version",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
    )
    .unwrap();
    let result = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(result.mode, "full");
    let stored = get_meta(&db, "extractor_version").unwrap().unwrap();
    assert_eq!(stored, extractor_version());
    let _ = dir;
}
