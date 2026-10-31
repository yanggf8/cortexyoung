use cort::remote::{RemoteDb, RemoteError, SqliteTransport, Transport};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Recording {
    inner: Arc<SqliteTransport>,
    read_only: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Vec<String>>>>,
    delay: Duration,
}
impl Transport for Recording {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        let request: Value = serde_json::from_slice(body).expect("request JSON");
        let statements: Vec<String> = request["requests"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| {
                item.get("stmt")
                    .and_then(|s| s.get("sql"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        if self.read_only.load(Ordering::SeqCst) {
            assert!(
                statements.iter().all(|s| s.starts_with("SELECT ")),
                "{statements:?}"
            );
            self.requests.lock().unwrap().push(statements);
            std::thread::sleep(self.delay);
        }
        self.inner.post(url, body, headers)
    }
}
type Requests = Arc<Mutex<Vec<Vec<String>>>>;
fn database() -> (RemoteDb, Arc<AtomicBool>, Requests) {
    database_with_delay(Duration::ZERO)
}
fn database_with_delay(delay: Duration) -> (RemoteDb, Arc<AtomicBool>, Requests) {
    let read_only = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let db = RemoteDb::new(
        "turso://unit.test",
        "test",
        Box::new(Recording {
            inner: Arc::new(SqliteTransport::new().unwrap()),
            read_only: Arc::clone(&read_only),
            requests: Arc::clone(&requests),
            delay,
        }),
    )
    .unwrap();
    (db, read_only, requests)
}
fn entry(id: &str, date: &str) -> String {
    json!({"id":id,"key":id,"claim":id,"source":"missing.md","source_line":1,
        "source_sha256":"missing","recorded_at":date,"supersedes":null})
    .to_string()
}
fn seed_ledger(db: &RemoteDb) {
    db.execute(
        "CREATE TABLE project_memory (project TEXT, id TEXT, body TEXT)",
        vec![],
    )
    .unwrap();
    for i in 0..25 {
        let id = format!("id-{i:02}");
        db.execute(
            "INSERT INTO project_memory VALUES (?, ?, ?)",
            vec![
                cort::remote::Cell::Text("ledger".into()),
                cort::remote::Cell::Text(id.clone()),
                cort::remote::Cell::Text(entry(&id, &format!("2026-10-{i:02}"))),
            ],
        )
        .unwrap();
    }
}
fn seed_notes(db: &RemoteDb) {
    db.execute("CREATE TABLE harness_notes (project TEXT, machine TEXT, source_dir TEXT, file_name TEXT, note_id TEXT, deleted INTEGER, updated_at TEXT)", vec![]).unwrap();
    db.execute(
        "CREATE TABLE harness_machines (machine TEXT, host TEXT)",
        vec![],
    )
    .unwrap();
    db.execute(
        "INSERT INTO harness_machines VALUES ('other', 'host-one')",
        vec![],
    )
    .unwrap();
    for i in 0..19 {
        let machine = if i == 0 {
            "self"
        } else if i == 1 {
            "nohost"
        } else {
            "other"
        };
        db.execute(
            "INSERT INTO harness_notes VALUES (?, ?, 'src', ?, ?, 0, ?)",
            vec![
                cort::remote::Cell::Text("origin".into()),
                cort::remote::Cell::Text(machine.into()),
                cort::remote::Cell::Text(format!("file-{i}")),
                cort::remote::Cell::Text(format!("note-{i}")),
                cort::remote::Cell::Text(format!("2026-10-27T00:{:02}:00Z", 59 - i)),
            ],
        )
        .unwrap();
    }
}
fn seed_alias(db: &RemoteDb) {
    db.execute(
        "CREATE TABLE project_aliases (old_project TEXT, new_project TEXT)",
        vec![],
    )
    .unwrap();
    db.execute(
        "INSERT INTO project_aliases VALUES ('old', 'ledger')",
        vec![],
    )
    .unwrap();
}
fn seed_old_entry(db: &RemoteDb, id: &str) {
    db.execute(
        "INSERT INTO project_memory VALUES (?, ?, ?)",
        vec![
            cort::remote::Cell::Text("old".into()),
            cort::remote::Cell::Text(id.into()),
            cort::remote::Cell::Text(entry(id, "2026-11-01")),
        ],
    )
    .unwrap();
}
#[test]
fn pipeline_reads_ledger_and_other_machine_notes_without_writes() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    seed_alias(&db);
    seed_old_entry(&db, "old-entry");
    seed_notes(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let (recall, omitted, notes, total) = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    )
    .unwrap();
    let (rendered, shown, note_count) =
        cort::hook_recall::render_with_notes(recall, omitted, &notes, total).unwrap();
    assert_eq!(shown, 20);
    assert_eq!(note_count, 15);
    assert_eq!(total, 18);
    assert!(rendered.contains("[old-entry]"));
    assert!(rendered.find("[id-24]").unwrap() < rendered.find("[id-23]").unwrap());
    assert!(rendered.contains("[nohost]"));
    assert!(rendered.contains("[host-one]"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(requests.lock().unwrap()[0].len(), 3);
}
#[test]
fn missing_notes_schema_retries_ledger_and_creates_nothing() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    seed_alias(&db);
    seed_old_entry(&db, "old-entry");
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let (recall, _, notes, total) = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(recall.hits.len(), 20);
    assert!(recall.hits.iter().any(|hit| hit.entry.id == "old-entry"));
    assert!(notes.is_empty());
    assert_eq!(total, 0);
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        vec![3, 1]
    );
    assert!(requests.lock().unwrap()[1][0].contains("project IN (WITH RECURSIVE"));
    read_only.store(false, Ordering::SeqCst);
    let tables = db
        .execute(
            "SELECT name FROM sqlite_master WHERE name IN ('harness_notes','harness_machines')",
            vec![],
        )
        .unwrap();
    assert!(tables.rows.is_empty());
}

#[test]
fn missing_host_table_keeps_ledger_and_does_not_create_host_table() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    seed_alias(&db);
    db.execute("CREATE TABLE harness_notes (project TEXT, machine TEXT, source_dir TEXT, file_name TEXT, note_id TEXT, deleted INTEGER, updated_at TEXT)", vec![]).unwrap();
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let (recall, _, notes, total) = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(recall.hits.len(), 20);
    assert!(notes.is_empty());
    assert_eq!(total, 0);
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        vec![3, 1]
    );
    read_only.store(false, Ordering::SeqCst);
    let tables = db
        .execute(
            "SELECT name FROM sqlite_master WHERE name='harness_machines'",
            vec![],
        )
        .unwrap();
    assert!(tables.rows.is_empty());
}

