use cort::audit_import::{ImportedUsage, ImportedUsageOutcome};
use cort::collect::{
    run_collect, CollectConfig, CollectRun, CollectState, ErrorKind, Failure, KickPaths,
};
use cort::measure_store::{CompactionRow, MachineHost, MeasureStore, OpenError, SqliteStore};
use cort::telemetry::TelemetryEvent;
use cort::usage::{insert_health_row_at, record_command_at, CommandRecord, HealthRow};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());
const NOW: i64 = 1_800_000_000_000;

fn sandbox(f: impl FnOnce(&KickPaths, &Path)) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let data = dir.path().join("data");
    let claude = dir.path().join("claude");
    let xdg = dir.path().join("xdg");
    for path in [&cache, &data, &claude, &xdg] {
        fs::create_dir_all(path).unwrap();
    }
    let vars: [(&str, PathBuf); 6] = [
        ("CORT_CACHE_DIR", cache.clone()),
        ("CORT_MEMORY_DIR", data.clone()),
        ("CLAUDE_CONFIG_DIR", claude),
        ("XDG_DATA_HOME", xdg),
        ("HOME", dir.path().to_path_buf()),
        ("CORT_MACHINE_ID", PathBuf::from("collect-test-machine")),
    ];
    let old: Vec<_> = vars
        .iter()
        .map(|(k, _)| (*k, std::env::var_os(k)))
        .collect();
    // All environment readers in this test binary run while ENV_LOCK is held.
    for (k, v) in &vars {
        unsafe { std::env::set_var(k, v) };
    }
    let paths = KickPaths {
        data_dir: data.clone(),
        usage_db: cache.join("usage.db"),
        cache_dir: cache,
        measure_config: data.join("turso-measure.json"),
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f(&paths, &dir.path().join("store.db"))
    }));
    for (k, v) in old {
        unsafe {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        };
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

fn state_path(paths: &KickPaths) -> PathBuf {
    paths.data_dir.join("measure-collect-state.json")
}
fn state(paths: &KickPaths) -> CollectState {
    let path = state_path(paths);
    assert!(path.exists(), "state file missing");
    CollectState::load(&path)
}
fn command(paths: &KickPaths) {
    record_command_at(
        &paths.usage_db,
        &CommandRecord {
            now_ms: NOW,
            project_id: None,
            command: "impact".into(),
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
fn health(paths: &KickPaths) {
    insert_health_row_at(
        &paths.usage_db,
        &HealthRow {
            id: 0,
            ts: NOW,
            machine_id: "collect-test-machine".into(),
            machine_source: "test".into(),
            root: Some("repo".into()),
            local_project_id: "local".into(),
            project_id: None,
            status: "ok".into(),
            error: None,
            fresh: Some("fresh".into()),
            chunks: Some(1),
            relationships: Some(0),
            not_chunked: Some(0),
            real_gap: Some(0),
            fts: Some("ok".into()),
        },
    )
    .unwrap();
}
fn run(
    paths: &KickPaths,
    db: &Path,
    harness: &str,
    push_fail: bool,
    pull_fail: bool,
) -> CollectRun {
    run_collect(
        &mut || {
            let mut store = SqliteStore::open(db).map_err(OpenError::Other)?;
            store.fail_push(push_fail);
            store.fail_pull(pull_fail);
            Ok(Box::new(store) as Box<dyn MeasureStore>)
        },
        paths,
        harness,
        NOW,
        &CollectConfig::from_lookup(|_| None),
        false,
    )
}

fn quick_cfg(run_deadline_ms: u64) -> CollectConfig {
    CollectConfig {
        run_deadline_ms,
        ..CollectConfig::from_lookup(|_| None)
    }
}

#[test]
fn deadline_stops_between_phases() {
    sandbox(|paths, db| {
        command(paths);
        let mut opener = || {
            std::thread::sleep(Duration::from_millis(20));
            Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>)
        };
        let result = run_collect(&mut opener, paths, "codex", NOW, &quick_cfg(1), false);
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Network))
        );
        assert!(
            result.summary["error"]
                .as_str()
                .is_some_and(|e| e.contains("run deadline exceeded after")),
            "{}",
            result.summary
        );
        // Nothing ran: no scan, no uploads, no push. The record's first write never happened, so
        // this was the checkpoint after the opener, not the one before the push.
        assert_eq!(result.summary["scan"], serde_json::Value::Null);
        assert!(SqliteStore::open(db)
            .unwrap()
            .machine_hosts()
            .unwrap()
            .is_empty());
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.last_ok_ms, None);
        assert_eq!(s.failures["codex"].count, 1);
        assert_eq!(s.failures["codex"].last_error_kind, "network");
        // Every lock the run held is released.
        drop(
            cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-collect.lock"))
                .expect("collect lock released"),
        );
    });
}

