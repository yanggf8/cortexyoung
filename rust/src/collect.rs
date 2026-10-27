//! Local collection state and the two event-driven due decisions.

use crate::data_dir::{data_dir, write_atomic};
use crate::measure::{self, RecordInputs, RecordOutcome};
use crate::measure_store::{MeasureStore, OpenError};
use crate::telemetry::{self, UploadInputs};
use crate::usage::{kick_counters_at, KickCounters};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WATCHDOG_GRACE_MS: u64 = 60_000;

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

/// Append one child-process result without depending on the detached stdout/stderr handles.
///
/// The line and its newline go out in one `write`: `O_APPEND` keeps a single write whole against
/// other appenders, but `writeln!` issues the text and the newline as two, and two collectors
/// finishing together would then glue their lines into one. It is one `write` call, not
/// `write_all`: `write_all` answers a short write by writing the rest, and another appender can
/// land between the two pieces. A short write is returned as an error instead -- the partial line
/// stays in the file, but nothing is appended to it later.
pub fn append_measure_log(data_dir: &Path, line: &str) -> Result<(), String> {
    append_log(data_dir, "measure.log", line)
}

pub fn append_log(data_dir: &Path, file_name: &str, line: &str) -> Result<(), String> {
    use std::io::Write;

    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join(file_name);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut record = String::with_capacity(line.len() + 1);
    record.push_str(line);
    record.push('\n');
    let written = file.write(record.as_bytes()).map_err(|e| e.to_string())?;
    if written == record.len() {
        Ok(())
    } else {
        Err(format!(
            "short write to {}: {written} of {} bytes",
            path.display(),
            record.len()
        ))
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
    if previous.is_some_and(|last| now_ms >= last && now_ms.saturating_sub(last) < interval_ms) {
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
    #[serde(alias = "push_pending")]
    pub upload_pending: bool,
    pub pending_since_ms: Option<i64>,
    #[serde(skip)]
    pub progress_path: Option<PathBuf>,
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

/// One state file as status reads it. The workers' `load` folds a broken file into the default
/// (a fresh lane that simply runs again); status must not, or a corrupt file would print as a
/// machine that never collected -- the whole history pending, never run, no failures -- and hide
/// both the corruption and whatever failures it recorded.
fn read_state<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T, &'static str> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|_| "state_unparseable"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(_) => Err("state_unreadable"),
    }
}

/// A read-only view of both lanes. Missing or unreadable usage data leaves pending unknown; a
/// state file that cannot be read leaves its whole lane unknown and names why.
pub fn collection_status(paths: &KickPaths) -> Value {
    let batch_path = paths.data_dir.join("measure-collect-state.json");
    let health_path = paths.data_dir.join("measure-health-state.json");
    let no_local_state = !batch_path.exists() && !health_path.exists();
    let counters = kick_counters_at(&paths.usage_db);
    let pending_error = counters.is_none().then_some("usage_db_unreadable");
    let batch = match read_state::<CollectState>(&batch_path) {
        Ok(mut batch) => {
            batch.progress_path = Some(paths.data_dir.join("measure-progress.json"));
            if batch.upload_pending {
                batch
                    .pending_since_ms
                    .get_or_insert_with(crate::usage::now_ms);
            }
            json!({
                "pending": counters.as_ref().map(|c| batch_pending(&batch, c)),
                "pending_error": pending_error,
                "last_run_ms": batch.last_run_ms,
                "last_ok_ms": batch.last_ok_ms,
                "upload_pending": batch.upload_pending,
                "pending_since_ms": batch.pending_since_ms,
                "upload_stalled_days": batch.stalled_days(crate::usage::now_ms()),
                "refused": batch.refused,
                "failures": batch.failures,
            })
        }
        Err(error) => json!({"pending": null, "state_error": error}),
    };
    let health = match read_state::<HealthState>(&health_path) {
        Ok(health) => json!({
            "pending": counters.as_ref().map(|c| health_pending(&health, c)),
            "pending_error": pending_error,
            "last_run_ms": health.last_run_ms,
            "unmapped_count": health.unmapped_count,
            "unmapped_recent": health.unmapped_recent,
        }),
        Err(error) => json!({"pending": null, "state_error": error}),
    };
    let mut value = json!({"no_local_state": no_local_state, "batch": batch, "health": health,
        "pruned_unuploaded_rows": crate::usage::pruned_unuploaded_rows_at(&paths.usage_db).unwrap_or(0)});
    for lane in ["batch", "health"] {
        if value[lane]["pending_error"].is_null() {
            if let Some(map) = value[lane].as_object_mut() {
                map.remove("pending_error");
            }
        }
    }
    value
}

impl CollectState {
    pub fn load(path: &Path) -> Self {
        let mut state: Self = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        state.progress_path = Some(path.with_file_name("measure-progress.json"));
        if state.upload_pending {
            state
                .pending_since_ms
                .get_or_insert_with(crate::usage::now_ms);
        }
        state
    }

    fn mark_pending(&mut self, now_ms: i64) {
        self.upload_pending = true;
        self.pending_since_ms.get_or_insert(now_ms);
    }

    fn stalled_days(&self, now_ms: i64) -> Option<i64> {
        self.upload_pending
            .then(|| {
                self.pending_since_ms
                    .map(|since| now_ms.saturating_sub(since).max(0) / 86_400_000)
            })
            .flatten()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|error| error.to_string())?;
        write_atomic(path, &text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Network,
    Storage,
    Other,
}

impl ErrorKind {
    fn label(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Storage => "storage",
            Self::Other => "other",
        }
    }
}

pub struct CollectRun {
    pub outcome: &'static str,
    pub refused: bool,
    pub refused_changed: bool,
    pub error_kind: Option<ErrorKind>,
    pub summary: Value,
}

fn collect_run(
    outcome: &'static str,
    harness: &str,
    refused: bool,
    error_kind: Option<ErrorKind>,
    scan: Option<Value>,
) -> CollectRun {
    CollectRun {
        outcome,
        refused,
        refused_changed: false,
        error_kind,
        summary: json!({"outcome": outcome, "harness": harness,
            "error_kind": error_kind.map(ErrorKind::label), "error": null, "refused": refused,
            "scan": scan}),
    }
}

impl CollectRun {
    /// The summary is all a background run leaves behind, so a failure carries its own words.
    fn with_error(mut self, error: impl Into<String>) -> Self {
        self.summary["error"] = Value::String(error.into());
        self
    }
}

/// A kicked child writes at most one line. Repeated refusals keep their status but omit the
/// explanatory text; the transition line retains it for diagnosis.
pub fn log_collect_run(data_dir: &Path, run: &CollectRun, now_ms: i64) -> Result<(), String> {
    if run.outcome == "skipped_busy" {
        return Ok(());
    }
    let mut summary = run.summary.clone();
    summary["lane"] = json!("batch");
    summary["ts_ms"] = json!(now_ms);
    if run.refused_changed {
        summary["refused_changed"] = json!(true);
    } else if summary["scan"]["events"]["status"] == "refused" {
        summary["scan"]["events"] = json!({"status":"refused"});
    }
    append_measure_log(data_dir, &summary.to_string())?;
    if let Some(days) =
        CollectState::load(&data_dir.join("measure-collect-state.json")).stalled_days(now_ms)
    {
        if days > 7 {
            append_measure_log(data_dir, &format!("warning: upload stalled {days} days"))?;
        }
    }
    Ok(())
}

fn collect_failure(state: &mut CollectState, harness: &str, now_ms: i64, kind: ErrorKind) {
    let failure = state.failures.entry(harness.to_string()).or_default();
    failure.count = failure.count.saturating_add(1);
    failure.last_error_kind = kind.label().to_string();
    failure.last_failed_ms = now_ms;
}

fn claim_run(claim: &AtomicBool) -> bool {
    claim
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn claim_normal(claim: &AtomicBool) {
    if !claim_run(claim) {
        // The watchdog owns the final record and is about to exit the process.
        loop {
            std::thread::park();
        }
    }
}

struct WatchdogSpec {
    limit: Duration,
    grace: Duration,
    exit: Box<dyn FnOnce(i32) + Send>,
}

fn background_watchdog_limit(cfg: &CollectConfig) -> Duration {
    Duration::from_millis(cfg.run_deadline_ms.saturating_add(WATCHDOG_GRACE_MS))
}

fn arm_collect_watchdog(
    data_dir: PathBuf,
    harness: String,
    limit: Duration,
    grace: Duration,
    elapsed: Duration,
    claim: Arc<AtomicBool>,
    exit: impl FnOnce(i32) + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("measure-collect-watchdog".to_string())
        .spawn(move || {
            std::thread::sleep(limit.saturating_sub(elapsed));
            if !claim_run(&claim) {
                // Final local state/log writes happen after the normal claim and can hang on disk.
                std::thread::sleep(grace);
                exit(1);
                return;
            }
            let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
            let rescue = std::thread::Builder::new()
                .name("measure-collect-watchdog-rescue".to_string())
                .spawn(move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let now_ms = crate::usage::now_ms();
                        let path = data_dir.join("measure-collect-state.json");
                        let mut state = CollectState::load(&path);
                        let message = format!("watchdog: run exceeded {}s", limit.as_secs());
                        let run = collect_run(
                            "failed",
                            &harness,
                            state.refused,
                            Some(ErrorKind::Network),
                            None,
                        )
                        .with_error(message);
                        let _ = log_collect_run(&data_dir, &run, now_ms);
                        collect_failure(&mut state, &harness, now_ms, ErrorKind::Network);
                        state.mark_pending(now_ms);
                        let _ = state.save(&path);
                    }));
                    let _ = done_tx.send(());
                });
            if rescue.is_ok() {
                let _ = done_rx.recv_timeout(grace);
            }
            exit(1);
        })
}

