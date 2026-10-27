//! Cheap notes-upload admission in hooks; network work belongs to the detached child.

use crate::data_dir::write_atomic;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const TRIGGER_ENV: &str = "CORT_NOTES_UPLOAD_TRIGGER";
const DEFAULT_INTERVAL_SECS: i64 = 1800;
const PENDING_RETRY_MS: i64 = 60_000;

pub struct NotesKickPaths {
    pub data_dir: PathBuf,
    pub memory_config: PathBuf,
    pub opt_in: PathBuf,
    pub claude_projects: PathBuf,
    pub cache_dir: PathBuf,
}

impl NotesKickPaths {
    pub fn current() -> Self {
        Self {
            data_dir: crate::data_dir::data_dir(),
            memory_config: crate::memory_store::config_path(),
            opt_in: crate::notes_upload::opt_in_path(),
            claude_projects: crate::measure::claude_projects_dir(),
            cache_dir: crate::db::cache_dir(),
        }
    }
}

pub struct NotesKickConfig {
    pub interval_ms: i64,
}

impl NotesKickConfig {
    pub fn from_env() -> Self {
        let secs = std::env::var("CORT_NOTES_UPLOAD_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        Self {
            interval_ms: secs.saturating_mul(1000),
        }
    }
}

pub trait NotesSpawner {
    fn spawn(&mut self, trigger: &str) -> std::io::Result<()>;
}

pub struct DetachedNotesSpawner;

impl NotesSpawner for DetachedNotesSpawner {
    fn spawn(&mut self, trigger: &str) -> std::io::Result<()> {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .args(["memory", "notes", "upload", "--background"])
            .env(TRIGGER_ENV, trigger);
        crate::heal::spawn_detached(command)
    }
}

fn is_memory_edit(edited: &Path, projects: &Path) -> bool {
    if edited.extension().is_none_or(|ext| ext != "md") {
        return false;
    }
    let Some(parent) = edited.parent().and_then(|p| p.canonicalize().ok()) else {
        return false;
    };
    if parent.file_name().is_none_or(|name| name != "memory") {
        return false;
    }
    let Some(project) = parent.parent() else {
        return false;
    };
    let Ok(projects) = projects.canonicalize() else {
        return false;
    };
    project.parent() == Some(projects.as_path())
}

pub fn kick_at(
    paths: &NotesKickPaths,
    edited: Option<&Path>,
    now_ms: i64,
    cfg: &NotesKickConfig,
    spawner: &mut dyn NotesSpawner,
) -> &'static str {
    if !paths.memory_config.is_file()
        || !paths.opt_in.is_file()
        || !fs::read_to_string(&paths.opt_in)
            .is_ok_and(|s| s.lines().any(|line| !line.trim().is_empty()))
    {
        return "not_configured";
    }
    let _upgrade = match crate::upgrade::try_protected_entry(&paths.cache_dir) {
        Ok(guard) => guard,
        Err(_) => return "upgrade",
    };
    let pending = paths.data_dir.join("notes-upload.pending");
    let stamp = paths.data_dir.join("notes-upload.spawned");
    let previous = fs::read_to_string(&stamp)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok());
    // A marker left behind counts as an edit: a child that had already passed its last pending
    // check when that edit arrived exits without it, so the next hook of any kind picks it up.
    // Unlike a direct edit it waits PENDING_RETRY_MS after the last spawn, so an upload that keeps
    // failing fast (network down) is retried once a minute rather than by every hook.
    let pending_due = pending.is_file()
        && !previous
            .is_some_and(|last| now_ms >= last && now_ms.saturating_sub(last) < PENDING_RETRY_MS);
    let edit = edited.is_some_and(|p| is_memory_edit(p, &paths.claude_projects)) || pending_due;
    if !edit
        && previous
            .is_some_and(|last| now_ms >= last && now_ms.saturating_sub(last) < cfg.interval_ms)
    {
        return "not_due";
    }
    if fs::create_dir_all(&paths.data_dir).is_err() {
        return "stamp_failed";
    }
    let busy = || {
        // Already there means already recorded; rewriting it would fsync on every hook of a run.
        if edit && !pending.is_file() && write_atomic(&pending, "1").is_err() {
            "stamp_failed"
        } else {
            "busy"
        }
    };
    match crate::upgrade::try_exclusive_lock(&paths.data_dir.join("notes-upload.lock")) {
        Ok(lock) => drop(lock),
        Err(crate::upgrade::LockError::AdmissionBusy) => return busy(),
        Err(_) => return "stamp_failed",
    }
    let _kick =
        match crate::upgrade::try_exclusive_lock(&paths.data_dir.join("notes-upload.kick.lock")) {
            Ok(lock) => lock,
            Err(crate::upgrade::LockError::AdmissionBusy) => return busy(),
            Err(_) => return "stamp_failed",
        };
    // Recheck under the kick lock: another hook may have stamped while we waited.
    if !edit
        && fs::read_to_string(&stamp)
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .is_some_and(|last| now_ms >= last && now_ms.saturating_sub(last) < cfg.interval_ms)
    {
        return "not_due";
    }
    if write_atomic(&stamp, &now_ms.to_string()).is_err() {
        return "stamp_failed";
    }
    let trigger = if edit { "edit" } else { "interval" };
    match spawner.spawn(trigger) {
        Ok(()) if edit => "spawned_edit",
        Ok(()) => "spawned_interval",
        Err(_) => "spawn_failed",
    }
}

