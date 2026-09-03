//! Plan §10 usage.db — TDD contract tests. Names cite the binding design points.

use cort::db::project_id_for;
use cort::usage::{
    parse_usage_days, query_usage_at, record_command_at, render_usage_lean, CommandRecord,
    DEFAULT_USAGE_DAYS, RETENTION_DAYS,
};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SENTINEL: &str = "PRIVACY_SENTINEL_9f3a";
const NOW_MS: i64 = 1_787_961_600_000; // 2026-08-29 00:00:00 UTC
const DAY_MS: i64 = 86_400_000;

const SAMPLE: &[(&str, &str)] = &[
    (
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    ),
    (
        "src/alpha.ts",
        "import { helper } from './helper';\n\
export function alpha(a: number) { return helper(a) + 1; }\n",
    ),
    (
        "src/utf8.ts",
        "export const greet = '你好'; // quote \" and \\\\ slash\n",
    ),
];

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

fn make_project(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-proj-")
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
        .prefix("cort-usage-cache-")
        .tempdir()
        .unwrap();
    let cache = cache_dir.path().to_path_buf();
    (proj, cwd, cache_dir, cache)
}

fn usage_db(cache: &Path) -> PathBuf {
    cache.join("usage.db")
}

fn project_db(cache: &Path, root: &Path) -> PathBuf {
    cache.join(format!("{}.db", project_id_for(root.to_str().unwrap())))
}

fn cache_names(cache: &Path) -> Vec<String> {
    if !cache.exists() {
        return Vec::new();
    }
    let mut names: Vec<String> = fs::read_dir(cache)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn rec(command: &str) -> CommandRecord {
    CommandRecord {
        now_ms: NOW_MS,
        project_id: None,
        command: command.to_string(),
        args_summary: r#"{"v":1}"#.to_string(),
        status: "ok".to_string(),
        error_code: None,
        read_source: None,
        requested_content_mode: None,
        effective_content_mode: None,
        receipt_hit: None,
        index_stale: None,
        bytes_out: 0,
        saved_bytes: 0,
    }
}

fn open_usage(cache: &Path) -> Connection {
    Connection::open(usage_db(cache)).expect("open usage.db")
}

fn log_rows(cache: &Path) -> Vec<Value> {
    let db = open_usage(cache);
    let mut stmt = db
        .prepare(
            "SELECT id, ts, project_id, command, args_summary, status, error_code,
                    read_source, requested_content_mode, effective_content_mode,
                    receipt_hit, index_stale, bytes_out, saved_bytes
               FROM command_log ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, i64>(0)?,
            "ts": r.get::<_, i64>(1)?,
            "project_id": r.get::<_, Option<String>>(2)?,
            "command": r.get::<_, String>(3)?,
            "args_summary": r.get::<_, String>(4)?,
            "status": r.get::<_, String>(5)?,
            "error_code": r.get::<_, Option<String>>(6)?,
            "read_source": r.get::<_, Option<String>>(7)?,
            "requested_content_mode": r.get::<_, Option<String>>(8)?,
            "effective_content_mode": r.get::<_, Option<String>>(9)?,
            "receipt_hit": r.get::<_, Option<i64>>(10)?,
            "index_stale": r.get::<_, Option<i64>>(11)?,
            "bytes_out": r.get::<_, i64>(12)?,
            "saved_bytes": r.get::<_, i64>(13)?,
        }))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

