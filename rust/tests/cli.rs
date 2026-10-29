//! D-42..D-47 — the Rust port kept the case ids (audit F-12).
//! Plus plan §7 B-gap canonicalize-before-hash, format errors, CORT_CACHE_DIR.

use cort::db::project_id_for;
use serde_json::{json, Value};
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

#[test]
fn memory_record_replace_conflicts_with_supersedes_at_parse_time() {
    let (_p, cwd, _c, cache) = sandbox();
    let run = run_cort(
        &[
            "memory",
            "record",
            "--key",
            "k",
            "--claim",
            "c",
            "--source",
            "x",
            "--line",
            "1",
            "--replace",
            "--supersedes",
            "12345678",
        ],
        &cwd,
        &cache,
    );
    assert_ne!(run.code, 0);
    assert!(format!("{}{}", run.stdout, run.stderr).contains("--replace"));
    assert!(format!("{}{}", run.stdout, run.stderr).contains("--supersedes"));
}

fn run_cort_env(args: &[&str], cwd: &Path, cache: &Path, envs: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(cort_bin());
    cmd.args(args).current_dir(cwd).env("CORT_CACHE_DIR", cache);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn cort");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The newest usage row for a command, with its `args_summary` parsed. The heal fields ride in
/// that summary, so "did this query repair the cache" is readable straight out of the db the
/// same way the census reads it -- not from a transcript.
fn latest_command_row(usage_db: &Path, command: &str) -> Value {
    let conn =
        rusqlite::Connection::open_with_flags(usage_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let summary: String = conn
        .query_row(
            "SELECT args_summary FROM command_log WHERE command = ?1 ORDER BY id DESC LIMIT 1",
            [command],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&summary)
        .unwrap_or_else(|e| panic!("args_summary not json ({e}): {summary}"))
}

#[test]
fn hooks_without_measure_configuration_record_collect_label() {
    use std::io::Write;
    use std::process::Stdio;
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let data = temp.path().join("data");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&data).unwrap();
    for (hook, payload, outcome) in [
        (
            "hook-refresh",
            json!({"tool_name":"Edit","tool_input":{"file_path":"x.rs"}}),
            None,
        ),
        (
            "hook-suggest",
            json!({"tool_name":"Bash","tool_input":{"command":"rg foo"}}),
            None,
        ),
        // Every readable payload is a fire now; the shape gate that used to reject this one is
        // gone, and the thin trigger still kicks collection from the same exit.
        (
            "hook-suggest",
            json!({"tool_name":"Bash","tool_input":{"command":"ls -la"}}),
            Some("fired"),
        ),
    ] {
        let mut child = Command::new(cort_bin())
            .arg(hook)
            .current_dir(temp.path())
            .env("CORT_CACHE_DIR", &cache)
            .env("CORT_MEMORY_DIR", &data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{hook}");
        assert_eq!(output.stdout, b"{}\n", "{hook}");
        assert!(output.stderr.is_empty(), "{hook}");
        let row = latest_command_row(&cache.join("usage.db"), hook);
        assert_eq!(row["collect"], "not_configured", "{hook} {payload}");
        if let Some(outcome) = outcome {
            assert_eq!(row["hook"], outcome, "{hook} {payload}");
        }
    }
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

/// The binary answers `--version` itself. Until 2026-09-10 the installer's shim intercepted it and
/// echoed a string baked in at install time, so `install.sh --check` compared its own constant
/// against its own writing and could not fail for the one reason worth catching -- a payload that
/// is not the version the manifest claims.
///
/// Plain text, exactly `cort <crate version> (rust)`, because `--check` reads `head -1`: a JSON
/// first line of `{` is what a stale shim produced on a live machine, and it read as a version
/// mismatch instead of as the stale shim it was.
#[test]
fn the_binary_answers_for_its_own_version_in_the_shape_check_parses() {
    let (_p, cwd, _c, cache) = sandbox();
    for args in [vec!["--version"], vec!["-V"]] {
        let r = run_cort(&args, &cwd, &cache);
        assert_eq!(r.code, 0, "{} stderr={}", args.join(" "), r.stderr);
        assert_eq!(
            r.stdout,
            format!("cort {} (rust)\n", env!("CARGO_PKG_VERSION")),
            "{} must print the crate version as one plain line",
            args.join(" ")
        );
        // Same promise --help makes: asking a binary its version creates nothing.
        assert_eq!(
            fs::read_dir(&cache).unwrap().count(),
            0,
            "{} must not touch the cache",
            args.join(" ")
        );
    }
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
        vec!["status", "-h"],
        vec!["projects", "--help"],
    ] {
        let r = run_cort(&args, &cwd, &cache);
        assert_eq!(r.code, 0, "{} should exit 0", args.join(" "));
        let p = payload(&r);
        assert_eq!(p["usage"], "cort <command> [options]");
        // Removed 2026-10-23 (zero invocations in 30 days); the map must not advertise them.
        for gone in ["context", "struct", "read", "recall"] {
            assert!(p["commands"].get(gone).is_none(), "{gone} is still listed");
        }
        assert_eq!(
            fs::read_dir(&cache).unwrap().count(),
            0,
            "{} must not touch the cache",
            args.join(" ")
        );
    }
}

/// `memory` has its own verbs and options, none of which fit on the one line the top-level
/// usage gives it, so `cort memory --help` used to answer with the whole top-level usage. The
/// memory usage is rendered from the parser itself, so a verb or option cannot drift out of it.
#[test]
fn memory_help_lists_each_memory_verb_with_its_options() {
    let (_p, cwd, _c, cache) = sandbox();
    for args in [
        vec!["memory", "--help"],
        vec!["memory", "-h"],
        vec!["memory", "help"],
        vec!["memory", "recall", "--help"],
    ] {
        let r = run_cort(&args, &cwd, &cache);
        assert_eq!(r.code, 0, "{} stderr={}", args.join(" "), r.stderr);
        let p = payload(&r);
        assert_eq!(
            p["usage"],
            "cort memory <command> [options]",
            "{}",
            args.join(" ")
        );
        let commands = p["commands"].as_object().unwrap();
        let mut verbs: Vec<&str> = commands.keys().map(String::as_str).collect();
        verbs.sort();
        assert_eq!(
            verbs,
            [
                "alias",
                "alias add",
                "alias list",
                "alias remove",
                "check",
                "configure",
                "import-config",
                "import-legacy",
                "notes",
                "notes list",
                "notes opt-in",
                "notes opt-out",
                "notes show",
                "notes status",
                "notes upload",
                "recall",
                "record",
                "resolve",
                "status",
            ]
        );
        assert!(commands["notes upload"]
            .as_str()
            .unwrap()
            .contains("--dry-run"));
        assert!(commands["alias add"]
            .as_str()
            .unwrap()
            .contains("(--from-remote <FROM_REMOTE> | --from-project <FROM_PROJECT>)"));
        for (verb, synopsis) in commands {
            assert!(
                synopsis
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("cort memory {verb}")),
                "{verb}: {synopsis}"
            );
        }
        assert!(commands["recall"].as_str().unwrap().contains("--harness"));
        assert!(commands["resolve"]
            .as_str()
            .unwrap()
            .contains("--supersedes"));
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

/// `context`, `struct`, `read` and `recall` were removed on 2026-10-23 (zero invocations in 30 days
/// on both machines). A caller still typing one gets the structured unknown-command error.
#[test]
fn the_removed_verbs_are_unknown_commands() {
    let (_p, cwd, _c, cache) = sandbox();
    for verb in ["context", "struct", "read", "recall"] {
        let r = run_cort(&[verb, "helper"], &cwd, &cache);
        assert_eq!(r.code, 1, "{verb} stdout={}", r.stdout);
        let p = payload(&r);
        assert_eq!(p["error"], "unknown_command", "{verb}");
        assert_eq!(p["detail"]["command"], verb);
    }
}

#[test]
fn unknown_format_is_a_structured_error_and_format_is_case_insensitive() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let bad = run_cort(
        &["impact", "--symbol", "helper", "-f", "yaml"],
        &cwd,
        &cache,
    );
    assert_eq!(bad.code, 1);
    let p = payload(&bad);
    assert_eq!(p["error"], "unknown_format");
    let lean = run_cort(
        &["impact", "--symbol", "helper", "-f", "LEAN"],
        &cwd,
        &cache,
    );
    assert_eq!(lean.code, 0, "stderr={}", lean.stderr);
    assert!(
        lean.stdout.starts_with("# impact helper"),
        "{}",
        lean.stdout
    );
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
fn missing_symbol_is_a_structured_error_not_a_panic() {
    let (_p, cwd, _c, cache) = sandbox();
    let impact = run_cort(&["impact"], &cwd, &cache);
    assert_eq!(impact.code, 1);
    assert_eq!(payload(&impact)["error"], "missing_symbol");
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

/// A call-site search that the rule fires on.
const FIRING_SEARCH: &str = "grep -rn 'helper(' src --include=*.ts";

/// The same spawn as `run_hook_suggest_payload`, with `CORT_SUGGEST` optionally set. The variable
/// used to re-enable the announcement; it is kept in the signature so the test can prove it now
/// changes nothing.
fn run_hook_suggest_with_cort_suggest(suggest: bool, cwd: &Path, cache: &Path) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut cmd = Command::new(cort_bin());
    cmd.arg("hook-suggest")
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        // `kick` reads the data dir; never let a hook test reach the real one.
        .env("CORT_MEMORY_DIR", cache.join("data"));
    if suggest {
        cmd.env("CORT_SUGGEST", "1");
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-suggest");
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": FIRING_SEARCH },
    });
    let body = serde_json::to_vec(&payload).unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().expect("wait cort hook-suggest");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The thin trigger has no announcement left to switch on, so `CORT_SUGGEST=1` must change
/// nothing. It used to restore the suggestion for the copy-locking tests; that whole vocabulary
/// went with the suggestion, and a variable nothing reads is worth a test that says so -- otherwise
/// the next person to "re-enable" it gets silence and no explanation.
///
/// No index is built on purpose: the fire row is now written before any project state is consulted.
#[test]
fn cort_suggest_no_longer_makes_the_hook_speak() {
    let (_p, cwd, _c, cache) = sandbox();
    for suggest in [false, true] {
        let r = run_hook_suggest_with_cort_suggest(suggest, &cwd, &cache);
        assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
        assert!(
            r.stderr.is_empty(),
            "the hook is silent on stderr too: {}",
            r.stderr
        );
        assert_eq!(
            payload(&r),
            serde_json::json!({}),
            "CORT_SUGGEST={} made the hook speak: {}",
            suggest,
            r.stdout
        );
    }
    let rows = read_hook_rows(&cache.join("usage.db"), "hook-suggest");
    assert_eq!(rows.len(), 2, "one fire, one row each: {rows:?}");
    for row in &rows {
        assert_eq!(row["hook"], "fired", "the fire is still recorded: {row}");
        assert_eq!(row["tool"], "Bash", "the tool is still recorded: {row}");
    }
}

/// Every `fired` row names the project it fired in, and that id is the one a later `cort index`
/// of the same directory takes -- so a fire joins the projects table without a second derivation.
///
/// The live db carried 13,233 hook-suggest rows and not one named a project, which made "did the
/// project this hook fired in ever get indexed" a question only transcripts could answer. The id
/// comes from the directory the hook ran in, canonicalised and hashed; nothing about the index is
/// needed to state it, so this holds whether or not the project was ever indexed.
///
/// The other direction is just as deliberate: a payload that was never read is not attributed. A
/// `no_payload` row keeps `project_id` NULL, because the harness has not said where it is.
#[test]
fn every_fired_row_names_the_project_it_fired_in() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage_db = cache.join("usage.db");
    let canon = fs::canonicalize(&cwd).unwrap();
    let expected = project_id_for(canon.to_str().unwrap());

    // No index is built: the fire row no longer depends on one.
    let fire = run_hook_suggest_payload(
        serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": FIRING_SEARCH },
            "session_id": "sess-adopt-1",
            "cwd": cwd.to_str().unwrap(),
        }),
        &[],
        &cwd,
        &cache,
    );
    assert_eq!(fire.code, 0, "{} {}", fire.stdout, fire.stderr);
    assert_eq!(
        row_project_id(&usage_db, "fired"),
        expected,
        "a fired row must name the project it fired in"
    );

    // Invalid stdin never gets as far as a harness or a project.
    let raw = run_hook_suggest_raw(b"not json at all", &cwd, &cache);
    assert_eq!(raw.code, 0, "{} {}", raw.stdout, raw.stderr);
    assert_eq!(
        row_project_id(&usage_db, "no_payload"),
        "",
        "an unread payload is not attributed to a project"
    );
}