pub fn kick(edited: Option<&Path>) -> &'static str {
    let now_ms = crate::usage::now_ms();
    kick_at(
        &NotesKickPaths::current(),
        edited,
        now_ms,
        &NotesKickConfig::from_env(),
        &mut DetachedNotesSpawner,
    )
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day) = crate::usage::civil_from_unix_days(seconds.div_euclid(86_400));
    let hour = seconds.rem_euclid(86_400) / 3600;
    let minute = seconds.rem_euclid(3600) / 60;
    let second = seconds.rem_euclid(60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn log(
    data_dir: &Path,
    trigger: &str,
    pass: usize,
    outcome: &str,
    result: Option<&Value>,
    error: Option<&str>,
    elapsed_ms: u128,
) {
    let counts = result.and_then(|r| r.get("counts"));
    let line = json!({
        "ts": timestamp(), "trigger": trigger, "pass": pass, "outcome": outcome,
        "counts": counts.map(|c| json!({"upserts":c["upserts"], "tombstones":c["tombstones"], "unchanged":c["unchanged"], "skipped_secret":c["skipped_secret"]})),
        "host_written": result.and_then(|r| r.get("host_written")),
        "error": error, "elapsed_ms": elapsed_ms, "machine_id": crate::usage::machine_id(),
    });
    let _ = crate::collect::append_log(data_dir, "memory-upload.log", &line.to_string());
}

pub fn run_background() {
    let data_dir = crate::data_dir::data_dir();
    let trigger = std::env::var(TRIGGER_ENV).unwrap_or_else(|_| "unknown".to_string());
    run_background_at(&data_dir, &trigger, true, &mut || {
        crate::memory_store::notes_upload(false, false, Duration::from_secs(5))
    });
}

fn run_background_at(
    data_dir: &Path,
    trigger: &str,
    watchdog: bool,
    upload: &mut dyn FnMut() -> Result<Value, String>,
) {
    let started = Instant::now();
    if fs::create_dir_all(data_dir).is_err() {
        log(
            data_dir,
            trigger,
            0,
            "error",
            None,
            Some("data_dir_unavailable"),
            started.elapsed().as_millis(),
        );
        return;
    }
    let _lock = match crate::upgrade::try_exclusive_lock(&data_dir.join("notes-upload.lock")) {
        Ok(lock) => lock,
        Err(crate::upgrade::LockError::AdmissionBusy) => {
            // A sibling spawned in the same window won the run lock, possibly after scanning; the
            // edit this child was spawned for must still reach it as one more pass.
            let pending = data_dir.join("notes-upload.pending");
            if trigger == "edit" && !pending.is_file() {
                let _ = write_atomic(&pending, "1");
            }
            log(
                data_dir,
                trigger,
                0,
                "busy",
                None,
                None,
                started.elapsed().as_millis(),
            );
            return;
        }
        Err(_) => {
            log(
                data_dir,
                trigger,
                0,
                "error",
                None,
                Some("run_lock_unavailable"),
                started.elapsed().as_millis(),
            );
            return;
        }
    };
    let deadline = std::env::var("CORT_NOTES_UPLOAD_DEADLINE_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(600);
    let watchdog_dir = data_dir.to_path_buf();
    let watchdog_trigger = trigger.to_string();
    if watchdog {
        let _ = std::thread::Builder::new()
            .name("notes-upload-watchdog".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(deadline));
                log(
                    &watchdog_dir,
                    &watchdog_trigger,
                    0,
                    "watchdog",
                    None,
                    Some("watchdog: run exceeded deadline"),
                    u128::from(deadline) * 1000,
                );
                std::process::exit(0);
            });
    }
    let pending = data_dir.join("notes-upload.pending");
    for pass in 1..=3 {
        let _ = fs::remove_file(&pending);
        let pass_start = Instant::now();
        match upload() {
            Ok(value) => log(
                data_dir,
                trigger,
                pass,
                "ok",
                Some(&value),
                None,
                pass_start.elapsed().as_millis(),
            ),
            Err(error) => log(
                data_dir,
                trigger,
                pass,
                "error",
                None,
                Some(&error),
                pass_start.elapsed().as_millis(),
            ),
        }
        if !pending.exists() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeSpawner {
        calls: Vec<String>,
        fail: bool,
    }
    impl NotesSpawner for FakeSpawner {
        fn spawn(&mut self, trigger: &str) -> std::io::Result<()> {
            self.calls.push(trigger.to_string());
            if self.fail {
                Err(std::io::Error::other("failed"))
            } else {
                Ok(())
            }
        }
    }

    fn paths(root: &Path) -> NotesKickPaths {
        NotesKickPaths {
            data_dir: root.join("data"),
            memory_config: root.join("config"),
            opt_in: root.join("opt-in"),
            claude_projects: root.join("claude/projects"),
            cache_dir: root.join("cache"),
        }
    }

    #[test]
    fn kick_outcomes_and_edit_shape() {
        let temp = tempfile::tempdir().unwrap();
        let p = paths(temp.path());
        let cfg = NotesKickConfig { interval_ms: 1000 };
        let mut spawn = FakeSpawner::default();
        assert_eq!(kick_at(&p, None, 1000, &cfg, &mut spawn), "not_configured");
        fs::write(&p.memory_config, "configured").unwrap();
        fs::write(&p.opt_in, "\n").unwrap();
        assert_eq!(kick_at(&p, None, 1000, &cfg, &mut spawn), "not_configured");
        fs::write(&p.opt_in, "project\n").unwrap();
        fs::create_dir_all(&p.cache_dir).unwrap();
        let memory = p.claude_projects.join("one/memory");
        fs::create_dir_all(&memory).unwrap();
        fs::create_dir_all(memory.join("nested")).unwrap();
        assert_eq!(
            kick_at(&p, None, 1000, &cfg, &mut spawn),
            "spawned_interval"
        );
        assert_eq!(
            kick_at(&p, Some(&memory.join("note.md")), 1001, &cfg, &mut spawn),
            "spawned_edit"
        );
        assert_eq!(
            kick_at(
                &p,
                Some(&memory.join("nested/note.md")),
                1002,
                &cfg,
                &mut spawn
            ),
            "not_due"
        );
        assert_eq!(
            kick_at(
                &p,
                Some(&p.claude_projects.join("one/other.md")),
                1002,
                &cfg,
                &mut spawn
            ),
            "not_due"
        );
        assert_eq!(
            kick_at(&p, None, 2002, &cfg, &mut spawn),
            "spawned_interval"
        );
        spawn.fail = true;
        assert_eq!(
            kick_at(&p, Some(&memory.join("note.md")), 2003, &cfg, &mut spawn),
            "spawn_failed"
        );
    }

    #[test]
    fn busy_run_lock_sets_pending_only_for_edit() {
        let temp = tempfile::tempdir().unwrap();
        let p = paths(temp.path());
        fs::write(&p.memory_config, "configured").unwrap();
        fs::write(&p.opt_in, "project\n").unwrap();
        fs::create_dir_all(&p.cache_dir).unwrap();
        let memory = p.claude_projects.join("one/memory");
        fs::create_dir_all(&memory).unwrap();
        fs::create_dir_all(&p.data_dir).unwrap();
        let lock =
            crate::upgrade::try_exclusive_lock(&p.data_dir.join("notes-upload.lock")).unwrap();
        let mut spawn = FakeSpawner::default();
        let cfg = NotesKickConfig { interval_ms: 1000 };
        assert_eq!(kick_at(&p, None, 1000, &cfg, &mut spawn), "busy");
        assert!(!p.data_dir.join("notes-upload.pending").exists());
        assert_eq!(
            kick_at(&p, Some(&memory.join("note.md")), 1000, &cfg, &mut spawn),
            "busy"
        );
        assert!(p.data_dir.join("notes-upload.pending").exists());
        drop(lock);
        // Left behind by a child that finished: the next interval-only hook uploads it, unthrottled.
        assert_eq!(kick_at(&p, None, 1001, &cfg, &mut spawn), "spawned_edit");
        assert_eq!(spawn.calls, ["edit"]);
    }

    #[test]
    fn upgrade_stamp_failure_and_busy_kick_lock_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let mut p = paths(temp.path());
        fs::write(&p.memory_config, "configured").unwrap();
        fs::write(&p.opt_in, "project\n").unwrap();
        let cfg = NotesKickConfig { interval_ms: 1000 };
        let mut spawn = FakeSpawner::default();
        assert_eq!(kick_at(&p, None, 1000, &cfg, &mut spawn), "upgrade");
        fs::create_dir_all(&p.cache_dir).unwrap();
        p.data_dir = temp.path().join("not-a-directory");
        fs::write(&p.data_dir, "file").unwrap();
        assert_eq!(kick_at(&p, None, 1000, &cfg, &mut spawn), "stamp_failed");
        p.data_dir = temp.path().join("data");
        fs::create_dir_all(&p.data_dir).unwrap();
        let lock =
            crate::upgrade::try_exclusive_lock(&p.data_dir.join("notes-upload.kick.lock")).unwrap();
        assert_eq!(kick_at(&p, None, 1000, &cfg, &mut spawn), "busy");
        drop(lock);
        assert!(spawn.calls.is_empty());
    }

    #[test]
    fn a_left_pending_marker_is_retried_at_most_once_a_minute() {
        let temp = tempfile::tempdir().unwrap();
        let p = paths(temp.path());
        fs::write(&p.memory_config, "configured").unwrap();
        fs::write(&p.opt_in, "project\n").unwrap();
        fs::create_dir_all(&p.cache_dir).unwrap();
        fs::create_dir_all(&p.data_dir).unwrap();
        let cfg = NotesKickConfig {
            interval_ms: 3_600_000,
        };
        let mut spawn = FakeSpawner::default();
        let pending = p.data_dir.join("notes-upload.pending");
        fs::write(&pending, "1").unwrap();
        assert_eq!(kick_at(&p, None, 1_000, &cfg, &mut spawn), "spawned_edit");
        // The child failed fast and a hook left the marker again: no respawn inside the minute.
        assert_eq!(kick_at(&p, None, 30_000, &cfg, &mut spawn), "not_due");
        assert_eq!(kick_at(&p, None, 61_000, &cfg, &mut spawn), "spawned_edit");
        assert_eq!(spawn.calls, ["edit", "edit"]);
    }

    #[test]
    fn pending_during_a_pass_causes_a_second_pass() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let mut passes = 0;
        run_background_at(dir, "edit", false, &mut || {
            passes += 1;
            if passes == 1 {
                fs::write(dir.join("notes-upload.pending"), "1").unwrap();
            }
            Ok(
                json!({"counts":{"upserts":1,"tombstones":0,"unchanged":0,"skipped_secret":0},"host_written":true}),
            )
        });
        assert_eq!(passes, 2);
        assert_eq!(
            fs::read_to_string(dir.join("memory-upload.log"))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }
}