/// The largest configurable budget runs to completion. On Linux `Instant` keeps whole seconds in
/// an i64, so even `u64::MAX` ms is representable and this cannot catch an unchecked add; it pins
/// the behaviour, and the `checked_add` in `RunDeadline` covers platforms that count nanoseconds
/// in 64 bits.
#[test]
fn largest_deadline_runs_to_completion() {
    sandbox(|paths, db| {
        command(paths);
        let mut opener = || Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>);
        let result = run_collect(
            &mut opener,
            paths,
            "codex",
            NOW,
            &quick_cfg(u64::MAX),
            false,
        );
        assert_eq!((result.outcome, result.error_kind), ("ok", None));
    });
}

/// The real [`SqliteStore`] with a first write that takes `delay`. `upsert_machine_host` is the
/// first store call of `record_with_scan`, so sleeping there spends the run's budget between the
/// uploads and the push without changing anything else the real store does.
struct SlowFirstWrite {
    inner: SqliteStore,
    delay: Duration,
}

macro_rules! forward_to_inner {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) -> $ret { self.inner.$name($($arg),*) })*
    };
}

impl MeasureStore for SlowFirstWrite {
    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String> {
        std::thread::sleep(self.delay);
        self.inner.upsert_machine_host(machine_id, host, seen_ms)
    }

    forward_to_inner! {
        upsert_compaction(row: &CompactionRow) -> Result<(), String>;
        compaction_session(machine_id: &str, boundary_uuid: &str) -> Result<Option<String>, String>;
        compactions_since(since_ms: i64) -> Result<Vec<CompactionRow>, String>;
        machine_hosts() -> Result<Vec<MachineHost>, String>;
        upsert_imported_usage(rows: &[ImportedUsage]) -> Result<ImportedUsageOutcome, String>;
        imported_usage() -> Result<Vec<ImportedUsage>, String>;
        insert_events(events: &[TelemetryEvent]) -> Result<usize, String>;
        remote_ids(machine_id: &str, source: &str) -> Result<HashMap<i64, String>, String>;
        remote_payload(machine_id: &str, source: &str, event_id: &str) -> Result<Option<String>, String>;
        remote_payloads_without_time(machine_id: &str, source: &str) -> Result<Vec<String>, String>;
        remote_payloads_in_range(machine_id: &str, source: &str, min_ms: i64, max_ms: i64) -> Result<Vec<String>, String>;
        event_watermark(machine_id: &str, source: &str) -> Result<Option<(i64, String)>, String>;
        set_event_watermark(machine_id: &str, source: &str, max_rowid: i64, row_fingerprint: &str) -> Result<(), String>;
        upsert_aliases(rows: &[(String, String, String)]) -> Result<(), String>;
        aliases() -> Result<Vec<(String, String, String)>, String>;
        events_page(after: Option<(String, String)>, limit: usize) -> Result<Vec<TelemetryEvent>, String>;
        events_in_key_range(first: &(String, String), last: &(String, String)) -> Result<Vec<TelemetryEvent>, String>;
        event_counts() -> Result<Vec<(String, String, usize)>, String>;
        push() -> Result<(), String>;
        pull() -> Result<(), String>;
    }
}

