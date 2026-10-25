use cort::collect::{
    run_collect, run_manual, CollectConfig, CollectState, ErrorKind, Failure, KickPaths,
    ManualMode, ManualStep,
};
use cort::measure_store::{MeasureStore, OpenError, SqliteStore};
use cort::usage::{record_command_at, CommandRecord};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

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
        ("HOME", dir.path().to_path_buf()),
        ("XDG_DATA_HOME", xdg),
        ("CLAUDE_CONFIG_DIR", claude),
        ("CORT_CACHE_DIR", cache.clone()),
        ("CORT_MEMORY_DIR", data.clone()),
        ("CORT_MACHINE_ID", PathBuf::from("manual-test-machine")),
    ];
    let old: Vec<_> = vars
        .iter()
        .map(|(k, _)| (*k, std::env::var_os(k)))
        .collect();
    for (key, value) in &vars {
        unsafe { std::env::set_var(key, value) };
    }
    let paths = KickPaths {
        data_dir: data.clone(),
        usage_db: cache.join("usage.db"),
        cache_dir: cache,
        measure_config: data.join("turso-measure.json"),
    };
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f(&paths, &dir.path().join("store.db"))
    }));
    for (key, value) in old {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        };
    }
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

fn state_path(paths: &KickPaths) -> PathBuf {
    paths.data_dir.join("measure-collect-state.json")
}

fn state(paths: &KickPaths) -> CollectState {
    CollectState::load(&state_path(paths))
}

fn config() -> CollectConfig {
    CollectConfig::from_lookup(|_| None)
}

fn opener(db: &Path) -> impl FnMut() -> Result<Box<dyn MeasureStore>, OpenError> + '_ {
    move || Ok(Box::new(SqliteStore::open(db).map_err(OpenError::Other)?))
}

/// The CLI with the sandbox's environment, so the child sees the same isolated directories.
fn cort_cli(paths: &KickPaths, args: &[&str]) -> std::process::Output {
    let root = paths.data_dir.parent().unwrap();
    Command::new(env!("CARGO_BIN_EXE_cort"))
        .args(args)
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("CORT_CACHE_DIR", &paths.cache_dir)
        .env("CORT_MEMORY_DIR", &paths.data_dir)
        .env("CORT_MACHINE_ID", "manual-test-machine")
        .env("CORT_CRONTAB_FILE", root.join("crontab"))
        .env("CORT_COLLECT_MANUAL_WAIT_SECS", "1")
        .output()
        .unwrap()
}

#[test]
fn manual_busy_times_out_with_holder() {
    sandbox(|paths, _| {
        fs::write(
            &paths.measure_config,
            r#"{"url":"libsql://unused.invalid","token":"test"}"#,
        )
        .unwrap();
        let mut holder =
            cort::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-collect.lock"))
                .unwrap();
        use std::io::Write;
        writeln!(holder, "pid=4242 started_ms={NOW}").unwrap();
        for args in [
            &["measure", "record"][..],
            &["measure", "sync"][..],
            &["measure", "sync", "--dry-run"][..],
        ] {
            let start = std::time::Instant::now();
            let output = cort_cli(paths, args);
            assert!(!output.status.success());
            assert!(start.elapsed() >= std::time::Duration::from_millis(900));
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["error"], "measure_collect_busy", "{value}");
            assert!(value["detail"]["message"]
                .as_str()
                .unwrap()
                .contains("pid 4242"));
            assert!(value["detail"]["message"]
                .as_str()
                .unwrap()
                .contains(&cort::measure::format_utc(NOW)));
        }
    });
}

#[test]
fn manual_runs_even_when_not_due() {
    sandbox(|paths, db| {
        let mut before = CollectState {
            last_run_ms: Some(NOW),
            ..CollectState::default()
        };
        before.failures.insert(
            "manual".into(),
            Failure {
                count: 2,
                last_error_kind: "network".into(),
                last_failed_ms: NOW,
            },
        );
        before.save(&state_path(paths)).unwrap();
        let mut called = false;
        run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::Write,
            &mut |_| {
                called = true;
                Ok(ManualStep {
                    value: (),
                    events: None,
                    failure: None,
                })
            },
        )
        .unwrap();
        assert!(called);
        assert!(!state(paths).failures.contains_key("manual"));
    });
}