fn finish_collect_state(
    state: &mut CollectState,
    harness: &str,
    now_ms: i64,
    kind: Option<ErrorKind>,
    events: Option<&Value>,
    health: Option<&Value>,
) {
    if let Some(report) = events {
        state.refused = report["status"] == "refused";
    }
    let _ = health;
    if let Some(kind) = kind {
        collect_failure(state, harness, now_ms, kind);
    } else {
        state.failures.remove(harness);
        state.last_ok_ms = Some(now_ms);
        // A refusal is the persistent state `refused` records, not pending work: keeping the lane
        // pending would make every kick due and rewrite the host row on each one.
        state.upload_pending = false;
        state.pending_since_ms = None;
    }
}

pub fn classify_error(error: &str) -> ErrorKind {
    if crate::network_timeout::network_failure(error) {
        ErrorKind::Network
    } else {
        ErrorKind::Storage
    }
}

/// One run's wall-clock budget, counted from the instant the run began.
///
/// The network bounds in `measure_store` cap a single operation; this caps the whole run, which
/// matters because several operations in a row can each stay inside its own bound and still leave
/// a collector alive far longer than a kick expects (the design's §2 "時間上限").
#[derive(Clone, Copy, Debug)]
struct RunDeadline {
    /// `None` when the limit lies past what `Instant` can represent (possible where it counts
    /// nanoseconds in 64 bits, not on Linux): such a run never expires rather than panicking.
    expires_at: Option<Instant>,
    after_ms: u64,
}

