use cort::notes_upload;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const URL: &str = "libsql://127.0.0.1:9";

fn replica(data: &Path) -> PathBuf {
    replica_url(data, URL)
}

fn replica_url(data: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    data.join("turso-memory").join(name).join("memory.db")
}

fn configured(data: &Path) {
    configured_url(data, URL);
}

fn configured_url(data: &Path, url: &str) {
    fs::create_dir_all(data).unwrap();
    fs::write(
        data.join("turso-memory.json"),
        json!({"url": url, "token": "test"}).to_string(),
    )
    .unwrap();
}

fn mark_synced(path: &Path) {
    let info = path.with_file_name("memory.db-info");
    let mut value: Value = serde_json::from_slice(&fs::read(&info).unwrap()).unwrap();
    value["synced_revision"] = json!({"type":"v1","revision":"test-revision"});
    fs::write(info, value.to_string()).unwrap();
}

fn seed_replica(path: &Path, url: &str, project: &str, entry: bool) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
            .with_remote_url(url).with_auth_token("test")
            .bootstrap_if_empty(false).build().await.unwrap();
        let conn = db.connect().await.unwrap();
        conn.execute("CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project,id))", ()).await.unwrap();
        if entry {
            let body = json!({"id":"id-1","key":"key","claim":"claim","source":"docs/decision.md","source_line":1,"source_sha256":"missing","recorded_at":"2026-10-27","supersedes":null}).to_string();
            conn.execute("INSERT INTO project_memory (project,id,body) VALUES (?,?,?)", (project,"id-1",body.as_str())).await.unwrap();
        }
    });
    mark_synced(path);
}

fn seed_notes(path: &Path, url: &str, project: &str, hosts: bool) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
            .with_remote_url(url).with_auth_token("test")
            .bootstrap_if_empty(false).build().await.unwrap();
        let conn = db.connect().await.unwrap();
        if hosts {
            notes_upload::ensure_notes_schema(&conn).await.unwrap();
            conn.execute("INSERT INTO harness_machines (machine,host,updated_at) VALUES ('remote-machine','NUC11i5','2026-10-27')", ()).await.unwrap();
        } else {
            conn.execute(notes_upload::CREATE_TABLE, ()).await.unwrap();
            conn.execute(notes_upload::CREATE_INDEX, ()).await.unwrap();
        }
        for i in 0..18 {
            let id = format!("note-{i:02}");
            let machine = if i == 1 { "nohost-machine" } else { "remote-machine" };
            let file = if i == 0 { "bad\n\u{202e}name.md".to_string() } else { format!("file-{i:02}.md") };
            conn.execute("INSERT INTO harness_notes (project,machine,harness,source_dir,file_name,note_id,sha256,body,updated_at) VALUES (?,?,?,?,?,?,'hash','body',?)", turso::params![project,machine,"claude-code","source",file.as_str(),id.as_str(),format!("2026-10-27T00:{:02}:00Z", 59-i).as_str()]).await.unwrap();
        }
        // Newest of all, so each would lead the list if the filter let it through. `CORT_MACHINE_ID`
        // is hashed into the id the hook compares against, so this machine's row stores the hash.
        let own: String = Sha256::digest(b"local-machine").iter().map(|b| format!("{b:02x}")).collect();
        for (project, machine, deleted, id) in [(project,&own[..16],0,"own"),(project,"remote-machine",1,"deleted"),("other-project","remote-machine",0,"other")] {
            conn.execute("INSERT INTO harness_notes (project,machine,harness,source_dir,file_name,note_id,sha256,body,deleted,updated_at) VALUES (?,?, 'claude-code','source',?,?,'hash','body',?,'2026-10-27T01:00:00Z')", (project,machine,id,id,deleted)).await.unwrap();
        }
    });
    mark_synced(path);
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

fn file_hashes(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            (
                name,
                Sha256::digest(fs::read(entry.path()).unwrap()).to_vec(),
            )
        })
        .filter(|(name, _)| name != "memory.lock")
        .collect()
}

fn assert_no_connection(listener: &TcpListener) {
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("unexpected remote connection: {other:?}"),
    }
}

fn run(data: &Path, cache: &Path, root: &Path, stdin: &str) -> (std::process::Output, Duration) {
    run_with_project(data, cache, root, stdin, Some("test-project"))
}