#[test]
fn manual_failure_counts_manual_only() {
    sandbox(|paths, db| {
        let mut before = CollectState::default();
        before.failures.insert(
            "codex".into(),
            Failure {
                count: 3,
                last_error_kind: "network".into(),
                last_failed_ms: NOW,
            },
        );
        before.save(&state_path(paths)).unwrap();
        let failed: Result<(), _> = run_manual(
            &mut || {
                let mut store = SqliteStore::open(db).map_err(OpenError::Other)?;
                store.fail_push(true);
                Ok(Box::new(store) as Box<dyn MeasureStore>)
            },
            paths,
            NOW,
            &config(),
            ManualMode::Write,
            &mut |store| {
                cort::measure::sync(store)
                    .map(|()| ManualStep {
                        value: (),
                        events: None,
                        failure: None,
                    })
                    .map_err(|error| (error, ErrorKind::Network))
            },
        );
        assert!(failed.unwrap_err().message().starts_with("push failed:"));
        assert_eq!(state(paths).failures["manual"].count, 1);
        assert_eq!(state(paths).failures["codex"].count, 3);
        run_manual(
            &mut opener(db),
            paths,
            NOW + 1,
            &config(),
            ManualMode::Write,
            &mut |store| {
                cort::measure::sync(store).unwrap();
                Ok(ManualStep {
                    value: (),
                    events: None,
                    failure: None,
                })
            },
        )
        .unwrap();
        assert!(!state(paths).failures.contains_key("manual"));
        assert_eq!(state(paths).failures["codex"].count, 3);
    });
}

#[test]
fn dry_run_pass_clears_refused_and_stamp() {
    sandbox(|paths, db| {
        let mut before = CollectState {
            refused: true,
            ..CollectState::default()
        };
        before.save(&state_path(paths)).unwrap();
        let stamp = paths.data_dir.join("measure-batch.spawned");
        fs::write(&stamp, "123").unwrap();
        let inputs = cort::telemetry::UploadInputs {
            usage_db: paths.usage_db.clone(),
            cost_dir: paths.data_dir.join("cost"),
            marker_path: paths.data_dir.join("measure-events-verified.json"),
            machine_id: "manual-test-machine".into(),
        };
        let failed: Result<(), _> = run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::DryRun,
            &mut |store| {
                cort::telemetry::verify_events(store, &inputs, NOW)
                    .map(|_| ManualStep {
                        value: (),
                        events: None,
                        failure: None,
                    })
                    .map_err(|error| (error, ErrorKind::Storage))
            },
        );
        assert!(failed.is_err());
        assert!(state(paths).refused);
        assert!(stamp.exists());
        let events = cort::telemetry::cort_events(&paths.usage_db, None).unwrap();
        let mut seeded = SqliteStore::open(db).unwrap();
        seeded.insert_events(&events).unwrap();
        seeded.push().unwrap();
        drop(seeded);
        run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::DryRun,
            &mut |store| {
                let report = cort::telemetry::verify_events(store, &inputs, NOW)
                    .map_err(|error| (error, ErrorKind::Storage))?;
                assert!(report.matched > 0);
                Ok(cort::collect::dry_run_step(report))
            },
        )
        .unwrap();
        before = state(paths);
        assert!(!before.refused);
        assert!(!before.push_pending);
        assert!(!stamp.exists());
    });
}

/// Pins the `scan` shape `record` prints (`ScanSummary::to_json`, unchanged from HEAD) as it
/// comes back through `run_manual`. The envelope is restated here, not read from the CLI: proving
/// it unchanged end to end needs a reachable store, which these tests never open.
#[test]
fn record_scan_shape_survives_run_manual() {
    sandbox(|paths, db| {
        let scan = cort::measure::ScanSummary::default();
        let result = run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::Write,
            &mut |_| {
                Ok(ManualStep {
                    value: scan.clone(),
                    events: None,
                    failure: None,
                })
            },
        )
        .unwrap();
        let output = json!({"status":"recorded","pushed":true,"scan":result.to_json()});
        assert_eq!(
            output,
            json!({"status":"recorded","pushed":true,"scan":{
                "files_seen":0,"files_scanned":0,"compactions":0,"skipped_lines":0,"unknown_type_lines":0
            }})
        );
    });
}

