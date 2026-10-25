//! Task E of `docs/superpowers/plans/2026-10-24-event-driven-collection.md`: the per-row health
//! worker behind `cort measure health` (the CLI half is E2).
//!
//! `run_health` reads the `command_log` rows nobody has looked at yet, observes one index at a
//! time, and appends to `index_health_log` only when what it saw differs from this machine's last
//! row for that project -- or when the UTC day did. `list_projects` and `health::observe` read the
//! process-wide `CORT_CACHE_DIR`, so every test holds `ENV_LOCK` for its whole body and points the
//! environment at its own temporary cache and data directories, exactly as
//! `tests/health_observe.rs` does; `KickPaths` is then built from those same paths by hand.

use cort::collect::{CollectConfig, HealthState, KickPaths, Lane, Spawner};
use cort::db::project_id_for;
use cort::health::{run_health, HealthRun};
use cort::usage::{health_rows_after_at, record_command_at, CommandRecord, HealthRow};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

const DAY_MS: i64 = 86_400_000;
const NOW: i64 = 1_800_000_000_000;
/// The machine `run_health` is told about when a test does not say otherwise. It is passed in, not
/// read from the environment, so the history filter is testable without touching `CORT_MACHINE_ID`.
const MACHINE: &str = "machine-a";

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Recorder(Vec<(Lane, String)>);

impl Spawner for Recorder {
    fn spawn(&mut self, lane: Lane, harness: &str) -> std::io::Result<()> {
        self.0.push((lane, harness.to_string()));
        Ok(())
    }
}

fn cfg() -> CollectConfig {
    CollectConfig::from_lookup(|_| None)
}

fn kick_paths(cache: &Path, data: &Path) -> KickPaths {
    KickPaths {
        data_dir: data.to_path_buf(),
        usage_db: cache.join("usage.db"),
        cache_dir: cache.to_path_buf(),
        measure_config: data.join("turso-measure.json"),
    }
}

fn record(usage_db: &Path, project_id: Option<&str>, command: &str, now_ms: i64) {
    record_command_at(
        usage_db,
        &CommandRecord {
            now_ms,
            project_id: project_id.map(str::to_owned),
            command: command.into(),
            args_summary: "{}".into(),
            status: "ok".into(),
            error_code: None,
            read_source: None,
            requested_content_mode: None,
            effective_content_mode: None,
            receipt_hit: None,
            index_stale: None,
            bytes_out: 0,
            saved_bytes: 0,
        },
    );
}

fn health(
    paths: &KickPaths,
    machine: &str,
    now_ms: i64,
    spawn: &mut Recorder,
) -> Result<HealthRun, String> {
    run_health(paths, machine, "test", now_ms, &cfg(), spawn)
}

fn state_of(paths: &KickPaths) -> HealthState {
    HealthState::load(&paths.data_dir.join("measure-health-state.json"))
}

fn rows(usage_db: &Path) -> Vec<HealthRow> {
    health_rows_after_at(usage_db, 0, 100).expect("read the health rows")
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

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
];

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-health-")
        .tempdir()
        .expect("tempdir");
    for (rel, body) in SAMPLE {
        let abs = dir.path().join(rel);
        fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        fs::write(&abs, body).expect("write sample file");
    }
    let root = fs::canonicalize(dir.path()).expect("canonicalize the project");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "t@e.com"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    (dir, root)
}

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

