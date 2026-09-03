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
    run_hook_suggest_with(command, &[], cwd, cache)
}

fn run_hook_suggest_with(command: &str, extra: &[&str], cwd: &Path, cache: &Path) -> Run {
    run_hook_suggest_full(command, None, extra, cwd, cache)
}

fn run_hook_suggest_full(
    command: &str,
    transcript_path: Option<&str>,
    extra: &[&str],
    cwd: &Path,
    cache: &Path,
) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(cort_bin())
        .arg("hook-suggest")
        .args(extra)
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-suggest");
    let mut payload = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
    });
    if let Some(t) = transcript_path {
        payload["transcript_path"] = serde_json::json!(t);
    }
    let body = serde_json::to_vec(&payload).unwrap();
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
    let counts = cort::usage::hook_outcomes_at(&usage_db, 0, None).expect("read usage db");
    assert_eq!(counts.get("no_index").and_then(Value::as_i64), Some(1));

    // A command the rule has nothing to say about is a different silence.
    run_hook_suggest("cargo test --workspace", &cwd, &cache);
    let counts = cort::usage::hook_outcomes_at(&usage_db, 0, None).expect("read usage db");
    assert_eq!(counts.get("no_shape").and_then(Value::as_i64), Some(1));
    assert_eq!(counts.get("no_index").and_then(Value::as_i64), Some(1));

    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    let counts = cort::usage::hook_outcomes_at(&usage_db, 0, None).expect("read usage db");
    assert_eq!(
        counts.get("hit").and_then(Value::as_i64),
        Some(1),
        "an injection must be countable from the db alone: {counts:?}"
    );
    // The silences did not move: an injection is not also a pass.
    assert_eq!(counts.get("no_index").and_then(Value::as_i64), Some(1));

    // The window is honoured, so a mining run cannot pick up rows from before the hook was wired.
    let future = cort::usage::now_ms() + 60_000;
    let later = cort::usage::hook_outcomes_at(&usage_db, future, None).expect("read usage db");
    assert!(
        later.is_empty(),
        "nothing was recorded after now: {later:?}"
    );
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
    fs::write(
        cwd.join("src/gamma.ts"),
        "export function gamma() { return 3; }\n",
    )
    .unwrap();
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
    let counts =
        cort::usage::hook_outcomes_at(&cache.join("usage.db"), 0, None).expect("read usage db");
    assert_eq!(
        counts.get("hit").and_then(Value::as_i64),
        Some(1),
        "{counts:?}"
    );
    assert_eq!(
        counts.get("hit_stale").and_then(Value::as_i64),
        Some(1),
        "{counts:?}"
    );
}

/// An index built before the current schema is not a busy database. `cort status` opens read-only
/// and so cannot migrate on the way past, which made the one command whose job is to audit indexes
/// the only one that failed on an old one -- reporting `storage_busy`, which says "retry and it
/// will clear", about a condition that never clears.
#[test]
fn an_index_predating_the_schema_is_reported_as_outdated_not_busy() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    // Reproduce the observed shape: a database from before `reading_notes` existed. The file is
    // located by scanning the sandbox cache -- `db_path_for` reads CORT_CACHE_DIR from *this*
    // process, which does not have it, and would silently create a stray database in the real one.
    let db_file = fs::read_dir(&cache)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "db") && p.file_name().unwrap() != "usage.db")
        .expect("index wrote a database into the sandbox cache");
    let db = rusqlite::Connection::open(&db_file).unwrap();
    db.execute_batch("DROP TABLE reading_notes;").unwrap();
    drop(db);

    let r = run_cort(&["status"], &cwd, &cache);
    let code = payload(&r)["error"].as_str().unwrap_or("").to_string();
    assert_eq!(code, "schema_outdated", "stdout={}", r.stdout);
    assert!(
        payload(&r)["detail"]["hint"]
            .as_str()
            .unwrap_or("")
            .contains("cort index"),
        "the error has to name the fix: {}",
        r.stdout
    );
}