impl RunDeadline {
    fn after(entered: Instant, limit_ms: u64) -> Self {
        Self {
            expires_at: entered.checked_add(Duration::from_millis(limit_ms)),
            after_ms: limit_ms,
        }
    }

    fn expired(&self) -> bool {
        self.expires_at.is_some_and(|at| Instant::now() >= at)
    }

    /// Whole seconds, truncated, matching the network timeout's wording.
    fn error(&self) -> String {
        format!("run deadline exceeded after {}s", self.after_ms / 1000)
    }
}

fn stamp_holder(lock: &mut fs::File, now_ms: i64) {
    // Descriptive metadata on the locked inode. Failure to write it never invalidates the lock.
    let _ = lock
        .set_len(0)
        .and_then(|_| lock.seek(SeekFrom::Start(0)))
        .and_then(|_| writeln!(lock, "pid={} started_ms={}", std::process::id(), now_ms));
}

#[derive(Clone, Copy)]
pub enum ManualMode {
    Write,
    DryRun,
}

pub struct ManualStep<T> {
    pub value: T,
    pub events: Option<Value>,
    pub failure: Option<ErrorKind>,
}

pub type ManualActionResult<T> = Result<ManualStep<T>, (String, ErrorKind)>;

/// A dry run passes only when it matched ids, because only then did `verify_events` write the
/// marker the upload gate reads. `matched == 0` is still a successful command, but clearing
/// `refused` (and the kick stamp) for it would only buy the next batch another refusal.
pub fn dry_run_step(report: crate::telemetry::DryRun) -> ManualStep<crate::telemetry::DryRun> {
    let failure = (report.matched == 0).then_some(ErrorKind::Other);
    ManualStep {
        value: report,
        events: None,
        failure,
    }
}

#[derive(Debug)]
pub enum ManualError {
    Busy(String),
    Other(String),
}

impl ManualError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Busy(_) => "measure_collect_busy",
            Self::Other(_) => "measure_error",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Busy(message) | Self::Other(message) => message,
        }
    }
}

fn holder_description(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let mut fields = content.split_whitespace();
    let pid = fields.next()?.strip_prefix("pid=")?.parse::<u32>().ok()?;
    let started = fields
        .next()?
        .strip_prefix("started_ms=")?
        .parse::<i64>()
        .ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some(format!(
        " (pid {pid}, started {})",
        measure::format_utc(started)
    ))
}