/// `cort index` in the sandbox, exactly as `tests/health_observe.rs` builds one: the child inherits
/// the cache directory this process set, so the binary and `run_health` agree on where it lives.
fn build_index(root: &Path) -> Output {
    let out = Command::new(cort_bin())
        .arg("index")
        .current_dir(root)
        .output()
        .expect("spawn cort index");
    assert!(
        out.status.success(),
        "cort index failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Point the test process's cache and data dirs at fresh temporary directories, run `f`, then put
/// every value back — including on panic, so a failing test cannot poison the ones after it.
fn in_sandbox<R>(cache: &Path, data: &Path, extra: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
    let _guard = env_guard();
    let mut keys: Vec<String> = vec!["CORT_CACHE_DIR".into(), "CORT_MEMORY_DIR".into()];
    keys.extend(extra.iter().map(|(k, _)| (*k).to_string()));
    let prev: Vec<(String, Option<std::ffi::OsString>)> = keys
        .iter()
        .map(|k| (k.clone(), std::env::var_os(k)))
        .collect();
    // SAFETY (edition-2024 terms; in this 2021 crate `set_var` is still a safe fn): every test that
    // changes these variables holds ENV_LOCK for its whole body, and the only code that reads them
    // runs under that lock. Rust reads elsewhere in this binary -- `tempfile` reading TMPDIR,
    // `Command` copying the environment on spawn -- go through std's own environment lock and never
    // see a torn write. What no lock covers is foreign code calling `getenv` on another thread
    // (SQLite's unix VFS can, for temp files); that is the exposure every environment-setting test
    // in this repo already has, not one this file adds.
    unsafe {
        std::env::set_var("CORT_CACHE_DIR", cache);
        std::env::set_var("CORT_MEMORY_DIR", data);
        for (k, v) in extra {
            std::env::set_var(k, v);
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        for (k, v) in &prev {
            match v {
                Some(value) => std::env::set_var(k, value),
                None => std::env::remove_var(k),
            }
        }
    }
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

// ---------------------------------------------------------------------------------------------
// What one pass observes, writes and counts.
// ---------------------------------------------------------------------------------------------

#[test]
fn touched_project_is_observed_and_written() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, Some(&id), "impact", NOW);
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");

        assert_eq!(
            run,
            HealthRun {
                observed: 1,
                written: 1,
                unmapped: 0,
                skipped_busy: false,
            }
        );
        // The handoff runs, but this sandbox has no measure configuration, so `kick_at` answers
        // `not_configured` before it can spawn anything (`handoff_kicks_batch_when_written`).
        assert!(spawn.0.is_empty(), "{:?}", spawn.0);

        let rows = rows(&paths.usage_db);
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert_eq!(row.status, "ok", "{row:?}");
        assert_eq!(row.ts, NOW);
        assert_eq!(row.machine_id, MACHINE);
        assert_eq!(row.machine_source, "test");
        assert_eq!(row.local_project_id, id);
        assert_eq!(row.root.as_deref(), root.to_str(), "{row:?}");
        assert_eq!(row.fresh.as_deref(), Some("fresh"), "{row:?}");
        assert!(row.chunks.unwrap_or(0) >= 1, "{row:?}");
        assert_eq!(row.fts.as_deref(), Some("synced"), "{row:?}");
        // The fixture's repository has no origin remote, so the normalised id is NULL.
        assert_eq!(row.project_id, None, "{row:?}");

        let state = state_of(&paths);
        assert_eq!(state.last_run_ms, Some(NOW));
        // `cort index` wrote a command row of its own, so the exact id is not a number this test
        // can count -- what it must be is the last non-`measure` row read, leaving none behind.
        assert!(state.health_cursor > 0, "{state:?}");
        let (left, _) = cort::usage::touched_project_ids_at(&paths.usage_db, state.health_cursor)
            .expect("read past the cursor");
        assert!(
            left.is_empty(),
            "the cursor covers every row it read: {left:?}"
        );
        assert!(state.instance_id.is_some());
    });
}

// ---------------------------------------------------------------------------------------------
// The write condition: same state on the same UTC day is not rewritten.
// ---------------------------------------------------------------------------------------------

#[test]
fn same_state_same_day_not_rewritten() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        let mut spawn = Recorder::default();
        record(&paths.usage_db, Some(&id), "impact", NOW);
        assert_eq!(health(&paths, MACHINE, NOW, &mut spawn).unwrap().written, 1);

        // A second touch, same day, nothing about the index changed.
        record(&paths.usage_db, Some(&id), "impact", NOW + 60_000);
        let run = health(&paths, MACHINE, NOW + 60_000, &mut spawn).expect("run_health");
        assert_eq!(
            run,
            HealthRun {
                observed: 1,
                written: 0,
                unmapped: 0,
                skipped_busy: false,
            }
        );
        assert_eq!(rows(&paths.usage_db).len(), 1);
    });
}

#[test]
fn same_state_next_day_rewritten() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        let mut spawn = Recorder::default();
        record(&paths.usage_db, Some(&id), "impact", NOW);
        assert_eq!(health(&paths, MACHINE, NOW, &mut spawn).unwrap().written, 1);

        // Same state, a different UTC day: the day is why the row exists, so it is written.
        let tomorrow = NOW + DAY_MS;
        record(&paths.usage_db, Some(&id), "impact", tomorrow);
        let run = health(&paths, MACHINE, tomorrow, &mut spawn).expect("run_health");
        assert_eq!(run.written, 1);
        let rows = rows(&paths.usage_db);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].ts, tomorrow);
        assert_eq!(rows[1].status, "ok", "{rows:?}");
    });
}