/// The row most worth deleting is the one whose directory is gone, and that is exactly the row
/// whose path cannot be canonicalised into the database name. Two of these were left in the cache
/// by the install smoke test and `cort delete` refused both.
#[test]
fn a_project_whose_directory_is_gone_can_still_be_deleted() {
    let (proj, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let gone = fs::canonicalize(&cwd).unwrap();
    let elsewhere = tempfile::Builder::new()
        .prefix("cort-cwd-")
        .tempdir()
        .unwrap();
    drop(proj); // the project directory no longer exists

    let r = run_cort(
        &["delete", gone.to_str().unwrap()],
        elsewhere.path(),
        &cache,
    );
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert_eq!(payload(&r)["deleted"], true, "stdout={}", r.stdout);

    let after = run_cort(&["projects"], elsewhere.path(), &cache);
    assert_eq!(
        payload(&after).as_array().map(Vec::len),
        Some(0),
        "the row survived the delete: {}",
        after.stdout
    );
}

/// A path that names no row and no directory is still an error -- the fallback must not turn a
/// typo into a silent success.
#[test]
fn deleting_a_path_that_is_neither_a_directory_nor_a_row_still_fails() {
    let (_p, _cwd, _c, cache) = sandbox();
    let elsewhere = tempfile::Builder::new()
        .prefix("cort-cwd-")
        .tempdir()
        .unwrap();
    let r = run_cort(
        &["delete", "/nonexistent/never/indexed"],
        elsewhere.path(),
        &cache,
    );
    assert_ne!(r.code, 0, "stdout={}", r.stdout);
    assert_eq!(
        payload(&r)["error"],
        "file_not_found",
        "stdout={}",
        r.stdout
    );
}

/// Every harness that wires this hook calls one binary and writes to one usage.db, and the mining
/// compares those rows against a single harness's transcripts. A row that cannot say which harness
/// wrote it cannot be counted on either side, and one from a different harness must not be folded
/// in -- it would raise the injection count while every guard still read green.
#[test]
fn a_usage_row_records_which_harness_fired_the_hook() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let usage_db = cache.join("usage.db");
    run_hook_suggest_with(FIRING_SEARCH, &["--harness", "claude-code"], &cwd, &cache);
    let mine = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    assert!(
        mine.get("hit").and_then(|v| v.as_i64()).unwrap_or(0)
            + mine.get("hit_stale").and_then(|v| v.as_i64()).unwrap_or(0)
            >= 1,
        "the claude-code fire was not counted: {mine:?}"
    );

    // A fire from somewhere else is visible, but never as this harness's injection.
    run_hook_suggest_with(FIRING_SEARCH, &["--harness", "grok"], &cwd, &cache);
    let mine = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    assert_eq!(
        mine.get("other_harness").and_then(|v| v.as_i64()),
        Some(1),
        "a grok fire was not held apart: {mine:?}"
    );

    // And a row from before the field existed is `unspecified`, not attributed to whichever
    // harness happened to be wired first.
    run_hook_suggest_with(FIRING_SEARCH, &[], &cwd, &cache);
    let mine = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    assert_eq!(
        mine.get("unspecified").and_then(|v| v.as_i64()),
        Some(1),
        "a harness-less row was attributed anyway: {mine:?}"
    );

    // With no filter the outcomes are reported as they are, so `cort usage` is unaffected.
    let all = cort::usage::hook_outcomes_at(&usage_db, 0, None).unwrap();
    assert!(
        all.get("other_harness").is_none(),
        "unfiltered read split by harness: {all:?}"
    );
}