fn manual_lock(path: &Path, wait: Duration, now_ms: i64) -> Result<fs::File, ManualError> {
    let start = Instant::now();
    loop {
        match crate::upgrade::try_exclusive_lock(path) {
            Ok(mut lock) => {
                stamp_holder(&mut lock, now_ms);
                return Ok(lock);
            }
            Err(crate::upgrade::LockError::AdmissionBusy) => {
                if start.elapsed() >= wait {
                    let holder = holder_description(path).unwrap_or_default();
                    return Err(ManualError::Busy(format!(
                        "measure collect is running{holder}; try again later"
                    )));
                }
            }
            Err(error) => {
                return Err(ManualError::Other(format!(
                    "measure_collect_lock_unavailable: {error:?}"
                )));
            }
        }
        // Same bounded 50 ms polling as memory_store::lock_in_within.
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Hold the collection lock, then open the bounded measure store and update shared collection
/// state around a manual record or verify. The caller supplies the original CLI operation.
pub fn run_manual<T>(
    store_opener: &mut dyn FnMut() -> Result<Box<dyn MeasureStore>, OpenError>,
    paths: &KickPaths,
    now_ms: i64,
    cfg: &CollectConfig,
    mode: ManualMode,
    action: &mut dyn FnMut(&mut dyn MeasureStore) -> ManualActionResult<T>,
) -> Result<T, ManualError> {
    let deadline = RunDeadline::after(Instant::now(), cfg.run_deadline_ms);
    fs::create_dir_all(&paths.data_dir).map_err(|e| ManualError::Other(e.to_string()))?;
    let _lock = manual_lock(
        &paths.data_dir.join("measure-collect.lock"),
        Duration::from_millis(cfg.manual_wait_ms),
        now_ms,
    )?;
    let state_path = paths.data_dir.join("measure-collect-state.json");
    let mut state = CollectState::load(&state_path);
    if matches!(mode, ManualMode::Write) {
        let counters = crate::usage::kick_counters_at(&paths.usage_db);
        if let Some(ref counters) = counters {
            state.instance_id = Some(counters.instance_id.clone());
        }
        state.mark_pending(now_ms);
        state.last_run_ms = Some(now_ms);
        if counters.is_none() {
            finish_collect_state(
                &mut state,
                "manual",
                now_ms,
                Some(ErrorKind::Storage),
                None,
                None,
            );
        }
        state.save(&state_path).map_err(ManualError::Other)?;
        if counters.is_none() {
            return Err(ManualError::Other("usage_db_unreadable".into()));
        }
    }
    let opened = match store_opener() {
        Ok(store) => Ok(store),
        Err(OpenError::Busy) => {
            return Err(ManualError::Busy(
                "measure store is busy; try again later".into(),
            ));
        }
        Err(OpenError::Network(error)) => Err((ErrorKind::Network, error)),
        Err(OpenError::Other(error)) => Err((ErrorKind::Other, error)),
    };
    let mut store = match opened {
        Ok(store) => store,
        Err((kind, error)) => {
            if matches!(mode, ManualMode::Write) {
                finish_collect_state(&mut state, "manual", now_ms, Some(kind), None, None);
                state.save(&state_path).map_err(ManualError::Other)?;
            }
            return Err(ManualError::Other(error));
        }
    };
    // The manual action is one opaque store operation owned by the CLI, so opening the store is
    // its last checkpoint this layer can see: a caller whose limit is already gone must not start
    // the network work. The background path additionally stops after the uploads.
    if deadline.expired() {
        if matches!(mode, ManualMode::Write) {
            finish_collect_state(
                &mut state,
                "manual",
                now_ms,
                Some(ErrorKind::Network),
                None,
                None,
            );
            state.save(&state_path).map_err(ManualError::Other)?;
        }
        return Err(ManualError::Other(deadline.error()));
    }
    let result = action(store.as_mut());
    match result {
        Ok(step) => {
            if matches!(mode, ManualMode::DryRun) {
                // Only a pass lifts the refusal; see `dry_run_step`.
                if step.failure.is_none() {
                    state.refused = false;
                    state.save(&state_path).map_err(ManualError::Other)?;
                    let stamp = paths.data_dir.join("measure-batch.spawned");
                    if let Err(error) = fs::remove_file(stamp) {
                        if error.kind() != std::io::ErrorKind::NotFound {
                            return Err(ManualError::Other(error.to_string()));
                        }
                    }
                }
            } else {
                finish_collect_state(
                    &mut state,
                    "manual",
                    now_ms,
                    step.failure,
                    step.events.as_ref(),
                    None,
                );
                state.save(&state_path).map_err(ManualError::Other)?;
            }
            Ok(step.value)
        }
        Err((message, kind)) => {
            if matches!(mode, ManualMode::Write) {
                finish_collect_state(&mut state, "manual", now_ms, Some(kind), None, None);
                state.save(&state_path).map_err(ManualError::Other)?;
            }
            Err(ManualError::Other(message))
        }
    }
}

/// Execute one background batch while holding the collection lock. Manual commands go through
/// [`run_manual`], which keeps their failures out of the harness counts; `manual: true` is refused.
pub fn run_collect(
    store_opener: &mut dyn FnMut() -> Result<Box<dyn MeasureStore>, OpenError>,
    paths: &KickPaths,
    harness: &str,
    now_ms: i64,
    cfg: &CollectConfig,
    manual: bool,
) -> CollectRun {
    run_collect_inner(store_opener, paths, harness, now_ms, cfg, manual, None)
}

pub fn run_background_collect(
    store_opener: &mut dyn FnMut() -> Result<Box<dyn MeasureStore>, OpenError>,
    paths: &KickPaths,
    harness: &str,
    now_ms: i64,
    cfg: &CollectConfig,
) -> Result<CollectRun, String> {
    let watchdog = WatchdogSpec {
        limit: background_watchdog_limit(cfg),
        grace: Duration::from_millis(WATCHDOG_GRACE_MS),
        exit: Box::new(|code| std::process::exit(code)),
    };
    background_collect_with(store_opener, paths, harness, now_ms, cfg, watchdog)
}

/// The body of [`run_background_collect`] with the watchdog supplied, so tests can pass an exit
/// that does not end the test binary.
fn background_collect_with(
    store_opener: &mut dyn FnMut() -> Result<Box<dyn MeasureStore>, OpenError>,
    paths: &KickPaths,
    harness: &str,
    now_ms: i64,
    cfg: &CollectConfig,
    watchdog: WatchdogSpec,
) -> Result<CollectRun, String> {
    let run = run_collect_inner(
        store_opener,
        paths,
        harness,
        now_ms,
        cfg,
        false,
        Some(watchdog),
    );
    log_collect_run(&paths.data_dir, &run, now_ms)?;
    log_prune_loss(&paths.data_dir, &paths.usage_db)?;
    Ok(run)
}

pub fn log_prune_loss(data_dir: &Path, usage_db: &Path) -> Result<(), String> {
    if let Some((lost, total)) = crate::usage::unlogged_pruned_rows_at(usage_db) {
        append_measure_log(data_dir, &format!("warning: pruned {lost} unuploaded rows"))?;
        let _ = crate::usage::mark_pruned_rows_logged_at(usage_db, total);
    }
    Ok(())
}

fn run_collect_inner(
    store_opener: &mut dyn FnMut() -> Result<Box<dyn MeasureStore>, OpenError>,
    paths: &KickPaths,
    harness: &str,
    now_ms: i64,
    cfg: &CollectConfig,
    manual: bool,
    watchdog: Option<WatchdogSpec>,
) -> CollectRun {
    if manual {
        return collect_run("failed", harness, false, Some(ErrorKind::Other), None)
            .with_error("manual collection is not implemented");
    }
    let started = Instant::now();
    let deadline = RunDeadline::after(started, cfg.run_deadline_ms);
    // open_lock does not create its parent; the first explicit collect may precede any kick.
    if let Err(e) = fs::create_dir_all(&paths.data_dir) {
        return collect_run("failed", harness, false, Some(ErrorKind::Other), None)
            .with_error(format!("cannot create {}: {e}", paths.data_dir.display()));
    }
    let mut lock =
        match crate::upgrade::try_exclusive_lock(&paths.data_dir.join("measure-collect.lock")) {
            Ok(lock) => lock,
            Err(crate::upgrade::LockError::AdmissionBusy) => {
                return collect_run("skipped_busy", harness, false, None, None);
            }
            Err(e) => {
                return collect_run("failed", harness, false, Some(ErrorKind::Other), None)
                    .with_error(format!("measure_collect_lock_unavailable: {e:?}"))
            }
        };
    // Checked, not held, as in the health pass. An admission lock file that cannot be opened is
    // not an upgrade -- no upgrader could hold it either -- so it does not stop the run.
    if let Err(crate::upgrade::LockError::AdmissionBusy) =
        crate::upgrade::try_protected_entry(&paths.cache_dir)
    {
        return collect_run("skipped_busy", harness, false, None, None);
    }
    // This is descriptive metadata on the already locked file; never replace its inode or fd.
    stamp_holder(&mut lock, now_ms);

    let state_path = paths.data_dir.join("measure-collect-state.json");
    let mut state = CollectState::load(&state_path);
    let refused_before = state.refused;
    let Some(counters) = crate::usage::kick_counters_at(&paths.usage_db) else {
        state.last_run_ms = Some(now_ms);
        state.mark_pending(now_ms);
        collect_failure(&mut state, harness, now_ms, ErrorKind::Storage);
        let _ = state.save(&state_path);
        return collect_run(
            "failed",
            harness,
            state.refused,
            Some(ErrorKind::Storage),
            None,
        )
        .with_error("usage_db_unreadable");
    };
    state.instance_id = Some(counters.instance_id);
    state.mark_pending(now_ms);
    state.last_run_ms = Some(now_ms);
    if let Err(e) = state.save(&state_path) {
        return collect_run(
            "failed",
            harness,
            state.refused,
            Some(ErrorKind::Storage),
            None,
        )
        .with_error(e);
    }

    let mut claim = None;
    if let Some(spec) = watchdog {
        let flag = Arc::new(AtomicBool::new(false));
        // No watchdog thread exists if the spawn fails, so nothing can contend for the claim.
        if let Err(e) = arm_collect_watchdog(
            paths.data_dir.clone(),
            harness.to_string(),
            spec.limit,
            spec.grace,
            started.elapsed(),
            Arc::clone(&flag),
            spec.exit,
        ) {
            let mut state = CollectState::load(&state_path);
            collect_failure(&mut state, harness, now_ms, ErrorKind::Other);
            let _ = state.save(&state_path);
            return collect_run(
                "failed",
                harness,
                state.refused,
                Some(ErrorKind::Other),
                None,
            )
            .with_error(format!("cannot start collect watchdog: {e}"));
        }
        claim = Some(flag);
    }

    let opened = match store_opener() {
        Ok(store) => Ok(store),
        Err(OpenError::Busy) => {
            if let Some(claim) = &claim {
                claim_normal(claim);
            }
            return collect_run("skipped_busy", harness, state.refused, None, None);
        }
        Err(OpenError::Network(e)) => Err((ErrorKind::Network, e)),
        Err(OpenError::Other(e)) => Err((ErrorKind::Other, e)),
    };
    let mut store = match opened {
        Ok(store) => store,
        Err((kind, e)) => {
            if let Some(claim) = &claim {
                claim_normal(claim);
            }
            collect_failure(&mut state, harness, now_ms, kind);
            let _ = state.save(&state_path);
            return collect_run("failed", harness, state.refused, Some(kind), None).with_error(e);
        }
    };
    // The opener's bootstrap is a network operation; if it alone spent the budget, stop before
    // the transcript scan rather than running a batch that cannot be uploaded.
    if deadline.expired() {
        drop(store);
        if let Some(claim) = &claim {
            claim_normal(claim);
        }
        collect_failure(&mut state, harness, now_ms, ErrorKind::Network);
        let _ = state.save(&state_path);
        return collect_run(
            "failed",
            harness,
            state.refused,
            Some(ErrorKind::Network),
            None,
        )
        .with_error(deadline.error());
    }
    let machine_id = crate::usage::machine_id().to_string();
    let input = RecordInputs {
        projects_dir: measure::claude_projects_dir(),
        marks_path: measure::Watermarks::default_path(),
        machine_id: machine_id.clone(),
        host: measure::host_name(),
        now_ms,
        events: Some(UploadInputs {
            usage_db: paths.usage_db.clone(),
            cost_dir: telemetry::default_cost_dir(),
            marker_path: telemetry::default_marker_path(),
            machine_id,
        }),
        health: Some(paths.usage_db.clone()),
    };
    // The second checkpoint: uploads are inside the record call, so the stop has to be handed to
    // it. `stopped` tells a deadline stop apart from a storage error raised before the checkpoint.
    // A deadline keeps pending state; successful pages have already saved their progress.
    let mut stopped = false;
    let result = measure::record_with_scan_until(store.as_mut(), &input, || {
        if deadline.expired() {
            stopped = true;
            Err(deadline.error())
        } else {
            Ok(())
        }
    });
    let (scan, kind, error) = match result {
        Err(e) if stopped => (None, Some(ErrorKind::Network), Some(e)),
        Err(e) => (None, Some(classify_error(&e)), Some(e)),
        Ok((scan, outcome)) => {
            let (kind, error) = match outcome {
                RecordOutcome::Uploaded => (None, None),
                RecordOutcome::UploadFailed { kind, error } => (Some(kind), Some(error)),
            };
            (Some(scan), kind, error)
        }
    };
    drop(store);
    if let Some(claim) = &claim {
        claim_normal(claim);
    }
    finish_collect_state(
        &mut state,
        harness,
        now_ms,
        kind,
        scan.as_ref().and_then(|s| s.events.as_ref()),
        scan.as_ref().and_then(|s| s.health.as_ref()),
    );
    let scan_json = scan.map(|s| s.to_json());
    // Decided before the save: a run whose state cannot be written still saw the transition, and
    // its line is the one that most needs the refusal's reason.
    let refused_changed = state.refused != refused_before;
    if let Err(e) = state.save(&state_path) {
        let mut run = collect_run(
            "failed",
            harness,
            state.refused,
            Some(ErrorKind::Storage),
            scan_json,
        );
        run.refused_changed = refused_changed;
        return run.with_error(e);
    }
    let mut run = collect_run(
        if kind.is_some() { "failed" } else { "ok" },
        harness,
        state.refused,
        kind,
        scan_json,
    );
    run.refused_changed = refused_changed;
    match error {
        Some(e) => run.with_error(e),
        None => run,
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
    let Some(crate::measure_progress::Load::Loaded(progress)) = state
        .progress_path
        .as_deref()
        .map(crate::measure_progress::load)
    else {
        return (0, 0);
    };
    let machine = crate::usage::machine_id();
    let command = progress
        .watermark(machine, crate::telemetry::SOURCE_COMMAND)
        .map_or(0, |mark| mark.rowid.max(0));
    let health = progress
        .watermark(machine, crate::telemetry::SOURCE_HEALTH)
        .map_or(0, |mark| mark.rowid.max(0));
    (
        // Compared against every row, not just the non-`measure` ones: the cursor counts the
        // `measure` rows the last batch copied too, so it legitimately sits past max_command_id.
        if command > counters.max_any_command_id {
            0
        } else {
            command
        },
        if health > counters.max_health_id {
            0
        } else {
            health
        },
    )
}

pub fn health_cursor(state: &HealthState, counters: &KickCounters) -> i64 {
    // Also against every row, and the two maxima still answer different questions: the comparison
    // here is the rewind test (a restored older `usage.db`), not a pending count. What the health
    // worker stores is `touched_project_ids_at`'s maximum **non-`measure`** id -- the pass never
    // reads `measure` rows, so its cursor names the last row it actually considered and never
    // exceeds `max_command_id <= max_any_command_id`.
    if state.instance_id.as_deref() != Some(counters.instance_id.as_str())
        || state.health_cursor > counters.max_any_command_id
    {
        0
    } else {
        state.health_cursor
    }
}

/// Time since `last_run_ms`, or `None` when the lane has never run. A `last_run_ms` in the future
/// (the clock was set back, or a state file came from a clock that ran ahead) also reads as `None`:
/// treating it as "just ran" would stall the lane for as long as the clock is behind. The
/// subtraction saturates: a hand-edited or restored state file can hold any i64, and a wrapped
/// negative elapsed would keep the lane under its interval forever.
fn since_last_run(last_run_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    last_run_ms
        .filter(|last| *last <= now_ms)
        .map(|last| now_ms.saturating_sub(last))
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
    let pending = batch_pending(state, counters);
    state.upload_pending
        || pending >= cfg.batch_rows
        || (pending >= 1 && elapsed >= cfg.idle_due_ms)
}

pub fn batch_pending(state: &CollectState, counters: &KickCounters) -> i64 {
    let (command_cursor, health_cursor) = batch_cursors(state, counters);
    // The clamp is not defensive: a cursor may legitimately sit past `max_command_id`, because the
    // rows it already copied include the `measure` rows that `max_command_id` excludes. Without the
    // floor that negative would eat into `health_pending`.
    let command_pending = if state.refused {
        0
    } else {
        counters
            .max_command_id
            .saturating_sub(command_cursor)
            .max(0)
    };
    let health_pending = counters.max_health_id.saturating_sub(health_cursor).max(0);
    command_pending.saturating_add(health_pending)
}

pub fn health_pending(state: &HealthState, counters: &KickCounters) -> i64 {
    counters
        .max_command_id
        .saturating_sub(health_cursor(state, counters))
        .max(0)
}

pub fn health_due(
    state: &HealthState,
    counters: &KickCounters,
    now_ms: i64,
    cfg: &CollectConfig,
) -> bool {
    let elapsed = since_last_run(state.last_run_ms, now_ms).unwrap_or(i64::MAX);
    health_pending(state, counters) > 0 && elapsed >= cfg.health_interval_ms
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    fn paths_with_command(dir: &Path) -> KickPaths {
        let paths = KickPaths {
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
            usage_db: dir.join("cache/usage.db"),
            measure_config: dir.join("data/turso-measure.json"),
        };
        fs::create_dir_all(&paths.cache_dir).unwrap();
        crate::usage::record_command_at(
            &paths.usage_db,
            &crate::usage::CommandRecord {
                now_ms: 1_800_000_000_000,
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

    #[test]
    fn background_watchdog_limit_saturates() {
        let mut cfg = CollectConfig::from_lookup(|_| None);
        assert_eq!(background_watchdog_limit(&cfg), Duration::from_secs(660));
        cfg.run_deadline_ms = u64::MAX;
        assert_eq!(
            background_watchdog_limit(&cfg),
            Duration::from_millis(u64::MAX)
        );
    }

    #[test]
    fn background_collect_logs_opener_failure_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_command(dir.path());
        let cfg = CollectConfig::from_lookup(|_| None);
        // A real `process::exit` here would outlive the test and end the test binary later on.
        let run = background_collect_with(
            &mut || Err(OpenError::Other("opener failed".into())),
            &paths,
            "codex",
            1_800_000_000_000,
            &cfg,
            WatchdogSpec {
                limit: Duration::from_secs(3_600),
                grace: Duration::from_millis(50),
                exit: Box::new(|_| {}),
            },
        )
        .unwrap();
        assert_eq!(run.outcome, "failed");
        assert_eq!(run.summary["error"], "opener failed");
        let lines = fs::read_to_string(paths.data_dir.join("measure.log")).unwrap();
        assert_eq!(lines.lines().count(), 1);
        let line: Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(line["outcome"], "failed");
        assert_eq!(line["error"], "opener failed");
    }

    #[test]
    fn background_collect_wires_watchdog_through_blocked_opener() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_command(dir.path());
        let (exit_tx, exit_rx) = mpsc::channel();
        let cfg = CollectConfig::from_lookup(|_| None);
        std::thread::spawn(move || {
            let mut opener = || {
                std::thread::sleep(Duration::from_secs(2));
                Err(OpenError::Busy)
            };
            let _ = run_collect_inner(
                &mut opener,
                &paths,
                "codex",
                1_800_000_000_000,
                &cfg,
                false,
                Some(WatchdogSpec {
                    limit: Duration::from_millis(50),
                    grace: Duration::from_secs(5),
                    exit: Box::new(move |code| exit_tx.send(code).unwrap()),
                }),
            );
        });
        assert_eq!(exit_rx.recv_timeout(Duration::from_secs(10)).unwrap(), 1);
        assert!(exit_rx.recv_timeout(Duration::from_millis(100)).is_err());
        let lines = fs::read_to_string(dir.path().join("data/measure.log")).unwrap();
        assert_eq!(lines.lines().count(), 1);
        let line: Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert!(line["error"].as_str().unwrap().starts_with("watchdog:"));
        let state = CollectState::load(&dir.path().join("data/measure-collect-state.json"));
        assert_eq!(state.failures["codex"].count, 1);
        assert!(state.upload_pending);
    }

    #[test]
    fn unclaimed_watchdog_records_failure_and_exits_once() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("measure-collect-state.json");
        let mut state = CollectState {
            instance_id: Some("original-instance".into()),
            upload_pending: true,
            pending_since_ms: Some(123),
            refused: true,
            last_run_ms: Some(123),
            last_ok_ms: Some(100),
            ..CollectState::default()
        };
        state.failures.insert(
            "codex".to_string(),
            Failure {
                count: 2,
                ..Failure::default()
            },
        );
        state.failures.insert(
            "claude".to_string(),
            Failure {
                count: 4,
                last_error_kind: "storage".into(),
                last_failed_ms: 99,
            },
        );
        state.save(&state_path).unwrap();
        let exits = Arc::new(AtomicUsize::new(0));
        let called = Arc::clone(&exits);
        let claim = Arc::new(AtomicBool::new(false));
        arm_collect_watchdog(
            dir.path().to_path_buf(),
            "codex".to_string(),
            Duration::from_millis(50),
            Duration::from_secs(5),
            Duration::ZERO,
            claim,
            move |code| {
                assert_eq!(code, 1);
                called.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap()
        .join()
        .unwrap();
        assert_eq!(exits.load(Ordering::SeqCst), 1);
        let lines = fs::read_to_string(dir.path().join("measure.log")).unwrap();
        assert_eq!(lines.lines().count(), 2);
        assert!(lines
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("warning: upload stalled "));
        let line: Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(line["lane"], "batch");
        assert_eq!(line["outcome"], "failed");
        assert_eq!(line["harness"], "codex");
        assert!(line["ts_ms"].as_i64().is_some());
        assert_eq!(line["error_kind"], "network");
        assert_eq!(line["error"], "watchdog: run exceeded 0s");
        assert_eq!(line["refused"], true);
        assert!(line["scan"].is_null());
        let saved = CollectState::load(&state_path);
        assert_eq!(saved.instance_id, state.instance_id);
        assert_eq!(saved.pending_since_ms, state.pending_since_ms);
        assert_eq!(saved.refused, state.refused);
        assert_eq!(saved.last_run_ms, state.last_run_ms);
        assert_eq!(saved.last_ok_ms, state.last_ok_ms);
        assert_eq!(saved.failures["claude"], state.failures["claude"]);
        assert_eq!(saved.failures["codex"].count, 3);
        assert_eq!(saved.failures["codex"].last_error_kind, "network");
        assert_eq!(
            saved.failures["codex"].last_failed_ms,
            line["ts_ms"].as_i64().unwrap()
        );
        assert!(saved.upload_pending);
    }

    #[test]
    fn normal_claim_gets_second_stage_exit_without_watchdog_record() {
        let dir = tempfile::tempdir().unwrap();
        let claim = Arc::new(AtomicBool::new(false));
        claim_normal(&claim);
        let exits = Arc::new(AtomicUsize::new(0));
        let called = Arc::clone(&exits);
        arm_collect_watchdog(
            dir.path().to_path_buf(),
            "codex".to_string(),
            Duration::from_millis(50),
            Duration::from_millis(50),
            Duration::ZERO,
            claim,
            move |code| {
                assert_eq!(code, 1);
                called.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap()
        .join()
        .unwrap();
        assert_eq!(exits.load(Ordering::SeqCst), 1);
        assert!(!dir.path().join("measure.log").exists());
        assert!(!dir.path().join("measure-collect-state.json").exists());
    }

    #[test]
    fn watchdog_claim_prevents_normal_record() {
        let dir = tempfile::tempdir().unwrap();
        let claim = Arc::new(AtomicBool::new(false));
        let watchdog_claim = Arc::clone(&claim);
        arm_collect_watchdog(
            dir.path().to_path_buf(),
            "codex".to_string(),
            Duration::from_millis(50),
            Duration::from_secs(5),
            Duration::ZERO,
            watchdog_claim,
            |_| {},
        )
        .unwrap()
        .join()
        .unwrap();
        assert!(!claim_run(&claim));
        let (about_tx, about_rx) = mpsc::channel();
        let (returned_tx, returned_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = about_tx.send(());
            claim_normal(&claim);
            let _ = returned_tx.send(());
        });
        about_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            returned_rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        let lines = fs::read_to_string(dir.path().join("measure.log")).unwrap();
        assert_eq!(lines.lines().count(), 1);
        assert_eq!(
            CollectState::load(&dir.path().join("measure-collect-state.json")).failures["codex"]
                .count,
            1
        );
    }
}