fn usage_files_contain(cache: &Path, needle: &str) -> bool {
    let needle = needle.as_bytes();
    for name in ["usage.db", "usage.db-wal", "usage.db-shm"] {
        let path = cache.join(name);
        if let Ok(bytes) = fs::read(&path) {
            if bytes.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn insert_log(
    db: &Connection,
    ts: i64,
    project_id: Option<&str>,
    command: &str,
    status: &str,
    bytes_out: i64,
    saved_bytes: i64,
    receipt_hit: Option<i64>,
    index_stale: Option<i64>,
) {
    db.execute(
        "INSERT INTO command_log
            (ts, project_id, command, args_summary, status, error_code,
             read_source, requested_content_mode, effective_content_mode,
             receipt_hit, index_stale, bytes_out, saved_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, ?6, ?7, ?8, ?9)",
        params![
            ts,
            project_id,
            command,
            r#"{"v":1}"#,
            status,
            receipt_hit,
            index_stale,
            bytes_out,
            saved_bytes,
        ],
    )
    .unwrap();
}

fn seed_schema(path: &Path) {
    record_command_at(path, &rec("usage"));
    let db = Connection::open(path).unwrap();
    db.execute("DELETE FROM command_log", []).unwrap();
    let _ = db.execute("DELETE FROM _usage_meta WHERE key = 'LAST_PRUNE_DAY'", []);
}

// --- status / help side effects ------------------------------------------------

/// §10 status read-only: unindexed cwd creates NO project DB.
#[test]
fn unindexed_cwd_status_creates_no_project_db() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["status"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert_eq!(p["indexed"], false);
    assert_eq!(p["project_id"], project_id_for(cwd.to_str().unwrap()));
    assert!(
        !project_db(&cache, &cwd).exists(),
        "status must not create a project DB; cache={:?}",
        cache_names(&cache)
    );
}

/// §10 help/--help/-h stays zero-side-effect: no cache dir, no usage.db.
#[test]
fn help_creates_no_files_at_all() {
    let (_p, cwd, _c, cache) = sandbox();
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["help"],
        vec!["usage", "--help"],
        vec!["usage", "-h"],
        vec!["status", "-h"],
        vec!["index", "--help"],
    ] {
        let r = run_cort(&args, &cwd, &cache);
        assert_eq!(r.code, 0, "{} stderr={}", args.join(" "), r.stderr);
        assert_eq!(
            cache_names(&cache),
            Vec::<String>::new(),
            "{} must not create files, got {:?}",
            args.join(" "),
            cache_names(&cache)
        );
    }
}

/// §10 usage works from an unindexed cwd (no project DB created).
#[test]
fn usage_works_from_unindexed_cwd() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["usage"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert_eq!(p["best_effort"], true);
    assert_eq!(p["days"], DEFAULT_USAGE_DAYS);
    assert!(
        !project_db(&cache, &cwd).exists(),
        "usage must not create a project DB; cache={:?}",
        cache_names(&cache)
    );
}

/// §10 multi-project rows aggregate in one central report.
#[test]
fn multi_project_rows_aggregate_in_one_report() {
    let cache_dir = tempfile::Builder::new()
        .prefix("cort-usage-multi-")
        .tempdir()
        .unwrap();
    let cache = cache_dir.path();
    let (_a, root_a) = make_project(SAMPLE);
    let (_b, root_b) = make_project(SAMPLE);
    assert_eq!(run_cort(&["index"], &root_a, cache).code, 0);
    assert_eq!(run_cort(&["index"], &root_b, cache).code, 0);
    assert_eq!(run_cort(&["status"], &root_a, cache).code, 0);
    assert_eq!(run_cort(&["status"], &root_b, cache).code, 0);
    let r = run_cort(&["usage"], &root_a, cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    let id_a = project_id_for(root_a.to_str().unwrap());
    let id_b = project_id_for(root_b.to_str().unwrap());
    assert!(p["projects"][&id_a].is_object(), "missing {id_a} in {}", p);
    assert!(p["projects"][&id_b].is_object(), "missing {id_b} in {}", p);
    assert_ne!(id_a, id_b);
}

/// §10 deleted project's history survives `cort delete`; delete does not recreate its DB.
#[test]
fn deleted_project_history_survives_delete_and_delete_does_not_recreate_db() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let pid = project_id_for(cwd.to_str().unwrap());
    let pdb = project_db(&cache, &cwd);
    assert!(pdb.exists());
    let del = run_cort(&["delete"], &cwd, &cache);
    assert_eq!(del.code, 0, "stderr={}", del.stderr);
    assert!(!pdb.exists(), "delete must remove the project DB");
    let r = run_cort(&["usage"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert!(
        p["projects"][&pid].is_object(),
        "history for deleted project {pid} must survive; got {p}"
    );
    assert!(
        !pdb.exists(),
        "delete / usage must not recreate the project DB"
    );
}

// --- recorder isolation -------------------------------------------------------

fn projects_stdout_baseline(cwd: &Path, cache: &Path) -> Run {
    let r = run_cort(&["projects"], cwd, cache);
    assert_eq!(r.code, 0, "baseline projects failed: {}", r.stderr);
    r
}

/// §10 recorder isolation: busy usage.db must not change command stdout/exit.
#[test]
fn recorder_isolation_busy_leaves_command_stdout_and_exit_unchanged() {
    let (_p, cwd, _c, cache) = sandbox();
    let baseline = projects_stdout_baseline(&cwd, &cache);
    record_command_at(&usage_db(&cache), &rec("projects"));
    let lock = Connection::open(usage_db(&cache)).unwrap();
    lock.busy_timeout(std::time::Duration::from_millis(0))
        .unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let r = run_cort(&["projects"], &cwd, &cache);
    drop(lock);
    assert_eq!(r.code, baseline.code);
    assert_eq!(r.stdout, baseline.stdout);
}

/// §10 recorder isolation: read-only usage.db.
#[test]
fn recorder_isolation_read_only_leaves_command_stdout_and_exit_unchanged() {
    let (_p, cwd, _c, cache) = sandbox();
    let baseline = projects_stdout_baseline(&cwd, &cache);
    record_command_at(&usage_db(&cache), &rec("projects"));
    let mut perms = fs::metadata(usage_db(&cache)).unwrap().permissions();
    perms.set_mode(0o444);
    fs::set_permissions(usage_db(&cache), perms).unwrap();
    let r = run_cort(&["projects"], &cwd, &cache);
    let mut restore = fs::metadata(usage_db(&cache)).unwrap().permissions();
    restore.set_mode(0o600);
    let _ = fs::set_permissions(usage_db(&cache), restore);
    assert_eq!(r.code, baseline.code);
    assert_eq!(r.stdout, baseline.stdout);
}

/// §10 recorder isolation: corrupt usage.db.
#[test]
fn recorder_isolation_corrupt_leaves_command_stdout_and_exit_unchanged() {
    let (_p, cwd, _c, cache) = sandbox();
    let baseline = projects_stdout_baseline(&cwd, &cache);
    fs::write(usage_db(&cache), b"not a sqlite database").unwrap();
    let r = run_cort(&["projects"], &cwd, &cache);
    assert_eq!(r.code, baseline.code);
    assert_eq!(r.stdout, baseline.stdout);
}

/// §10 recorder isolation: SQLITE_FULL is absorbed, no panic, stdout/exit unchanged.
#[test]
fn recorder_isolation_sqlite_full_leaves_command_stdout_and_exit_unchanged() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-full-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let filler = Connection::open(&path).unwrap();
    filler.pragma_update(None, "max_page_count", 4).unwrap();
    let blob = "x".repeat(2000);
    let mut last_count = -1i64;
    for _ in 0..40 {
        let mut row = rec("fill");
        row.args_summary = format!(r#"{{"v":1,"symbol":"{blob}"}}"#);
        row.bytes_out = 1;
        record_command_at(&path, &row);
        let count: i64 = filler
            .query_row("SELECT COUNT(*) FROM command_log", [], |r| r.get(0))
            .unwrap_or(0);
        if count > 20 || count == last_count {
            break;
        }
        last_count = count;
        let page_count: i64 = filler
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        let max_page: i64 = filler
            .query_row("PRAGMA max_page_count", [], |r| r.get(0))
            .unwrap_or(0);
        if page_count >= max_page && max_page > 0 {
            break;
        }
        if count == 0 {
            filler.pragma_update(None, "max_page_count", 2).unwrap();
        }
    }
    let before: i64 = filler
        .query_row("SELECT COUNT(*) FROM command_log", [], |r| r.get(0))
        .unwrap_or(0);
    let mut row = rec("projects");
    row.bytes_out = 99;
    record_command_at(&path, &row);
    let after: i64 = filler
        .query_row("SELECT COUNT(*) FROM command_log", [], |r| r.get(0))
        .unwrap_or(before);
    assert!(
        after == before || after == before + 1,
        "SQLITE_FULL must not panic; before={before} after={after}"
    );
}

/// §10 recorder isolation: mkdir-fail is absorbed.
#[test]
fn recorder_isolation_mkdir_fail_leaves_command_stdout_and_exit_unchanged() {
    let (_p, cwd, _c, _cache) = sandbox();
    let tmp = tempfile::Builder::new()
        .prefix("cort-usage-mkdir-")
        .tempdir()
        .unwrap();
    let blocker = tmp.path().join("not-a-dir");
    fs::write(&blocker, b"file").unwrap();
    let baseline = run_cort(&["projects"], &cwd, &blocker);
    assert_eq!(baseline.code, 0, "stderr={}", baseline.stderr);
    let again = run_cort(&["projects"], &cwd, &blocker);
    assert_eq!(again.code, baseline.code);
    assert_eq!(again.stdout, baseline.stdout);
}

/// §10 prune failure does not roll back the insert.
#[test]
fn prune_failure_does_not_roll_back_the_insert() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-prune-fail-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    insert_log(
        &db,
        NOW_MS - (RETENTION_DAYS + 1) * DAY_MS,
        None,
        "old",
        "ok",
        1,
        0,
        None,
        None,
    );
    db.execute_batch(
        "CREATE TRIGGER no_delete BEFORE DELETE ON command_log BEGIN
            SELECT RAISE(ABORT, 'prune blocked');
         END;",
    )
    .unwrap();
    let mut row = rec("fresh");
    row.bytes_out = 7;
    record_command_at(&path, &row);
    let commands: Vec<String> = db
        .prepare("SELECT command FROM command_log ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        commands.iter().any(|c| c == "fresh"),
        "insert must survive prune failure: {commands:?}"
    );
    assert!(
        commands.iter().any(|c| c == "old"),
        "expired row remains when prune fails: {commands:?}"
    );
}

/// §10 existing busy/corrupt usage.db is a structured error for the query itself.
#[test]
fn usage_query_busy_or_corrupt_is_structured_error() {
    let (_p, cwd, _c, cache) = sandbox();
    fs::write(usage_db(&cache), b"not a sqlite database").unwrap();
    let corrupt = run_cort(&["usage"], &cwd, &cache);
    assert_eq!(corrupt.code, 1, "stdout={}", corrupt.stdout);
    let p = payload(&corrupt);
    assert_eq!(p["error"], "usage_corrupt");

    fs::remove_file(usage_db(&cache)).ok();
    record_command_at(&usage_db(&cache), &rec("usage"));
    let lock = Connection::open(usage_db(&cache)).unwrap();
    lock.busy_timeout(std::time::Duration::from_millis(0))
        .unwrap();
    lock.pragma_update(None, "locking_mode", "EXCLUSIVE")
        .unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    lock.execute(
        "UPDATE command_log SET bytes_out = bytes_out WHERE id = id",
        [],
    )
    .unwrap();
    let busy = run_cort(&["usage"], &cwd, &cache);
    drop(lock);
    assert_eq!(busy.code, 1, "stdout={}", busy.stdout);
    assert_eq!(payload(&busy)["error"], "usage_busy");
}

// --- retention / days ---------------------------------------------------------

/// §10 days strict integer 1..=90 (cap == retention).
#[test]
fn usage_days_boundaries_1_30_89_90_pass() {
    let (_p, cwd, _c, cache) = sandbox();
    for days in ["1", "30", "89", "90"] {
        let r = run_cort(&["usage", days], &cwd, &cache);
        assert_eq!(r.code, 0, "days={days} stderr={}", r.stderr);
        assert_eq!(payload(&r)["days"], days.parse::<i64>().unwrap());
    }
    assert_eq!(parse_usage_days(None).unwrap(), 30);
    assert_eq!(parse_usage_days(Some("1")).unwrap(), 1);
    assert_eq!(parse_usage_days(Some("90")).unwrap(), 90);
}

/// §10 reject 0 / negative / float / non-numeric / 366 with a structured error.
#[test]
fn usage_days_0_negative_float_non_numeric_366_are_structured_errors() {
    let (_p, cwd, _c, cache) = sandbox();
    for days in ["0", "-1", "1.5", "abc", "366"] {
        let r = run_cort(&["usage", days], &cwd, &cache);
        assert_eq!(r.code, 1, "days={days} stdout={}", r.stdout);
        let p = payload(&r);
        assert_eq!(p["error"], "invalid_usage_days", "days={days} {p}");
        assert_eq!(p["detail"]["provided"], days);
    }
    for raw in [Some("0"), Some("-1"), Some("1.5"), Some("abc"), Some("366")] {
        let err = parse_usage_days(raw).unwrap_err();
        assert_eq!(err.code, "invalid_usage_days");
    }
}

/// §10 queries: ts < cutoff excluded, ts == cutoff kept.
#[test]
fn retention_ts_before_cutoff_excluded_ts_equal_cutoff_kept() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-cutoff-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    let cutoff = NOW_MS - 30 * DAY_MS;
    insert_log(&db, cutoff - 1, None, "old", "ok", 11, 0, None, None);
    insert_log(&db, cutoff, None, "edge", "ok", 22, 0, None, None);
    insert_log(&db, cutoff + 1, None, "new", "ok", 33, 0, None, None);
    let report = query_usage_at(&path, 30, NOW_MS).unwrap();
    assert_eq!(report["commands"]["old"], Value::Null);
    assert_eq!(report["commands"]["edge"]["ok"], 1);
    assert_eq!(report["commands"]["edge"]["bytes_out"], 22);
    assert_eq!(report["commands"]["new"]["ok"], 1);
    assert_eq!(report["commands"]["new"]["bytes_out"], 33);
}

/// §10 unpruned expired rows are still excluded by queries.
#[test]
fn unpruned_expired_rows_still_excluded_by_queries() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-unpruned-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    insert_log(
        &db,
        NOW_MS - (RETENTION_DAYS + 5) * DAY_MS,
        None,
        "expired",
        "ok",
        99,
        0,
        None,
        None,
    );
    db.execute(
        "INSERT INTO _usage_meta(key, value) VALUES ('LAST_PRUNE_DAY', '2026-08-29')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )
    .ok();
    let report = query_usage_at(&path, 90, NOW_MS).unwrap();
    assert_eq!(report["commands"].get("expired"), None);
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE command='expired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "row remains on disk until prune");
}

/// §10 prune at most once per day (throttle row).
#[test]
fn daily_prune_throttle_runs_at_most_once_per_day() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-throttle-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    insert_log(
        &db,
        NOW_MS - (RETENTION_DAYS + 1) * DAY_MS,
        None,
        "expired_a",
        "ok",
        1,
        0,
        None,
        None,
    );
    let mut first = rec("first");
    first.bytes_out = 1;
    record_command_at(&path, &first);
    let leftover_a: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE command='expired_a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        leftover_a, 0,
        "first prune of the day must delete expired_a"
    );
    insert_log(
        &db,
        NOW_MS - (RETENTION_DAYS + 1) * DAY_MS,
        None,
        "expired_b",
        "ok",
        1,
        0,
        None,
        None,
    );
    let mut second = rec("second");
    second.bytes_out = 2;
    record_command_at(&path, &second);
    let leftover_b: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE command='expired_b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover_b, 1, "second prune same day must be throttled");
}