#[test]
fn deadline_after_uploads_skips_the_push() {
    sandbox(|paths, db| {
        command(paths);
        health(paths);
        let mut opener = || {
            Ok(Box::new(SlowFirstWrite {
                inner: SqliteStore::open(db).unwrap(),
                delay: Duration::from_millis(2_100),
            }) as Box<dyn MeasureStore>)
        };
        // Wall-clock, not an injected clock: the work before the first write (opening the store,
        // the counters, the state save) takes milliseconds, so only a stall of 2 s before the
        // opener returns would trip the earlier checkpoint instead -- and then the health-row
        // assertion below fails loudly rather than passing for the wrong reason.
        let result = run_collect(&mut opener, paths, "codex", NOW, &quick_cfg(2_000), false);
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Network))
        );
        assert!(
            result.summary["error"]
                .as_str()
                .is_some_and(|e| e.contains("run deadline exceeded after")),
            "{}",
            result.summary
        );
        // The stop came after the uploads: the health row reached the replica before it.
        let counts = SqliteStore::open(db).unwrap().event_counts().unwrap();
        assert!(
            counts
                .iter()
                .any(|(source, _, count)| source == "cort_index_health" && *count == 1),
            "{counts:?}"
        );
        // A stop before the push is a network failure with the push still owed, not a success.
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.last_ok_ms, None);
        assert_eq!(s.failures["codex"].last_error_kind, "network");
    });
}

#[test]
fn push_pending_persisted_before_replica_write() {
    sandbox(|paths, db| {
        command(paths);
        let mut opener = || {
            let s = state(paths);
            assert!(s.push_pending);
            assert_eq!(s.last_run_ms, Some(NOW));
            Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>)
        };
        let result = run_collect(
            &mut opener,
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "ok");
        assert!(!state(paths).push_pending);
    });
}

#[test]
fn push_failure_keeps_push_pending_and_counts_harness() {
    sandbox(|paths, db| {
        command(paths);
        health(paths);
        let result = run(paths, db, "codex", true, false);
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Network))
        );
        assert!(result.summary["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("push failed: ")));
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.failures["codex"].count, 1);
        assert_eq!(s.failures["codex"].last_error_kind, "network");
        assert_eq!(s.failures["codex"].last_failed_ms, NOW);
        assert_eq!(s.last_ok_ms, None);
        // The health row reached the replica before the push failed; the cursor follows the
        // replica's watermark, not the run's outcome.
        assert_eq!(result.summary["scan"]["health"]["status"], "uploaded");
        assert_eq!(s.health_uploaded_through_id, 1);
    });
}

#[test]
fn success_clears_push_pending_and_only_that_harness() {
    sandbox(|paths, db| {
        command(paths);
        for _ in 0..2 {
            assert_eq!(run(paths, db, "codex", true, false).outcome, "failed");
        }
        assert_eq!(state(paths).failures["codex"].count, 2);
        let ok = run(paths, db, "claude-code", false, false);
        assert_eq!(ok.outcome, "ok");
        assert_eq!(ok.summary["error"], serde_json::Value::Null);
        let s = state(paths);
        assert!(!s.push_pending);
        assert_eq!(s.last_ok_ms, Some(NOW));
        assert_eq!(s.failures["codex"].count, 2);
        assert!(!s.failures.contains_key("claude-code"));
    });
}

#[test]
fn instance_change_resets_and_reuploads_nothing_twice() {
    sandbox(|paths, db| {
        command(paths);
        health(paths);
        let mut s = CollectState {
            instance_id: Some("old-instance".into()),
            uploaded_through_id: 500,
            health_uploaded_through_id: 500,
            ..CollectState::default()
        };
        s.save(&state_path(paths)).unwrap();
        assert_eq!(run(paths, db, "codex", false, false).outcome, "ok");
        s = state(paths);
        assert_ne!(s.instance_id.as_deref(), Some("old-instance"));
        assert_eq!(s.uploaded_through_id, 0); // command upload awaits a dry-run marker
        assert_eq!(s.health_uploaded_through_id, 1);
        let mut store = SqliteStore::open(db).unwrap();
        let first = store.event_counts().unwrap();
        assert_eq!(run(paths, db, "codex", false, false).outcome, "ok");
        assert_eq!(
            SqliteStore::open(db).unwrap().event_counts().unwrap(),
            first
        );
        assert_eq!(state(paths).health_uploaded_through_id, 1);
    });
}

#[test]
fn lock_file_names_holder() {
    sandbox(|paths, db| {
        command(paths);
        let mut opener = || {
            let text = fs::read_to_string(paths.data_dir.join("measure-collect.lock")).unwrap();
            assert!(text.contains(&std::process::id().to_string()), "{text}");
            assert!(text.contains(&NOW.to_string()), "{text}");
            Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>)
        };
        assert_eq!(
            run_collect(
                &mut opener,
                paths,
                "codex",
                NOW,
                &CollectConfig::from_lookup(|_| None),
                false
            )
            .outcome,
            "ok"
        );
        assert!(!state(paths).push_pending);
    });
}

