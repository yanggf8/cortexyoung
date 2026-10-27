use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    claude: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let data = root.join("data");
        let cache = root.join("cache");
        let claude = root.join("claude");
        for path in [&data, &cache, &claude] {
            fs::create_dir_all(path).unwrap();
        }
        Self {
            _temp: temp,
            root,
            data,
            cache,
            claude,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cort"));
        cmd.current_dir(&self.root)
            .env("CORT_MEMORY_DIR", &self.data)
            .env("CORT_CACHE_DIR", &self.cache)
            .env("CLAUDE_CONFIG_DIR", &self.claude)
            .env("HOME", &self.root)
            .env("XDG_DATA_HOME", &self.root)
            .args(args);
        cmd
    }

    fn lines(&self) -> Vec<Value> {
        fs::read_to_string(self.data.join("memory-sync.log"))
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}

fn output_with_stdin(mut command: Command, payload: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn background_without_config_is_silent_and_logs_error() {
    let f = Fixture::new();
    let out = f
        .command(&["memory", "notes", "sync", "--background"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    let lines = f.lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["outcome"], "error");
}

#[test]
fn background_with_held_run_lock_logs_busy() {
    let f = Fixture::new();
    let lock = cort::upgrade::try_exclusive_lock(&f.data.join("notes-sync.lock")).unwrap();
    let out = f
        .command(&["memory", "notes", "sync", "--background"])
        .env("CORT_NOTES_SYNC_TRIGGER", "edit")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert_eq!(f.lines()[0]["outcome"], "busy");
    // The sibling holding the run lock may have scanned before this edit: leave it one more pass.
    assert!(f.data.join("notes-sync.pending").is_file());
    drop(lock);
}

#[test]
fn background_repeats_when_a_pending_edit_arrives_during_sync() {
    let f = Fixture::new();
    let url = "https://example.invalid";
    fs::write(
        f.data.join("turso-memory.json"),
        format!(r#"{{"url":"{url}","token":"test"}}"#),
    )
    .unwrap();
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let lock_path = f.data.join("turso-memory").join(hex).join("memory.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let memory_lock = cort::upgrade::try_exclusive_lock(&lock_path).unwrap();
    let child = f
        .command(&["memory", "notes", "sync", "--background"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let run_lock_path = f.data.join("notes-sync.lock");
    let mut running = false;
    for _ in 0..200 {
        if matches!(
            cort::upgrade::try_exclusive_lock(&run_lock_path),
            Err(cort::upgrade::LockError::AdmissionBusy)
        ) {
            running = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(running, "background child never acquired run lock");
    std::thread::sleep(Duration::from_millis(100));
    fs::write(f.data.join("notes-sync.pending"), "1").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    let lines = f.lines();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["outcome"], "error");
    assert_eq!(lines[1]["outcome"], "error");
    drop(memory_lock);
}

#[test]
fn hook_refresh_kicks_memory_edit_without_an_index_and_records_tag() {
    let f = Fixture::new();
    fs::write(f.data.join("turso-memory.json"), "invalid").unwrap();
    fs::write(f.data.join("notes-opt-in"), "project\n").unwrap();
    let memory = f.claude.join("projects/one/memory");
    fs::create_dir_all(&memory).unwrap();
    let note = memory.join("note.md");
    fs::write(&note, "note").unwrap();
    let payload =
        serde_json::json!({"tool_name":"Write","tool_input":{"file_path":note},"cwd":f.root});
    let out = output_with_stdin(
        f.command(&["hook-refresh", "--harness", "claude-code"]),
        &payload.to_string(),
    );
    assert!(out.status.success());
    assert!(f.data.join("notes-sync.spawned").exists());
    // The detached child must finish inside the tempdir, or it recreates `data` after cleanup.
    for _ in 0..500 {
        if f.data.join("memory-sync.log").is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.lines()[0]["trigger"], "edit");
    let usage_path = f.cache.join("usage.db");
    let db = rusqlite::Connection::open(usage_path).unwrap();
    let row: String = db.query_row("SELECT args_summary FROM command_log WHERE command='hook-refresh' ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    let value: Value = serde_json::from_str(&row).unwrap();
    assert_eq!(value["notes"], "spawned_edit");
}
