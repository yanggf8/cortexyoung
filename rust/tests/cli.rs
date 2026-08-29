//! D-42..D-47 — the Rust port kept the case ids (audit F-12).
//! Plus plan §7 B-gap canonicalize-before-hash, format errors, CORT_CACHE_DIR.

use cort::db::project_id_for;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

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

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_cort(args: &[&str], cwd: &Path, cache: &Path) -> Run {
    let out = Command::new(cort_bin())
        .args(args)
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .output()
        .expect("spawn cort");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn payload(run: &Run) -> Value {
    serde_json::from_str(run.stdout.trim_end()).unwrap_or_else(|e| {
        panic!(
            "json parse failed: {e}; stdout={:?} stderr={:?}",
            run.stdout, run.stderr
        )
    })
}

fn sandbox() -> (tempfile::TempDir, PathBuf, tempfile::TempDir, PathBuf) {
    let (proj, cwd) = make_project(SAMPLE);
    let cache_dir = tempfile::Builder::new()
        .prefix("cort-cache-")
        .tempdir()
        .unwrap();
    let cache = cache_dir.path().to_path_buf();
    (proj, cwd, cache_dir, cache)
}

/// D-42
#[test]
fn asking_a_command_for_help_explains_it_instead_of_running_it() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["index", "--help"], &cwd, &cache);
    assert_eq!(r.code, 0);
    let p = payload(&r);
    assert!(p["commands"]["index"]
        .as_str()
        .unwrap()
        .starts_with("cort index"));
    assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
}

/// D-43
#[test]
fn every_spelling_of_help_reaches_the_same_usage_and_none_of_them_is_an_error() {
    let (_p, cwd, _c, cache) = sandbox();
    for args in [
        vec!["help"],
        vec!["--help"],
        vec!["-h"],
        vec!["impact", "-h"],
        vec!["delete", "--help"],
        vec!["struct", "--help"],
        vec!["context", "-h"],
        vec!["read", "-h"],
        vec!["recall", "--help"],
        vec!["status", "-h"],
        vec!["projects", "--help"],
    ] {
        let r = run_cort(&args, &cwd, &cache);
        assert_eq!(r.code, 0, "{} should exit 0", args.join(" "));
        let p = payload(&r);
        assert_eq!(p["usage"], "cort <command> [options]");
        assert_eq!(
            fs::read_dir(&cache).unwrap().count(),
            0,
            "{} must not touch the cache",
            args.join(" ")
        );
    }
}

/// D-44
#[test]
fn usage_documents_every_command_the_dispatcher_actually_knows() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage = payload(&run_cort(&["--help"], &cwd, &cache));
    let known = payload(&run_cort(&["nope"], &cwd, &cache));
    let mut usage_keys: Vec<String> = usage["commands"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    usage_keys.sort();
    let mut known_keys: Vec<String> = known["detail"]["known"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    known_keys.sort();
    assert_eq!(usage_keys, known_keys);
}

/// D-45
#[test]
fn an_unknown_command_is_still_a_failure_not_usage() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["nope"], &cwd, &cache);
    assert_eq!(r.code, 1);
    let p = payload(&r);
    assert_eq!(p["error"], "unknown_command");
    assert_eq!(p["detail"]["command"], "nope");
}

/// D-46
#[test]
fn index_without_help_still_indexes_so_the_guard_did_not_swallow_the_command() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["index"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert!(p["chunks"].as_i64().unwrap() > 0);
    let names: Vec<String> = fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with(".db") && n != "usage.db"),
        "index must write a project db, names={names:?}"
    );
}

/// D-47 — second auto-read is store/receipt (proposal §1); recall still finds it.
#[test]
fn read_persists_a_fragment_and_recall_finds_it_through_fts() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let first = run_cort(
        &["read", "src/alpha.ts", "--start", "2", "--end", "2"],
        &cwd,
        &cache,
    );
    assert_eq!(first.code, 0, "stderr={}", first.stderr);
    let first_p = payload(&first);
    assert_eq!(first_p["source"], "filesystem");
    let second = run_cort(
        &["read", "src/alpha.ts", "--start", "2", "--end", "2"],
        &cwd,
        &cache,
    );
    let second_p = payload(&second);
    assert_eq!(second_p["source"], "store");
    assert_eq!(second_p["content_mode"], "receipt");
    assert!(second_p.get("content").is_none());
    let recalled = run_cort(&["recall", "alpha"], &cwd, &cache);
    assert_eq!(recalled.code, 0);
    let rec = payload(&recalled);
    assert_eq!(rec["reading_count"], 1);
    assert_eq!(rec["readings"][0]["file_path"], "src/alpha.ts");
}