#[test]
fn busy_store_is_skipped_not_failed() {
    sandbox(|paths, _| {
        command(paths);
        let mut s = CollectState::default();
        s.failures.insert(
            "codex".into(),
            Failure {
                count: 2,
                last_error_kind: "network".into(),
                last_failed_ms: NOW - 1,
            },
        );
        s.save(&state_path(paths)).unwrap();
        let result = run_collect(
            &mut || Err(OpenError::Busy),
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "skipped_busy");
        let after = state(paths);
        assert!(after.push_pending);
        assert_eq!(after.failures["codex"].count, 2);
        assert_eq!(after.last_run_ms, Some(NOW));
    });
}

#[test]
fn refused_is_not_failure() {
    sandbox(|paths, db| {
        command(paths);
        health(paths);
        let result = run(paths, db, "codex", false, false);
        assert_eq!(result.outcome, "ok");
        assert!(result.refused);
        let s = state(paths);
        assert!(s.refused);
        assert!(!s.push_pending);
        assert!(s.failures.is_empty());
        assert_eq!(s.uploaded_through_id, 0);
        assert_eq!(s.health_uploaded_through_id, 1);
        let counts = SqliteStore::open(db).unwrap().event_counts().unwrap();
        assert!(counts
            .iter()
            .any(|(source, _, count)| source == "cort_index_health" && *count == 1));
    });
}

#[test]
fn skipped_busy_when_collect_lock_held() {
    sandbox(|paths, _| {
        command(paths);
        let _held = cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-collect.lock"))
            .unwrap();
        let mut called = false;
        let result = run_collect(
            &mut || {
                called = true;
                Err(OpenError::Busy)
            },
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "skipped_busy");
        assert!(!called);
        assert!(!state_path(paths).exists());
    });
}

#[test]
fn a_collect_run_never_pulls() {
    // A failing pull cannot touch a run that never pulls: it pushes, succeeds, and clears
    // `push_pending` (see `record_pushes_and_never_pulls`).
    sandbox(|paths, db| {
        command(paths);
        let result = run(paths, db, "codex", false, true);
        assert_eq!((result.outcome, result.error_kind), ("ok", None));
        let s = state(paths);
        assert!(!s.push_pending);
        assert!(!s.failures.contains_key("codex"));
    });
}

#[test]
fn manual_is_unimplemented_without_state() {
    sandbox(|paths, _| {
        let mut called = false;
        let result = run_collect(
            &mut || {
                called = true;
                Err(OpenError::Busy)
            },
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            true,
        );
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Other))
        );
        assert!(!called);
        assert!(!state_path(paths).exists());
    });
}

#[test]
fn unreadable_usage_db_records_storage_failure() {
    sandbox(|paths, _| {
        let mut called = false;
        let result = run_collect(
            &mut || {
                called = true;
                Err(OpenError::Busy)
            },
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Storage))
        );
        assert!(!called);
        assert_eq!(result.summary["error"], "usage_db_unreadable");
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.failures["codex"].last_error_kind, "storage");
    });
}

#[test]
fn opener_error_records_other_failure() {
    sandbox(|paths, _| {
        command(paths);
        let result = run_collect(
            &mut || Err(OpenError::Other("test".into())),
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Other))
        );
        assert_eq!(result.summary["error"], "test");
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.failures["codex"].last_error_kind, "other");
    });
}

#[test]
fn bootstrap_timeout_records_network_failure() {
    sandbox(|paths, _| {
        command(paths);
        let result = run_collect(
            &mut || Err(OpenError::Network("network timeout after 120s".into())),
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Network))
        );
        assert_eq!(result.summary["error"], "network timeout after 120s");
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.failures["codex"].last_error_kind, "network");
    });
}

