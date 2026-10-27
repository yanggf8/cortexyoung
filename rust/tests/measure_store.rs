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
fn compaction_owners_are_scoped_to_machine() {
    let (_d, mut s) = store();
    assert!(s.compaction_owners("m1").unwrap().is_empty());
    s.upsert_compaction(&row("s1", 1)).unwrap();
    assert_eq!(
        s.compaction_owners("m1").unwrap(),
        vec![("b1".into(), "s1".into())]
    );
    assert!(s.compaction_owners("m2").unwrap().is_empty());
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
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CORT_MEMORY_DIR", dir.path());
    configure("https://x.turso.io", "tok").unwrap();
    let inside = Arc::new(AtomicUsize::new(0));
    let overlap = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = channel::<()>();
    let (asking_tx, asking_rx) = channel::<Instant>();
    let a_inside = inside.clone();
    let a = std::thread::spawn(move || {
        let mut released_at = None;
        with_measure_lock(|| {
            a_inside.fetch_add(1, Ordering::SeqCst);
            entered_tx.send(()).unwrap();
            // Hold the lock until B has announced it is about to ask, then give it time to block.
            let asked_at = asking_rx.recv().unwrap();
            std::thread::sleep(Duration::from_millis(100));
            a_inside.fetch_sub(1, Ordering::SeqCst);
            released_at = Some(Instant::now());
            Ok(asked_at)
        })
        .map(|asked_at| (asked_at, released_at.unwrap()))
        .unwrap()
    });
    // B is spawned only once A is provably inside the critical section.
    entered_rx.recv().unwrap();
    let (b_inside, b_overlap) = (inside.clone(), overlap.clone());
    let b = std::thread::spawn(move || {
        let asked_at = Instant::now();
        asking_tx.send(asked_at).unwrap();
        let mut entered_at = None;
        with_measure_lock(|| {
            entered_at = Some(Instant::now());
            if b_inside.fetch_add(1, Ordering::SeqCst) != 0 {
                b_overlap.fetch_add(1, Ordering::SeqCst);
            }
            b_inside.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        entered_at.unwrap()
    });
    let (b_asked, a_released) = a.join().unwrap();
    let b_entered = b.join().unwrap();
    assert_eq!(overlap.load(Ordering::SeqCst), 0);
    // B asked while A still held the lock, and got in only after A let go.
    assert!(b_asked < a_released, "B asked after A had already released");
    assert!(b_entered >= a_released, "B entered before A released");
    assert!(dir
        .path()
        .join("locks/turso-measure")
        .read_dir()
        .unwrap()
        .any(|d| d.unwrap().path().join("measure.lock").exists()));
}

#[test]
fn write_failure_keeps_existing_rows() {
    let (_d, mut s) = store();
    s.upsert_compaction(&row("s1", 1)).unwrap();
    s.fail_write(true);
    assert!(s.upsert_compaction(&row("s2", 2)).is_err());
    assert_eq!(s.compactions_since(0).unwrap(), vec![row("s1", 1)]);
    s.fail_write(false);
    s.upsert_compaction(&row("s2", 2)).unwrap();
}

#[test]
fn write_failure_rejects_events() {
    let (_d, mut s) = store();
    let event = cort::telemetry::TelemetryEvent {
        source: "cort_command".into(),
        event_id: "1".into(),
        machine_id: "machine".into(),
        machine_source: "test".into(),
        project_id: None,
        time_ms: Some(1),
        payload: "{\"id\":1}".into(),
        rowid: None,
    };
    s.fail_write(true);
    assert!(s.insert_events(std::slice::from_ref(&event)).is_err());
    assert!(s.insert_events(&[]).is_ok());
    assert!(s.event_counts().unwrap().is_empty());
    s.fail_write(false);
    assert_eq!(s.insert_events(&[event]).unwrap(), 1);
}