fn run_with_project(
    data: &Path,
    cache: &Path,
    root: &Path,
    stdin: &str,
    project: Option<&str>,
) -> (std::process::Output, Duration) {
    let start = Instant::now();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cort"));
    cmd.args(["hook-recall", "--harness", "claude-code"])
        .env("CORT_MEMORY_DIR", data)
        .env("CORT_CACHE_DIR", cache)
        .env("HOME", root)
        .env("CORT_MACHINE_ID", "local-machine")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(project) = project {
        cmd.env("CORT_MEMORY_PROJECT", project);
    } else {
        cmd.env_remove("CORT_MEMORY_PROJECT")
            .env_remove("CLAUDECAT_MEMORY_PROJECT");
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    (child.wait_with_output().unwrap(), start.elapsed())
}

fn outcome(cache: &Path) -> Value {
    let db = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
    let row: String = db
        .query_row(
            "SELECT args_summary FROM command_log WHERE command='hook-recall' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

#[test]
fn silent_outcomes_and_non_json() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    let payload = json!({"cwd":root,"hook_event_name":"SessionStart"}).to_string();
    let (out, _) = run(&data, &cache, root, &payload);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "not_configured");
    configured(&data);
    let (out, _) = run(&data, &cache, root, &payload);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!replica(&data).exists());
    assert_eq!(outcome(&cache)["hook"], "no_replica");
    let (out, _) = run(&data, &cache, root, "not JSON");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "failed");
    let census = cort::usage::hook_census_at(&cache.join("usage.db"), "hook-recall", 0).unwrap();
    assert_eq!(census["not_configured"], 1);
    assert_eq!(census["no_replica"], 1);
    assert_eq!(census["failed"], 1);
    assert!(census.keys().all(|key| !key.starts_with("unknown/")));
}

#[cfg(unix)]
#[test]
fn dangling_config_symlink_is_failed() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    fs::create_dir_all(&data).unwrap();
    symlink(root.join("missing"), data.join("turso-memory.json")).unwrap();
    let (out, _) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "failed");
}

#[test]
fn busy_lock_returns_quickly() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    let project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, URL, &project, false);
    let lock = fs::File::create(path.parent().unwrap().join("memory.lock")).unwrap();
    lock.lock().unwrap();
    let (out, elapsed) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(elapsed < Duration::from_millis(3200));
    assert_eq!(outcome(&cache)["hook"], "busy");
}

#[test]
fn existing_replica_does_not_contact_unreachable_remote() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
            .with_remote_url(URL)
            .with_auth_token("test")
            .bootstrap_if_empty(false)
            .build()
            .await
            .unwrap();
        let conn = db.connect().await.unwrap();
        conn.execute("CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project,id))", ())
            .await
            .unwrap();
    });
    mark_synced(&path);
    let before = file_hashes(path.parent().unwrap());
    let (out, elapsed) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(elapsed < Duration::from_secs(3));
    assert_eq!(outcome(&cache)["hook"], "none");
    assert_eq!(file_hashes(path.parent().unwrap()), before);
}

#[test]
fn local_entries_emit_session_start_json_newest_first() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
            .with_remote_url(URL)
            .with_auth_token("test")
            .bootstrap_if_empty(false)
            .build()
            .await
            .unwrap();
        let conn = db.connect().await.unwrap();
        conn.execute("CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project,id))", ())
            .await
            .unwrap();
        for (id, key, claim, recorded_at) in [
            ("id-1", "older", "first claim", "2026-10-01"),
            ("id-2", "newer", "second claim", "2026-10-27"),
        ] {
            let body = json!({
                "id":id,"key":key,"claim":claim,"source":"docs/decision.md",
                "source_line":3,"source_sha256":"missing", "recorded_at":recorded_at,
                "supersedes":null
            })
            .to_string();
            conn.execute(
                "INSERT INTO project_memory (project,id,body) VALUES (?,?,?)",
                (project.as_str(), id, body.as_str()),
            )
            .await
            .unwrap();
        }
    });
    mark_synced(&path);
    let before = file_hashes(path.parent().unwrap());
    let (out, elapsed) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.status.success());
    assert!(elapsed < Duration::from_secs(3));
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(response.as_object().unwrap().len(), 1);
    assert_eq!(
        response["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let context = response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.starts_with("cort memory: local replica"));
    assert!(context.contains("- [newer] second claim (docs/decision.md:3, source_missing)"));
    assert!(context.find("[newer]").unwrap() < context.find("[older]").unwrap());
    assert_eq!(outcome(&cache)["hook"], "injected");
    assert_eq!(outcome(&cache)["entries"], 2);
    assert_eq!(file_hashes(path.parent().unwrap()), before);
}

#[test]
fn unsafe_metadata_is_rejected_before_open() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    let project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, URL, &project, true);
    let info = path.with_file_name("memory.db-info");
    let mut value: Value = serde_json::from_slice(&fs::read(&info).unwrap()).unwrap();
    value["synced_revision"] = json!({"type":"legacy","generation":1,"synced_frame_no":null});
    fs::write(&info, value.to_string()).unwrap();
    let (out, _) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "unsynced_replica");
    fs::remove_file(info).unwrap();
    let (out, _) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "unsynced_replica");
}