/// One settings file can be read by more than one harness: Grok loads `~/.claude/settings.json` for
/// Claude Code compatibility, so the entry the installer wired there fires inside Grok carrying
/// `--harness claude-code`. Measured on 2026-09-02: every such fire would have been counted as a
/// Claude Code injection with no Claude transcript to match it. `transcript_path` is the harness
/// naming its own session file, and it settles the question without guessing at the environment.
#[test]
fn the_harness_is_taken_from_the_payload_not_from_the_flag_alone() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let usage_db = cache.join("usage.db");

    // Wired as claude-code, but the transcript says the process running it is Grok.
    run_hook_suggest_full(
        FIRING_SEARCH,
        Some("/home/u/.grok/sessions/x/updates.jsonl"),
        &["--harness", "claude-code"],
        &cwd,
        &cache,
    );
    let as_claude = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    assert_eq!(
        as_claude.get("other_harness").and_then(|v| v.as_i64()),
        Some(1),
        "a Grok fire was credited to claude-code: {as_claude:?}"
    );
    let as_grok = cort::usage::hook_outcomes_at(&usage_db, 0, Some("grok")).unwrap();
    assert_eq!(
        as_grok.get("hit").and_then(|v| v.as_i64()).unwrap_or(0)
            + as_grok
                .get("hit_stale")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        1,
        "the fire was not attributed to grok: {as_grok:?}"
    );

    // A transcript path naming nothing we recognise leaves the declared value standing: a declared
    // harness is still better than none.
    run_hook_suggest_full(
        FIRING_SEARCH,
        Some("/var/tmp/somewhere/else.jsonl"),
        &["--harness", "claude-code"],
        &cwd,
        &cache,
    );
    let as_claude = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    assert_eq!(
        as_claude.get("hit").and_then(|v| v.as_i64()).unwrap_or(0)
            + as_claude
                .get("hit_stale")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        1,
        "an unrecognised transcript path discarded the declared harness: {as_claude:?}"
    );
}

/// Codex 0.152.1 rejects the entire hook output when `suppressOutput` is present -- it reports the
/// hook `Failed` and the context never reaches the model -- despite listing the field in its own
/// embedded `pre-tool-use.command.output` schema. Bisected by emitting the shapes one at a time;
/// `hookSpecificOutput` with `additionalContext` alone both completes and delivers. The field is
/// dropped only for the harness that breaks on it, because it is what keeps the raw JSON out of the
/// transcript view everywhere else.
#[test]
fn suppress_output_is_omitted_for_codex_and_kept_for_the_others() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let for_codex = run_hook_suggest_full(
        FIRING_SEARCH,
        Some("/home/u/.codex/sessions/2026/09/02/rollout-x.jsonl"),
        &[],
        &cwd,
        &cache,
    );
    let v = payload(&for_codex);
    assert!(
        v["hookSpecificOutput"]["additionalContext"].is_string(),
        "codex still needs the context: {}",
        for_codex.stdout
    );
    assert!(
        v.get("suppressOutput").is_none(),
        "suppressOutput would make codex discard the whole output: {}",
        for_codex.stdout
    );

    let for_claude = run_hook_suggest_full(
        FIRING_SEARCH,
        Some("/home/u/.claude/projects/x/y.jsonl"),
        &[],
        &cwd,
        &cache,
    );
    assert_eq!(
        payload(&for_claude)["suppressOutput"],
        serde_json::json!(true),
        "the others still suppress the raw JSON: {}",
        for_claude.stdout
    );
}

// --- Kimi: a structured search surface, and a contract that has to differ ----------------------

/// Send a raw payload rather than a shell command, so a structured tool call can be tested the way
/// the harness actually sends one.
fn run_hook_suggest_payload(
    payload: serde_json::Value,
    extra: &[&str],
    cwd: &Path,
    cache: &Path,
) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(cort_bin())
        .arg("hook-suggest")
        .args(extra)
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-suggest");
    let body = serde_json::to_vec(&payload).unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().expect("wait cort hook-suggest");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn kimi_grep(pattern: &str, path: &str, session: &str) -> serde_json::Value {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "session_id": session,
        "tool_name": "Grep",
        "tool_input": { "pattern": pattern, "path": path, "output_mode": "content", "-n": true },
    })
}