/// §10 clock going backwards does not mass-prune.
#[test]
fn clock_going_backwards_does_not_mass_prune() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-clock-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    db.execute(
        "INSERT INTO _usage_meta(key, value) VALUES ('LAST_PRUNE_DAY', '2099-01-01')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )
    .unwrap();
    insert_log(
        &db,
        NOW_MS - (RETENTION_DAYS + 1) * DAY_MS,
        None,
        "expired",
        "ok",
        1,
        0,
        None,
        None,
    );
    record_command_at(&path, &rec("now"));
    let leftover: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE command='expired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover, 1, "future LAST_PRUNE_DAY must skip prune");
}

// --- bytes / receipt / stale --------------------------------------------------

fn stored_body_len(cache: &Path, root: &Path, file: &str) -> i64 {
    let db = Connection::open(project_db(cache, root)).unwrap();
    db.query_row(
        "SELECT LENGTH(content) FROM reading_notes WHERE file_path = ?1 LIMIT 1",
        params![file],
        |r| r.get(0),
    )
    .unwrap()
}

/// §10 bytes_out is exact UTF-8 length for json AND lean, including multi-byte chars and escaping.
#[test]
fn bytes_out_is_exact_utf8_len_for_json_and_lean_including_multibyte_and_escaping() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let json_r = run_cort(&["read", "src/utf8.ts"], &cwd, &cache);
    assert_eq!(json_r.code, 0, "stderr={}", json_r.stderr);
    let json_bytes = json_r.stdout.len() as i64;
    let lean_r = run_cort(
        &["read", "src/utf8.ts", "--content", "full", "-f", "lean"],
        &cwd,
        &cache,
    );
    assert_eq!(lean_r.code, 0, "stderr={}", lean_r.stderr);
    assert!(
        lean_r.stdout.contains("你好"),
        "lean must keep multi-byte chars: {}",
        lean_r.stdout
    );
    let lean_bytes = lean_r.stdout.len() as i64;
    let rows = log_rows(&cache);
    let json_row = rows
        .iter()
        .find(|r| {
            r["command"] == "read"
                && r["effective_content_mode"] == "full"
                && r["bytes_out"] == json_bytes
        })
        .or_else(|| rows.iter().find(|r| r["command"] == "read"));
    assert!(json_row.is_some(), "missing read row: {rows:?}");
    let read_rows: Vec<&Value> = rows.iter().filter(|r| r["command"] == "read").collect();
    assert!(
        read_rows.iter().any(|r| r["bytes_out"] == json_bytes),
        "json bytes_out wanted {json_bytes}, rows={read_rows:?}"
    );
    assert!(
        read_rows.iter().any(|r| r["bytes_out"] == lean_bytes),
        "lean bytes_out wanted {lean_bytes}, rows={read_rows:?}"
    );
}