#[test]
fn manual_holds_collect_lock_while_running() {
    sandbox(|paths, db| {
        run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::Write,
            &mut |_| {
                let background = run_collect(
                    &mut || panic!("background opened store"),
                    paths,
                    "codex",
                    NOW,
                    &config(),
                    false,
                );
                assert_eq!(background.outcome, "skipped_busy");
                Ok(ManualStep {
                    value: (),
                    events: None,
                    failure: None,
                })
            },
        )
        .unwrap();
    });
}

#[test]
fn measure_lock_busy_is_collect_busy() {
    sandbox(|paths, _| {
        let result: Result<(), _> = run_manual(
            &mut || Err(OpenError::Busy),
            paths,
            NOW,
            &config(),
            ManualMode::Write,
            &mut |_| panic!("action called"),
        );
        let error = result.unwrap_err();
        assert_eq!(error.code(), "measure_collect_busy");
        assert_eq!(error.message(), "measure store is busy; try again later");
    });
}

#[test]
fn dry_run_without_matches_keeps_refusal() {
    sandbox(|paths, db| {
        CollectState {
            refused: true,
            ..CollectState::default()
        }
        .save(&state_path(paths))
        .unwrap();
        let stamp = paths.data_dir.join("measure-batch.spawned");
        fs::write(&stamp, "123").unwrap();
        // matched == 0 wrote no marker, so the next batch would be refused again.
        let report = run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::DryRun,
            &mut |_| {
                Ok(cort::collect::dry_run_step(cort::telemetry::DryRun {
                    matched: 0,
                    mismatched: Vec::new(),
                    local_only: 3,
                    remote_only: 0,
                }))
            },
        )
        .unwrap();
        assert_eq!(report.matched, 0);
        assert!(state(paths).refused);
        assert!(stamp.exists());
    });
}

#[test]
fn manual_sync_refusal_is_not_a_failure() {
    sandbox(|paths, db| {
        let (events, synced) = run_manual(
            &mut opener(db),
            paths,
            NOW,
            &config(),
            ManualMode::Write,
            &mut |_| {
                Ok(cort::collect::sync_step(
                    cort::telemetry::EventsReport::Refused("not verified".into()),
                    Ok(()),
                ))
            },
        )
        .unwrap();
        // The CLI still fails this sync with measure_events_not_uploaded; only the state differs.
        assert!(!events.is_ok());
        assert!(synced.is_ok());
        let after = state(paths);
        assert!(after.refused);
        assert!(!after.push_pending);
        assert!(!after.failures.contains_key("manual"));
        let failed = |events, synced| cort::collect::sync_step(events, synced).failure;
        assert_eq!(
            failed(
                cort::telemetry::EventsReport::Failed("disk".into()),
                Err("push failed: x".into())
            ),
            Some(ErrorKind::Storage)
        );
        assert_eq!(
            failed(
                cort::telemetry::EventsReport::Refused("not verified".into()),
                Err("push failed: x".into())
            ),
            Some(ErrorKind::Network)
        );
    });
}

#[test]
fn unconfigured_keeps_each_command_code() {
    sandbox(|paths, _| {
        for (args, code) in [
            (&["measure", "record"][..], "measure_error"),
            (&["measure", "sync"][..], "measure_error"),
            (
                &["measure", "sync", "--dry-run"][..],
                "measure_dry_run_failed",
            ),
        ] {
            let output = cort_cli(paths, args);
            assert!(!output.status.success());
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["error"], code, "{args:?} {value}");
            assert!(value["detail"]["message"]
                .as_str()
                .unwrap()
                .contains("not configured"));
        }
        assert!(!paths.data_dir.join("measure-collect.lock").exists());
    });
}