#[test]
fn missing_alias_table_retries_full_pipeline_and_keeps_notes() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    seed_notes(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let (recall, _, notes, total) = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(recall.hits.len(), 20);
    assert_eq!(notes.len(), 15);
    assert_eq!(total, 18);
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![3, 3]
    );
    assert!(requests[0][0].contains("project IN (WITH RECURSIVE"));
    assert!(requests[1][0].contains("project = ?"));
    assert_eq!(&requests[0][1..], &requests[1][1..]);
}

#[test]
fn missing_alias_and_notes_tables_retries_unaliased_ledger_only() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let (recall, _, notes, total) = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(recall.hits.len(), 20);
    assert!(notes.is_empty());
    assert_eq!(total, 0);
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![3, 3, 1]
    );
    assert!(requests[0][0].contains("project IN (WITH RECURSIVE"));
    assert!(requests[1][0].contains("project = ?"));
    assert_eq!(requests[1][0], requests[2][0]);
}

#[test]
fn duplicate_aliased_entry_is_returned_once() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    seed_alias(&db);
    seed_old_entry(&db, "shared");
    db.execute(
        "INSERT INTO project_memory VALUES (?, ?, ?)",
        vec![
            cort::remote::Cell::Text("ledger".into()),
            cort::remote::Cell::Text("shared".into()),
            cort::remote::Cell::Text(entry("shared", "2026-11-01")),
        ],
    )
    .unwrap();
    seed_notes(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let (recall, omitted, _, _) = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        recall
            .hits
            .iter()
            .filter(|hit| hit.entry.id == "shared")
            .count(),
        1
    );
    assert_eq!(omitted, 6);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn conflicting_aliased_entry_returns_sql_error() {
    let (db, read_only, requests) = database();
    seed_ledger(&db);
    seed_alias(&db);
    seed_old_entry(&db, "shared");
    db.execute(
        "INSERT INTO project_memory VALUES (?, ?, ?)",
        vec![
            cort::remote::Cell::Text("ledger".into()),
            cort::remote::Cell::Text("shared".into()),
            cort::remote::Cell::Text(entry("shared", "2026-11-02")),
        ],
    )
    .unwrap();
    seed_notes(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let result = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now(),
    );
    assert!(matches!(result, Err(cort::remote::ErrorKind::Sql)));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn alias_failure_without_budget_for_retry_is_timeout() {
    // 1150 ms remain before the pipeline; the 1000 ms response leaves under 200 ms for the retry.
    let (db, read_only, requests) = database_with_delay(Duration::from_millis(1000));
    seed_ledger(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let result = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now() - (cort::remote::RECALL_DEADLINE - Duration::from_millis(1150)),
    );
    assert!(matches!(result, Err(cort::remote::ErrorKind::Timeout)));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0][0].contains("project IN (WITH RECURSIVE"));
}