/// §10 first auto read filesystem/full -> saved_bytes=0.
#[test]
fn first_auto_read_filesystem_full_saved_bytes_is_zero() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let r = run_cort(&["read", "src/helper.ts"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert_eq!(p["source"], "filesystem");
    assert_eq!(p["content_mode"], "full");
    let rows = log_rows(&cache);
    let row = rows.iter().find(|r| r["command"] == "read").unwrap();
    assert_eq!(row["saved_bytes"], 0);
    assert_eq!(row["read_source"], "filesystem");
    assert_eq!(row["requested_content_mode"], "auto");
    assert_eq!(row["effective_content_mode"], "full");
    assert_eq!(row["receipt_hit"], 0);
    assert_eq!(row["bytes_out"], r.stdout.len() as i64);
}

/// §10 second auto store/receipt -> saved_bytes == stored body byte len.
#[test]
fn second_auto_store_receipt_saved_bytes_equals_stored_body_byte_len() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    assert_eq!(run_cort(&["read", "src/helper.ts"], &cwd, &cache).code, 0);
    let body_len = stored_body_len(&cache, &cwd, "src/helper.ts");
    assert!(body_len > 0);
    let r = run_cort(&["read", "src/helper.ts"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert_eq!(p["source"], "store");
    assert_eq!(p["content_mode"], "receipt");
    let rows = log_rows(&cache);
    let row = rows
        .iter()
        .rev()
        .find(|r| r["command"] == "read" && r["read_source"] == "store")
        .unwrap();
    assert_eq!(row["saved_bytes"], body_len);
    assert_eq!(row["effective_content_mode"], "receipt");
    assert_eq!(row["receipt_hit"], 1);
    assert_eq!(row["bytes_out"], r.stdout.len() as i64);
}

/// §10 store + explicit full -> saved_bytes=0.
#[test]
fn store_plus_explicit_full_saved_bytes_is_zero() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    assert_eq!(run_cort(&["read", "src/helper.ts"], &cwd, &cache).code, 0);
    let r = run_cort(
        &["read", "src/helper.ts", "--content", "full"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    assert_eq!(payload(&r)["content_mode"], "full");
    let rows = log_rows(&cache);
    let row = rows
        .iter()
        .rev()
        .find(|r| r["command"] == "read" && r["requested_content_mode"] == "full")
        .unwrap();
    assert_eq!(row["saved_bytes"], 0);
    assert_eq!(row["receipt_hit"], Value::Null);
}

/// §10 explicit receipt first read -> saved_bytes=0.
#[test]
fn explicit_receipt_first_read_saved_bytes_is_zero() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let r = run_cort(
        &["read", "src/helper.ts", "--content", "receipt"],
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert_eq!(p["source"], "filesystem");
    assert_eq!(p["content_mode"], "receipt");
    let rows = log_rows(&cache);
    let row = rows.iter().find(|r| r["command"] == "read").unwrap();
    assert_eq!(row["saved_bytes"], 0);
    assert_eq!(row["receipt_hit"], Value::Null);
}

/// §10 error path: bytes_out == rendered error bytes, error_code is CortError code only.
#[test]
fn error_response_bytes_out_equals_rendered_error_bytes() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["context"], &cwd, &cache);
    assert_eq!(r.code, 1);
    let p = payload(&r);
    assert_eq!(p["error"], "missing_query");
    let rows = log_rows(&cache);
    let row = rows.iter().find(|r| r["command"] == "context").unwrap();
    assert_eq!(row["status"], "error");
    assert_eq!(row["error_code"], "missing_query");
    assert_eq!(row["bytes_out"], r.stdout.len() as i64);
}

/// §10 receipt rate denominator = successful auto reads only (explicit full/receipt excluded).
#[test]
fn receipt_rate_denominator_not_polluted_by_explicit_full_or_receipt() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    assert_eq!(run_cort(&["read", "src/helper.ts"], &cwd, &cache).code, 0);
    assert_eq!(run_cort(&["read", "src/helper.ts"], &cwd, &cache).code, 0);
    assert_eq!(
        run_cort(
            &["read", "src/helper.ts", "--content", "full"],
            &cwd,
            &cache
        )
        .code,
        0
    );
    assert_eq!(
        run_cort(
            &["read", "src/helper.ts", "--content", "receipt"],
            &cwd,
            &cache
        )
        .code,
        0
    );
    let r = run_cort(&["usage"], &cwd, &cache);
    assert_eq!(r.code, 0, "stderr={}", r.stderr);
    let p = payload(&r);
    assert_eq!(p["commands"]["read"]["receipt_hit_rate"], 0.5);
}