#[test]
fn changed_state_rewritten() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        let mut spawn = Recorder::default();
        record(&paths.usage_db, Some(&id), "impact", NOW);
        assert_eq!(health(&paths, MACHINE, NOW, &mut spawn).unwrap().written, 1);

        let edited = root.join("src/alpha.ts");
        let mut body = fs::read_to_string(&edited).expect("read alpha");
        body.push_str("export function gamma() { return 3; }\n");
        fs::write(&edited, body).expect("edit alpha");
        record(&paths.usage_db, Some(&id), "impact", NOW + 1_000);
        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("run_health");

        assert_eq!(run.written, 1);
        let rows = rows(&paths.usage_db);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].fresh.as_deref(), Some("STALE/refresh"), "{rows:?}");
    });
}

// ---------------------------------------------------------------------------------------------
// Mapping a touched project back to a root: live index, history row, or neither.
// ---------------------------------------------------------------------------------------------

#[test]
fn deleted_index_maps_back_via_history() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        let mut spawn = Recorder::default();
        record(&paths.usage_db, Some(&id), "impact", NOW);
        assert_eq!(health(&paths, MACHINE, NOW, &mut spawn).unwrap().written, 1);
        let first = rows(&paths.usage_db).remove(0);

        // The project's index is gone, but this machine's history still names its root, so the
        // observation happens against the directory and records `no_index`.
        for suffix in ["", "-wal", "-shm"] {
            let path = cache.path().join(format!("{id}.db{suffix}"));
            if path.exists() {
                fs::remove_file(&path).expect("delete the index");
            }
        }
        record(&paths.usage_db, Some(&id), "impact", NOW + 1_000);
        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("run_health");
        assert_eq!(run.observed, 1);
        assert_eq!(run.written, 1);

        let rows = rows(&paths.usage_db);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].status, "no_index", "{rows:?}");
        assert_eq!(
            rows[1].root, first.root,
            "the history row supplied the root"
        );
    });
}

#[test]
fn unreadable_entry_recorded_not_unmapped() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        // A garbage `<project_id>.db` is what `list_projects` reports as `Unreadable`.
        fs::write(cache.path().join("unreadable-proj.db"), b"not a database")
            .expect("write garbage");
        record(&paths.usage_db, Some("unreadable-proj"), "impact", NOW);
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");

        assert_eq!(
            run.observed, 1,
            "an unreadable entry is looked at, not observed"
        );
        assert_eq!(run.written, 1);
        assert_eq!(run.unmapped, 0);

        let rows = rows(&paths.usage_db);
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert_eq!(row.status, "unreadable", "{row:?}");
        assert!(
            row.error.as_deref().is_some_and(|e| !e.is_empty()),
            "an unreadable entry must carry the scan's reason: {row:?}"
        );
        assert_eq!(row.root, None, "no history row names a root: {row:?}");
        assert_eq!(row.project_id, None, "{row:?}");
        assert_eq!(row.fresh, None, "{row:?}");
        assert_eq!(row.chunks, None, "{row:?}");
        assert_eq!(row.fts, None, "{row:?}");

        let state = state_of(&paths);
        assert_eq!(state.unmapped_count, 0);
        assert!(state.unmapped_recent.is_empty());
        assert_eq!(state.health_cursor, 1);
    });
}

/// `error` is part of the state the write condition compares: the same `unreadable` status with a
/// different reason, on the same day, is a new row.
#[test]
fn a_different_unreadable_reason_same_day_is_rewritten() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        let index = cache.path().join("reason-proj.db");
        fs::write(&index, b"not a database").expect("write garbage");
        record(&paths.usage_db, Some("reason-proj"), "impact", NOW);
        let mut spawn = Recorder::default();
        assert_eq!(health(&paths, MACHINE, NOW, &mut spawn).unwrap().written, 1);

        // Now a real SQLite file with no `projects` table: still unreadable, for another reason.
        fs::remove_file(&index).expect("remove garbage");
        Connection::open(&index)
            .expect("create sqlite")
            .execute_batch("CREATE TABLE other (x INTEGER);")
            .expect("create a table");
        record(&paths.usage_db, Some("reason-proj"), "impact", NOW + 1_000);
        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("run_health");
        assert_eq!(run.written, 1, "a new reason is a new state");

        let rows = rows(&paths.usage_db);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].status, "unreadable");
        assert_eq!(rows[1].status, "unreadable");
        assert_ne!(rows[0].error, rows[1].error, "{rows:?}");
    });
}

/// A range whose rows name no project still moves the cursor, and nothing is observed.
#[test]
fn a_range_without_projects_moves_the_cursor() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, None, "hook-suggest", NOW);
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");
        assert_eq!(run, HealthRun::default());
        let state = state_of(&paths);
        assert_eq!(state.health_cursor, 1);
        assert_eq!(state.last_run_ms, Some(NOW));
        assert!(state.instance_id.is_some());
    });
}

