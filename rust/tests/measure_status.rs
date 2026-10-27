use cort::collect::{
    batch_pending, collection_status, health_pending, CollectState, Failure, HealthState, KickPaths,
};
use cort::usage::{kick_counters_at, record_command_at, CommandRecord};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn sandbox(f: impl FnOnce(&KickPaths)) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let paths = KickPaths {
        data_dir: root.join("data"),
        usage_db: root.join("cache/usage.db"),
        cache_dir: root.join("cache"),
        measure_config: root.join("data/turso-measure.json"),
    };
    fs::create_dir_all(&paths.data_dir).unwrap();
    fs::create_dir_all(&paths.cache_dir).unwrap();
    f(&paths);
}

fn cli(paths: &KickPaths, args: &[&str]) -> std::process::Output {
    let root = paths.data_dir.parent().unwrap();
    Command::new(env!("CARGO_BIN_EXE_cort"))
        .args(args)
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("CORT_CACHE_DIR", &paths.cache_dir)
        .env("CORT_MEMORY_DIR", &paths.data_dir)
        .env("CORT_MACHINE_ID", "status-test-machine")
        .env("CORT_CRONTAB_FILE", root.join("crontab"))
        .env("CORT_COLLECT_NET_TIMEOUT_SECS", "1")
        .output()
        .unwrap()
}

/// A loopback listener that accepts and closes every connection, so the remote fails at once and
/// the same way everywhere. A closed port is not enough: under WSL's mirrored networking a
/// connection to one is silently dropped and only times out.
fn closing_remote() -> u16 {
    static PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    *PORT.get_or_init(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                drop(stream);
            }
        });
        port
    })
}

fn configured(paths: &KickPaths) {
    fs::write(
        &paths.measure_config,
        format!(
            r#"{{"url":"libsql://127.0.0.1:{}","token":"test"}}"#,
            closing_remote()
        ),
    )
    .unwrap();
}

/// Status needs the remote now; the config names a loopback listener that closes every connection, so the command exits non-zero with
/// `measure_remote_unavailable` and the local status in `detail.local`.
fn local_status(output: &std::process::Output) -> Value {
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!output.status.success(), "{value}");
    assert_eq!(value["error"], "measure_remote_unavailable", "{value}");
    let local = value["detail"]["local"].clone();
    // A loopback listener that closes the connection: deterministic, and never leaves the machine.
    assert_eq!(value["detail"]["kind"], "unreachable", "{value}");
    assert_eq!(local["remote"]["error"], value["detail"]["kind"], "{value}");
    local
}