/// §10 stale tri-state: true / false / not-evaluated(NULL) are distinct.
#[test]
fn stale_tristate_true_false_and_null_not_evaluated_are_distinct() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-stale-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    insert_log(&db, NOW_MS, Some("p"), "status", "ok", 1, 0, None, Some(1));
    insert_log(&db, NOW_MS, Some("p"), "status", "ok", 1, 0, None, Some(0));
    insert_log(&db, NOW_MS, Some("p"), "status", "ok", 1, 0, None, None);
    let report = query_usage_at(&path, 30, NOW_MS).unwrap();
    assert_eq!(report["commands"]["status"]["stale_evaluated"], 2);
    assert_eq!(report["commands"]["status"]["stale_true"], 1);
    assert_eq!(report["commands"]["status"]["ok"], 3);
}

/// §10 unindexed status does not count as stale.
#[test]
fn unindexed_status_does_not_count_as_stale() {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["status"], &cwd, &cache).code, 0);
    let rows = log_rows(&cache);
    let row = rows.iter().find(|r| r["command"] == "status").unwrap();
    assert_eq!(row["index_stale"], Value::Null);
    let r = run_cort(&["usage"], &cwd, &cache);
    let p = payload(&r);
    assert_eq!(p["commands"]["status"]["stale_evaluated"], 0);
    assert_eq!(p["commands"]["status"]["stale_true"], 0);
}

