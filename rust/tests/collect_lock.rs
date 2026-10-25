use cort::measure_store::{self, OpenError};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn in_sandbox<R>(data: &Path, f: impl FnOnce() -> R) -> R {
    let _guard = env_guard();
    let previous = std::env::var_os("CORT_MEMORY_DIR");
    // Every test changing this process-wide variable holds ENV_LOCK for its whole body.
    unsafe { std::env::set_var("CORT_MEMORY_DIR", data) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        match previous {
            Some(value) => std::env::set_var("CORT_MEMORY_DIR", value),
            None => std::env::remove_var("CORT_MEMORY_DIR"),
        }
    }
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn configure_unreachable(data: &Path) {
    fs::write(
        data.join("turso-measure.json"),
        r#"{"url":"libsql://unreachable.invalid","token":"test-token"}"#,
    )
    .expect("test config");
}

#[test]
fn lock_in_within_times_out() {
    let data = tempfile::tempdir().expect("data dir");
    in_sandbox(data.path(), || {
        configure_unreachable(data.path());
        measure_store::with_measure_lock(|| {
            let start = Instant::now();
            let result = measure_store::open_measure_store_within(Duration::from_millis(300));
            assert!(matches!(result, Err(OpenError::Busy)));
            assert!(start.elapsed() < Duration::from_secs(1));
            Ok(())
        })
        .expect("hold measure lock");
        measure_store::with_measure_lock(|| Ok(())).expect("lock released");
    });
}

#[test]
fn busy_store_opens_nothing() {
    let data = tempfile::tempdir().expect("data dir");
    in_sandbox(data.path(), || {
        configure_unreachable(data.path());
        measure_store::with_measure_lock(|| {
            let start = Instant::now();
            let result = measure_store::open_measure_store_within(Duration::from_millis(300));
            assert!(matches!(result, Err(OpenError::Busy)));
            assert!(start.elapsed() < Duration::from_secs(1));
            Ok(())
        })
        .expect("hold measure lock");
        let replica_root = data.path().join("turso-measure");
        for entry in fs::read_dir(replica_root).expect("lock directory") {
            let entry = entry.expect("database directory");
            let files: Vec<_> = fs::read_dir(entry.path())
                .expect("database directory entries")
                .map(|entry| entry.expect("database file").file_name())
                .collect();
            assert_eq!(files, ["measure.lock"]);
        }
    });
}

#[test]
fn unconfigured_store_is_other() {
    let data = tempfile::tempdir().expect("data dir");
    in_sandbox(data.path(), || {
        let result = measure_store::open_measure_store_within(Duration::from_millis(300));
        match result {
            Err(OpenError::Other(message)) => assert_eq!(
                message,
                "Turso measurement is not configured; run `cort measure configure --url URL --token-stdin`"
            ),
            _ => panic!("expected unconfigured error"),
        }
    });
}
