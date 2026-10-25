use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(data: &Path, args: &[&str], harness: Option<&str>) -> Output {
    let cache = data.parent().unwrap().join("cache");
    let claude = data.parent().unwrap().join("claude");
    let xdg = data.parent().unwrap().join("xdg");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cort"));
    cmd.args(args)
        .env("HOME", data.parent().unwrap())
        .env("XDG_DATA_HOME", xdg)
        .env("CLAUDE_CONFIG_DIR", claude)
        .env("CORT_CACHE_DIR", cache)
        .env("CORT_MEMORY_DIR", data)
        .env("CORT_MACHINE_ID", "collect-cli-machine")
        .env("CORT_CRONTAB_FILE", data.parent().unwrap().join("crontab"));
    if let Some(harness) = harness {
        cmd.env("CORT_COLLECT_HARNESS", harness);
    } else {
        cmd.env_remove("CORT_COLLECT_HARNESS");
    }
    cmd.output().unwrap()
}

#[test]
fn collect_cli_background_prints_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        &dir.path().join("data"),
        &["measure", "collect", "--background"],
        None,
    );
    assert!(out.status.success(), "{:?}", out.status);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn background_failure_line_carries_error() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let out = run(
        &data,
        &["measure", "collect", "--background"],
        Some("codex"),
    );
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    let log = fs::read_to_string(data.join("measure.log")).unwrap();
    assert_eq!(log.lines().count(), 1);
    let line: Value = serde_json::from_str(log.trim()).unwrap();
    assert_eq!(line["lane"], "batch");
    assert_eq!(line["outcome"], "failed");
    assert_eq!(line["harness"], "codex");
    assert!(line["error"].as_str().is_some_and(|s| !s.is_empty()));
    let state: Value =
        serde_json::from_slice(&fs::read(data.join("measure-collect-state.json")).unwrap())
            .unwrap();
    assert_eq!(state["failures"]["codex"]["count"], 1);
}

#[test]
fn background_log_write_failure_is_a_cli_error() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    fs::create_dir_all(data.join("measure.log")).unwrap();
    let out = run(&data, &["measure", "collect", "--background"], None);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn foreground_prints_summary_and_background_parse_errors_are_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let foreground = run(&data, &["measure", "collect"], None);
    assert!(foreground.status.success());
    let summary: Value = serde_json::from_slice(&foreground.stdout).unwrap();
    assert_eq!(summary["outcome"], "failed");
    assert_eq!(summary["harness"], "unknown");
    assert!(!data.join("measure.log").exists());
    for extra in ["--help", "--version", "--invalid", "--background=true"] {
        let out = run(&data, &["measure", "collect", "--background", extra], None);
        assert!(out.stdout.is_empty(), "{extra}");
        assert!(out.stderr.is_empty(), "{extra}");
    }
    let other = run(
        &data,
        &["measure", "record", "--background", "--help"],
        None,
    );
    assert!(!other.stdout.is_empty());
}