// --- privacy / distribution / golden ------------------------------------------

/// §10 privacy: sentinel in context query, recall query, struct pattern, unknown flag, clap error
/// appears NOWHERE in usage.db; only allowlisted fields in args_summary; no home paths.
#[test]
fn privacy_sentinels_from_context_recall_struct_unknown_flag_and_clap_error_are_absent_from_usage_db(
) {
    let (_p, cwd, _c, cache) = sandbox();
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let _ = run_cort(&["context", SENTINEL], &cwd, &cache);
    let _ = run_cort(&["recall", SENTINEL], &cwd, &cache);
    let _ = run_cort(&["struct", "-p", SENTINEL, "--lang", "ts"], &cwd, &cache);
    let flag = format!("--{SENTINEL}");
    let _ = run_cort(&["read", flag.as_str(), "src/helper.ts"], &cwd, &cache);
    let _ = run_cort(
        &["read", "src/helper.ts", "--start", SENTINEL],
        &cwd,
        &cache,
    );
    let abs = cwd.join("src/helper.ts");
    let _ = run_cort(&["read", abs.to_str().unwrap()], &cwd, &cache);
    assert!(
        !usage_files_contain(&cache, SENTINEL),
        "sentinel leaked into usage db: {:?}",
        log_rows(&cache)
    );
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        assert!(
            !usage_files_contain(&cache, &home),
            "home path leaked into usage db"
        );
    }
    for row in log_rows(&cache) {
        let summary: Value = serde_json::from_str(row["args_summary"].as_str().unwrap()).unwrap();
        let obj = summary.as_object().unwrap();
        for key in obj.keys() {
            assert!(
                matches!(key.as_str(), "v" | "symbol" | "path" | "start" | "end"),
                "non-allowlisted key {key} in {summary}"
            );
        }
        if let Some(path) = obj.get("path").and_then(Value::as_str) {
            assert!(
                !path.starts_with('/'),
                "path must be project-relative: {path}"
            );
            assert!(!path.contains(&home) || home.is_empty());
        }
    }
}