#[test]
fn never_observed_deleted_index_is_unmapped() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, Some("vanished-project"), "impact", NOW);
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");

        assert_eq!(
            run,
            HealthRun {
                observed: 0,
                written: 0,
                unmapped: 1,
                skipped_busy: false,
            }
        );
        assert!(rows(&paths.usage_db).is_empty());

        let state = state_of(&paths);
        assert_eq!(state.unmapped_count, 1);
        assert_eq!(state.unmapped_recent, vec!["vanished-project".to_string()]);
        // The cursor still advances past an id that was skipped: it was read and decided about.
        assert_eq!(state.health_cursor, 1);
    });
}

#[test]
fn unmapped_recent_keeps_five_newest() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        // `touched_project_ids_at` returns its ids ordered by `project_id`, so processing order is
        // the lexicographic order of these names: u1 first, u7 last.
        for name in ["u1", "u2", "u3", "u4", "u5", "u6", "u7"] {
            record(&paths.usage_db, Some(name), "impact", NOW);
        }
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");
        assert_eq!(run.unmapped, 7);

        let state = state_of(&paths);
        assert_eq!(state.unmapped_count, 7);
        assert_eq!(
            state.unmapped_recent,
            ["u7", "u6", "u5", "u4", "u3"].map(str::to_string).to_vec(),
            "the five newest, newest first"
        );

        // The same id again moves to the front and is not kept twice.
        record(&paths.usage_db, Some("u5"), "impact", NOW + 1_000);
        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("run_health");
        assert_eq!(run.unmapped, 1);
        let state = state_of(&paths);
        assert_eq!(state.unmapped_count, 8);
        assert_eq!(
            state.unmapped_recent,
            ["u5", "u7", "u6", "u4", "u3"].map(str::to_string).to_vec()
        );
    });
}

// ---------------------------------------------------------------------------------------------
// Which rows are the worker's business at all.
// ---------------------------------------------------------------------------------------------

#[test]
fn measure_rows_do_not_trigger_observation() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, Some("measure-project"), "measure", NOW);
        record(&paths.usage_db, Some("measure-project"), "measure", NOW);
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");

        assert_eq!(
            run,
            HealthRun {
                observed: 0,
                written: 0,
                unmapped: 0,
                skipped_busy: false,
            }
        );
        assert!(rows(&paths.usage_db).is_empty());
        let state = state_of(&paths);
        assert_eq!(state.health_cursor, 0);
        assert_eq!(state.last_run_ms, Some(NOW));
    });
}

#[test]
fn other_machine_history_is_ignored() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        let mut spawn = Recorder::default();
        record(&paths.usage_db, Some(&id), "impact", NOW);
        assert_eq!(
            health(&paths, "machine-b", NOW, &mut spawn)
                .unwrap()
                .written,
            1
        );

        // Same project, same index, same day -- but the row that exists belongs to another machine,
        // so this machine has never recorded this state and must.
        record(&paths.usage_db, Some(&id), "impact", NOW + 1_000);
        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("run_health");
        assert_eq!(run.written, 1);

        let machines: Vec<String> = rows(&paths.usage_db)
            .into_iter()
            .map(|row| row.machine_id)
            .collect();
        assert_eq!(machines, vec!["machine-b".to_string(), MACHINE.to_string()]);
    });
}

// ---------------------------------------------------------------------------------------------
// The run lock and the handoff to the batch lane.
// ---------------------------------------------------------------------------------------------

#[test]
fn busy_lock_skips() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        let _held = cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-health.lock"))
            .expect("hold the health run lock");

        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("a busy lock is not an error");
        assert_eq!(
            run,
            HealthRun {
                observed: 0,
                written: 0,
                unmapped: 0,
                skipped_busy: true,
            }
        );
        // Nothing was read or written: not even the state file, and not the usage database.
        assert!(!paths.data_dir.join("measure-health-state.json").exists());
        assert!(!paths.usage_db.exists());
        assert!(spawn.0.is_empty());
    });
}

/// The origin lookup stands in for `git remote get-url` on a hung mount: it never answers within
/// the pass. The pass must still end inside the per-root budget, record the row with a NULL
/// `project_id`, and let the run lock go.
fn hung_origin(_: &Path) -> Result<String, String> {
    std::thread::sleep(std::time::Duration::from_secs(30));
    Err("never answered".into())
}