/// A hook must never panic, and every fire now derives a project from the directory it ran in.
/// A directory name that is not UTF-8 has no project id to give -- the id is a hash of the path
/// string -- so the fire is still recorded and kicked, just without a project.
#[cfg(unix)]
#[test]
fn a_fire_in_a_non_utf8_directory_is_recorded_without_a_project() {
    use std::os::unix::ffi::OsStrExt;
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    let cwd = temp.path().join(std::ffi::OsStr::from_bytes(b"proj-\xff"));
    if let Err(e) = fs::create_dir_all(&cwd) {
        // APFS refuses names that are not UTF-8 (EILSEQ), so such a directory cannot exist there.
        if cfg!(target_os = "macos") {
            println!("SKIP: this filesystem refuses non-UTF-8 names: {e}");
            return;
        }
        panic!("cannot create the non-UTF-8 directory: {e}");
    }
    let r = run_hook_suggest_payload(
        serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": FIRING_SEARCH },
        }),
        &[],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    assert!(
        r.stderr.is_empty(),
        "a hook says nothing on stderr: {}",
        r.stderr
    );
    assert_eq!(r.stdout, "{}\n");
    assert_eq!(
        row_project_id(&cache.join("usage.db"), "fired"),
        "",
        "a path with no UTF-8 spelling has no project id"
    );
}