fn command(path: &Path) {
    record_command_at(
        path,
        &CommandRecord {
            now_ms: 1_800_000_000_000,
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

#[test]
fn status_reports_both_lanes() {
    sandbox(|paths| {
        configured(paths);
        command(&paths.usage_db);
        let counters = kick_counters_at(&paths.usage_db).unwrap();
        let batch = CollectState {
            instance_id: Some(counters.instance_id.clone()),
            last_run_ms: Some(123),
            last_ok_ms: Some(120),
            ..Default::default()
        };
        let health = HealthState {
            instance_id: Some(counters.instance_id.clone()),
            last_run_ms: Some(122),
            unmapped_count: 2,
            unmapped_recent: vec!["missing".into()],
            ..Default::default()
        };
        batch
            .save(&paths.data_dir.join("measure-collect-state.json"))
            .unwrap();
        health
            .save(&paths.data_dir.join("measure-health-state.json"))
            .unwrap();
        let value = local_status(&cli(paths, &["measure", "status"]));
        assert_eq!(value["progress"], "missing");
        assert_eq!(value["batch"]["pending"], batch_pending(&batch, &counters));
        assert_eq!(
            value["health"]["pending"],
            health_pending(&health, &counters)
        );
        assert_eq!(value["batch"]["last_run_ms"], 123);
        assert_eq!(value["pruned_unuploaded_rows"], 0);
        // A readable usage.db leaves no error field behind, not even a null one.
        assert!(value["batch"].get("pending_error").is_none(), "{value}");
        assert!(value["health"].get("pending_error").is_none(), "{value}");
        assert_eq!(
            value["health"]["unmapped_recent"],
            serde_json::json!(["missing"])
        );
        // The lean renderer is exercised on the same local value; the CLI cannot print it when
        // the remote read fails.
        let lines = cort::measure::render_status(&value);
        assert_eq!(lines.lines().count(), 4, "{lines}");
        assert!(lines.contains("progress=missing\n"), "{lines}");
        assert!(lines.contains("remote error="), "{lines}");
        assert!(
            lines.lines().next().unwrap().starts_with("batch "),
            "{lines}"
        );
        assert!(
            lines.lines().nth(1).unwrap().starts_with("health "),
            "{lines}"
        );
        assert!(!lines.contains("\t\t"), "{lines}");
        // Times are for a person: UTC, one token each; a lane that never ran says so.
        let utc = |ms| {
            cort::measure::format_utc(ms)
                .replacen(' ', "T", 1)
                .replace(" UTC", "Z")
        };
        assert!(lines.contains(&format!("last_run={}", utc(123))), "{lines}");
        assert!(!lines.contains("last_run_ms="), "{lines}");
    });
}

#[test]
fn status_reports_whole_stalled_days() {
    sandbox(|paths| {
        configured(paths);
        command(&paths.usage_db);
        let state = CollectState {
            upload_pending: true,
            pending_since_ms: Some(cort::usage::now_ms() - 8 * 86_400_000),
            ..Default::default()
        };
        state
            .save(&paths.data_dir.join("measure-collect-state.json"))
            .unwrap();
        let value = collection_status(paths);
        assert_eq!(value["batch"]["upload_pending"], true);
        assert_eq!(value["batch"]["upload_stalled_days"], 8);
    });
}

#[test]
fn status_separates_refused_from_network_failure() {
    sandbox(|paths| {
        configured(paths);
        command(&paths.usage_db);
        let counters = kick_counters_at(&paths.usage_db).unwrap();
        let mut state = CollectState {
            instance_id: Some(counters.instance_id),
            refused: true,
            ..Default::default()
        };
        state.failures.insert(
            "codex".into(),
            Failure {
                count: 2,
                last_error_kind: "network".into(),
                last_failed_ms: 456,
            },
        );
        state
            .save(&paths.data_dir.join("measure-collect-state.json"))
            .unwrap();
        let value = local_status(&cli(paths, &["measure", "status"]));
        assert_eq!(value["batch"]["pending"], 0);
        assert_eq!(value["batch"]["refused"], true);
        assert_eq!(
            value["batch"]["failures"]["codex"]["last_error_kind"],
            "network"
        );
        let lean = cort::measure::render_status(&value);
        assert!(lean.contains("refused=true"), "{lean}");
        assert!(lean.contains("codex:2/network"), "{lean}");
    });
}

#[test]
fn status_unconfigured_errors() {
    sandbox(|paths| {
        let output = cli(paths, &["measure", "status"]);
        assert!(!output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"], "measure_not_configured");
    });
}

#[test]
fn unreadable_usage_and_missing_state_are_explicit_and_read_only() {
    sandbox(|paths| {
        let value = collection_status(paths);
        assert_eq!(value["no_local_state"], true);
        assert!(value["batch"]["pending"].is_null());
        assert_eq!(value["batch"]["pending_error"], "usage_db_unreadable");
        assert!(!paths.usage_db.exists());
        assert!(!paths.data_dir.join("measure-collect-state.json").exists());
    });
}

#[test]
fn broken_state_file_is_unknown_not_healthy() {
    sandbox(|paths| {
        configured(paths);
        command(&paths.usage_db);
        fs::write(
            paths.data_dir.join("measure-collect-state.json"),
            "{not json",
        )
        .unwrap();
        let value = local_status(&cli(paths, &["measure", "status"]));
        assert!(value["batch"]["pending"].is_null(), "{value}");
        assert_eq!(value["batch"]["state_error"], "state_unparseable");
        // Nothing of the default state is passed off as fact.
        assert!(value["batch"].get("refused").is_none(), "{value}");
        assert!(value["batch"].get("failures").is_none(), "{value}");
        // The other lane is unaffected.
        assert!(value["health"]["pending"].is_i64(), "{value}");
        let lean = cort::measure::render_status(&value);
        assert!(
            lean.starts_with("batch pending=unknown state_error=state_unparseable\n"),
            "{lean}"
        );
        assert!(!lean.contains("failures=none"), "{lean}");
    });
}