/// Kimi's search surface is its structured `Grep` tool, not the shell. A hook that only reads
/// `tool_input.command` is silent on the majority of that harness's traffic.
#[test]
fn a_structured_grep_payload_is_read_the_same_as_a_shell_search() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let r = run_hook_suggest_payload(
        kimi_grep("helper", "rust/src", "s1"),
        &["--harness", "claude-code"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0);
    let ctx = payload(&r)["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(ctx.contains("cort impact --symbol 'helper'"), "got: {ctx}");
}

/// On Kimi and only on Kimi the suggestion has to arrive as a deny, because its `PreToolUse` keeps
/// only blocking results and discards the rest before the model sees them. It fires once per symbol
/// per session and yields afterwards, so a false positive costs one turn rather than the search.
#[test]
fn kimi_denies_once_per_symbol_then_gets_out_of_the_way() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let kimi = &["--harness", "kimi-code"];

    let first = run_hook_suggest_payload(
        kimi_grep("helper", "rust/src", "sess-a"),
        kimi,
        &cwd,
        &cache,
    );
    let out = payload(&first)["hookSpecificOutput"].clone();
    assert_eq!(out["permissionDecision"].as_str(), Some("deny"));
    let reason = out["permissionDecisionReason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("cort impact --symbol 'helper'"),
        "got: {reason}"
    );
    assert!(
        reason.contains("Issue exactly the same search again"),
        "a stop the agent cannot get past is worse than any false positive: {reason}"
    );
    assert!(
        out["additionalContext"].is_null(),
        "the allow-shaped field is discarded by Kimi and must not be sent there"
    );

    // Same symbol, same session: yields.
    let second = run_hook_suggest_payload(
        kimi_grep("helper", "rust/src", "sess-a"),
        kimi,
        &cwd,
        &cache,
    );
    assert_eq!(payload(&second), serde_json::json!({}));

    // A different session has not been told anything yet.
    let other = run_hook_suggest_payload(
        kimi_grep("helper", "rust/src", "sess-b"),
        kimi,
        &cwd,
        &cache,
    );
    assert_eq!(
        payload(&other)["hookSpecificOutput"]["permissionDecision"].as_str(),
        Some("deny")
    );
}

/// The other three harnesses keep the contract the rule was calibrated under: never block.
#[test]
fn no_other_harness_ever_receives_a_deny() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    for harness in ["claude-code", "codex", "grok"] {
        let r = run_hook_suggest_with(FIRING_SEARCH, &["--harness", harness], &cwd, &cache);
        let out = payload(&r)["hookSpecificOutput"].clone();
        assert!(
            out["permissionDecision"].is_null(),
            "{harness} must never be denied"
        );
        assert!(out["additionalContext"].is_string(), "{harness} got: {out}");
    }
}

/// A disk that will not cooperate is an error, never a panic — and never a noisy hook.
///
/// `ensure_schema` and `open_db` used to `expect` on four storage failures, one of them carrying a
/// message inherited from the JavaScript version this crate replaced. On 2026-09-03 a macOS CI
/// runner returned `SQLITE_IOERR_FSYNC` for a few seconds and eight tests died reporting a panic
/// instead of a disk problem. The cost grew when `hook-refresh` arrived: it reaches both functions
/// and promises to be silent and exit 0 whatever happens, which a panic would break on every edit
/// for as long as the disk misbehaved.
#[test]
#[cfg(unix)]
fn a_cache_directory_that_cannot_be_created_is_an_error_not_a_panic() {
    use std::os::unix::fs::PermissionsExt;

    let d = tempfile::Builder::new()
        .prefix("cort-ro-cache-")
        .tempdir()
        .unwrap();
    let readonly = d.path().join("readonly");
    std::fs::create_dir(&readonly).unwrap();
    let cache = readonly.join("nested");
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o500)).unwrap();

    let r = run_cort(&["index"], d.path(), &cache);
    // Restore before asserting, so a failure cannot leave an undeletable tempdir behind.
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o700)).unwrap();

    let v = payload(&r);
    assert_eq!(
        v["error"].as_str(),
        Some("storage_busy"),
        "expected a structured error, got: {}",
        r.stdout
    );
    assert!(
        v["detail"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("could not create the cache directory"),
        "the message has to name what failed: {}",
        r.stdout
    );
    assert!(
        !r.stderr.contains("panicked"),
        "storage failures must not panic: {}",
        r.stderr
    );
}

/// The same disk, through the hook that must never say anything.
#[test]
#[cfg(unix)]
fn hook_refresh_stays_silent_when_the_cache_is_unwritable() {
    use std::os::unix::fs::PermissionsExt;

    let d = tempfile::Builder::new()
        .prefix("cort-ro-hook-")
        .tempdir()
        .unwrap();
    let readonly = d.path().join("readonly");
    std::fs::create_dir(&readonly).unwrap();
    let cache = readonly.join("nested");
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o500)).unwrap();

    let r = run_hook_refresh(d.path(), &cache);
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    assert!(!r.stderr.contains("panicked"), "stderr={}", r.stderr);
}