/// The hook's raw stdin path, for payloads that are not JSON at all.
fn run_hook_suggest_raw(body: &[u8], cwd: &Path, cache: &Path) -> Run {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(cort_bin())
        .arg("hook-suggest")
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        // `kick` reads the data dir; never let a hook test reach the real one.
        .env("CORT_MEMORY_DIR", cache.join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cort hook-suggest");
    child.stdin.take().unwrap().write_all(body).unwrap();
    let out = child.wait_with_output().expect("wait cort hook-suggest");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The single hook-suggest row of the given outcome, reduced to its `project_id` (empty string
/// when the column is NULL -- the shape this test exists to fail on).
fn row_project_id(usage_db: &Path, outcome: &str) -> String {
    let db = rusqlite::Connection::open(usage_db).expect("open usage.db");
    let mut stmt = db
        .prepare(
            "SELECT project_id FROM command_log
              WHERE command = 'hook-suggest' AND args_summary LIKE ?1",
        )
        .expect("prepare");
    let hits: Vec<Option<String>> = stmt
        .query_map(
            rusqlite::params![format!("%\"hook\":\"{outcome}\"%")],
            |r| r.get(0),
        )
        .expect("query")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(hits.len(), 1, "one {outcome} row expected, got {hits:?}");
    hits.into_iter().next().unwrap().unwrap_or_default()
}

/// A file-exclusive lock on the project database is contention that clears, not a store that
/// is occupied. `impact` used to report `storage_busy` one busy timeout into such a lock --
/// from the open path's WAL pragma or first schema statement, before any retry could apply --
/// which read to the caller as "the index database is in use, the command is unavailable".
/// Open, schema and query each retry now; the lock below outlasts one timeout (5s) but not
/// the retry budget, so the run must survive a BUSY round and answer once it lifts.
#[test]
fn impact_waits_out_an_exclusive_db_lock_instead_of_reporting_the_store_occupied() {
    let (_p, cwd, _c, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db = cort::db::db_path_for(cwd.to_str().unwrap());
    let db = cache.join(db.file_name().unwrap());
    let lock = rusqlite::Connection::open(&db).unwrap();
    lock.busy_timeout(std::time::Duration::from_millis(0))
        .unwrap();
    lock.pragma_update(None, "locking_mode", "EXCLUSIVE")
        .unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let holder = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(7));
        drop(lock);
    });
    let r = run_cort(&["impact", "--symbol", "helper"], &cwd, &cache);
    holder.join().unwrap();
    assert_eq!(r.code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
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
    // The observed shape was a database from before `reading_notes` existed. Since `read`/`recall`
    // were removed `status` no longer reads that table, so its absence must now be harmless. The
    // file is located by scanning the sandbox cache -- `db_path_for` reads CORT_CACHE_DIR from
    // *this* process, which does not have it, and would silently create a stray database in the
    // real one.
    let db_file = fs::read_dir(&cache)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "db") && p.file_name().unwrap() != "usage.db")
        .expect("index wrote a database into the sandbox cache");
    let db = rusqlite::Connection::open(&db_file).unwrap();
    db.execute_batch("DROP TABLE reading_notes;").unwrap();
    drop(db);
    let ok = run_cort(&["status"], &cwd, &cache);
    assert_eq!(ok.code, 0, "stdout={} stderr={}", ok.stdout, ok.stderr);
    assert_eq!(payload(&ok)["indexed"], true, "stdout={}", ok.stdout);

    // A table `status` does read, missing, is still an outdated schema and not contention.
    let db = rusqlite::Connection::open(&db_file).unwrap();
    db.execute_batch("DROP TABLE relationships;").unwrap();
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
///
/// The count is taken from the rows themselves, not from a verdict word: what the removed judge
/// concluded about a search was a different question from who fired the hook, and this attribution
/// outlived that vocabulary. No index is built on purpose -- the thin trigger writes one row per
/// fire whatever the project's index state.
#[test]
fn a_usage_row_records_which_harness_fired_the_hook() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage_db = cache.join("usage.db");
    let fire = |extra: &[&str]| {
        run_hook_suggest_payload(
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": { "command": FIRING_SEARCH },
            }),
            extra,
            &cwd,
            &cache,
        )
    };
    fire(&["--harness", "claude-code"]);
    fire(&["--harness", "grok"]);
    // A fire from a harness the installer never named is not attributed to whichever one it did.
    fire(&[]);

    let rows = read_hook_rows(&usage_db, "hook-suggest");
    assert_eq!(rows.len(), 3, "every fire is exactly one row: {rows:?}");
    assert_eq!(
        rows[0].get("harness").and_then(Value::as_str),
        Some("claude-code"),
        "the fire did not name its own harness: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[1].get("harness").and_then(Value::as_str),
        Some("grok"),
        "a grok fire was folded into claude-code: {:?}",
        rows[1]
    );
    assert_eq!(
        rows[2].get("harness").and_then(Value::as_str),
        Some("unspecified"),
        "a harness-less row was attributed to a real harness instead of `unspecified`: {:?}",
        rows[2]
    );

    // And the row names the machine that produced it -- a figure that cannot say where it came
    // from is not comparable to anything.
    let conn = rusqlite::Connection::open(&usage_db).expect("open usage db");
    let mut stmt = conn
        .prepare(
            "SELECT machine_id, machine_source FROM command_log
              WHERE command = 'hook-suggest' ORDER BY id",
        )
        .expect("prepare");
    let stamps: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(stamps.len(), 3);
    for (id, source) in &stamps {
        assert_eq!(
            id,
            cort::usage::machine_id(),
            "the row names another machine"
        );
        assert_eq!(source, cort::usage::machine_id_source());
    }
}

/// One settings file can be read by more than one harness: Grok loads `~/.claude/settings.json` for
/// Claude Code compatibility, so the entry the installer wired there fires inside Grok carrying
/// `--harness claude-code`. Measured on 2026-09-02: every such fire would have been counted as a
/// Claude Code injection with no Claude transcript to match it. `transcript_path` is the harness
/// naming its own session file, and it settles the question without guessing at the environment.
#[test]
fn the_harness_is_taken_from_the_payload_not_from_the_flag_alone() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage_db = cache.join("usage.db");
    let fire = |transcript: &str| {
        run_hook_suggest_payload(
            serde_json::json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": FIRING_SEARCH },
                "transcript_path": transcript,
            }),
            &["--harness", "claude-code"],
            &cwd,
            &cache,
        )
    };

    // Wired as claude-code, but the transcript says the process running it is Grok.
    fire("/home/u/.grok/sessions/x/updates.jsonl");
    // A transcript path naming nothing we recognise leaves the declared value standing: a declared
    // harness is still better than none.
    fire("/var/tmp/somewhere/else.jsonl");
    // Codex sends Claude Code's payload byte for byte; only its transcript says who it is.
    fire("/home/u/.codex/sessions/2026/10/25/rollout-x.jsonl");

    let rows = read_hook_rows(&usage_db, "hook-suggest");
    assert_eq!(rows.len(), 3, "one fire, one row: {rows:?}");
    assert_eq!(
        rows[0].get("harness").and_then(Value::as_str),
        Some("grok"),
        "a Grok fire was credited to claude-code: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[1].get("harness").and_then(Value::as_str),
        Some("claude-code"),
        "an unrecognised transcript path discarded the declared harness: {:?}",
        rows[1]
    );
    assert_eq!(
        rows[2].get("harness").and_then(Value::as_str),
        Some("codex"),
        "a Codex fire was credited to claude-code: {:?}",
        rows[2]
    );
}

