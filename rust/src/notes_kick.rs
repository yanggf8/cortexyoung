//! Cheap notes-upload admission in hooks; network work belongs to the detached child.

use crate::data_dir::write_atomic;
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
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

pub fn is_memory_edit(edited: &Path, projects: &Path) -> bool {
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

/// The note's `metadata.type`, read by the same parser that uploads notes (`notes::frontmatter`),
/// so the two can never disagree about a file. Only the first 8 KiB is read; a frontmatter block
/// longer than that, or one never closed, has no type.
fn memory_type(path: &Path) -> Result<Option<String>, ()> {
    let mut head = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(8192).read_to_end(&mut head))
        .map_err(|_| ())?;
    Ok(crate::notes::frontmatter(&String::from_utf8_lossy(&head)).kind)
}

#[cfg(test)]
mod capture_tests {
    use super::memory_type;
    use std::fs;

    #[test]
    fn reads_metadata_type_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        for (input, expected) in [
            (
                "---\nname: x\ndescription: a: b: c\nmetadata:\n  node_type: memory\n  type: project\n---\nbody",
                Some("project"),
            ),
            ("---\nmetadata: \n  type: feedback\n---\n", Some("feedback")),
            ("---\nmetadata:\n  node_type: project\n---\n", None),
            ("---\ntype: feedback\n---\n", None),
            ("no frontmatter\n", None),
            ("---\nmetadata:\n  type: project\n", None),
            ("---\r\nmetadata:\r\n  type: project\r\n---\r\nbody", Some("project")),
        ] {
            fs::write(&path, input).unwrap();
            assert_eq!(memory_type(&path).unwrap().as_deref(), expected, "{input:?}");
        }
        assert!(memory_type(&dir.path().join("missing.md")).is_err());
    }
}

/// Return a closed telemetry tag and, for qualifying saves, the hook context.
pub fn capture(
    paths: &NotesKickPaths,
    harness: &str,
    edited: Option<&Path>,
) -> (&'static str, Option<String>) {
    if harness != "claude-code" {
        return ("not_claude", None);
    }
    let Some(path) = edited.filter(|p| is_memory_edit(p, &paths.claude_projects)) else {
        return ("not_memory", None);
    };
    if path.file_name().is_some_and(|n| n == "MEMORY.md") {
        return ("index_file", None);
    }
    if !paths.memory_config.is_file() {
        return ("not_configured", None);
    }
    let kind = match memory_type(path) {
        Err(()) => return ("read_failed", None),
        Ok(None) => return ("no_type", None),
        Ok(Some(kind)) => kind,
    };
    let tag = match kind.as_str() {
        "project" => "nudged_project",
        "feedback" => "nudged_feedback",
        "user" => return ("type_user", None),
        "reference" => return ("type_reference", None),
        _ => return ("type_other", None),
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let text = format!("cort memory: you saved `{name}` (type `{kind}`) to private memory, which no other machine or harness reads. If it states a project fact, decision or correction and you have not recorded it yet, also run `cort memory record --key <key> --claim <claim>` (cite `--source/--line` only when a line states it). Skip if it only describes how you work.");
    (tag, Some(text))
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
        "tombstone_skipped": result.and_then(|r| r.get("tombstone_skipped")),
        "tombstones_unknown": result.and_then(|r| r.get("tombstones_unknown")),
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
                json!({"counts":{"upserts":1,"tombstones":0,"unchanged":0,"skipped_secret":0},"host_written":true,
                    "tombstone_skipped":0,"tombstones_unknown":false}),
            )
        });
        assert_eq!(passes, 2);
        let log = fs::read_to_string(dir.join("memory-upload.log")).unwrap();
        assert_eq!(log.lines().count(), 2);
        let first: Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
        assert_eq!(first["tombstone_skipped"], 0);
        assert_eq!(first["tombstones_unknown"], false);
    }
}
