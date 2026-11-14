//! `cort hook-remind`: Claude Code's `UserPromptSubmit` entry. It reads only the local snapshot
//! and the session's shown set -- no memory config, no network -- and is silent on every failure.

use cort::remind::{parse_event, ReminderEvent};
use cort::remind_snapshot::{write_snapshot, Snapshot};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DAY: i128 = 86_400_000_000_000;

fn now_ns() -> i128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i128
}

fn temp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cort-hook-remind-")
        .tempdir()
        .unwrap()
}

fn git_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    assert!(Command::new("git")
        .arg("init")
        .arg(&repo)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.com/team/project.git"
        ])
        .current_dir(&repo)
        .output()
        .unwrap()
        .status
        .success());
    repo
}

fn overdue_add(due_at: i128) -> ReminderEvent {
    parse_event(&format!(
        r#"{{"id":"a614d4e7aaaa","kind":"add","text":"Reboot the NUC","on":"2026-11-10","utc_offset":"+00:00","due_at":"{due_at}","lead":1,"recorded_at":"1"}}"#
    ))
    .unwrap()
}

/// Write a snapshot for the repo's project under `data`; `read_at` in ns.
fn seed(data: &Path, repo: &Path, read_at: i128, events: Vec<ReminderEvent>) {
    let project = cort::memory_store::session_project_ids(repo).unwrap().0;
    assert!(write_snapshot(data, &Snapshot::new(&project, read_at, events)).unwrap());
}

fn run(root: &Path, stdin: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cort"))
        .args(["hook-remind", "--harness", "claude-code"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root)
        .env("CORT_MEMORY_DIR", root.join("data"))
        .env("CORT_CACHE_DIR", root.join("cache"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("CORT_MACHINE_ID", "self")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    let _ = pipe.write_all(stdin.as_bytes());
    drop(pipe);
    child.wait_with_output().unwrap()
}

fn payload(repo: &Path, session: &str) -> String {
    json!({"hook_event_name":"UserPromptSubmit","session_id":session,"cwd":repo}).to_string()
}

fn outcomes(root: &Path) -> Vec<Value> {
    let db = rusqlite::Connection::open(root.join("cache/usage.db")).unwrap();
    let mut stmt = db
        .prepare(
            "SELECT args_summary FROM command_log WHERE command = 'hook-remind' ORDER BY rowid",
        )
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|s| serde_json::from_str(&s.unwrap()).unwrap())
        .collect()
}

#[test]
fn hook_remind_shows_once_then_silent() {
    let root = temp();
    let repo = git_repo(root.path());
    let data = root.path().join("data");
    seed(
        &data,
        &repo,
        now_ns(),
        vec![overdue_add(now_ns() - 2 * DAY)],
    );
    assert!(
        !data.join("turso-memory.json").exists(),
        "the hook must need no memory config"
    );

    let first = run(root.path(), &payload(&repo, "s1"));
    assert_eq!(first.status.code(), Some(0));
    let out: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(
        out["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    let text = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("a614d4e7"), "{text}");
    assert!(text.contains("Reboot the NUC"), "{text}");

    let second = run(root.path(), &payload(&repo, "s1"));
    assert_eq!(second.status.code(), Some(0));
    assert!(second.stdout.is_empty(), "{:?}", second.stdout);

    // Another session has its own shown set.
    let other = run(root.path(), &payload(&repo, "s2"));
    assert!(!other.stdout.is_empty());

    let rows = outcomes(root.path());
    assert_eq!(rows[0]["hook"], "shown");
    assert_eq!(rows[0]["reminders_shown"], 1);
    assert_eq!(rows[1]["hook"], "none");
}

#[test]
fn hook_remind_stale_snapshot_line_once() {
    let root = temp();
    let repo = git_repo(root.path());
    let data = root.path().join("data");
    seed(
        &data,
        &repo,
        now_ns() - 4 * DAY,
        vec![overdue_add(now_ns() - 6 * DAY)],
    );
    let first = run(root.path(), &payload(&repo, "s1"));
    assert_eq!(first.status.code(), Some(0));
    let out: Value = serde_json::from_slice(&first.stdout).unwrap();
    let text = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.starts_with("cort reminders: the local snapshot is from "),
        "{text}"
    );
    assert!(!text.contains("a614d4e7"), "no reminder lines: {text}");
    let second = run(root.path(), &payload(&repo, "s1"));
    assert!(second.stdout.is_empty());
    let rows = outcomes(root.path());
    assert_eq!(rows[0]["hook"], "stale_snapshot");
}

#[test]
fn hook_remind_no_snapshot_silent() {
    let root = temp();
    let repo = git_repo(root.path());
    let out = run(root.path(), &payload(&repo, "s1"));
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert_eq!(outcomes(root.path())[0]["hook"], "no_snapshot");
}

#[test]
fn hook_remind_missing_session_and_no_project_are_silent() {
    let root = temp();
    let repo = git_repo(root.path());
    let no_session = run(
        root.path(),
        &json!({"hook_event_name":"UserPromptSubmit","cwd":repo}).to_string(),
    );
    assert_eq!(no_session.status.code(), Some(0));
    assert!(no_session.stdout.is_empty());
    let bare = root.path().join("bare");
    fs::create_dir(&bare).unwrap();
    let no_project = run(root.path(), &payload(&bare, "s1"));
    assert_eq!(no_project.status.code(), Some(0));
    assert!(no_project.stdout.is_empty());
    let rows = outcomes(root.path());
    assert_eq!(rows[0]["hook"], "missing_session_id");
    assert_eq!(rows[1]["hook"], "no_project");
}

#[test]
fn hook_remind_bad_stdin_exit_0() {
    let root = temp();
    for stdin in ["", "not json", "[1,2]", "{\"cwd\":3}"] {
        let out = run(root.path(), stdin);
        assert_eq!(out.status.code(), Some(0), "stdin {stdin:?}");
        assert!(out.stdout.is_empty(), "stdin {stdin:?}");
    }
}