/// Field NAMES are stable vocabulary; payload content never enters the log. The row is counted
/// whatever the hook concluded about the call, so this holds for any tool and any outcome.
#[test]
fn payload_content_never_enters_the_hook_row() {
    let (_p, cwd, _c, cache) = sandbox();
    let run = run_hook_suggest_payload(
        serde_json::json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Read",
            "tool_input": { "file_path": "/tmp/secret-notes.md" },
            "cwd": cwd.to_str().unwrap(),
            "session_id": "fp1",
        }),
        &[],
        &cwd,
        &cache,
    );
    assert_eq!(run.code, 0);
    let rows = read_hook_rows(&cache.join("usage.db"), "hook-suggest");
    assert_eq!(rows.len(), 1, "one fire, one row: {rows:?}");
    let summary = rows[0].to_string();
    assert!(
        !summary.contains("secret"),
        "payload content must stay out of the log: {summary}"
    );
}

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
        // `kick` reads the data dir; never let a hook test reach the real one.
        .env("CORT_MEMORY_DIR", cache.join("data"))
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
        // `kick` reads the data dir; never let a hook test reach the real one.
        .env("CORT_MEMORY_DIR", cache.join("data"))
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
        // `kick` reads the data dir; never let a hook test reach the real one.
        .env("CORT_MEMORY_DIR", cache.join("data"))
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
        Some(h) => {
            cort::usage::outcomes_of_hook_at(&usage_db, "hook-refresh", 0, None, Some(h)).unwrap()
        }
        None => cort::usage::outcomes_of_hook_at(&usage_db, "hook-refresh", 0, None, None).unwrap(),
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
    let suggest = cort::usage::hook_outcomes_at(&usage_db, 0, None, None).unwrap();
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
    // v6 is the thin trigger: `symbol`/`why`/`demand` belong to v≤5 rows, and a fire row now
    // carries `tool` instead.
    assert_eq!(last.get("v").and_then(Value::as_i64), Some(6));

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

    // The primary number is the row count, and it never mentions `model`: one fire is one row
    // however many models sat behind the harness. No index is built on purpose -- both the current
    // binary and the thin trigger that replaces it write exactly one row per fire.
    let conn = rusqlite::Connection::open(&usage_db).expect("open usage db");
    let fired: i64 = conn
        .query_row(
            "SELECT count(*) FROM command_log WHERE command = 'hook-suggest'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fired, 4, "the harness total must count every fire");
    let rows = read_hook_rows(&usage_db, "hook-suggest");
    let for_harness = rows
        .iter()
        .filter(|r| r.get("harness").and_then(Value::as_str) == Some("claude-code"))
        .count();
    assert_eq!(
        for_harness, 4,
        "every row must name the harness that fired it: {rows:?}"
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
        "the breakdown and the total disagree about how many rows exist: {models:?} vs {fired}"
    );

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
        9,
        "all harnesses have recall plus the two tool events: {lines:?}"
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
    assert_eq!(files.len(), 9);
    assert_eq!(
        s.stdout
            .lines()
            .filter(|l| l.starts_with("claude-code\tsession\t"))
            .count(),
        1
    );
    assert_eq!(
        s.stdout
            .lines()
            .filter(|l| l.starts_with("codex\tsession\t"))
            .count(),
        1
    );
    assert!(s
        .stdout
        .lines()
        .any(|l| l.starts_with("kimi-code\tsession\t")));
    for l in s.stdout.lines().filter(|l| !l.is_empty()) {
        assert_eq!(l.split('\t').nth(2), Some("wired"), "not wired: {l}");
    }
    assert_eq!(
        files.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "nine entries across three files: {files:?}"
    );
    let removed = run(&["--all", "--remove", "--lean"]);
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    let removed_rows: Vec<&str> = removed.stdout.lines().collect();
    assert_eq!(removed_rows.len(), 9);
    assert_eq!(
        removed_rows
            .iter()
            .filter(|l| l.starts_with("claude-code\tsession\t"))
            .count(),
        1
    );
    let after = run(&["--all", "--status", "--lean"]);
    assert!(after
        .stdout
        .lines()
        .all(|l| l.split('\t').nth(2) == Some("not_wired")));
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

fn git_in_fixture(root: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "test"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "init"],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(&args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }
}

fn refresh_outcomes(cache: &Path) -> serde_json::Map<String, Value> {
    cort::usage::outcomes_of_hook_at(&cache.join("usage.db"), "hook-refresh", 0, None, None)
        .expect("read usage db")
}

/// The bug, as a test -- and the fixture is built so that **cwd and the edited file resolve to
/// different projects**. That is the whole point. A fixture where both walk up to the same root
/// cannot tell an implementation that reads the payload from one that ignores it and resolves from
/// cwd, which is the only property this change exists to establish and the way it will regress.
///
/// Here cwd is an unindexed scratch directory: resolving from cwd yields nothing and the hook would
/// refuse. A `refreshed` therefore proves the payload path was read.
#[test]
fn a_refresh_resolves_the_project_from_the_payload_not_the_shell() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 3; }\nexport function extra() { return 1; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(
        &[],
        serde_json::from_str(&payload).unwrap(),
        elsewhere.path(),
        &cache,
    );
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("refreshed").and_then(Value::as_i64),
        Some(1),
        "cwd owns no index, so a `refreshed` here can only come from the payload path: {counts:?}"
    );
}

/// The shape the reproduction actually used: a **relative** path, from a subdirectory. Claude Code
/// sends absolute paths (`docs/2026-09-05-posttooluse-payloads.md`), but Codex and Kimi were never
/// captured, and a harness that sends a relative one would fail silently in production with the
/// absolute-path test green.
#[test]
fn a_relative_path_in_the_payload_resolves_against_the_directory_the_hook_runs_in() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 5; }\n",
    )
    .unwrap();
    let sub = cwd.join("src");
    let r = run_hook_refresh_with(
        &[],
        serde_json::json!({"tool_name":"Edit","tool_input":{"file_path":"helper.ts"}}),
        &sub,
        &cache,
    );
    assert_eq!(r.code, 0);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("refreshed").and_then(Value::as_i64),
        Some(1),
        "a relative path resolves against the directory the hook runs in: {counts:?}"
    );
}

/// A relative path needs a base, and the payload's `cwd` is the base -- not the directory the hook
/// process happens to have inherited. The no-path arm already applied that precedence; this pins
/// the same rule for the path arm, which is where a review found it missing.
///
/// The fixture makes the two bases resolve to **different projects**: the process runs in an
/// unindexed scratch directory, the payload asserts the indexed one. Reading the process cwd yields
/// `no_index`, so a `refreshed` here can only come from the payload's `cwd`.
#[test]
fn a_relative_path_resolves_against_the_payload_cwd_before_the_process_cwd() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 7; }\n",
    )
    .unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let r = run_hook_refresh_with(
        &[],
        serde_json::json!({
            "tool_name": "Edit",
            "cwd": cwd.to_str().unwrap(),
            "tool_input": {"file_path": "src/helper.ts"},
        }),
        elsewhere.path(),
        &cache,
    );
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("refreshed").and_then(Value::as_i64),
        Some(1),
        "the process cwd owns no index, so a `refreshed` proves the payload cwd was the base: {counts:?}"
    );
}

/// No usable path -- `Bash` carries none, confirmed by interception. cwd stays the answer, which is
/// the whole of the behaviour before this change for that shape.
#[test]
fn a_refresh_with_no_path_in_the_payload_still_uses_the_working_directory() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let r = run_hook_refresh_with(
        &[],
        serde_json::json!({"tool_name":"Bash","tool_input":{}}),
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("already_current").and_then(Value::as_i64),
        Some(1),
        "{counts:?}"
    );
}

/// An explicit path owning no index is a finding, not a reason to repair something else. Falling
/// back to cwd here would refresh a project that was never edited.
#[test]
fn an_edited_path_outside_every_index_refuses_rather_than_repairing_cwd() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("stray.ts");
    std::fs::write(&outside, "export function stray() { return 1; }\n").unwrap();
    let payload = format!(
        r#"{{"tool_name":"Write","tool_input":{{"file_path":"{}"}}}}"#,
        outside.display()
    );
    let r = run_hook_refresh_with(&[], serde_json::from_str(&payload).unwrap(), &cwd, &cache);
    assert_eq!(r.code, 0);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("no_index").and_then(Value::as_i64),
        Some(1),
        "an explicit path under nothing indexed must not repair cwd's project: {counts:?}"
    );
}

