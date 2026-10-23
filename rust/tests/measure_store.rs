use cort::measure_store::{
    configure, with_measure_lock, CompactionRow, MeasureStore, SqliteStore, WindowCounts,
};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// `configure` and the lock read `CORT_MEMORY_DIR`, which is process-wide.
static ENV: Mutex<()> = Mutex::new(());

fn counts(n: i64) -> WindowCounts {
    WindowCounts {
        tool_calls: n,
        rereads: n + 1,
        repeated_commands: n + 2,
        transcript_lookups: n + 3,
        human_messages: n + 4,
        corrections: n + 5,
    }
}

fn row(session: &str, n: i64) -> CompactionRow {
    CompactionRow {
        machine_id: "m1".into(),
        boundary_uuid: "b1".into(),
        session_id: session.into(),
        time_ms: 1_000 + n,
        project_id: Some("p".into()),
        trigger: "auto".into(),
        pre_tokens: Some(100 + n),
        post_tokens: None,
        pre: counts(n),
        post: counts(n * 2),
        post_complete: n > 1,
    }
}

fn store() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let s = SqliteStore::open(&dir.path().join("m.db")).unwrap();
    (dir, s)
}

#[test]
fn compaction_upsert_replaces_same_key() {
    let (_d, mut s) = store();
    s.upsert_compaction(&row("s1", 1)).unwrap();
    s.upsert_compaction(&row("s1", 2)).unwrap();
    let rows = s.compactions_since(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].time_ms, 1_002);
    assert_eq!(rows[0].pre_tokens, Some(102));
    assert_eq!(rows[0].post_tokens, None);
    assert_eq!(rows[0].pre.corrections, 7);
    assert_eq!(rows[0].post.tool_calls, 4);
    assert!(rows[0].post_complete);
    assert_eq!(s.compactions_since(2_000).unwrap().len(), 0);
}

#[test]
fn compaction_session_reports_first_writer() {
    let (_d, mut s) = store();
    assert_eq!(s.compaction_session("m1", "b1").unwrap(), None);
    s.upsert_compaction(&row("s1", 1)).unwrap();
    assert_eq!(
        s.compaction_session("m1", "b1").unwrap().as_deref(),
        Some("s1")
    );
    assert_eq!(s.compaction_session("m2", "b1").unwrap(), None);
}

#[test]
fn machine_host_upsert_keeps_latest_seen() {
    let (_d, mut s) = store();
    s.upsert_machine_host("m1", "nuc", 500).unwrap();
    s.upsert_machine_host("m1", "nuc", 900).unwrap();
    s.upsert_machine_host("m1", "nuc", 700).unwrap();
    s.upsert_machine_host("m1", "other", 100).unwrap();
    let mut hosts = s.machine_hosts().unwrap();
    hosts.sort_by(|a, b| a.host.cmp(&b.host));
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].host, "nuc");
    assert_eq!(hosts[0].last_seen_ms, 900);
    assert_eq!(hosts[1].last_seen_ms, 100);
}

#[test]
fn configure_refuses_symlink_and_empty_token() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CORT_MEMORY_DIR", dir.path());
    assert!(configure("https://x.turso.io", "  ").is_err());
    assert!(configure("ftp://x", "tok").is_err());
    #[cfg(unix)]
    {
        let target = dir.path().join("elsewhere");
        fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("turso-measure.json")).unwrap();
        let err = configure("https://x.turso.io", "tok").unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }
}

#[test]
fn config_file_is_private() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CORT_MEMORY_DIR", dir.path());
    let path = configure("https://x.turso.io/", " tok \n").unwrap();
    assert_eq!(path, dir.path().join("turso-measure.json"));
    let body: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(body["url"], "https://x.turso.io");
    assert_eq!(body["token"], "tok");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn store_lock_serializes_two_holders() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CORT_MEMORY_DIR", dir.path());
    configure("https://x.turso.io", "tok").unwrap();
    let inside = Arc::new(AtomicUsize::new(0));
    let overlap = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (inside, overlap) = (inside.clone(), overlap.clone());
            std::thread::spawn(move || {
                with_measure_lock(|| {
                    if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                        overlap.fetch_add(1, Ordering::SeqCst);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(overlap.load(Ordering::SeqCst), 0);
    assert!(dir
        .path()
        .join("turso-measure")
        .read_dir()
        .unwrap()
        .any(|d| d.unwrap().path().join("measure.lock").exists()));
}

#[test]
fn push_failure_is_returned() {
    let (_d, mut s) = store();
    s.upsert_compaction(&row("s1", 1)).unwrap();
    s.fail_push(true);
    assert!(s.push().is_err());
    assert_eq!(s.compactions_since(0).unwrap().len(), 1);
    s.fail_push(false);
    assert!(s.push().is_ok());
}