/// §10 _global (NULL project_id) vs _unknown (non-canonicalizable) stay separated.
#[test]
fn global_vs_unknown_project_distribution_are_separated() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-dist-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    insert_log(&db, NOW_MS, None, "projects", "ok", 5, 0, None, None);
    insert_log(
        &db,
        NOW_MS,
        Some("_unknown"),
        "status",
        "error",
        6,
        0,
        None,
        None,
    );
    insert_log(&db, NOW_MS, Some("abc"), "index", "ok", 7, 0, None, None);
    let report = query_usage_at(&path, 30, NOW_MS).unwrap();
    assert_eq!(report["projects"]["_global"]["ok"], 1);
    assert_eq!(report["projects"]["_global"]["bytes_out"], 5);
    assert_eq!(report["projects"]["_unknown"]["error"], 1);
    assert_eq!(report["projects"]["_unknown"]["bytes_out"], 6);
    assert_eq!(report["projects"]["abc"]["ok"], 1);
    assert_ne!(
        report["projects"]["_global"],
        report["projects"]["_unknown"]
    );
}

/// §10 missing usage.db / empty DB → stable all-zero report.
#[test]
fn empty_db_yields_stable_zero_report() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-empty-")
        .tempdir()
        .unwrap();
    let missing = dir.path().join("nope.db");
    let a = query_usage_at(&missing, 30, NOW_MS).unwrap();
    let b = query_usage_at(&missing, 30, NOW_MS).unwrap();
    assert_eq!(a, b);
    assert_eq!(a["best_effort"], true);
    assert_eq!(a["days"], 30);
    assert_eq!(a["commands"], serde_json::json!({}));
    assert_eq!(a["projects"], serde_json::json!({}));
    assert!(a["note"]
        .as_str()
        .unwrap()
        .contains("raw body bytes omitted"));
    assert!(
        !a["note"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("total-output savings")
            || a["note"]
                .as_str()
                .unwrap()
                .contains("not total-output savings")
    );
}

/// §10 cort usage aggregates FIRST then records itself — current call absent, next sees it.
#[test]
fn current_usage_call_absent_from_own_report_but_present_in_the_next() {
    let (_p, cwd, _c, cache) = sandbox();
    let first = run_cort(&["usage"], &cwd, &cache);
    assert_eq!(first.code, 0, "stderr={}", first.stderr);
    let p1 = payload(&first);
    assert_eq!(p1["commands"].get("usage"), None);
    let second = run_cort(&["usage"], &cwd, &cache);
    assert_eq!(second.code, 0, "stderr={}", second.stderr);
    let p2 = payload(&second);
    assert_eq!(p2["commands"]["usage"]["ok"], 1);
}

/// §10 golden json + lean snapshots.
#[test]
fn golden_json_and_lean_snapshots() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-golden-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let db = Connection::open(&path).unwrap();
    insert_log(
        &db,
        NOW_MS,
        Some("aaa"),
        "read",
        "ok",
        100,
        50,
        Some(1),
        None,
    );
    insert_log(
        &db,
        NOW_MS,
        Some("aaa"),
        "read",
        "ok",
        200,
        0,
        Some(0),
        None,
    );
    insert_log(&db, NOW_MS, None, "status", "ok", 10, 0, None, None);
    insert_log(
        &db,
        NOW_MS,
        Some("_unknown"),
        "status",
        "error",
        20,
        0,
        None,
        None,
    );
    let mut report = query_usage_at(&path, 30, NOW_MS).unwrap();
    // The machine block is the one part of this report that is different on every machine, which is
    // its whole purpose. Pinned to sentinels here so the golden stays a statement about the
    // aggregation; `a_report_names_the_machine_and_says_when_a_db_holds_two` owns the real thing.
    for k in ["id", "source", "db_created_on", "db_created_on_source"] {
        report["machine"][k] = serde_json::json!(format!("<{k}>"));
    }
    let json = format!("{}\n", serde_json::to_string_pretty(&report).unwrap());
    const GOLDEN_JSON: &str = r#"{
  "best_effort": true,
  "commands": {
    "read": {
      "bytes_out": 300,
      "error": 0,
      "ok": 2,
      "receipt_hit_rate": 0.5,
      "saved_bytes": 50,
      "stale_evaluated": 0,
      "stale_true": 0
    },
    "status": {
      "bytes_out": 30,
      "error": 1,
      "ok": 1,
      "receipt_hit_rate": null,
      "saved_bytes": 0,
      "stale_evaluated": 0,
      "stale_true": 0
    }
  },
  "days": 30,
  "machine": {
    "db_created_on": "<db_created_on>",
    "db_created_on_source": "<db_created_on_source>",
    "id": "<id>",
    "mixed": false,
    "source": "<source>"
  },
  "note": "saved_bytes is raw body bytes omitted, not total-output savings",
  "projects": {
    "_global": {
      "bytes_out": 10,
      "error": 0,
      "ok": 1,
      "saved_bytes": 0
    },
    "_unknown": {
      "bytes_out": 20,
      "error": 1,
      "ok": 0,
      "saved_bytes": 0
    },
    "aaa": {
      "bytes_out": 300,
      "error": 0,
      "ok": 2,
      "saved_bytes": 50
    }
  }
}
"#;
    assert_eq!(json, GOLDEN_JSON);

    const GOLDEN_LEAN: &str = "\
# usage days=30 best_effort=true
# machine=<id> source=<source>
# saved_bytes is raw body bytes omitted, not total-output savings
read\tok=2 error=0 bytes_out=300 saved_bytes=50 receipt_hit_rate=0.5 stale=0/0
status\tok=1 error=1 bytes_out=30 saved_bytes=0 receipt_hit_rate=- stale=0/0
# projects
_global\tok=1 error=0 bytes_out=10 saved_bytes=0
_unknown\tok=0 error=1 bytes_out=20 saved_bytes=0
aaa\tok=2 error=0 bytes_out=300 saved_bytes=50
";
    assert_eq!(render_usage_lean(&report), GOLDEN_LEAN);
}