/// A schema-only database at cwd is not an index, and the repair hook must not turn it into one.
/// This pins a bug the rewrite fixes as a side effect: `index_state()` passed a db file that merely
/// existed, `open_project_tracked` does not check for the row, and `incremental_index` falls through
/// to `full_index`, which INSERTS it -- the hook creating an index nobody asked for.
#[test]
fn a_schema_only_database_at_cwd_is_not_refreshed_into_an_index() {
    let (_p, cwd, _c, cache) = sandbox();
    // Any command that opens the project writes the schema without indexing anything.
    run_cort(&["impact", "--symbol", "helper"], &cwd, &cache);
    let r = run_hook_refresh_with(
        &[],
        serde_json::json!({"tool_name":"Bash","tool_input":{}}),
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("no_index").and_then(Value::as_i64),
        Some(1),
        "an empty index is not an index: {counts:?}"
    );
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(db_file).unwrap();
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the hook must not have created a project row");
}

/// The incident in one assertion: an index whose stored extractor is not the one this binary uses
/// must say so, while `stale` stays false because no git head moved -- which is exactly the shape
/// that read as "all current" on 2026-09-05.
#[test]
fn projects_reports_extractor_drift_while_git_says_fresh() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    // The healthy answer, asserted before anything is tampered with. Nothing else pins it: the
    // shape test runs on an empty population and accepts any of the three words, so an
    // implementation that can never say `compatible` passes every other test in this file and
    // tells a fully repaired machine that its compatibility is unknown, forever.
    let ok = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(
        ok.stdout, "indexes\tcompatible\t0\t0\n",
        "a freshly indexed project is compatible: {:?}",
        ok.stdout
    );

    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);

    let r = run_cort(&["projects"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let rows: Value = serde_json::from_str(&r.stdout).expect("projects emits json");
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("path").and_then(Value::as_str) == Some(cwd.to_str().unwrap()))
        .expect("our project is listed");
    assert_eq!(
        row.get("extractor_version").and_then(Value::as_str),
        Some("not-the-one-that-ships")
    );
    assert_eq!(
        row.get("drifted").and_then(Value::as_bool),
        Some(true),
        "{row}"
    );
    assert_eq!(
        row.get("stale").and_then(Value::as_bool),
        Some(false),
        "git head did not move: {row}"
    );

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t0\n",
        "verdict was {:?}",
        v.stdout
    );
}

/// Schema drift is a second, independent axis. A `drifted` computed only from the extractor passes
/// the test above and fails this one.
#[test]
fn projects_reports_schema_drift_independently_of_the_extractor() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "SCHEMA_VERSION", "3").unwrap();
    drop(db);

    let r = run_cort(&["projects"], &cwd, &cache);
    let rows: Value = serde_json::from_str(&r.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("path").and_then(Value::as_str) == Some(cwd.to_str().unwrap()))
        .expect("our project is listed");
    assert_eq!(row.get("schema_version").and_then(Value::as_str), Some("3"));
    assert_eq!(
        row.get("drifted").and_then(Value::as_bool),
        Some(true),
        "{row}"
    );

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t0\n",
        "verdict was {:?}",
        v.stdout
    );
}

/// An unreadable index is an absence of facts, not a version mismatch. Reporting it as `drifted`
/// would claim knowledge the binary does not have (spec §2).
#[test]
fn an_unreadable_index_makes_the_verdict_unknown_not_drifted() {
    let (_p, cwd, _c, cache) = sandbox();
    std::fs::create_dir(cache.join("adirectory.db")).unwrap();
    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert_eq!(
        v.stdout, "indexes\tunknown\t0\t1\n",
        "verdict was {:?}",
        v.stdout
    );

    // The row shape itself, which only the verdict counted above. A future consumer reads these
    // keys, and nothing else in the suite would notice them being renamed.
    let r = run_cort(&["projects"], &cwd, &cache);
    let rows: Value = serde_json::from_str(&r.stdout).expect("projects emits json");
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("unreadable").is_some())
        .expect("the unreadable entry is listed, not dropped");
    assert!(
        row.get("db_path")
            .and_then(Value::as_str)
            .is_some_and(|p| p.ends_with("adirectory.db")),
        "the unreadable row names the database it could not read: {row}"
    );
    assert!(
        row.get("stale").is_none() && row.get("drifted").is_none(),
        "an unreadable entry claims neither staleness nor drift: {row}"
    );
}

/// Drift and unreadable together: the word names the half we know, and each number stands only for
/// its own fact. An implementation that lets any unreadable entry force `unknown` passes all three
/// tests above while violating the spec's rule that drift is a fact and must be named.
#[test]
fn drift_outranks_unreadable_and_each_count_stands_alone() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    std::fs::create_dir(cache.join("adirectory.db")).unwrap();
    std::fs::write(cache.join("junk.db"), b"not a sqlite file").unwrap();

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t2\n",
        "one drifted and two unreadable: the word names the drift, and neither count absorbs the \
         other: {:?}",
        v.stdout
    );
}

/// A drifted index whose directory is gone is still a drifted index. Excluding it from the count
/// and denying the fact are different decisions; only the first would be defensible, and this
/// pins that the fact is not denied.
#[test]
fn a_drifted_index_whose_directory_is_gone_is_still_counted_as_drift() {
    let (_p, _cwd, _c, cache) = sandbox();
    let gone = tempfile::tempdir().unwrap();
    // Canonicalize like `make_project` does: the CLI canonicalizes root before hashing it into
    // a project id, and on macOS the temp dir arrives through a symlink (`/var` -> `/private/var`).
    // Hashing the raw path opens a db file that was never written -- an empty file sqlite creates
    // on open, so `set_meta` dies on `no such table` instead of testing anything.
    let gone_path = fs::canonicalize(gone.path()).unwrap();
    std::fs::write(
        gone_path.join("a.ts"),
        "export function a() { return 1; }\n",
    )
    .unwrap();
    git_in_fixture(&gone_path);
    let idx = run_cort(&["index"], &gone_path, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(gone_path.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    drop(gone);

    let elsewhere = tempfile::tempdir().unwrap();
    let v = run_cort(&["projects", "--verdict"], elsewhere.path(), &cache);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t0\n",
        "the index is readable and does not match, whether or not its tree still exists: {:?}",
        v.stdout
    );
}

/// `install.sh --check` parses this line with `read`, so its shape is a contract: exactly one line,
/// four tab-separated non-empty fields, and two numeric counts.
#[test]
fn the_verdict_line_is_one_line_of_four_non_empty_fields() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.lines().count(), 1, "stdout was {:?}", r.stdout);
    let fields: Vec<&str> = r.stdout.trim_end_matches('\n').split('\t').collect();
    assert_eq!(fields.len(), 4, "stdout was {:?}", r.stdout);
    assert!(
        fields.iter().all(|f| !f.is_empty()),
        "stdout was {:?}",
        r.stdout
    );
    assert_eq!(fields[0], "indexes");
    assert!(
        matches!(fields[1], "compatible" | "drifted" | "unknown"),
        "stdout was {:?}",
        r.stdout
    );
    assert!(
        fields[2].parse::<u64>().is_ok(),
        "stdout was {:?}",
        r.stdout
    );
    assert!(
        fields[3].parse::<u64>().is_ok(),
        "stdout was {:?}",
        r.stdout
    );
}

/// An index whose incremental was killed mid-run has current versions and an incomplete graph.
/// `cort status` calls that stale; `--verdict` must not call it compatible. Two answers to one
/// question, disagreeing on a population that exists, is the defect this plan is about.
#[test]
fn the_verdict_and_staleness_agree_about_an_incomplete_graph() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "graph_pending", "1").unwrap();
    drop(db);

    let st = run_cort(&["status"], &cwd, &cache);
    let status: Value = serde_json::from_str(&st.stdout).expect("status emits json");
    assert_eq!(
        status.get("index_is_stale").and_then(Value::as_bool),
        Some(true),
        "an incomplete graph is staleness: {status}"
    );

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t0\n",
        "the verdict must not call compatible what status calls stale: {:?}",
        v.stdout
    );
}

