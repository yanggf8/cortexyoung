use cort::collect::{
    self, CollectConfig, CollectState, HealthState, KickPaths, Lane, Spawner, HARNESS_ENV,
};
use cort::usage::{record_command_at, CommandRecord};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const NOW: i64 = 1_800_000_000_000;

#[derive(Default)]
struct FakeSpawner(Vec<Lane>);
impl Spawner for FakeSpawner {
    fn spawn(&mut self, lane: Lane, _: &str) -> std::io::Result<()> {
        self.0.push(lane);
        Ok(())
    }
}

fn fixture(dir: &Path) -> KickPaths {
    let data_dir = dir.join("data");
    let cache_dir = dir.join("cache");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    let paths = KickPaths {
        measure_config: data_dir.join("turso-measure.json"),
        usage_db: cache_dir.join("usage.db"),
        data_dir,
        cache_dir,
    };
    fs::write(&paths.measure_config, "{}").unwrap();
    record_command_at(
        &paths.usage_db,
        &CommandRecord {
            now_ms: NOW,
            project_id: None,
            command: "impact".into(),
            args_summary: "{}".into(),
            status: "ok".into(),
            error_code: None,
            read_source: None,
            requested_content_mode: None,
            effective_content_mode: None,
            receipt_hit: None,
            index_stale: None,
            bytes_out: 0,
            saved_bytes: 0,
        },
    );
    paths
}

fn health_not_due(paths: &KickPaths) {
    HealthState {
        last_run_ms: Some(NOW),
        ..Default::default()
    }
    .save(&paths.data_dir.join("measure-health-state.json"))
    .unwrap();
}

fn batch_not_due(paths: &KickPaths) {
    CollectState {
        last_run_ms: Some(NOW),
        ..Default::default()
    }
    .save(&paths.data_dir.join("measure-collect-state.json"))
    .unwrap();
}

#[test]
fn not_configured_touches_nothing() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    fs::remove_file(&p.measure_config).unwrap();
    let before = fs::read_dir(&p.data_dir).unwrap().count();
    let mut spawn = FakeSpawner::default();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
        "not_configured"
    );
    assert_eq!(fs::read_dir(&p.data_dir).unwrap().count(), before);
    assert!(spawn.0.is_empty());
}

#[test]
fn due_batch_spawns_once_and_writes_stamp() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    let mut spawn = FakeSpawner::default();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
        "spawned_batch"
    );
    assert_eq!(spawn.0, [Lane::Batch]);
    assert_eq!(
        fs::read_to_string(p.data_dir.join("measure-batch.spawned")).unwrap(),
        NOW.to_string()
    );
}

#[test]
fn throttled_within_interval() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    let mut spawn = FakeSpawner::default();
    let cfg = CollectConfig::from_env();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &cfg, &mut spawn),
        "spawned_batch"
    );
    assert_eq!(
        collect::kick_at(&p, "codex", NOW + 1, &cfg, &mut spawn),
        "throttled"
    );
    assert_eq!(spawn.0.len(), 1);
}

#[test]
fn concurrent_kicks_spawn_once() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let b = barrier.clone();
                let p = &p;
                scope.spawn(move || {
                    let mut spawn = FakeSpawner::default();
                    b.wait();
                    (
                        collect::kick_at(p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
                        spawn.0.len(),
                    )
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().map(|(_, n)| n).sum::<usize>(), 1);
}

/// Holds the lane's kick lock open by blocking inside `spawn` until released.
struct BlockingSpawner {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
impl Spawner for BlockingSpawner {
    fn spawn(&mut self, _: Lane, _: &str) -> std::io::Result<()> {
        self.entered.send(()).unwrap();
        // Bounded: if the test body panics before releasing, the scope must still join.
        let _ = self.release.recv_timeout(Duration::from_secs(10));
        Ok(())
    }
}

/// The race `concurrent_kicks_spawn_once` cannot force: a second kick arriving while the first is
/// still inside its critical section. It must be turned away by the kick lock (`busy`); without
/// the lock it would only be stopped by the first kick's stamp (`throttled`), which leaves the
/// read-then-write of the stamp unguarded.
#[test]
fn kick_while_another_holds_the_lane_is_busy() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let p = &p;
        let first = scope.spawn(move || {
            let mut spawn = BlockingSpawner {
                entered: entered_tx,
                release: release_rx,
            };
            collect::kick_at(p, "codex", NOW, &CollectConfig::from_env(), &mut spawn)
        });
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the first kick never reached spawn");
        let mut spawn = FakeSpawner::default();
        let second = collect::kick_at(p, "codex", NOW, &CollectConfig::from_env(), &mut spawn);
        release_tx.send(()).unwrap();
        assert_eq!(second, "busy");
        assert!(spawn.0.is_empty());
        assert_eq!(first.join().unwrap(), "spawned_batch");
    });
}

/// The stamp is plain text: a restored or hand-edited one can hold i64::MIN. The throttle test
/// must saturate, not panic on the hook path (debug) or wrap into a permanent `throttled` (release).
#[test]
fn extreme_past_stamp_does_not_stall_the_lane() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    fs::write(
        p.data_dir.join("measure-batch.spawned"),
        i64::MIN.to_string(),
    )
    .unwrap();
    let mut spawn = FakeSpawner::default();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
        "spawned_batch"
    );
    assert_eq!(spawn.0, [Lane::Batch]);
}