#[test]
fn unknown_format_is_a_structured_error_and_format_is_case_insensitive() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let bad = run_cort(&["context", "helper", "-f", "yaml"], &cwd, &cache);
    assert_eq!(bad.code, 1);
    let p = payload(&bad);
    assert_eq!(p["error"], "unknown_format");
    let lean = run_cort(&["context", "helper", "-f", "LEAN"], &cwd, &cache);
    assert_eq!(lean.code, 0, "stderr={}", lean.stderr);
    assert!(lean.stdout.starts_with("# context helper"));
}

#[test]
fn cort_cache_dir_is_honoured() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["index"], &cwd, &cache);
    assert_eq!(r.code, 0);
    let entries: Vec<_> = fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(entries.iter().any(|n| n.ends_with(".db")), "{entries:?}");
}

/// Plan §7 B-gap: CLI canonicalizes root before project_id_for.
#[test]
fn cli_canonicalizes_root_before_project_id_for() {
    let tmp = tempfile::Builder::new()
        .prefix("cort-proj-")
        .tempdir()
        .unwrap();
    let real = tmp.path().join("real_root");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("a.ts"), "export function a() { return 1; }\n").unwrap();
    let link = tmp.path().join("link_root");
    symlink(&real, &link).unwrap();
    let cache = tempfile::Builder::new()
        .prefix("cort-cache-")
        .tempdir()
        .unwrap();
    let r = run_cort(&["index", link.to_str().unwrap()], tmp.path(), cache.path());
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let canon = fs::canonicalize(&real).unwrap();
    let expected = format!("{}.db", project_id_for(canon.to_str().unwrap()));
    let names: Vec<_> = fs::read_dir(cache.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&expected),
        "names={names:?} expected={expected}"
    );
    let hashed_link = format!("{}.db", project_id_for(link.to_str().unwrap()));
    assert_ne!(expected, hashed_link);
}

#[test]
fn missing_query_and_missing_symbol_are_structured_errors_not_panics() {
    let (_p, cwd, _c, cache) = sandbox();
    let ctx = run_cort(&["context"], &cwd, &cache);
    assert_eq!(ctx.code, 1);
    assert_eq!(payload(&ctx)["error"], "missing_query");
    let impact = run_cort(&["impact"], &cwd, &cache);
    assert_eq!(impact.code, 1);
    assert_eq!(payload(&impact)["error"], "missing_symbol");
    let strukt = run_cort(&["struct"], &cwd, &cache);
    assert_eq!(strukt.code, 1);
    assert_eq!(payload(&strukt)["error"], "missing_pattern");
}

#[test]
fn type_method_end_to_end_cli_json_and_lean() {
    let body = [
        "struct Ledger;",
        "impl Ledger {",
        "    fn run(&self) {}",
        "}",
        "",
    ]
    .join("\n");
    let (proj, cwd) = make_project(&[("src/lib.rs", body.as_str())]);
    let cache = tempfile::Builder::new()
        .prefix("cort-cache-")
        .tempdir()
        .unwrap();
    assert_eq!(run_cort(&["index"], &cwd, cache.path()).code, 0);
    let json_r = run_cort(
        &["context", "Ledger::run", "--content", "full", "-f", "json"],
        &cwd,
        cache.path(),
    );
    assert_eq!(json_r.code, 0, "stderr={}", json_r.stderr);
    let p = payload(&json_r);
    assert_eq!(p["resolution"], "exact_symbol");
    assert_eq!(p["seeds"][0]["symbol_name"], "Ledger::run");
    assert_eq!(p["seeds"][0]["chunk_type"], "method");
    let lean = run_cort(
        &["context", "Ledger::run", "--content", "full", "-f", "lean"],
        &cwd,
        cache.path(),
    );
    assert!(
        lean.stdout.contains("Ledger::run\tmethod"),
        "{}",
        lean.stdout
    );
    let none = run_cort(
        &["context", "Ledger::nope", "-f", "json"],
        &cwd,
        cache.path(),
    );
    assert_eq!(payload(&none)["resolution"], "none");
    let _ = proj;
}