/// The treadmill, as a test. An index whose extractor is superseded used to make every edit hook
/// attempt a full rebuild inside a five-second budget; killed before commit, it advanced nothing
/// and tried again on the next edit, forever and silently.
#[test]
fn the_refresh_hook_refuses_a_full_rebuild_and_says_so() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 11; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(&[], serde_json::from_str(&payload).unwrap(), &cwd, &cache);
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("rebuild_required").and_then(Value::as_i64),
        Some(1),
        "the hook records the debt instead of attempting it: {counts:?}"
    );

    // And it records *which* debt. Asserting only the counter lets an implementation write an
    // empty reason vector, or one hardcoded name for every refusal, and still pass -- the name of
    // this test says "and says so", so the "so" has to be checked.
    let st = run_cort(&["status"], &cwd, &cache);
    let status: Value = serde_json::from_str(&st.stdout).expect("status emits json");
    let reasons = status
        .get("rebuild_required")
        .and_then(Value::as_array)
        .expect("status names why a rebuild is owed");
    assert!(
        reasons
            .iter()
            .any(|r| r.as_str() == Some("extractor_changed")),
        "the extractor is the reason here, and nothing else is: {status}"
    );
    assert!(
        !reasons.iter().any(|r| r.as_str() == Some("schema_changed")),
        "the schema was untouched: {status}"
    );

    // And the stored extractor is untouched: a refusal must not look like a repair.
    let db = cort::db::open_db(&db_file).unwrap();
    assert_eq!(
        cort::db::get_meta(&db, "extractor_version")
            .unwrap()
            .as_deref(),
        Some("not-the-one-that-ships"),
        "refusing must not stamp a version it did not build"
    );
}

/// 查詢即自癒: the query that finds the index behind is itself the cheapest actor to heal it.
///
/// Measured before this was built (2026-09-14): the edit hook refused 905 rebuilds in 7 days
/// while the chained-command suggestion converted 1 time in 4 days, and a full pipeline costs
/// 1.4-2.3s on these hundred-file projects. So `impact` answers only after repairing the cache,
/// and the payload says so. A full heal here means the extractor was superseded -- the exact
/// debt the treadmill fixture above creates and the hook refuses.
#[test]
fn impact_heals_a_drifted_index_instead_of_answering_stale() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 11; }\n",
    )
    .unwrap();

    let r = run_cort(
        &["impact", "--symbol", "helper", "--depth", "1", "-f", "json"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let v = payload(&r);
    assert_eq!(v["self_healed"], json!(true), "{v}");
    assert_eq!(v["heal_mode"], json!("full"), "{v}");
    // The answer is post-heal fresh, by construction: nothing is owed anymore, so the
    // payload's own honesty fields say so and no maintenance instruction remains.
    assert_eq!(v["index_is_stale"], json!(false), "{v}");
    assert_eq!(v["repair"], json!("none"), "{v}");
    // The measurement rides in the usage row, not in a transcript.
    let row = latest_command_row(&cache.join("usage.db"), "impact");
    assert_eq!(row["self_healed"], json!(true), "{row}");
    assert_eq!(row["heal_mode"], json!("full"), "{row}");

    // And the second query has nothing to heal, so it adds no keys at all: the green-path
    // payload stays byte-identical to the pre-heal contract.
    let r2 = run_cort(
        &["impact", "--symbol", "helper", "-f", "json"],
        &cwd,
        &cache,
    );
    assert_eq!(r2.code, 0, "{} {}", r2.stdout, r2.stderr);
    let v2 = payload(&r2);
    assert!(
        v2.get("self_healed").is_none() && v2.get("heal_mode").is_none(),
        "a fresh index must add no heal keys: {v2}"
    );
}

/// The common staleness -- an edited-but-uncommitted tree -- is healed by the same entry, and
/// the answer reflects the edit the index never saw.
#[test]
fn impact_heals_uncommitted_edits_before_answering() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    std::fs::write(
        cwd.join("src/helper.ts"),
        "export function helper(n: number) { return n * 2; }\n\
         export function healedFn() { return helper(1); }\n",
    )
    .unwrap();

    // Control: with healing off, the new symbol is not in the index and no seed resolves.
    let ctl = run_cort_env(
        &["impact", "--symbol", "healedFn", "-f", "json"],
        &cwd,
        &cache,
        &[("CORT_NO_HEAL", "1")],
    );
    assert_eq!(ctl.code, 0, "{} {}", ctl.stdout, ctl.stderr);
    assert_eq!(
        payload(&ctl)["seed_count"],
        json!(0),
        "the control must show the staleness the heal then fixes"
    );

    let r = run_cort(
        &["impact", "--symbol", "healedFn", "-f", "json"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let v = payload(&r);
    assert_eq!(v["self_healed"], json!(true), "{v}");
    assert_eq!(v["heal_mode"], json!("incremental"), "{v}");
    assert!(
        v["seed_count"].as_i64().unwrap_or(0) >= 1,
        "the healed answer resolves the symbol the edit added: {v}"
    );
}

/// A tree too big to heal inline must not block the answer behind a rebuild: it answers with
/// the honest disclosure and hands the repair to a single-flight background rebuilder.
#[test]
fn a_repo_past_the_heal_threshold_defers_to_a_background_healer() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 11; }\n",
    )
    .unwrap();

    let r = run_cort_env(
        &["impact", "--symbol", "helper", "-f", "json"],
        &cwd,
        &cache,
        &[("CORT_HEAL_MAX_FILES", "1")],
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let v = payload(&r);
    assert_eq!(v["self_healed"], json!(false), "{v}");
    assert!(
        v["heal_deferred"].is_string(),
        "the payload names why the heal was deferred: {v}"
    );

    // The background child inherits the cache dir and finishes in well under a second on
    // this fixture; poll rather than sleep so the test stays as fast as the work allows.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let st = run_cort(&["status"], &cwd, &cache);
        let status: Value = serde_json::from_str(&st.stdout).unwrap_or(Value::Null);
        if status["rebuild_required"]
            .as_array()
            .is_some_and(|a| a.is_empty())
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the background healer never cleared the debt: {status}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// An upgrade holding the locks is not a reason to block a read: the query stands down on the
/// heal, answers from the index it has, and says so.
#[test]
fn an_upgrade_in_flight_defers_the_heal_instead_of_blocking_the_answer() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);

    let _locks = cort::upgrade::acquire_upgrade_locks(&cache, std::time::Duration::from_secs(1))
        .expect("the test holds the upgrade locks");
    let r = run_cort(
        &["impact", "--symbol", "helper", "-f", "json"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let v = payload(&r);
    assert_eq!(v["self_healed"], json!(false), "{v}");
    assert_eq!(
        v["heal_deferred"],
        json!("upgrade_in_flight"),
        "the payload names the stand-down: {v}"
    );
    // Answered from the stale index, honestly: the disclosure survives the stand-down.
    assert_eq!(v["index_is_stale"], json!(true), "{v}");
}

/// The never-create boundary is a refusal to bootstrap, not a malfunction — but a husk
/// (schema present, `file_state` empty: what a create that died mid-run leaves behind) is
/// exactly where silence was misread. On 2026-09-24 a sandboxed create left one on
/// travel-2026, and `repair=rebuild_required` with no heal marker was read as "the heal
/// never fired" for every query that touched it after. The boundary now names itself; the
/// repair lever stays an explicit `cort index`.
#[test]
fn an_empty_index_names_the_never_create_boundary_instead_of_staying_silent() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    db.execute("DELETE FROM file_state", []).unwrap();
    drop(db);

    let r = run_cort(
        &["impact", "--symbol", "helper", "-f", "json"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let v = payload(&r);
    assert_eq!(v["self_healed"], json!(false), "{v}");
    assert_eq!(
        v["heal_deferred"],
        json!("empty_index_never_creates"),
        "the boundary names itself: {v}"
    );
    // The refusal to bootstrap is not a staleness repair either: the disclosure survives.
    assert_eq!(v["index_is_stale"], json!(true), "{v}");
}

/// `CORT_NO_HEAL=1` is the read-only consumer's escape hatch (evals run impact in parallel
/// threads). It must restore today's behavior exactly: no heal, no heal keys, same disclosures.
#[test]
fn cort_no_heal_keeps_the_old_contract() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 11; }\n",
    )
    .unwrap();

    let r = run_cort_env(
        &["impact", "--symbol", "helper", "-f", "json"],
        &cwd,
        &cache,
        &[("CORT_NO_HEAL", "1")],
    );
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let v = payload(&r);
    assert!(
        v.get("self_healed").is_none() && v.get("heal_mode").is_none(),
        "the opt-out must add no heal keys: {v}"
    );
    assert_eq!(v["index_is_stale"], json!(true), "{v}");
    let st = run_cort(&["status"], &cwd, &cache);
    let status: Value = serde_json::from_str(&st.stdout).unwrap_or(Value::Null);
    assert!(
        status["rebuild_required"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "no heal happened, so the debt is still named: {status}"
    );
}