#[test]
fn a_hung_origin_lookup_does_not_hold_the_pass() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, Some(&id), "impact", NOW);
        let cfg = CollectConfig::from_lookup(|key| {
            (key == "CORT_HEALTH_ROOT_TIMEOUT_SECS").then(|| "2".to_string())
        });
        let mut spawn = Recorder::default();
        let started = std::time::Instant::now();
        let run = cort::health::run_health_with_origin(
            &paths,
            MACHINE,
            "test",
            NOW,
            &cfg,
            &mut spawn,
            hung_origin,
        )
        .expect("run_health");
        let elapsed = started.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "two 2 s budgets, not the 30 s lookup: {elapsed:?}"
        );
        assert_eq!(run.written, 1);
        let rows = rows(&paths.usage_db);
        assert_eq!(rows[0].project_id, None, "{rows:?}");
        assert!(
            cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-health.lock")).is_ok(),
            "the run lock is released when the pass returns"
        );
    });
}

/// An upgrade holding admission is the same quiet skip as a busy run lock: nothing read or written.
#[test]
fn an_upgrade_in_progress_skips() {
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    in_sandbox(cache.path(), data.path(), &[], || {
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, Some("some-project"), "impact", NOW);
        let _upgrade =
            cort::upgrade::acquire_upgrade_locks(cache.path(), std::time::Duration::from_secs(1))
                .expect("hold the upgrade locks");

        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("an upgrade is not an error");
        assert_eq!(
            run,
            HealthRun {
                skipped_busy: true,
                ..HealthRun::default()
            }
        );
        assert!(!paths.data_dir.join("measure-health-state.json").exists());
        assert!(rows(&paths.usage_db).is_empty());
    });
}

#[test]
fn handoff_kicks_batch_when_written() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        // `kick_at` stops at `not_configured` without this file.
        fs::write(&paths.measure_config, "{}").expect("write the measure config");

        record(&paths.usage_db, Some(&id), "impact", NOW);
        let mut spawn = Recorder::default();
        let run = health(&paths, MACHINE, NOW, &mut spawn).expect("run_health");
        assert_eq!(run.written, 1);
        // The batch lane is due -- no batch state exists, so with one pending row and no prior run
        // it is at once past its idle rule -- while the health lane is not (its last run is now).
        assert_eq!(
            spawn.0,
            vec![(Lane::Batch, "measure-health".to_string())],
            "one handoff, for the batch lane, in this process's own name"
        );

        // A second pass with nothing new writes nothing and hands off nothing.
        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("run_health");
        assert_eq!(run.written, 0);
        assert_eq!(spawn.0.len(), 1);
    });
}

// ---------------------------------------------------------------------------------------------
// Failure: a row that cannot be written leaves the cursor where it was.
// ---------------------------------------------------------------------------------------------

/// The failure is made with a SQLite trigger that aborts every insert into `index_health_log`, so
/// the write fails while everything up to it -- the lock, the counters, the observation -- works.
/// Nothing is stubbed: this is the same path a full disk would take.
#[test]
fn insert_failure_does_not_advance_the_cursor() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let paths = kick_paths(cache.path(), data.path());
        record(&paths.usage_db, Some(&id), "impact", NOW);
        let block = Connection::open(&paths.usage_db).expect("open the usage database");
        block
            .execute_batch(
                "CREATE TRIGGER block_health BEFORE INSERT ON index_health_log
                 BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
            )
            .expect("create the blocking trigger");

        let mut spawn = Recorder::default();
        let err = health(&paths, MACHINE, NOW, &mut spawn).expect_err("the blocked insert fails");
        assert!(
            err.contains("index_health_log_write_failed"),
            "the error names the failed write: {err}"
        );
        assert!(rows(&paths.usage_db).is_empty());
        assert!(
            !paths.data_dir.join("measure-health-state.json").exists(),
            "the state must not be saved after a failed write"
        );
        let state = state_of(&paths);
        assert_eq!(state.health_cursor, 0);
        assert_eq!(state.instance_id, None);
        assert_eq!(state.last_run_ms, None);
        // The run lock was released with the error, so the retry below can take it.
        block
            .execute_batch("DROP TRIGGER block_health")
            .expect("drop the blocking trigger");
        drop(block);

        let run = health(&paths, MACHINE, NOW + 1_000, &mut spawn).expect("the retry succeeds");
        assert_eq!(run.written, 1);
        let state = state_of(&paths);
        assert!(state.health_cursor > 0, "{state:?}");
        let (left, _) = cort::usage::touched_project_ids_at(&paths.usage_db, state.health_cursor)
            .expect("read past the cursor");
        assert!(left.is_empty(), "{left:?}");
        assert!(state.instance_id.is_some());
    });
}