#[test]
fn expired_dispatch_budget_sends_no_request() {
    let (db, read_only, requests) = database();
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let result = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now() - (cort::remote::RECALL_DEADLINE - Duration::from_millis(100)),
    );
    assert!(matches!(result, Err(cort::remote::ErrorKind::Timeout)));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn notes_failure_without_budget_for_retry_is_timeout() {
    let (db, read_only, requests) = database_with_delay(Duration::from_millis(1000));
    seed_ledger(&db);
    seed_alias(&db);
    read_only.store(true, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    // 1150 ms remain before the pipeline; the 1000 ms response leaves under 200 ms for the retry.
    // The windows are wide so a loaded machine cannot spend the budget before the first request.
    let result = cort::memory_store::session_recall_with(
        temp.path(),
        &db,
        "self",
        ("ledger", "origin"),
        Instant::now() - (cort::remote::RECALL_DEADLINE - Duration::from_millis(1150)),
    );
    assert!(matches!(result, Err(cort::remote::ErrorKind::Timeout)));
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        vec![3]
    );
}
fn config(data: &Path, url: &str) {
    fs::create_dir_all(data).unwrap();
    fs::write(
        data.join("turso-memory.json"),
        json!({"url":url,"token":"test"}).to_string(),
    )
    .unwrap();
}
fn git_repo(root: &Path) -> std::path::PathBuf {
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
fn run(
    root: &Path,
    cwd: &Path,
    stdin: &str,
    timeout: Option<&str>,
) -> (std::process::Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cort"));
    cmd.args(["hook-recall", "--harness", "claude-code"])
        .env("CORT_MEMORY_DIR", root.join("data"))
        .env("CORT_CACHE_DIR", root.join("cache"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("HOME", root)
        .env_remove("CORT_MEMORY_PROJECT")
        .env_remove("CLAUDECAT_MEMORY_PROJECT")
        .env("CORT_MACHINE_ID", "self")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(value) = timeout {
        cmd.env("CORT_RECALL_TIMEOUT_MS", value);
    }
    let started = Instant::now();
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    (child.wait_with_output().unwrap(), started.elapsed())
}
fn outcome(root: &Path) -> Value {
    let db = rusqlite::Connection::open(root.join("cache/usage.db")).unwrap();
    let raw: String = db.query_row("SELECT args_summary FROM command_log WHERE command='hook-recall' ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn notice(out: &std::process::Output, kind: &str) {
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["hookSpecificOutput"]["additionalContext"], format!("cort memory: the shared database could not be read at session start ({kind}); run `cort memory recall \"<topic>\"` when you need it"));
}
#[test]
fn cli_unconfigured_and_invalid_input_are_silent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    let payload = json!({"cwd":repo}).to_string();
    let (out, _) = run(root, root, &payload, None);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(root)["hook"], "not_configured");
    config(&root.join("data"), "turso://127.0.0.1:9");
    let (out, _) = run(root, root, "not JSON", None);
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(root)["hook"], "failed");
}
#[cfg(unix)]
#[test]
fn cli_dangling_config_symlink_is_failed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    fs::create_dir(root.join("data")).unwrap();
    std::os::unix::fs::symlink(root.join("missing"), root.join("data/turso-memory.json")).unwrap();
    let (out, _) = run(root, root, &json!({"cwd":repo}).to_string(), None);
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(root)["hook"], "failed");
}
struct SilentServer {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl SilentServer {
    fn new() -> Option<Self> {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("SKIP: sandbox prohibits local listener: {e}");
                return None;
            }
            Err(e) => panic!("listener: {e}"),
        };
        listener.set_nonblocking(true).unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut streams = Vec::new();
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => streams.push(stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            }
        });
        Some(Self {
            url,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for SilentServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
#[test]
fn cli_silent_server_times_out_and_clamps() {
    let Some(server) = SilentServer::new() else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    config(&root.join("data"), &server.url);
    let payload = json!({"cwd":repo}).to_string();
    // 99999 clamps to the 3700 ms cap. The notice and its row are the real check: they are missing
    // when the watchdog (deadline + 900 ms) exits first. The bound is the harnesses' 5 s hook limit.
    for (setting, bound) in [("400", 3000), ("99999", 4950)] {
        let (out, elapsed) = run(root, root, &payload, Some(setting));
        assert!(out.status.success());
        assert!(elapsed < Duration::from_millis(bound), "{elapsed:?}");
        assert!(
            !out.stdout.is_empty(),
            "setting {setting}: no output after {elapsed:?}, so the watchdog exited first"
        );
        notice(&out, "remote_timeout");
        assert_eq!(outcome(root)["hook"], "remote_timeout");
    }
}
#[test]
fn cli_unbound_port_and_payload_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    config(&root.join("data"), "turso://127.0.0.1:9");
    let (out, elapsed) = run(root, root, &json!({"cwd":repo}).to_string(), Some("400"));
    assert!(elapsed < Duration::from_millis(3500));
    let kind = outcome(root)["hook"].as_str().unwrap().to_owned();
    assert!(
        kind == "remote_unreachable" || kind == "remote_timeout",
        "{kind}"
    );
    notice(&out, &kind);
    assert!(outcome(root)["hook"].as_str().is_some());
}

#[test]
fn kimi_prompt_claims_session_even_when_remote_fails() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    config(&root.join("data"), "turso://127.0.0.1:9");
    let payload = |id: Option<&str>| {
        json!({"hook_event_name":"UserPromptSubmit", "session_id":id, "cwd":repo}).to_string()
    };
    let (first, _) = run(root, root, &payload(Some("session_one")), Some("400"));
    assert!(first.status.success());
    let value: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(value["message"].as_str().unwrap().contains("remote_"));
    assert!(!String::from_utf8_lossy(&first.stdout).contains("permissionDecision"));
    assert!(!String::from_utf8_lossy(&first.stdout).contains("block"));
    // An invalid config proves the second call does not read it.
    fs::write(root.join("data/turso-memory.json"), "invalid").unwrap();
    let (second, _) = run(root, root, &payload(Some("session_one")), Some("400"));
    assert!(second.status.success() && second.stdout.is_empty());
    assert_eq!(outcome(root)["hook"], "already_injected");
    config(&root.join("data"), "turso://127.0.0.1:9");
    let (other, _) = run(root, root, &payload(Some("session_two")), Some("400"));
    assert!(other.status.success());
    assert!(serde_json::from_slice::<Value>(&other.stdout).unwrap()["message"].is_string());
    let (missing, _) = run(root, root, &payload(None), Some("400"));
    assert!(missing.status.success() && missing.stdout.is_empty());
    assert_eq!(outcome(root)["hook"], "missing_session_id");
}