#[test]
fn open_time_recovery_and_config_rewrite_states_are_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    let project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, URL, &project, true);
    let (_, _) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert_eq!(outcome(&cache)["hook"], "injected");

    // An interrupted replace-base apply: open_db would restore files and maybe pull.
    let marker = path.with_file_name("memory.db-replace-base-apply");
    fs::write(&marker, "pending").unwrap();
    let before = file_hashes(path.parent().unwrap());
    let (out, _) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "unsynced_replica");
    assert_eq!(file_hashes(path.parent().unwrap()), before);
    fs::remove_file(&marker).unwrap();

    // Metadata without a saved configuration: open_db would rewrite -info.
    let info = path.with_file_name("memory.db-info");
    let mut value: Value = serde_json::from_slice(&fs::read(&info).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("saved_configuration");
    fs::write(&info, value.to_string()).unwrap();
    let before = file_hashes(path.parent().unwrap());
    let (out, _) = run(&data, &cache, root, &json!({"cwd":root}).to_string());
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "unsynced_replica");
    assert_eq!(file_hashes(path.parent().unwrap()), before);
}

#[test]
fn synced_and_unsynced_replicas_never_connect_or_change_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let data = root.join("data");
    let cache = root.join("cache");
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("SKIP: sandbox prohibits binding a local TCP listener: {e}");
            return;
        }
        Err(e) => panic!("cannot bind local TCP listener: {e}"),
    };
    listener.set_nonblocking(true).unwrap();
    let url = format!("libsql://{}", listener.local_addr().unwrap());
    configured_url(&data, &url);
    let path = replica_url(&data, &url);
    let project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, &url, &project, true);
    let payload = json!({"cwd":root}).to_string();
    let before = file_hashes(path.parent().unwrap());
    let (out, _) = run(&data, &cache, root, &payload);
    assert!(out.status.success());
    assert_eq!(outcome(&cache)["hook"], "injected");
    assert_eq!(file_hashes(path.parent().unwrap()), before);
    assert_no_connection(&listener);

    let (out, _) = run_with_project(&data, &cache, root, &payload, Some("other-project"));
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "none");
    assert_eq!(file_hashes(path.parent().unwrap()), before);
    assert_no_connection(&listener);

    let info = path.with_file_name("memory.db-info");
    let mut value: Value = serde_json::from_slice(&fs::read(&info).unwrap()).unwrap();
    value["synced_revision"] = json!({"type":"legacy","generation":1,"synced_frame_no":null});
    fs::write(&info, value.to_string()).unwrap();
    let (out, _) = run(&data, &cache, root, &payload);
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "unsynced_replica");
    assert_no_connection(&listener);
    fs::remove_file(info).unwrap();
    let (out, _) = run(&data, &cache, root, &payload);
    assert!(out.stdout.is_empty());
    assert_eq!(outcome(&cache)["hook"], "unsynced_replica");
    assert_no_connection(&listener);
}

#[test]
fn payload_cwd_resolves_project_from_git_origin() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    assert!(Command::new("git")
        .arg("init")
        .arg(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.com/team/project.git"
        ])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let project = cort::memory_store::project_from_git_origin(&repo).unwrap();
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    seed_replica(&replica(&data), URL, &project, true);
    let (out, _) = run_with_project(&data, &cache, root, &json!({"cwd":repo}).to_string(), None);
    assert!(out.status.success());
    assert_eq!(outcome(&cache)["hook"], "injected");
}