/// `hook-refresh` with an edit payload on stdin, which is all the harness ever sends it.
fn run_hook_refresh(cwd: &Path, cache: &Path) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(cort_bin())
        .arg("hook-refresh")
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-refresh");
    let body = serde_json::to_vec(
        &serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": "x.rs" } }),
    )
    .unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().expect("wait cort hook-refresh");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

// ── hook-install: which file a format defaults to ──────────────────────────────

/// `hook-install` with the three harness homes pointed at a sandbox, so a test never reads or
/// writes the developer's own `~/.claude`, `~/.codex` or `~/.kimi-code`.
fn run_hook_install(args: &[&str], home: &Path) -> Run {
    let out = Command::new(cort_bin())
        .arg("hook-install")
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("CLAUDE_SKILL_HOME", home.join(".claude"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("KIMI_CODE_HOME", home.join(".kimi-code"))
        .output()
        .expect("spawn cort hook-install");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Without `--settings`, the file follows `--format` -- and nothing else.
///
/// The path used to be resolved before the format was read, so every `--settings`-less call landed
/// on Claude Code's `settings.json` and was then parsed as whichever dialect `--format` named.
/// `--status --format kimi` therefore answered `wired: false` on a machine where the Kimi entry was
/// wired and firing: the same false negative as `docs/2026-09-02-hook-wiring-correction.md`, by a
/// different route. `install.sh` always passes `--settings`, which is why nothing caught it.
#[test]
fn each_format_resolves_its_own_settings_file_when_settings_is_not_given() {
    let home = tempfile::Builder::new()
        .prefix("cort-hook-home-")
        .tempdir()
        .unwrap();
    let h = home.path();
    let cases = [
        (vec![], ".claude/settings.json"),
        (vec!["--format", "codex"], ".codex/config.toml"),
        (vec!["--format", "kimi"], ".kimi-code/config.toml"),
    ];

    for (fmt, rel) in &cases {
        let expected = h.join(rel);
        let mut install = fmt.clone();
        install.extend_from_slice(&["--command", "/bin/cort hook-suggest"]);
        let r = run_hook_install(&install, h);
        assert_eq!(r.code, 0, "install {fmt:?}: {}", r.stderr);
        assert_eq!(
            payload(&r)["settings"].as_str(),
            Some(expected.to_string_lossy().as_ref()),
            "install {fmt:?} wrote the wrong file"
        );
        assert!(expected.exists(), "install {fmt:?} created no {rel}");

        let mut status = fmt.clone();
        status.push("--status");
        let s = run_hook_install(&status, h);
        assert_eq!(s.code, 0, "status {fmt:?}: {}", s.stderr);
        let p = payload(&s);
        assert_eq!(
            p["settings"].as_str(),
            Some(expected.to_string_lossy().as_ref()),
            "status {fmt:?} read the wrong file"
        );
        assert_eq!(
            p["wired"].as_bool(),
            Some(true),
            "status {fmt:?} could not see the entry it had just written"
        );
    }

    // Each install landed in its own file rather than three times in one: the bug's other half.
    for (_, rel) in &cases {
        let body = fs::read_to_string(h.join(rel)).unwrap();
        assert_eq!(
            body.matches("hook-suggest").count(),
            1,
            "{rel} holds more than one entry"
        );
    }
}

/// `--settings` still wins, and with it the extension still picks the dialect for the two files
/// that are not Kimi's -- the rule `install.sh` has always relied on.
#[test]
fn an_explicit_settings_path_still_overrides_the_format_default() {
    let home = tempfile::Builder::new()
        .prefix("cort-hook-home-")
        .tempdir()
        .unwrap();
    let h = home.path();
    let elsewhere = h.join("elsewhere/config.toml");

    let r = run_hook_install(
        &[
            "--format",
            "kimi",
            "--settings",
            elsewhere.to_str().unwrap(),
            "--command",
            "/bin/cort hook-suggest",
        ],
        h,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(elsewhere.exists(), "explicit path was not written");
    assert!(
        !h.join(".kimi-code/config.toml").exists(),
        "the format's default file was written despite an explicit --settings"
    );
}

/// `hook-refresh` with a `--harness` on its command line, which is what the installer wires.
fn run_hook_refresh_with(
    extra: &[&str],
    payload: serde_json::Value,
    cwd: &Path,
    cache: &Path,
) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(cort_bin())
        .arg("hook-refresh")
        .args(extra)
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-refresh");
    let body = serde_json::to_vec(&payload).unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().expect("wait cort hook-refresh");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn edit_payload(transcript: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_input": { "file_path": "src/helper.ts" },
    });
    if let Some(t) = transcript {
        v["transcript_path"] = serde_json::json!(t);
    }
    v
}

/// A post-event row has to say which harness wrote it, for the same reason the pre-event one does.
///
/// Six entries ship -- three harnesses times two events -- and they all call this one binary and
/// write to one database. `hook-refresh` was discarding its whole argument list (`_args`) and
/// writing a bare `outcome=…` summary, so every reindex was an anonymous, non-JSON row: with three
/// harnesses wired, "is Kimi's post-hook firing at all?" had no answer in the data. That is
/// `f3cb567f`'s defect surviving on the event it was never carried across to.
#[test]
fn a_refresh_row_says_which_harness_wrote_it_and_is_never_summed_with_the_suggest_rows() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let usage_db = cache.join("usage.db");
    let refresh = |h: Option<&str>| match h {
        Some(h) => cort::usage::outcomes_of_hook_at(&usage_db, "hook-refresh", 0, Some(h)).unwrap(),
        None => cort::usage::outcomes_of_hook_at(&usage_db, "hook-refresh", 0, None).unwrap(),
    };

    let r = run_hook_refresh_with(
        &["--harness", "kimi-code"],
        edit_payload(None),
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "the refresh hook must exit 0 whatever happens");
    let mine = refresh(Some("kimi-code"));
    let attributed: i64 = mine.values().filter_map(Value::as_i64).sum::<i64>()
        - mine.get("unspecified").and_then(Value::as_i64).unwrap_or(0)
        - mine
            .get("other_harness")
            .and_then(Value::as_i64)
            .unwrap_or(0);
    assert!(
        attributed >= 1,
        "the kimi refresh was not attributed: {mine:?}"
    );

    // Another harness's fire is visible but held apart, never folded into kimi's.
    run_hook_refresh_with(&["--harness", "codex"], edit_payload(None), &cwd, &cache);
    assert_eq!(
        refresh(Some("kimi-code"))
            .get("other_harness")
            .and_then(Value::as_i64),
        Some(1),
        "a codex refresh was not held apart from kimi's"
    );

    // No `--harness` at all is `unspecified`, not whichever harness was wired first.
    run_hook_refresh_with(&[], edit_payload(None), &cwd, &cache);
    assert_eq!(
        refresh(Some("kimi-code"))
            .get("unspecified")
            .and_then(Value::as_i64),
        Some(1),
        "a harness-less refresh row was attributed anyway"
    );

    // The transcript outranks the flag here too: Grok runs the entry installed as `claude-code`.
    run_hook_refresh_with(
        &["--harness", "claude-code"],
        edit_payload(Some("/home/u/.grok/sessions/s.jsonl")),
        &cwd,
        &cache,
    );
    assert_eq!(
        refresh(Some("grok"))
            .values()
            .filter_map(Value::as_i64)
            .sum::<i64>()
            - refresh(Some("grok"))
                .get("other_harness")
                .and_then(Value::as_i64)
                .unwrap_or(0)
            - refresh(Some("grok"))
                .get("unspecified")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        1,
        "a grok refresh was recorded as claude-code"
    );

    // The two events are never summed: the suggest funnel saw none of these four rows.
    let suggest = cort::usage::hook_outcomes_at(&usage_db, 0, None).unwrap();
    assert!(
        suggest.values().filter_map(Value::as_i64).sum::<i64>() == 0,
        "refresh rows leaked into the suggestion funnel: {suggest:?}"
    );
    assert_eq!(
        refresh(None)
            .values()
            .filter_map(Value::as_i64)
            .sum::<i64>(),
        4,
        "the four refresh fires are not all readable: {:?}",
        refresh(None)
    );
}

/// The row says which model answered, not just which harness ran.
///
/// A router launches the real Claude Code against another vendor's endpoint: same binary, same
/// `settings.json`, same `~/.claude/projects`, so `harness_of` correctly answers `claude-code` and
/// nothing downstream can tell the two apart. On this machine that is not hypothetical -- the local
/// Claude Code corpus held ~2,167 assistant messages from `glm-5.*`, `stealth/ox-alpha`,
/// `muse-spark-1.2-contributor`, `deepseek-v4-flash`, `k3` and `qwen3.5:4b` against ~5,072
/// Anthropic ones on 2026-09-03. Every behavioural number this repo quotes is a claim about the
/// model, so a corpus that cannot name it cannot support the claim. The payload has always carried
/// `model`; nothing read it.
#[test]
fn a_hook_row_names_the_model_that_answered_and_never_invents_one() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let usage_db = cache.join("usage.db");
    // A router session: the harness is genuinely claude-code, the model is not Anthropic's.
    let mut routed = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "session_id": "routed",
        "transcript_path": "/home/u/.claude/projects/p/s.jsonl",
        "tool_name": "Bash",
        "tool_input": { "command": FIRING_SEARCH },
    });
    routed["model"] = serde_json::json!("glm-5.3");
    let r = run_hook_suggest_payload(routed, &["--harness", "claude-code"], &cwd, &cache);
    assert_eq!(r.code, 0);

    let rows = read_hook_rows(&usage_db, "hook-suggest");
    let last = rows.last().expect("a row was written");
    assert_eq!(
        last.get("harness").and_then(Value::as_str),
        Some("claude-code"),
        "the harness really is claude-code -- that was never the ambiguous part"
    );
    assert_eq!(
        last.get("model").and_then(Value::as_str),
        Some("glm-5.3"),
        "the model the payload named was dropped: {last}"
    );
    assert_eq!(last.get("v").and_then(Value::as_i64), Some(3));

    // A payload with no model gets no model. Absence is visible; a wrong name would not be.
    let bare = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "session_id": "bare",
        "tool_name": "Bash",
        "tool_input": { "command": FIRING_SEARCH },
    });
    run_hook_suggest_payload(bare, &["--harness", "claude-code"], &cwd, &cache);
    let rows = read_hook_rows(&usage_db, "hook-suggest");
    let last = rows.last().expect("a second row was written");
    assert!(
        last.get("model").is_none(),
        "a model was invented for a payload that named none: {last}"
    );
}