#[test]
fn upload_failure_precedes_push_failure_in_classification() {
    sandbox(|paths, db| {
        command(paths);
        let mut opener = || {
            // Counters were read already; losing the source now makes both upload reports fail.
            fs::remove_file(&paths.usage_db).unwrap();
            fs::create_dir(&paths.usage_db).unwrap();
            let mut store = SqliteStore::open(db).unwrap();
            store.fail_push(true);
            Ok(Box::new(store) as Box<dyn MeasureStore>)
        };
        let result = run_collect(
            &mut opener,
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Storage))
        );
        assert_eq!(result.summary["scan"]["events"]["status"], "refused");
        assert_eq!(result.summary["scan"]["health"]["status"], "failed");
        // The message is the failed upload's own, not the push failure that followed it.
        assert_eq!(
            result.summary["error"],
            result.summary["scan"]["health"]["error"]
        );
        assert!(result.summary["error"]
            .as_str()
            .is_some_and(|e| !e.is_empty()));
        let s = state(paths);
        assert!(s.push_pending);
        assert_eq!(s.failures["codex"].last_error_kind, "storage");
    });
}

#[test]
fn upgrade_in_progress_skips_without_state() {
    sandbox(|paths, _| {
        command(paths);
        let _upgrade = cort::upgrade::acquire_upgrade_locks(
            &paths.cache_dir,
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        let mut called = false;
        let result = run_collect(
            &mut || {
                called = true;
                Err(OpenError::Busy)
            },
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "skipped_busy");
        assert!(!called);
        assert!(!state_path(paths).exists());
    });
}

#[test]
fn unopenable_admission_lock_is_not_an_upgrade() {
    sandbox(|paths, db| {
        command(paths);
        // A directory where the admission lock file belongs: no upgrader could hold it either.
        fs::create_dir(paths.cache_dir.join(".upgrade-admission.lock")).unwrap();
        let result = run(paths, db, "codex", false, false);
        assert_eq!((result.outcome, result.error_kind), ("ok", None));
        assert!(!state(paths).push_pending);
    });
}

#[test]
fn fresh_data_dir_is_created_before_the_lock() {
    sandbox(|paths, _| {
        command(paths);
        fs::remove_dir_all(&paths.data_dir).unwrap();
        let result = run_collect(
            &mut || Err(OpenError::Busy),
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "skipped_busy");
        assert!(paths.data_dir.join("measure-collect.lock").exists());
    });
}

#[test]
fn data_dir_creation_error_is_structured_other() {
    sandbox(|paths, _| {
        fs::remove_dir_all(&paths.data_dir).unwrap();
        fs::write(&paths.data_dir, "occupied").unwrap();
        let result = run_collect(
            &mut || panic!("opener called"),
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "failed");
        assert_eq!(result.error_kind, Some(ErrorKind::Other));
        assert!(result.summary["error"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
    });
}

#[test]
fn refusal_is_not_failure_and_logs_on_transition_only() {
    sandbox(|paths, db| {
        command(paths);
        for iteration in 0..3 {
            let result = run(paths, db, "codex", false, false);
            assert_eq!(result.outcome, "ok");
            assert!(result.refused);
            assert_eq!(result.refused_changed, iteration == 0);
            cort::collect::log_collect_run(&paths.data_dir, &result, NOW).unwrap();
        }
        assert!(state(paths).failures.is_empty());
        let lines = fs::read_to_string(paths.data_dir.join("measure.log")).unwrap();
        let logs: Vec<serde_json::Value> = lines
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(logs.len(), 3);
        assert_eq!(
            logs.iter().filter(|v| v["refused_changed"] == true).count(),
            1
        );
        assert!(logs[0]["scan"]["events"]["reason"].is_string());
        for log in &logs[1..] {
            assert_eq!(
                log["scan"]["events"],
                serde_json::json!({"status":"refused"})
            );
        }
        // A valid marker uses the current command_log column fingerprint.
        let columns = cort::telemetry::command_log_columns(&paths.usage_db).unwrap();
        fs::write(
            paths.data_dir.join("measure-events-verified.json"),
            serde_json::json!({"machine_id":cort::usage::machine_id(),"columns":columns})
                .to_string(),
        )
        .unwrap();
        let result = run(paths, db, "codex", false, false);
        assert!(!result.refused);
        assert!(result.refused_changed);
        cort::collect::log_collect_run(&paths.data_dir, &result, NOW).unwrap();
        let lines = fs::read_to_string(paths.data_dir.join("measure.log")).unwrap();
        let last: serde_json::Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
        assert_eq!(last["refused"], false);
        assert_eq!(last["refused_changed"], true);
    });
}

#[test]
fn background_busy_is_silent() {
    sandbox(|paths, _| {
        command(paths);
        let _held = cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-collect.lock"))
            .unwrap();
        let result = run_collect(
            &mut || panic!("opener called"),
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(result.outcome, "skipped_busy");
        cort::collect::log_collect_run(&paths.data_dir, &result, NOW).unwrap();
        assert!(!paths.data_dir.join("measure.log").exists());
    });
}

// The harness reaches run_collect as an argument. Reading CORT_COLLECT_HARNESS is the CLI's job,
// covered in collect_cli.rs by background_failure_line_carries_error and the foreground test.
#[test]
fn push_failure_lands_in_the_named_harness_bucket() {
    sandbox(|paths, db| {
        command(paths);
        let result = run(paths, db, "codex", true, false);
        assert_eq!(result.outcome, "failed");
        assert_eq!(state(paths).failures["codex"].count, 1);
    });
}

#[test]
fn lock_order_collect_then_measure() {
    sandbox(|paths, db| {
        command(paths);
        fs::write(
            &paths.measure_config,
            r#"{"url":"libsql://unreachable.invalid","token":"test-token"}"#,
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let collect_tx = tx.clone();
            scope.spawn(move || {
                let result = run_collect(
                    &mut || {
                        cort::measure_store::with_measure_lock(|| {
                            collect_tx.send("collect").unwrap();
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>)
                        })
                        .map_err(OpenError::Other)
                    },
                    paths,
                    "codex",
                    NOW,
                    &CollectConfig::from_lookup(|_| None),
                    false,
                );
                assert_eq!(result.outcome, "ok");
            });
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
                "collect"
            );
            scope.spawn(|| {
                cort::measure_store::with_measure_lock(|| {
                    tx.send("report").unwrap();
                    Ok(())
                })
                .unwrap();
            });
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
                "report"
            );
        });
    });
}

