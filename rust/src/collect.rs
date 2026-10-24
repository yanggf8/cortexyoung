//! Local collection state and the two event-driven due decisions.

use crate::data_dir::{data_dir, write_atomic};
use crate::usage::KickCounters;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Batch,
    Health,
}

pub trait Spawner {
    fn spawn(&mut self, lane: Lane, harness: &str) -> std::io::Result<()>;
}

pub struct DetachedSpawner;

pub const HARNESS_ENV: &str = "CORT_COLLECT_HARNESS";

pub fn spawn_args(lane: Lane) -> [&'static str; 3] {
    match lane {
        Lane::Batch => ["measure", "collect", "--background"],
        Lane::Health => ["measure", "health", "--background"],
    }
}

pub fn detached_command(exe: &Path, lane: Lane, harness: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(spawn_args(lane)).env(HARNESS_ENV, harness);
    cmd
}

impl Spawner for DetachedSpawner {
    fn spawn(&mut self, lane: Lane, harness: &str) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        crate::heal::spawn_detached(detached_command(&exe, lane, harness))
    }
}

pub struct KickPaths {
    pub data_dir: PathBuf,
    pub usage_db: PathBuf,
    pub cache_dir: PathBuf,
    pub measure_config: PathBuf,
}

impl KickPaths {
    pub fn current() -> Option<Self> {
        Some(Self {
            data_dir: crate::data_dir::data_dir(),
            usage_db: crate::usage::usage_db_path()?,
            cache_dir: crate::db::cache_dir(),
            measure_config: crate::measure_store::config_path(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KickResult {
    Spawned,
    Busy,
    Throttled,
    StampFailed,
    SpawnFailed,
}

fn try_lane(
    paths: &KickPaths,
    lane: Lane,
    harness: &str,
    now_ms: i64,
    interval_ms: i64,
    spawner: &mut dyn Spawner,
) -> KickResult {
    let (name, run_lock) = match lane {
        Lane::Batch => ("batch", "measure-collect.lock"),
        Lane::Health => ("health", "measure-health.lock"),
    };
    // The child owns the run lock. This probe must end before taking the kick lock.
    match crate::upgrade::try_exclusive_lock(&paths.data_dir.join(run_lock)) {
        Ok(lock) => drop(lock),
        Err(crate::upgrade::LockError::AdmissionBusy) => return KickResult::Busy,
        Err(_) => return KickResult::StampFailed,
    }
    let kick_lock = paths.data_dir.join(format!("measure-{name}.kick.lock"));
    let _guard = match crate::upgrade::try_exclusive_lock(&kick_lock) {
        Ok(lock) => lock,
        Err(crate::upgrade::LockError::AdmissionBusy) => return KickResult::Busy,
        Err(_) => return KickResult::StampFailed,
    };
    let stamp = paths.data_dir.join(format!("measure-{name}.spawned"));
    let previous = fs::read_to_string(&stamp)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok());
    if previous.is_some_and(|last| now_ms >= last && now_ms - last < interval_ms) {
        return KickResult::Throttled;
    }
    if write_atomic(&stamp, &now_ms.to_string()).is_err() {
        return KickResult::StampFailed;
    }
    match spawner.spawn(lane, harness) {
        Ok(()) => KickResult::Spawned,
        Err(_) => KickResult::SpawnFailed,
    }
}

pub fn kick_at(
    paths: &KickPaths,
    harness: &str,
    now_ms: i64,
    cfg: &CollectConfig,
    spawner: &mut dyn Spawner,
) -> &'static str {
    if !paths.measure_config.is_file() {
        return "not_configured";
    }
    let _upgrade = match crate::upgrade::try_protected_entry(&paths.cache_dir) {
        Ok(guard) => guard,
        Err(_) => return "upgrade",
    };
    let counters = match crate::usage::kick_counters_at(&paths.usage_db) {
        Some(counters) => counters,
        None => return "no_counters",
    };
    let batch = CollectState::load(&paths.data_dir.join("measure-collect-state.json"));
    let health = HealthState::load(&paths.data_dir.join("measure-health-state.json"));
    let batch_due = batch_due(&batch, &counters, harness, now_ms, cfg);
    let health_due = health_due(&health, &counters, now_ms, cfg);
    if !batch_due && !health_due {
        return "not_due";
    }
    let batch_result = batch_due.then(|| {
        try_lane(
            paths,
            Lane::Batch,
            harness,
            now_ms,
            min_interval_ms(&batch, harness, cfg),
            spawner,
        )
    });
    let health_result = health_due.then(|| {
        try_lane(
            paths,
            Lane::Health,
            harness,
            now_ms,
            cfg.health_interval_ms,
            spawner,
        )
    });
    match (batch_result, health_result) {
        (Some(KickResult::Spawned), Some(KickResult::Spawned)) => "spawned_both",
        (Some(KickResult::Spawned), _) => "spawned_batch",
        (_, Some(KickResult::Spawned)) => "spawned_health",
        (Some(KickResult::SpawnFailed), _) | (_, Some(KickResult::SpawnFailed)) => "spawn_failed",
        (Some(KickResult::StampFailed), _) | (_, Some(KickResult::StampFailed)) => "stamp_failed",
        (Some(KickResult::Busy), _) | (_, Some(KickResult::Busy)) => "busy",
        _ => "throttled",
    }
}

pub fn kick(harness: &str) -> &'static str {
    let Some(paths) = KickPaths::current() else {
        return "no_counters";
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    kick_at(
        &paths,
        harness,
        now_ms,
        &CollectConfig::from_env(),
        &mut DetachedSpawner,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectConfig {
    pub base_interval_ms: i64,
    pub max_interval_ms: i64,
    pub batch_rows: i64,
    pub idle_due_ms: i64,
    pub health_interval_ms: i64,
    pub net_timeout_ms: u64,
    pub run_deadline_ms: u64,
    pub manual_wait_ms: u64,
    pub root_timeout_ms: u64,
}

impl CollectConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Every value must be a positive integer; anything else keeps the default. Zero or a
    /// negative number would switch off a throttle (`BATCH_ROWS=0` makes an empty backlog "due",
    /// `BASE_SECS=0` drops the interval), and the kick runs on every hook.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        fn positive(get: &impl Fn(&str) -> Option<String>, key: &str) -> Option<u64> {
            get(key)
                .and_then(|raw| raw.trim().parse::<u64>().ok())
                .filter(|value| *value > 0)
        }

        fn secs_i64(get: &impl Fn(&str) -> Option<String>, key: &str, default: i64) -> i64 {
            positive(get, key)
                .and_then(|seconds| i64::try_from(seconds).ok())
                .and_then(|seconds| seconds.checked_mul(1_000))
                .unwrap_or(default * 1_000)
        }

        fn secs_u64(get: &impl Fn(&str) -> Option<String>, key: &str, default: u64) -> u64 {
            positive(get, key)
                .and_then(|seconds| seconds.checked_mul(1_000))
                .unwrap_or(default * 1_000)
        }

        Self {
            base_interval_ms: secs_i64(&get, "CORT_COLLECT_BASE_SECS", 300),
            max_interval_ms: secs_i64(&get, "CORT_COLLECT_MAX_SECS", 3_600),
            batch_rows: positive(&get, "CORT_COLLECT_BATCH_ROWS")
                .and_then(|rows| i64::try_from(rows).ok())
                .unwrap_or(2_000),
            idle_due_ms: secs_i64(&get, "CORT_COLLECT_IDLE_SECS", 900),
            health_interval_ms: secs_i64(&get, "CORT_HEALTH_INTERVAL_SECS", 600),
            net_timeout_ms: secs_u64(&get, "CORT_COLLECT_NET_TIMEOUT_SECS", 120),
            run_deadline_ms: secs_u64(&get, "CORT_COLLECT_DEADLINE_SECS", 600),
            manual_wait_ms: secs_u64(&get, "CORT_COLLECT_MANUAL_WAIT_SECS", 60),
            root_timeout_ms: secs_u64(&get, "CORT_HEALTH_ROOT_TIMEOUT_SECS", 30),
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Failure {
    pub count: u32,
    pub last_error_kind: String,
    pub last_failed_ms: i64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct CollectState {
    pub instance_id: Option<String>,
    pub last_run_ms: Option<i64>,
    pub last_ok_ms: Option<i64>,
    pub failures: BTreeMap<String, Failure>,
    pub push_pending: bool,
    pub uploaded_through_id: i64,
    pub health_uploaded_through_id: i64,
    pub refused: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct HealthState {
    pub instance_id: Option<String>,
    pub health_cursor: i64,
    pub last_run_ms: Option<i64>,
    pub unmapped_count: u64,
    pub unmapped_recent: Vec<String>,
}

pub fn collect_state_path() -> PathBuf {
    data_dir().join("measure-collect-state.json")
}

pub fn health_state_path() -> PathBuf {
    data_dir().join("measure-health-state.json")
}

impl CollectState {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|error| error.to_string())?;
        write_atomic(path, &text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))
    }
}

impl HealthState {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|error| error.to_string())?;
        write_atomic(path, &text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))
    }
}

pub fn batch_cursors(state: &CollectState, counters: &KickCounters) -> (i64, i64) {
    if state.instance_id.as_deref() != Some(counters.instance_id.as_str()) {
        return (0, 0);
    }
    (
        if state.uploaded_through_id > counters.max_command_id {
            0
        } else {
            state.uploaded_through_id
        },
        if state.health_uploaded_through_id > counters.max_health_id {
            0
        } else {
            state.health_uploaded_through_id
        },
    )
}

pub fn health_cursor(state: &HealthState, counters: &KickCounters) -> i64 {
    if state.instance_id.as_deref() != Some(counters.instance_id.as_str())
        || state.health_cursor > counters.max_command_id
    {
        0
    } else {
        state.health_cursor
    }
}

/// Time since `last_run_ms`, or `None` when the lane has never run. A `last_run_ms` in the future
/// (the clock was set back, or a state file came from a clock that ran ahead) also reads as `None`:
/// treating it as "just ran" would stall the lane for as long as the clock is behind.
fn since_last_run(last_run_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    last_run_ms
        .filter(|last| *last <= now_ms)
        .map(|last| now_ms - last)
}

pub fn min_interval_ms(state: &CollectState, harness: &str, cfg: &CollectConfig) -> i64 {
    let count = state
        .failures
        .get(harness)
        .map_or(0, |failure| failure.count);
    cfg.base_interval_ms
        .saturating_mul(2_i64.saturating_pow(count))
        .min(cfg.max_interval_ms)
}

pub fn batch_due(
    state: &CollectState,
    counters: &KickCounters,
    harness: &str,
    now_ms: i64,
    cfg: &CollectConfig,
) -> bool {
    let elapsed = since_last_run(state.last_run_ms, now_ms).unwrap_or(i64::MAX);
    if elapsed < min_interval_ms(state, harness, cfg) {
        return false;
    }
    let (command_cursor, health_cursor) = batch_cursors(state, counters);
    let command_pending = if state.refused {
        0
    } else {
        counters
            .max_command_id
            .saturating_sub(command_cursor)
            .max(0)
    };
    let health_pending = counters.max_health_id.saturating_sub(health_cursor).max(0);
    let pending = command_pending.saturating_add(health_pending);
    state.push_pending || pending >= cfg.batch_rows || (pending >= 1 && elapsed >= cfg.idle_due_ms)
}

pub fn health_due(
    state: &HealthState,
    counters: &KickCounters,
    now_ms: i64,
    cfg: &CollectConfig,
) -> bool {
    let elapsed = since_last_run(state.last_run_ms, now_ms).unwrap_or(i64::MAX);
    counters.max_command_id > health_cursor(state, counters) && elapsed >= cfg.health_interval_ms
}
