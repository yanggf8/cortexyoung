use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    claude: PathBuf,
    url: String,
}

impl Fixture {
    fn new(url: String) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let data = root.join("memory");
        let claude = root.join("claude");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&claude).unwrap();
        assert!(Command::new("git")
            .arg("init")
            .arg(&root)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("git")
            .current_dir(&root)
            .args([
                "remote",
                "add",
                "origin",
                "https://example.com/deadline-test.git"
            ])
            .output()
            .unwrap()
            .status
            .success());
        fs::write(
            data.join("turso-memory.json"),
            json!({"url":url,"token":"test"}).to_string(),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            data,
            claude,
            url,
        }
    }

    fn run(&self, args: &[&str], lock_wait: &str) -> (Output, Value, Duration) {
        let start = Instant::now();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cort"))
            .current_dir(&self.root)
            .env("CORT_MEMORY_DIR", &self.data)
            .env("CORT_CACHE_DIR", self.root.join("cache"))
            .env("CLAUDE_CONFIG_DIR", &self.claude)
            .env("HOME", &self.root)
            .env("CORT_MEMORY_PROJECT", "deadline-test")
            .env("CORT_COLLECT_NET_TIMEOUT_SECS", "1")
            .env("CORT_MEMORY_LOCK_WAIT_SECS", lock_wait)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if start.elapsed() >= Duration::from_secs(10) {
                child.kill().unwrap();
                panic!("cort did not return within 10 seconds: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        let bytes = if output.stdout.is_empty() {
            &output.stderr
        } else {
            &output.stdout
        };
        let value = serde_json::from_slice(bytes)
            .unwrap_or_else(|_| panic!("invalid output: {}", String::from_utf8_lossy(bytes)));
        (output, value, start.elapsed())
    }

    fn lock_path(&self) -> PathBuf {
        let hash = Sha256::digest(self.url.as_bytes());
        let name: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
        self.data
            .join("turso-memory")
            .join(name)
            .join("memory.lock")
    }
}

struct SilentServer {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SilentServer {
    fn new() -> Option<Self> {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("SKIP: sandbox prohibits binding a local TCP listener: {error}");
                return None;
            }
            Err(error) => panic!("cannot bind local listener: {error}"),
        };
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut connections = Vec::new();
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => connections.push(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("listener failed: {error}"),
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
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn notes_lock_wait_is_bounded_before_network() {
    let fixture = Fixture::new("http://127.0.0.1:9".into());
    let path = fixture.lock_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .unwrap();
    lock.lock().unwrap();
    for args in [
        &["memory", "notes", "upload"][..],
        &["memory", "notes", "list"][..],
        &["memory", "notes", "show", "x"][..],
    ] {
        let (output, value, elapsed) = fixture.run(args, "1");
        assert!(!output.status.success(), "{value}");
        assert_eq!(value["error"], "memory_error", "{value}");
        assert!(
            value["detail"]["message"]
                .as_str()
                .unwrap()
                .starts_with("memory.lock busy after "),
            "{value}"
        );
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }
}

fn bounded_failure(output: &Output, value: &Value, elapsed: Duration) {
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}: {value}");
    assert!(!output.status.success(), "{value}");
    assert_eq!(value["error"], "memory_error", "{value}");
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(!message.starts_with("memory.lock busy after "), "{value}");
    // The server accepts and never answers, so only the deadline can end the call. An earlier
    // refusal would mean this test no longer exercises the bound at all.
    assert!(message.contains("network timeout after "), "{value}");
}

#[test]
fn silent_server_bounds_notes_upload_and_releases_lock() {
    let Some(server) = SilentServer::new() else {
        return;
    };
    let fixture = Fixture::new(server.url.clone());
    for _ in 0..2 {
        let (output, value, elapsed) = fixture.run(&["memory", "notes", "upload"], "1");
        bounded_failure(&output, &value, elapsed);
    }
}

#[test]
fn silent_server_bounds_sync_and_recall() {
    let Some(server) = SilentServer::new() else {
        return;
    };
    let fixture = Fixture::new(server.url.clone());
    let (output, value, elapsed) = fixture.run(&["memory", "sync"], "1");
    bounded_failure(&output, &value, elapsed);
    let (output, value, elapsed) = fixture.run(&["memory", "recall", "x"], "1");
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}: {value}");
    if output.status.success() {
        assert_eq!(value["sync_status"], "offline_or_pending", "{value}");
    } else {
        // A fresh replica times out while bootstrapping, before the best-effort push.
        bounded_failure(&output, &value, elapsed);
    }
}

#[test]
fn sub_second_lock_wait_is_reported_as_given() {
    let fixture = Fixture::new("http://127.0.0.1:9".into());
    let path = fixture.lock_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .unwrap();
    lock.lock().unwrap();
    let (output, value, elapsed) = fixture.run(&["memory", "notes", "list"], "0.5");
    assert!(!output.status.success(), "{value}");
    assert_eq!(
        value["detail"]["message"], "memory.lock busy after 500ms",
        "{value}"
    );
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

/// A replica that already synced opens without the network, so the deadline fires on the
/// best-effort push or pull instead; the read after it must still answer from local entries.
#[test]
fn synced_replica_answers_locally_after_a_sync_timeout() {
    let Some(server) = SilentServer::new() else {
        return;
    };
    let fixture = Fixture::new(server.url.clone());
    let path = fixture.lock_path().with_file_name("memory.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // The id `CORT_MEMORY_PROJECT=deadline-test` resolves to.
    let project: String = Sha256::digest(b"deadline-test")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
            .with_remote_url(&fixture.url)
            .with_auth_token("test")
            .bootstrap_if_empty(false)
            .build()
            .await
            .unwrap();
        let conn = db.connect().await.unwrap();
        conn.execute("CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project,id))", ())
            .await
            .unwrap();
        let body = json!({"id":"id-1","key":"deadline","claim":"deadline claim","source":"docs/decision.md","source_line":1,"source_sha256":"missing","recorded_at":"2026-10-27","supersedes":null}).to_string();
        conn.execute(
            "INSERT INTO project_memory (project,id,body) VALUES (?,?,?)",
            (project.as_str(), "id-1", body.as_str()),
        )
        .await
        .unwrap();
    });
    let info = path.with_file_name("memory.db-info");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&info).unwrap()).unwrap();
    metadata["synced_revision"] = json!({"type":"v1","revision":"test-revision"});
    fs::write(info, metadata.to_string()).unwrap();

    let (output, value, elapsed) = fixture.run(&["memory", "recall", "deadline"], "1");
    assert!(output.status.success(), "{value}");
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}: {value}");
    assert_eq!(value["sync_status"], "offline_or_pending", "{value}");
    assert_eq!(value["hits"][0]["id"], "id-1", "{value}");
    let (output, value, elapsed) = fixture.run(&["memory", "status"], "1");
    assert!(output.status.success(), "{value}");
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}: {value}");
    assert_eq!(value["sync_status"], "offline_or_pending", "{value}");
    assert_eq!(value["total_entries"], 1, "{value}");
}
