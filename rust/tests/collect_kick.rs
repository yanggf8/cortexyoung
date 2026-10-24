use cort::collect::{self, KickPaths, Lane, HARNESS_ENV};
use std::ffi::OsStr;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

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
