//! `kick_at`'s lock sequences, in a test binary of their own that never spawns a process.
//!
//! These tests drop a `flock` and take it again microseconds later. In the library's unit-test
//! binary other tests spawn real children on other threads, and a child between fork and exec
//! holds a duplicate of every descriptor, this test's lock included (`O_CLOEXEC` closes it only
//! at exec), so the retake answered `busy` on CI's ubuntu runner (run 41017790902, 2026-10-30).
//! A hook calls `kick_at` once per process; only the test harness retakes a lock while other
//! threads fork. Nothing in this file may start a process.

use cort::notes_kick::{kick_at, NotesKickConfig, NotesKickPaths, NotesSpawner};
use std::fs;
use std::path::Path;

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
    let lock = cort::upgrade::try_exclusive_lock(&p.data_dir.join("notes-upload.lock")).unwrap();
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
        cort::upgrade::try_exclusive_lock(&p.data_dir.join("notes-upload.kick.lock")).unwrap();
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