#[test]
fn kimi_unwritable_marker_is_silent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("data"), "not a directory").unwrap();
    let (out, _) = run(
        root,
        root,
        &json!({"hook_event_name":"UserPromptSubmit", "session_id":"s"}).to_string(),
        None,
    );
    assert!(out.status.success() && out.stdout.is_empty());
    assert_eq!(outcome(root)["hook"], "marker_unavailable");
}
#[test]
fn render_orders_and_caps_entries() {
    use cort::memory::{MemoryEntry, MemoryHit, MemoryRecall};
    let hits = (0..30)
        .map(|i| MemoryHit {
            entry: MemoryEntry {
                id: format!("id-{i}"),
                key: if i == 29 {
                    "key\u{1b}-29".into()
                } else {
                    format!("key-{i}")
                },
                claim: if i == 29 {
                    format!("tab\t{}", "é".repeat(500))
                } else {
                    "é".repeat(500)
                },
                source: if i == 29 {
                    "docs/\u{1b}memory.md".into()
                } else {
                    "docs/memory.md".into()
                },
                source_line: i + 1,
                source_sha256: String::new(),
                unsourced: i == 28,
                source_line_sha256: None,
                recorded_at: format!("2026-10-{:02}", i + 1),
                supersedes: None,
                resolves: vec![],
                recorded_machine: None,
                recorded_machine_source: None,
            },
            matched_terms: None,
            evidence_status: "source_unchanged".into(),
            source_file_changed: Some(false),
            source_line_status: None,
            source_line_now: None,
        })
        .collect();
    let (text, count) = cort::hook_recall::render(MemoryRecall {
        ledger_status: "present".into(),
        sync_status: None,
        match_kind: None,
        hits,
    })
    .unwrap();
    assert!(text.len() <= 6000);
    assert!(count <= 20);
    assert!(text.contains("key -29"));
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\t'));
    assert!(text.contains("entries left out"));
    assert!(text.contains("(unsourced)"));
    assert!(text.find("key -29").unwrap() < text.find("key-28").unwrap());

    let short_hits = (0..30)
        .map(|i| MemoryHit {
            entry: MemoryEntry {
                id: format!("id-{i}"),
                key: format!("key-{i}"),
                claim: "short".into(),
                source: "x".into(),
                source_line: 1,
                source_sha256: String::new(),
                unsourced: false,
                source_line_sha256: None,
                recorded_at: format!("{i:03}"),
                supersedes: None,
                resolves: vec![],
                recorded_machine: None,
                recorded_machine_source: None,
            },
            matched_terms: None,
            evidence_status: if i == 29 {
                "source_moved"
            } else {
                "source_missing"
            }
            .into(),
            source_file_changed: (i == 29).then_some(true),
            source_line_status: (i == 29).then(|| "moved".into()),
            source_line_now: (i == 29).then_some(7),
        })
        .collect();
    let (short, count) = cort::hook_recall::render(MemoryRecall {
        ledger_status: "present".into(),
        sync_status: None,
        match_kind: None,
        hits: short_hits,
    })
    .unwrap();
    assert_eq!(count, 20);
    assert!(short.contains("10 entries left out"));
    // A moved entry is cited at the line it sits on now, not the recorded one.
    assert!(short.contains("- [key-29] short (x:7, source_moved)"));
}