/// The foreground keeps its rebuild. `cort index --incremental` is a typed command a person ran on
/// purpose; refusing it would leave the debt with no actor at all, which this repository has
/// measured as worthless (19 re-index runs against 2,700+ hook fires).
#[test]
fn a_foreground_incremental_still_rebuilds() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);

    let r = run_cort(&["index", "--incremental"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let db = cort::db::open_db(&db_file).unwrap();
    assert_eq!(
        cort::db::get_meta(&db, "extractor_version").unwrap(),
        Some(cort::pack::extractor_version().unwrap()),
        "the foreground path repaid the debt"
    );
}

/// The extractor is not the only reason a full rebuild is owed. An index whose incremental was
/// killed mid-run has current versions and `graph_pending=1`, and rebuilding it is the same
/// unbounded work under the same five-second budget.
#[test]
fn the_refresh_hook_refuses_an_incomplete_graph_too() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "graph_pending", "1").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 17; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(&[], serde_json::from_str(&payload).unwrap(), &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("rebuild_required").and_then(Value::as_i64),
        Some(1),
        "graph_incomplete is a refusal too: {counts:?}"
    );
}

/// `status` printed `extractor_version` from the `projects` column while `rebuild_required` was
/// derived from the `_cortex_meta` copy, so one command's output contradicted itself: the version
/// it named was the current one, beside a reason saying the version had changed. Two readers of one
/// fact, and nothing keeping them in step.
#[test]
fn status_does_not_contradict_itself_about_the_extractor() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
    drop(db);

    let r = run_cort(&["status"], &cwd, &cache);
    let st: Value = serde_json::from_str(&r.stdout).expect("status emits json");
    assert!(
        st.get("rebuild_required")
            .and_then(Value::as_array)
            .is_some_and(|v| v.iter().any(|x| x.as_str() == Some("extractor_changed"))),
        "the fixture drifted the extractor: {st}"
    );
    assert_eq!(
        st.get("extractor_version").and_then(Value::as_str),
        Some("superseded"),
        "the version status names must be the one the decision was made from: {st}"
    );
    assert_eq!(
        st.get("candidates_narrowed").and_then(Value::as_bool),
        Some(true),
        "the raw narrowing fact rides beside the reasons, in the git fixture it narrowed: {st}"
    );
}

/// The third refusal reason has its own gate, because it is the only one not derived from stored
/// metadata: a git candidate set that cannot be narrowed honestly widens to everything, which is a
/// full re-extraction under the same five-second budget. Removing that gate left all 453 tests
/// green while restoring the treadmill for every project git will not speak for.
#[test]
fn the_refresh_hook_refuses_when_git_cannot_narrow_the_candidates() {
    let (p, cwd, _c, cache) = sandbox();
    // Deliberately NOT a git repository: `git_candidates` cannot narrow, so it widens to
    // everything rather than quietly to nothing (`incremental.rs`, `narrowed: false`).
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 19; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(&[], serde_json::from_str(&payload).unwrap(), &cwd, &cache);
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("rebuild_required").and_then(Value::as_i64),
        Some(1),
        "a candidate set git will not narrow is a refusal, not a rebuild: {counts:?}"
    );
}

/// A schema migration is a table rebuild. The edit hook has a five-second budget and no way to
/// report failure, so it must not be the thing that performs one -- it records the debt and leaves
/// the database exactly as it found it.
#[test]
fn the_refresh_hook_does_not_migrate_the_schema() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "SCHEMA_VERSION", "4").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 13; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(&[], serde_json::from_str(&payload).unwrap(), &cwd, &cache);
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);

    let db = cort::db::open_db(&db_file).unwrap();
    assert_eq!(
        cort::db::get_meta(&db, "SCHEMA_VERSION")
            .unwrap()
            .as_deref(),
        Some("4"),
        "the hook migrated a schema it has no budget to migrate"
    );

    // Not migrating is half the property. A hook that bailed silently -- opened, gave up, wrote
    // nothing -- would satisfy the assertion above and leave the debt unrecorded.
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("rebuild_required").and_then(Value::as_i64),
        Some(1),
        "the hook records the schema debt it declined: {counts:?}"
    );
}

