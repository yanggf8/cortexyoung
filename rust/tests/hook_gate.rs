use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(dir: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cort"))
        .args(args)
        .env("CORT_MEMORY_DIR", dir.path())
        .env("CORT_CACHE_DIR", dir.path().join("cache"))
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn absent_set_status_clear_round_trip() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        json(&run(&dir, &["hook-gate", "status"]))["state"],
        "unconfigured"
    );
    let executable = std::env::current_exe().unwrap();
    let executable = executable.to_str().unwrap();
    let set = run(
        &dir,
        &[
            "hook-gate",
            "set",
            "--timeout-ms",
            "50",
            "--",
            executable,
            "--timeout-ms",
            "999",
        ],
    );
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stdout)
    );
    let status = json(&run(&dir, &["hook-gate", "status", "-f", "json"]));
    assert_eq!(status["state"], "configured");
    assert_eq!(
        status["command"],
        serde_json::json!([executable, "--timeout-ms", "999"])
    );
    assert_eq!(status["timeout_ms"], 50);
    assert_eq!(status["executable_exists"], true);
    assert_eq!(status["executable"], true);
    assert_eq!(
        status["path"],
        dir.path().join("gate.json").to_str().unwrap()
    );
    let lean = run(&dir, &["hook-gate", "status", "-f", "lean"]);
    assert!(String::from_utf8_lossy(&lean.stdout).contains("state\tconfigured"));
    assert_eq!(json(&run(&dir, &["hook-gate", "clear"]))["removed"], true);
    assert_eq!(json(&run(&dir, &["hook-gate", "clear"]))["removed"], false);
    assert_eq!(
        json(&run(&dir, &["hook-gate", "status"]))["state"],
        "unconfigured"
    );
}

#[test]
fn help_and_version_after_separator_belong_to_the_gate() {
    let dir = TempDir::new().unwrap();
    let executable = std::env::current_exe().unwrap();
    let executable = executable.to_str().unwrap();
    for flag in ["--help", "-h", "--version", "-V"] {
        let set = run(&dir, &["hook-gate", "set", "--", executable, flag]);
        assert!(set.status.success(), "{flag}");
        let status = json(&run(&dir, &["hook-gate", "status"]));
        assert_eq!(status["command"], serde_json::json!([executable, flag]));
    }
}

#[test]
fn relative_executable_refused() {
    let dir = TempDir::new().unwrap();
    let result = run(&dir, &["hook-gate", "set", "--", "relative", "gate"]);
    assert!(!result.status.success());
    assert!(!Path::new(&dir.path().join("gate.json")).exists());
}

#[test]
fn invalid_files_report_reason() {
    let dir = TempDir::new().unwrap();
    for body in [
        "not json",
        "null",
        "[]",
        "{}",
        "{\"v\":2,\"command\":[\"/bin/x\"]}",
        "{\"v\":\"1\",\"command\":[\"/bin/x\"]}",
        "{\"v\":1}",
        "{\"v\":1,\"command\":[]}",
        "{\"v\":1,\"command\":[\"relative\"]}",
        "{\"v\":1,\"command\":\"/bin/x\"}",
        "{\"v\":1,\"command\":[3]}",
        "{\"v\":1,\"command\":[\"/bin/x\",false]}",
        "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":49}",
        "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":1001}",
        "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":\"200\"}",
        "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":null}",
    ] {
        fs::write(dir.path().join("gate.json"), body).unwrap();
        let status = json(&run(&dir, &["hook-gate", "status"]));
        assert_eq!(status["state"], "invalid", "{body}");
        assert!(status["reason"].as_str().is_some_and(|s| !s.is_empty()));
    }
    fs::remove_file(dir.path().join("gate.json")).unwrap();
    fs::create_dir(dir.path().join("gate.json")).unwrap();
    assert_eq!(
        json(&run(&dir, &["hook-gate", "status"]))["state"],
        "invalid"
    );
}

#[test]
fn missing_executable_is_reported_without_invalidating_config() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let path = missing.to_str().unwrap();
    assert!(run(&dir, &["hook-gate", "set", "--", path])
        .status
        .success());
    let status = json(&run(&dir, &["hook-gate", "status"]));
    assert_eq!(status["state"], "configured");
    assert_eq!(status["timeout_ms"], 200);
    assert_eq!(status["executable_exists"], false);
    assert_eq!(status["executable"], false);
}