/// Every `args_summary` this command wrote, parsed, oldest first.
fn read_hook_rows(usage_db: &Path, command: &str) -> Vec<Value> {
    let conn =
        rusqlite::Connection::open_with_flags(usage_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open usage db");
    let mut stmt = conn
        .prepare("SELECT args_summary FROM command_log WHERE command = ?1 ORDER BY id")
        .expect("prepare");
    let rows: Vec<String> = stmt
        .query_map([command], |r| r.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    rows.iter()
        .filter_map(|r| serde_json::from_str::<Value>(r).ok())
        .collect()
}

/// The model breakdown is a second lens, never a split of the first.
///
/// "How often did the hook intercept on Claude Code" is a real question with one answer, and it
/// does not stop being one because a router put several models behind that harness. So the harness
/// total is computed without reference to `model` and must be identical whether the rows carry one
/// model, five, or none -- the tempting mistake is to make every figure conditional on a dimension
/// only some rows have, which would leave the primary number unquotable the moment a v1 row appears.
#[test]
fn a_model_breakdown_never_splits_the_harness_total() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let usage_db = cache.join("usage.db");
    let fire = |model: Option<&str>| {
        let mut p = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s",
            "transcript_path": "/home/u/.claude/projects/p/s.jsonl",
            "tool_name": "Bash",
            "tool_input": { "command": FIRING_SEARCH },
        });
        if let Some(m) = model {
            p["model"] = serde_json::json!(m);
        }
        run_hook_suggest_payload(p, &["--harness", "claude-code"], &cwd, &cache);
    };

    fire(Some("claude-opus-5"));
    fire(Some("glm-5.3"));
    fire(Some("glm-5.3"));
    fire(None); // the harness named no model
    let total_before = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    let fired: i64 = total_before.values().filter_map(Value::as_i64).sum();
    assert_eq!(
        fired, 4,
        "the harness total must count every fire: {total_before:?}"
    );

    let models =
        cort::usage::hook_models_at(&usage_db, "hook-suggest", 0, Some("claude-code")).unwrap();
    assert_eq!(models.get("claude-opus-5").and_then(Value::as_i64), Some(1));
    assert_eq!(models.get("glm-5.3").and_then(Value::as_i64), Some(2));
    assert_eq!(
        models.get("unreported").and_then(Value::as_i64),
        Some(1),
        "a payload that named no model is `unreported`, not folded into a model that did"
    );

    // The two lenses see the same rows: the breakdown sums to the total, and neither is derived
    // from the other.
    let model_sum: i64 = models.values().filter_map(Value::as_i64).sum();
    assert_eq!(
        model_sum, fired,
        "the breakdown and the total disagree about how many rows exist: {models:?} vs {total_before:?}"
    );

    // Adding the lens changed nothing about the primary number.
    let total_after = cort::usage::hook_outcomes_at(&usage_db, 0, Some("claude-code")).unwrap();
    assert_eq!(total_before, total_after);

    // A different harness's rows are not in this harness's breakdown.
    assert!(
        cort::usage::hook_models_at(&usage_db, "hook-suggest", 0, Some("codex"))
            .unwrap()
            .is_empty(),
        "codex has fired nothing here"
    );
}