/// The structural fix for a funnel that does not sum: the outcome vocabulary is a constant in the
/// library, the census is the tool's own partition, and this test drives every branch the thin
/// trigger has through the real binary and holds all three against each other.
///
/// There are only two branches left. `no_payload` is stdin that was not a payload this hook could
/// read (invalid JSON, or a missing or null `tool_input`); `fired` is a payload that was read, whatever tool it
/// names. A new outcome emitted without a vocabulary entry fails here the day it lands, and the
/// partition's sum is checked against the row count.
#[test]
fn the_suggest_vocabulary_is_closed_and_the_census_sums_to_the_fires() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage_db = cache.join("usage.db");

    // Invalid stdin never reaches the parser: the default outcome, recorded before anything read.
    let invalid = run_hook_suggest_raw(b"not json at all", &cwd, &cache);
    assert_eq!(invalid.code, 0, "{} {}", invalid.stdout, invalid.stderr);
    assert_eq!(
        invalid.stdout.trim(),
        "{}",
        "no injection: the empty payload only"
    );

    // JSON, but the harness has not said what is about to run: still not a fire.
    let no_input = run_hook_suggest_payload(
        serde_json::json!({ "tool_name": "Bash" }),
        &[],
        &cwd,
        &cache,
    );
    assert_eq!(no_input.code, 0, "{} {}", no_input.stdout, no_input.stderr);
    // A null `tool_input` says no more than a missing one.
    let null_input = run_hook_suggest_payload(
        serde_json::json!({ "tool_name": "Bash", "tool_input": null }),
        &[],
        &cwd,
        &cache,
    );
    assert_eq!(
        null_input.code, 0,
        "{} {}",
        null_input.stdout, null_input.stderr
    );

    let fire = |tool: &str, input: Value| {
        run_hook_suggest_payload(
            serde_json::json!({ "tool_name": tool, "tool_input": input }),
            &[],
            &cwd,
            &cache,
        )
    };
    // A shell line the old rule had no suggestion for, and one it did.
    fire(
        "Bash",
        serde_json::json!({ "command": "cargo test --workspace" }),
    );
    fire("Bash", serde_json::json!({ "command": FIRING_SEARCH }));
    // Kimi's structured search surface, and a tool that is not a search at all.
    fire(
        "Grep",
        serde_json::json!({ "pattern": "helper", "path": "src" }),
    );
    fire("Read", serde_json::json!({ "file_path": "src/helper.ts" }));

    let rows = read_hook_rows(&usage_db, "hook-suggest");
    assert_eq!(rows.len(), 7, "one row per fire: {rows:?}");
    let expected: [(&str, Option<&str>); 7] = [
        ("no_payload", None),
        ("no_payload", None),
        ("no_payload", None),
        ("fired", Some("Bash")),
        ("fired", Some("Bash")),
        ("fired", Some("Grep")),
        ("fired", Some("Read")),
    ];
    for (row, (outcome, tool)) in rows.iter().zip(expected) {
        assert_eq!(row["hook"], outcome, "{row}");
        assert_eq!(row["v"], 6, "the thin trigger is v6: {row}");
        match tool {
            Some(tool) => {
                assert_eq!(row["tool"], tool, "{row}");
                assert!(row.get("collect").is_some(), "a fire kicks: {row}");
            }
            None => assert!(
                row.get("collect").is_none(),
                "an unread payload does not kick: {row}"
            ),
        }
    }

    // The partition sums to the fires, and every row landed in one of the two named buckets.
    let census = cort::usage::hook_census_at(&usage_db, "hook-suggest", 0).expect("read usage db");
    let total: i64 = census.values().filter_map(|v| v.as_i64()).sum();
    assert_eq!(total, 7, "the census must close: {census:?}");
    assert_eq!(census.get("no_payload").and_then(Value::as_i64), Some(3));
    assert_eq!(census.get("fired").and_then(Value::as_i64), Some(4));
    assert!(
        census
            .keys()
            .all(|k| cort::hook::SUGGEST_OUTCOMES.contains(&k.as_str())),
        "the vocabulary above must already cover what this binary emits: {census:?}"
    );
}

/// `tool` is an identifier, not content: a name that fits `[A-Za-z0-9_.:-]{1,64}` is recorded
/// verbatim, and everything else collapses to one stable value. The alternatives would be to drop
/// the field (losing "which tool fired" for every MCP-style name) or to record arbitrary payload
/// text into a log that is read by mining.
#[test]
fn an_unrecognisable_tool_name_is_recorded_as_unrecognised() {
    let (_p, cwd, _c, cache) = sandbox();
    let long = "a".repeat(65);
    let cases: [(Option<Value>, &str); 5] = [
        (Some(json!("mcp__x__y")), "mcp__x__y"),
        (Some(json!("has space/and/slash")), "unrecognised"),
        (Some(json!(long)), "unrecognised"),
        (Some(json!(7)), "unrecognised"),
        (None, "unrecognised"),
    ];
    for (tool_name, expected) in cases {
        let mut payload = json!({ "tool_input": { "command": "ls -la" } });
        if let Some(name) = tool_name {
            payload["tool_name"] = name;
        }
        let r = run_hook_suggest_payload(payload, &[], &cwd, &cache);
        assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
        let row = latest_command_row(&cache.join("usage.db"), "hook-suggest");
        assert_eq!(row["tool"], expected, "{row}");
    }
}

/// The same closure for the other hook: `hook-refresh`'s outcomes are their own constant, and the
/// census buckets them verbatim — which is why the first real run no longer reports
/// `unknown/already_current=3947`. Representative branches, not all eight: an unindexed project
/// (`no_index`), a current index (`already_current`), and a real edit repaired on the hook's own
/// back (`refreshed`). The guard branches (`db_unavailable`, `no_ast_grep`,
/// `upgrade_stood_down`, `busy_or_failed`, `rebuild_required`) need fault injection to reach;
/// the unit census test holds their strings against the writer's shape instead.
#[test]
fn the_refresh_vocabulary_is_closed_and_the_census_sums_to_the_fires() {
    let (_p, cwd, _c, cache) = sandbox();
    let usage_db = cache.join("usage.db");

    // The index does not exist yet: the hook refuses rather than creating one.
    let r = run_hook_refresh(&cwd, &cache);
    assert_eq!(
        r.code, 0,
        "a refresh is always silent success: {}",
        r.stderr
    );
    assert_eq!(r.stdout.trim(), "{}");

    // With an index behind it, an edit to a current file changes nothing...
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    let indexed = idx.code == 0;
    if indexed {
        run_hook_refresh_with(&[], edit_payload(None), &cwd, &cache);
        // ...and a real edit is repaired on the hook's own back.
        std::fs::write(
            cwd.join("src/helper.ts"),
            "export function helper(n: number) { return n * 3; }\n",
        )
        .unwrap();
        run_hook_refresh_with(&[], edit_payload(None), &cwd, &cache);
    }

    let conn = rusqlite::Connection::open(&usage_db).expect("open usage db");
    let fires: i64 = conn
        .query_row(
            "SELECT count(*) FROM command_log WHERE command = 'hook-refresh'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(fires >= 1, "{fires}");

    let mut stmt = conn
        .prepare("SELECT args_summary FROM command_log WHERE command = 'hook-refresh'")
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for raw in &rows {
        let parsed: Value = serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("every row the writer makes is JSON: {e} in {raw}"));
        let hook = parsed.get("hook").and_then(Value::as_str).unwrap();
        assert!(
            cort::hook::REFRESH_OUTCOMES.contains(&hook),
            "an unlisted outcome means the vocabulary const rotted: {hook}"
        );
    }

    let census = cort::usage::hook_census_at(&usage_db, "hook-refresh", 0).expect("read usage db");
    let total: i64 = census.values().filter_map(|v| v.as_i64()).sum();
    assert_eq!(total, fires, "the census must close: {census:?}");
    assert!(census.contains_key("no_index"), "{census:?}");
    if indexed {
        assert!(census.contains_key("already_current"), "{census:?}");
        assert!(census.contains_key("refreshed"), "{census:?}");
    }
    assert!(
        !census.keys().any(|k| k.starts_with("unknown/")),
        "the refresh vocabulary must already cover what this binary emits: {census:?}"
    );
}

/// `help` is a help request only where a command word can be: first, or right after `memory`.
/// Anywhere else it is a value -- `cort memory recall help` searches the ledger for the topic
/// "help", and used to print usage instead of running.
#[test]
fn a_query_word_help_is_a_query_and_not_a_request_for_usage() {
    let (_p, cwd, _c, cache) = sandbox();
    let empty_memory = tempfile::tempdir().unwrap();
    let mem = empty_memory.path().to_str().unwrap();
    let r = run_cort_env(
        &["memory", "recall", "help", "--harness", "claude-code"],
        &cwd,
        &cache,
        &[("CORT_MEMORY_DIR", mem)],
    );
    assert!(
        !r.stdout.contains("\"commands\""),
        "recall of the word help printed usage: {}",
        r.stdout
    );
    // It ran the recall: the sandbox project has no origin remote and the memory dir holds no
    // connection, so the verb answers with a memory error -- and never reaches the network.
    assert_eq!(
        payload(&r)["error"],
        "memory_error",
        "stdout={:?}",
        r.stdout
    );
    assert_ne!(r.code, 0);
}