#[test]
fn notes_section_byte_cap_binds_before_the_count_cap() {
    use cort::memory::MemoryRecall;
    use cort::notes_upload::NoteBrief;
    let notes: Vec<NoteBrief> = (0..15)
        .map(|i| NoteBrief {
            note_id: format!("note-{i:02}"),
            machine: "remote-machine".into(),
            host: Some("NUC11i5".into()),
            file_name: format!("{i:02}{}", "é".repeat(200)),
            updated_at: "2026-10-27T00:00:00Z".into(),
        })
        .collect();
    let recall = MemoryRecall {
        ledger_status: "present".into(),
        sync_status: None,
        match_kind: None,
        hits: Vec::new(),
    };
    let (text, entries, shown) =
        cort::hook_recall::render_with_notes(recall, 0, &notes, 20).unwrap();
    assert_eq!(entries, 0);
    assert!(shown > 0 && shown < 15, "{shown}");
    let section = &text[text.find("unreviewed harness notes").unwrap()..];
    assert!(section.len() <= 2000, "{}", section.len());
    assert!(
        section.ends_with(&format!("... {} more notes", 20 - shown)),
        "{section}"
    );
    // The file name is capped at 80 characters, so no line carries the full 202.
    assert!(!section.contains(&"é".repeat(81)));
}