#[cfg(unix)]
#[test]
fn stamp_write_failure_spawns_nothing() {
    use std::os::unix::fs::PermissionsExt;
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: geteuid has no arguments and only reads the current process credentials.
    if unsafe { geteuid() } == 0 {
        println!("SKIP: root bypasses directory mode bits");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    // Both locks must open, so the stamp write is the step that fails.
    fs::write(p.data_dir.join("measure-collect.lock"), "").unwrap();
    fs::write(p.data_dir.join("measure-batch.kick.lock"), "").unwrap();
    fs::set_permissions(&p.data_dir, fs::Permissions::from_mode(0o500)).unwrap();
    let mut spawn = FakeSpawner::default();
    let result = collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn);
    fs::set_permissions(&p.data_dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, "stamp_failed");
    assert!(spawn.0.is_empty());
}

#[test]
fn busy_lane_lock_spawns_nothing() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    let lock = cort::upgrade::try_exclusive_lock(&p.data_dir.join("measure-collect.lock")).unwrap();
    let mut spawn = FakeSpawner::default();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
        "busy"
    );
    assert!(spawn.0.is_empty());
    drop(lock);
}

#[test]
fn kick_releases_lane_lock() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    health_not_due(&p);
    let mut spawn = FakeSpawner::default();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
        "spawned_batch"
    );
    assert!(cort::upgrade::try_exclusive_lock(&p.data_dir.join("measure-collect.lock")).is_ok());
}

#[test]
fn both_lanes_due_spawn_both() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    let mut spawn = FakeSpawner::default();
    assert_eq!(
        collect::kick_at(&p, "codex", NOW, &CollectConfig::from_env(), &mut spawn),
        "spawned_both"
    );
    assert_eq!(spawn.0, [Lane::Batch, Lane::Health]);
}

#[test]
fn kick_median_cost() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path());
    let mut conn = rusqlite::Connection::open(&p.usage_db).unwrap();
    let tx = conn.transaction().unwrap();
    {
        let mut stmt = tx.prepare("INSERT INTO command_log (ts, command, args_summary, status, bytes_out) VALUES (?1, 'impact', '{}', 'ok', 0)").unwrap();
        for _ in 0..200_000 {
            stmt.execute([NOW]).unwrap();
        }
    }
    tx.commit().unwrap();
    batch_not_due(&p);
    health_not_due(&p);
    let mut samples = Vec::new();
    let mut spawn = FakeSpawner::default();
    let cfg = CollectConfig::from_env();
    for _ in 0..200 {
        let start = Instant::now();
        assert_eq!(
            collect::kick_at(&p, "codex", NOW, &cfg, &mut spawn),
            "not_due"
        );
        samples.push(start.elapsed());
    }
    samples.sort();
    let median = samples[100];
    println!("kick_median_cost: {median:?}");
    assert!(median < Duration::from_millis(20));
}

#[test]
fn spawn_args_are_stable() {
    assert_eq!(
        collect::spawn_args(Lane::Batch),
        ["measure", "collect", "--background"]
    );
    assert_eq!(
        collect::spawn_args(Lane::Health),
        ["measure", "health", "--background"]
    );
}

#[test]
fn detached_command_carries_lane_and_harness() {
    let exe = std::path::Path::new("cort-test-exe");
    for lane in [Lane::Batch, Lane::Health] {
        let command = collect::detached_command(exe, lane, "codex");
        assert_eq!(command.get_program(), exe.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            collect::spawn_args(lane)
                .iter()
                .map(OsStr::new)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            command.get_envs().find(|(key, _)| *key == HARNESS_ENV),
            Some((OsStr::new(HARNESS_ENV), Some(OsStr::new("codex"))))
        );
    }
}

#[test]
fn kick_paths_follow_env() {
    let _guard = ENV_LOCK.lock().expect("environment test mutex poisoned");
    let old_memory = std::env::var_os("CORT_MEMORY_DIR");
    let old_cache = std::env::var_os("CORT_CACHE_DIR");
    let temp = tempfile::tempdir().expect("temporary directory");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    std::env::set_var("CORT_MEMORY_DIR", &data);
    std::env::set_var("CORT_CACHE_DIR", &cache);

    let paths = KickPaths::current().expect("usage db path with cache env");
    assert_eq!(paths.data_dir, data);
    assert_eq!(paths.cache_dir, cache);
    assert_eq!(paths.usage_db, cache.join("usage.db"));
    assert_eq!(paths.measure_config, data.join("turso-measure.json"));

    match old_memory {
        Some(value) => std::env::set_var("CORT_MEMORY_DIR", value),
        None => std::env::remove_var("CORT_MEMORY_DIR"),
    }
    match old_cache {
        Some(value) => std::env::set_var("CORT_CACHE_DIR", value),
        None => std::env::remove_var("CORT_CACHE_DIR"),
    }
}
