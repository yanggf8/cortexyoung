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

/// The coverage screen reached only through its `attach()` unit tests until now -- the audit noted
/// no test drove the flag through the CLI itself. This is that test: `--coverage` on the binary must
/// attach the screen, and lean must print the method line and the seed row the skill tells agents
/// to read.
#[test]
fn coverage_flows_through_the_cli_in_lean() {
    let body = [
        "fn helper(x: u8) -> u8 { x }",
        "fn caller() -> u8 { helper(1) }",
        "",
    ]
    .join("\n");
    let (proj, cwd) = make_project(&[("src/util.rs", &body)]);
    let cache_dir = tempfile::Builder::new()
        .prefix("cort-cov-cli-")
        .tempdir()
        .unwrap();
    let cache = cache_dir.path().to_path_buf();
    let idx = run_cort(&["index", "."], &cwd, &cache);
    assert_eq!(idx.code, 0, "{}", idx.stdout);
    let r = run_cort(
        &[
            "impact",
            "--symbol",
            "helper",
            "--depth",
            "1",
            "--coverage",
            "-f",
            "lean",
        ],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let out = r.stdout;
    assert!(
        out.contains("# coverage coverage-v2"),
        "the screen must attach through the CLI: {out}"
    );
    assert!(out.contains("seed\thelper\tmentions="), "{out}");
    assert!(
        !out.contains("truncated"),
        "an uncut gap list must not announce a cut: {out}"
    );
    let _ = proj;
}

// ── the hook's index gate ──────────────────────────────────────────
// Regression for a live failure: the gate was `the db file exists`, and opening a project creates
// the schema, so an unindexed tree with a 0-chunk db told the agent "cort has an index for this
// project". `impact` there can only answer `no_seed_resolved / stale=true` -- a turn spent to learn
// nothing, which is what makes the next suggestion ignorable.

fn run_hook_suggest(command: &str, cwd: &Path, cache: &Path) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(cort_bin())
        .arg("hook-suggest")
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-suggest");
    let body = serde_json::to_vec(&serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
    }))
    .unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().expect("wait cort hook-suggest");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A call-site search that the rule fires on, so the only variable under test is the gate.
const FIRING_SEARCH: &str = "grep -rn 'helper(' src --include=*.ts";

#[test]
fn the_hook_stays_quiet_when_the_db_exists_but_holds_no_index() {
    let (_p, cwd, _c, cache) = sandbox();
    // Any command that opens the project writes the schema without indexing anything.
    run_cort(&["impact", "--symbol", "helper"], &cwd, &cache);
    let db = cort::db::db_path_for(cwd.to_str().unwrap());
    let db = cache.join(db.file_name().unwrap());
    assert!(db.exists(), "the precondition is a db file that exists");

    let r = run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    assert_eq!(r.code, 0);
    assert_eq!(
        payload(&r),
        serde_json::json!({}),
        "an empty index is not an index: {} {}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn the_hook_speaks_once_the_project_is_actually_indexed() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let r = run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    assert_eq!(r.code, 0);
    let ctx = payload(&r)["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(ctx.contains("cort impact --symbol 'helper'"), "got: {ctx}");
}

/// The usage row has to say what the hook *did*, not merely that it ran.
///
/// Every PreToolUse:Bash fires `hook-suggest`, so a row per invocation counts Bash calls and
/// nothing else. That made the injection count a single-source number -- the transcript -- while
/// the 09-01 report described it as two sources agreeing. The outcome is what makes `usage.db` an
/// independent second reading, and `no_index` is the one silence worth its own name: the rule
/// matched a real call-site search and the gate declined it, which is a missed opportunity rather
/// than a correct pass.
#[test]
fn the_usage_row_records_which_outcome_the_hook_reached() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage_db = cache.join("usage.db");

    // No index yet: the rule matches, the gate declines.
    run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    let counts = cort::usage::hook_outcomes_at(&usage_db, 0).expect("read usage db");
    assert_eq!(counts.get("no_index").and_then(Value::as_i64), Some(1));

    // A command the rule has nothing to say about is a different silence.
    run_hook_suggest("cargo test --workspace", &cwd, &cache);
    let counts = cort::usage::hook_outcomes_at(&usage_db, 0).expect("read usage db");
    assert_eq!(counts.get("no_shape").and_then(Value::as_i64), Some(1));
    assert_eq!(counts.get("no_index").and_then(Value::as_i64), Some(1));

    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    let counts = cort::usage::hook_outcomes_at(&usage_db, 0).expect("read usage db");
    assert_eq!(
        counts.get("hit").and_then(Value::as_i64),
        Some(1),
        "an injection must be countable from the db alone: {counts:?}"
    );
    // The silences did not move: an injection is not also a pass.
    assert_eq!(counts.get("no_index").and_then(Value::as_i64), Some(1));

    // The window is honoured, so a mining run cannot pick up rows from before the hook was wired.
    let future = cort::usage::now_ms() + 60_000;
    let later = cort::usage::hook_outcomes_at(&usage_db, future).expect("read usage db");
    assert!(later.is_empty(), "nothing was recorded after now: {later:?}");
}

fn git_in(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A stale index does not silence the hook, but it does change what the hook is allowed to say.
///
/// The gate was `indexed: true` and nothing else, so on a tree whose index was built two commits
/// ago the injected line still read "cort has an index for this project" -- the half of the
/// sentence that flatters the tool. Every `impact` row recorded on this machine up to 2026-09-02
/// carried `index_stale=1`, so this was the normal case rather than the edge one.
#[test]
fn a_stale_index_is_disclosed_in_the_line_the_agent_reads() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in(&cwd, &["init", "-q"]);
    git_in(&cwd, &["config", "user.email", "t@e.com"]);
    git_in(&cwd, &["config", "user.name", "t"]);
    git_in(&cwd, &["add", "-A"]);
    git_in(&cwd, &["commit", "-qm", "one"]);

    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let fresh = run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    let ctx = payload(&fresh)["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(ctx.contains("cort impact --symbol 'helper'"), "got: {ctx}");
    assert!(
        !ctx.contains("older commit"),
        "a fresh index must not warn about staleness: {ctx}"
    );

    // Move the tree on without re-indexing: the index is now provably behind.
    fs::write(cwd.join("src/gamma.ts"), "export function gamma() { return 3; }\n").unwrap();
    git_in(&cwd, &["add", "-A"]);
    git_in(&cwd, &["commit", "-qm", "two"]);

    let stale = run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    let ctx = payload(&stale)["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        ctx.contains("older commit") && ctx.contains("stale=true"),
        "a behind-head index must say so: {ctx}"
    );
    // Still a suggestion, not a refusal: a stale index resolves most seeds.
    assert!(ctx.contains("cort impact --symbol 'helper'"), "got: {ctx}");

    // And the two are countable apart from the db alone.
    let counts = cort::usage::hook_outcomes_at(&cache.join("usage.db"), 0).expect("read usage db");
    assert_eq!(counts.get("hit").and_then(Value::as_i64), Some(1), "{counts:?}");
    assert_eq!(counts.get("hit_stale").and_then(Value::as_i64), Some(1), "{counts:?}");
}