/// Recorded now_ms is UTC unix ms (smoke around wall clock).
#[test]
fn recorded_ts_is_utc_unix_ms() {
    let (_p, cwd, _c, cache) = sandbox();
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert_eq!(run_cort(&["projects"], &cwd, &cache).code, 0);
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let rows = log_rows(&cache);
    let row = rows.iter().find(|r| r["command"] == "projects").unwrap();
    let ts = row["ts"].as_i64().unwrap();
    assert!(
        ts >= before - 1000 && ts <= after + 1000,
        "ts={ts} before={before} after={after}"
    );
}

/// A report says which machine its rows came from, and shouts when one file holds two.
///
/// Numbers out of this database end up quoted in documents. Two machines averaged into one figure
/// is wrong in a way that cannot be recovered once the figure is written down -- the rows carry no
/// machine of their own, so after the fact nothing can separate them. All the report can honestly
/// do is refuse to be quiet about it, which is why the warning is a header line and not a footnote.
#[test]
fn a_report_names_the_machine_and_says_when_a_db_holds_two() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-machine-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);

    // A database this machine created: stamped with us, and nothing to disagree with.
    let report = query_usage_at(&path, 30, NOW_MS).unwrap();
    let m = &report["machine"];
    let here = cort::usage::machine_id();
    assert_eq!(m["id"].as_str(), Some(here));
    assert_eq!(m["db_created_on"].as_str(), Some(here));
    assert_eq!(m["mixed"].as_bool(), Some(false));
    assert_ne!(
        m["source"].as_str(),
        Some(""),
        "the source is recorded so a reader knows how much the id is worth"
    );
    assert!(
        !render_usage_lean(&report).contains("mixed_machines"),
        "a single-machine database must not carry the warning"
    );
    assert!(
        render_usage_lean(&report).contains(&format!("# machine={here}")),
        "the lean header is the line people paste into documents"
    );

    // Now the same file as it would look carried to a second machine: the stamp names the machine
    // that created it and we are not that machine.
    let db = Connection::open(&path).unwrap();
    db.execute(
        "INSERT INTO _usage_meta (key, value) VALUES ('MACHINE_ID', 'aaaaaaaaaaaaaaaa')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )
    .unwrap();
    drop(db);

    let report = query_usage_at(&path, 30, NOW_MS).unwrap();
    assert_eq!(report["machine"]["mixed"].as_bool(), Some(true));
    assert_eq!(
        report["machine"]["db_created_on"].as_str(),
        Some("aaaaaaaaaaaaaaaa")
    );
    assert_eq!(report["machine"]["id"].as_str(), Some(here));
    let lean = render_usage_lean(&report);
    assert!(
        lean.contains("WARNING mixed_machines"),
        "a file holding two machines must say so in the header: {lean}"
    );

    // And the stamp is never rewritten -- a database that keeps naming its origin is what makes the
    // disagreement visible at all. Reopening for a write must not quietly adopt this machine.
    record_command_at(&path, &rec("usage"));
    let after = query_usage_at(&path, 30, NOW_MS).unwrap();
    assert_eq!(
        after["machine"]["db_created_on"].as_str(),
        Some("aaaaaaaaaaaaaaaa"),
        "writing from a second machine overwrote the stamp, hiding the mixture"
    );
    assert_eq!(after["machine"]["mixed"].as_bool(), Some(true));
}

/// The id is derived, so deleting the cache does not invent a second machine.
#[test]
fn the_machine_id_is_stable_across_a_deleted_database() {
    let dir = tempfile::Builder::new()
        .prefix("cort-usage-machine2-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("usage.db");
    seed_schema(&path);
    let first = query_usage_at(&path, 30, NOW_MS).unwrap()["machine"]["db_created_on"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(&path).unwrap();
    seed_schema(&path);
    let second = query_usage_at(&path, 30, NOW_MS).unwrap()["machine"]["db_created_on"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        first, second,
        "a regenerated id would read as a second machine, which is the confusion this removes"
    );
}

/// The id is a hash, never the machine's name -- the same discipline `project_id` keeps.
#[test]
fn the_machine_id_never_carries_a_hostname() {
    let id = cort::usage::machine_id();
    if id == "unknown" {
        return; // No stable source on this host; `unknown` is the honest answer, not a leak.
    }
    assert_eq!(id.len(), 16, "expected 16 hex chars, got {id:?}");
    assert!(
        id.bytes().all(|b| b.is_ascii_hexdigit()),
        "not a hash: {id:?}"
    );
    if let Ok(host) = std::env::var("HOSTNAME") {
        if !host.is_empty() {
            assert!(!id.contains(&host), "the hostname leaked into the id");
        }
    }
}
