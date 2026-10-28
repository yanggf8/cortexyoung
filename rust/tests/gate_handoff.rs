use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn fake_gate() -> &'static str {
    env!("CARGO_BIN_EXE_fake_gate")
}

fn configure(dir: &TempDir, args: &[&str], timeout_ms: u64) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cort"));
    cmd.args([
        "hook-gate",
        "set",
        "--timeout-ms",
        &timeout_ms.to_string(),
        "--",
    ])
    .args(args)
    .env("CORT_MEMORY_DIR", dir.path().join("memory"))
    .env("CORT_CACHE_DIR", dir.path().join("cache"));
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

fn run(dir: &TempDir, tool: &str, command: &str) -> (Value, Value, Duration) {
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cort"))
        .arg("hook-suggest")
        .env("CORT_MEMORY_DIR", dir.path().join("memory"))
        .env("CORT_CACHE_DIR", dir.path().join("cache"))
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = json!({"tool_name": tool, "tool_input": {"command": command}}).to_string();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let elapsed = started.elapsed();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = dir.path().join("cache/usage.db");
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let row: String = conn.query_row(
        "SELECT args_summary FROM command_log WHERE command='hook-suggest' ORDER BY id DESC LIMIT 1",
        [], |r| r.get(0),
    ).unwrap();
    (response, serde_json::from_str(&row).unwrap(), elapsed)
}

fn check(dir: &TempDir, args: &[&str], timeout: u64, expected: &str, reason: Option<&str>) {
    configure(dir, args, timeout);
    let (response, row, _) = run(dir, "Bash", "cargo test");
    assert_eq!(row["hook"], "fired");
    assert_eq!(row["gate"], expected);
    if expected == "spawn_failed" {
        assert!(row.get("gate_ms").is_none());
    } else {
        assert!(row["gate_ms"].as_str().unwrap().parse::<u64>().is_ok());
    }
    match reason {
        Some(reason) => {
            assert_eq!(
                response["hookSpecificOutput"]["hookEventName"],
                "PreToolUse"
            );
            assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
            assert_eq!(
                response["hookSpecificOutput"]["permissionDecisionReason"],
                reason
            );
            assert!(response["hookSpecificOutput"]
                .get("suppressOutput")
                .is_none());
        }
        None => assert_eq!(response, json!({})),
    }
}

#[test]
fn gate_exit_modes_and_reason_shape() {
    for (args, tag, reason) in [
        (vec![fake_gate(), "allow"], "allow", None),
        (
            vec![fake_gate(), "deny", " denied "],
            "deny",
            Some("denied"),
        ),
        (
            vec![fake_gate(), "deny-empty"],
            "deny",
            Some("resource gate denied this command (no reason given)"),
        ),
        (vec![fake_gate(), "fail"], "error", None),
        (vec![fake_gate(), "abort"], "error", None),
    ] {
        check(&TempDir::new().unwrap(), &args, 200, tag, reason);
    }
}

#[test]
fn timeout_reaps_direct_child() {
    let dir = TempDir::new().unwrap();
    let pid_file = dir.path().join("pid");
    configure(
        &dir,
        &[fake_gate(), "sleep", "10000", pid_file.to_str().unwrap()],
        100,
    );
    let (response, row, elapsed) = run(&dir, "Bash", "cargo test");
    assert_eq!(response, json!({}));
    assert_eq!(row["gate"], "timeout");
    assert!(row["gate_ms"].as_str().is_some());
    // The gate sleeps 10 s: a hook that waited for it would take that long, so 5 s separates a
    // regression from a loaded CI runner without timing the hook's own bookkeeping.
    assert!(elapsed < SLOW_GATE / 2, "{elapsed:?}");
    let pid = std::fs::read_to_string(pid_file).unwrap();
    // `kill -0` succeeds on a live process and on an unreaped zombie alike, and works where there
    // is no `/proc` (macOS), so a failure here means the child was killed and reaped.
    let probe = Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!probe.success(), "gate child {pid} still exists");
}

/// How long the fixtures that must not hold the hook keep running.
const SLOW_GATE: Duration = Duration::from_secs(10);

#[test]
fn blocked_stdin_and_inherited_stdout_do_not_hold_hook() {
    let dir = TempDir::new().unwrap();
    // A gate that never reads a payload larger than the pipe buffer: a hook that wrote stdin on
    // its main thread would block until the gate exits 10 s later instead of timing out.
    configure(&dir, &[fake_gate(), "ignore-stdin", "10000"], 200);
    let (response, row, elapsed) = run(&dir, "Bash", &"x".repeat(1_100_000));
    assert_eq!(response, json!({}));
    assert_eq!(row["gate"], "timeout");
    assert!(elapsed < SLOW_GATE / 2, "{elapsed:?}");

    // The gate exits 0 at once but leaves a 10 s grandchild holding its stdout.
    configure(&dir, &[fake_gate(), "grandchild"], 200);
    let (response, row, elapsed) = run(&dir, "Bash", "cargo test");
    assert_eq!(response, json!({}));
    assert_eq!(row["gate"], "allow");
    assert!(elapsed < SLOW_GATE / 2, "{elapsed:?}");
}

#[test]
fn deny_reason_is_capped_at_4096_bytes() {
    let dir = TempDir::new().unwrap();
    configure(&dir, &[fake_gate(), "flood"], 200);
    let (response, row, _) = run(&dir, "Bash", "cargo test");
    assert_eq!(row["gate"], "deny");
    assert_eq!(
        response["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .len(),
        4096
    );
}

#[test]
fn missing_invalid_and_non_shell_do_not_spawn() {
    let dir = TempDir::new().unwrap();
    let (response, row, _) = run(&dir, "Bash", "cargo test");
    assert_eq!(response, json!({}));
    assert_eq!(row["gate"], "unconfigured");
    assert!(row.get("gate_ms").is_none());

    std::fs::create_dir_all(dir.path().join("memory")).unwrap();
    for body in [
        "bad json",
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
        std::fs::write(dir.path().join("memory/gate.json"), body).unwrap();
        let (response, row, _) = run(&dir, "Bash", "cargo test");
        assert_eq!(response, json!({}), "{body}");
        assert_eq!(row["gate"], "config_invalid", "{body}");
        assert!(row.get("gate_ms").is_none());
    }

    configure(&dir, &["/missing/fake_gate"], 200);
    let (_, row, _) = run(&dir, "Bash", "cargo test");
    assert_eq!(row["gate"], "spawn_failed");
    assert!(row.get("gate_ms").is_none());

    configure(&dir, &[fake_gate(), "deny", "should not run"], 200);
    let (response, row, _) = run(&dir, "Grep", "cargo test");
    assert_eq!(response, json!({}));
    assert_eq!(row["gate"], "not_shell");
    assert!(row.get("gate_ms").is_none());
}