#[test]
fn refusal_transition_survives_a_failed_final_save() {
    sandbox(|paths, db| {
        command(paths);
        let state_file = state_path(paths);
        let mut opener = || {
            // The pre-run save already happened; a directory in its place fails the final one.
            fs::remove_file(&state_file).unwrap();
            fs::create_dir(&state_file).unwrap();
            Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>)
        };
        let result = run_collect(
            &mut opener,
            paths,
            "codex",
            NOW,
            &CollectConfig::from_lookup(|_| None),
            false,
        );
        assert_eq!(
            (result.outcome, result.error_kind),
            ("failed", Some(ErrorKind::Storage))
        );
        assert!(result.refused);
        assert!(result.refused_changed);
        cort::collect::log_collect_run(&paths.data_dir, &result, NOW).unwrap();
        let line: serde_json::Value = serde_json::from_str(
            fs::read_to_string(paths.data_dir.join("measure.log"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(line["refused_changed"], true);
        assert!(line["scan"]["events"]["reason"].is_string());
    });
}

#[test]
fn lock_order_collect_waits_for_report() {
    sandbox(|paths, db| {
        command(paths);
        fs::write(
            &paths.measure_config,
            r#"{"url":"libsql://unreachable.invalid","token":"test-token"}"#,
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let report_tx = tx.clone();
            // The report path takes measure.lock alone and holds it until told to let go.
            scope.spawn(move || {
                cort::measure_store::with_measure_lock(|| {
                    report_tx.send("report_holds").unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
            });
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
                "report_holds"
            );
            let collect_tx = tx.clone();
            let collect = scope.spawn(move || {
                run_collect(
                    &mut || {
                        collect_tx.send("collect_wants_measure_lock").unwrap();
                        cort::measure_store::with_measure_lock(|| {
                            Ok(Box::new(SqliteStore::open(db).unwrap()) as Box<dyn MeasureStore>)
                        })
                        .map_err(OpenError::Other)
                    },
                    paths,
                    "codex",
                    NOW,
                    &CollectConfig::from_lookup(|_| None),
                    false,
                )
                .outcome
            });
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
                "collect_wants_measure_lock"
            );
            // Collect holds the first lock while it waits for the second.
            assert!(matches!(
                cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-collect.lock")),
                Err(cort::upgrade::LockError::AdmissionBusy)
            ));
            release_tx.send(()).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !collect.is_finished() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "collect never finished"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert_eq!(collect.join().unwrap(), "ok");
        });
    });
}