/// `--all` speaks one line per entry, and no field is ever empty.
///
/// Tab is an IFS *whitespace* character, so `read` collapses a run of tabs into one delimiter and
/// silently drops the empty field between them: `a\t\tb` arrives as two fields, not three. The
/// first version of this format left `detail` empty when there was no trust to report, so the
/// installer read `command` into `detail` on Claude Code and Kimi and got it right on Codex --
/// wrong on four of six entries and green on the two that happened to carry a value. The rule is
/// therefore "never emit an empty field", not "be careful in bash".
#[test]
fn the_lean_hook_report_has_six_non_empty_fields_on_every_line() {
    let home = tempfile::Builder::new()
        .prefix("cort-hook-lean-")
        .tempdir()
        .unwrap();
    let h = home.path();
    let run = |args: &[&str]| -> Run {
        let out = Command::new(cort_bin())
            .arg("hook-install")
            .args(args)
            .current_dir(h)
            .env("HOME", h)
            .env("CLAUDE_SKILL_HOME", h.join(".claude"))
            .env("CODEX_HOME", h.join(".codex"))
            .env("KIMI_CODE_HOME", h.join(".kimi-code"))
            .output()
            .expect("spawn");
        Run {
            code: out.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };

    let r = run(&["--all", "--lean", "--command-prefix", "/bin/cort"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        6,
        "three harnesses times two events: {lines:?}"
    );
    for l in &lines {
        let f: Vec<&str> = l.split('\t').collect();
        assert_eq!(f.len(), 6, "expected six fields, got {}: {l:?}", f.len());
        for (i, v) in f.iter().enumerate() {
            assert!(
                !v.is_empty(),
                "field {i} is empty, which `read` would drop: {l:?}"
            );
        }
    }

    // The command names what the caller asked for, never this test binary -- the installed layout
    // puts a shim in front of the real executable and the wired command has to name the shim.
    for l in &lines {
        let command = l.split('\t').nth(5).unwrap();
        assert!(
            command.starts_with("/bin/cort "),
            "--command-prefix was not honoured: {command:?}"
        );
        assert!(
            command.contains("--harness "),
            "the entry must carry the harness it was wired for: {command:?}"
        );
    }

    // Round-trips: status sees exactly what install wrote, and each harness kept its own file.
    let s = run(&["--all", "--status", "--lean"]);
    let files: Vec<&str> = s
        .stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').nth(3).unwrap())
        .collect();
    assert_eq!(files.len(), 6);
    for l in s.stdout.lines().filter(|l| !l.is_empty()) {
        assert_eq!(l.split('\t').nth(2), Some("wired"), "not wired: {l}");
    }
    assert_eq!(
        files.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "six entries across three files: {files:?}"
    );

    // And `--all` needs to be told which binary to name; defaulting to this one would wire the
    // executable behind the installer's shim.
    let missing = run(&["--all", "--lean"]);
    assert_ne!(
        missing.code, 0,
        "--all without --command-prefix must refuse"
    );
    assert!(
        missing.stdout.contains("command-prefix") || missing.stderr.contains("command-prefix"),
        "the refusal must name the missing flag: {} {}",
        missing.stdout,
        missing.stderr
    );
}