#[test]
fn other_machine_notes_are_local_filtered_and_capped() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    let data = root.join("data");
    let cache = root.join("cache");
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("SKIP: sandbox prohibits binding a local TCP listener: {e}");
            return;
        }
        Err(e) => panic!("cannot bind local TCP listener: {e}"),
    };
    listener.set_nonblocking(true).unwrap();
    let url = format!("libsql://{}", listener.local_addr().unwrap());
    configured_url(&data, &url);
    let path = replica_url(&data, &url);
    let project = cort::memory_store::project_from_git_origin(&repo).unwrap();
    let ledger_project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, &url, &ledger_project, false);
    seed_notes(&path, &url, &project, true);
    let before = file_hashes(path.parent().unwrap());
    let payload = json!({"cwd":repo}).to_string();
    let (out, _) = run(&data, &cache, root, &payload);
    assert!(out.status.success());
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    let context = response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.starts_with("cort memory: local replica"));
    assert!(context.contains("unreviewed harness notes from other machines"));
    assert!(
        context.contains("- note-00 [NUC11i5] bad name.md"),
        "{context}"
    );
    assert!(
        context.contains("- note-01 [nohost-m] file-01.md"),
        "{context}"
    );
    assert!(!context.contains("\u{202e}"));
    assert!(!context.contains("\nname.md"));
    assert!(!context.contains("- own ["));
    assert!(!context.contains("deleted"));
    assert!(!context.contains("other.md"));
    assert!(context.contains("... 3 more notes"), "{context}");
    assert!(context.len() <= 2000 + "cort memory: local replica (may be behind the shared database; `cort memory recall \"<topic>\"` refreshes)".len());
    assert_eq!(outcome(&cache)["hook"], "injected");
    assert_eq!(outcome(&cache)["entries"], 0);
    assert_eq!(outcome(&cache)["notes"], 15);
    assert_eq!(file_hashes(path.parent().unwrap()), before);
    assert_no_connection(&listener);
}

#[test]
fn missing_host_table_falls_back_without_creating_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    let project = cort::memory_store::project_from_git_origin(&repo).unwrap();
    let ledger_project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, URL, &ledger_project, false);
    seed_notes(&path, URL, &project, false);
    let before = file_hashes(path.parent().unwrap());
    let (out, _) = run(&data, &cache, root, &json!({"cwd":repo}).to_string());
    assert!(out.status.success());
    let context = serde_json::from_slice::<Value>(&out.stdout).unwrap()["hookSpecificOutput"]
        ["additionalContext"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(context.contains("[remote-m]"), "{context}");
    assert_eq!(outcome(&cache)["notes"], 15);
    assert_eq!(file_hashes(path.parent().unwrap()), before);
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
                source_line_sha256: None,
                recorded_at: format!("2026-10-{:02}", i + 1),
                supersedes: None,
                resolves: vec![],
                recorded_machine: None,
                recorded_machine_source: None,
            },
            evidence_status: "source_unchanged".into(),
            source_line_status: None,
            source_line_now: None,
        })
        .collect();
    let (text, count) = cort::hook_recall::render(MemoryRecall {
        ledger_status: "present".into(),
        sync_status: None,
        hits,
    })
    .unwrap();
    assert!(text.len() <= 6000);
    assert!(count <= 20);
    assert!(text.contains("key -29"));
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\t'));
    assert!(text.contains("entries left out"));
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
                source_line_sha256: None,
                recorded_at: format!("{i:03}"),
                supersedes: None,
                resolves: vec![],
                recorded_machine: None,
                recorded_machine_source: None,
            },
            evidence_status: "source_missing".into(),
            source_line_status: None,
            source_line_now: None,
        })
        .collect();
    let (short, count) = cort::hook_recall::render(MemoryRecall {
        ledger_status: "present".into(),
        sync_status: None,
        hits: short_hits,
    })
    .unwrap();
    assert_eq!(count, 20);
    assert!(short.contains("10 entries left out"));
}

#[test]
fn a_failing_notes_query_leaves_the_ledger_outcome_alone() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = git_repo(root);
    let data = root.join("data");
    let cache = root.join("cache");
    configured(&data);
    let path = replica(&data);
    let ledger_project: String = Sha256::digest(b"test-project")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    seed_replica(&path, URL, &ledger_project, true);
    // A harness_notes table the briefs query cannot read: the SELECT fails, not the table check.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = turso::sync::Builder::new_remote(path.to_str().unwrap())
            .with_remote_url(URL)
            .with_auth_token("test")
            .bootstrap_if_empty(false)
            .build()
            .await
            .unwrap();
        let conn = db.connect().await.unwrap();
        conn.execute("CREATE TABLE harness_notes (project TEXT)", ())
            .await
            .unwrap();
    });
    let (out, _) = run(&data, &cache, root, &json!({"cwd":repo}).to_string());
    assert!(out.status.success());
    let context = serde_json::from_slice::<Value>(&out.stdout).unwrap()["hookSpecificOutput"]
        ["additionalContext"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(context.contains("[key] claim"), "{context}");
    assert!(!context.contains("unreviewed harness notes"), "{context}");
    assert_eq!(outcome(&cache)["hook"], "injected");
    assert_eq!(outcome(&cache)["entries"], 1);
    assert_eq!(outcome(&cache)["notes"], 0);
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
