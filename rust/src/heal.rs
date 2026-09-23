//! Query-time self-healing: the index is a cache, not a source of truth.
//!
//! Measured before this was built (2026-09-14): the edit hook refused 905 rebuilds in 7 days
//! while the "chain `cort index` yourself" suggestion converted 1 time in 4 days, and a full
//! pipeline costs 1.4-2.3s on this machine's hundred-file projects. So the query that finds
//! the index behind is the cheapest actor to repair it, and no maintenance work is ever
//! assigned to the agent again: `impact`/`context` answer only after `ensure_fresh`, and a
//! tree too big to heal inline defers to a single-flight background rebuilder instead of
//! blocking the answer.
//!
//! Two boundaries hold here as everywhere else. *Never create*: a schema-only or empty
//! database is not healed into an index — `ensure_fresh` no-ops, and since 2026-09-24 it
//! says so (`empty_index_never_creates`), because a husk left by a create that died
//! mid-run otherwise reads as "the heal is broken" on every later query. *Hooks never
//! heal*: they are read-only probes inside a
//! five-second budget, and `hook-refresh` keeps `RebuildPolicy::Forbid`; the foreground query
//! is the only caller with a budget wide enough for a full rebuild.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::db::Db;
use crate::incremental::{incremental_index, RebuildPolicy};

/// Above this many indexed files a foreground query no longer heals inline. The anchor is
/// measured: a full rebuild costs 1.4-2.3s at ~80 files, and the one repository on this
/// machine that dwarfs the rest (gwebcdb, 9,022 source files) would spend tens of seconds —
/// that is the background path's job, not a read's. `CORT_HEAL_MAX_FILES` overrides it; a
/// value that cannot parse keeps the default rather than inventing a third policy.
pub const DEFAULT_HEAL_MAX_FILES: i64 = 2000;

fn heal_disabled() -> bool {
    std::env::var("CORT_NO_HEAL").ok().as_deref() == Some("1")
}

fn heal_max_files() -> i64 {
    heal_max_files_from(std::env::var("CORT_HEAL_MAX_FILES").ok())
}

fn heal_max_files_from(env: Option<String>) -> i64 {
    env.and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_HEAL_MAX_FILES)
}

fn cache_dir() -> Option<PathBuf> {
    crate::usage::usage_db_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// What the heal did before the answer was produced.
///
/// The payload keys exist only when there is something to say — a heal that repaired
/// something, or a heal that was deferred and why. A fresh index or a disabled heal adds no
/// keys at all, so the green-path payload stays byte-identical to the pre-heal contract.
/// An empty index is not the green path: it names the never-create boundary it stopped at,
/// because `repair=rebuild_required` with no marker was read as "the heal is broken" before
/// 2026-09-24.
#[derive(Debug, Default, PartialEq)]
pub struct HealOutcome {
    healed: bool,
    mode: Option<String>,
    elapsed_ms: i64,
    deferred: Option<&'static str>,
}

fn deferred(reason: &'static str) -> HealOutcome {
    HealOutcome {
        deferred: Some(reason),
        ..HealOutcome::default()
    }
}

impl HealOutcome {
    /// Insert the heal fields into a command payload, when there is something to say.
    pub fn attach_to(&self, payload: &mut Value) {
        let Some(obj) = payload.as_object_mut() else {
            return;
        };
        if self.healed {
            obj.insert("self_healed".into(), Value::Bool(true));
            obj.insert("heal_mode".into(), json!(self.mode));
            obj.insert("heal_ms".into(), json!(self.elapsed_ms));
        } else if let Some(reason) = self.deferred {
            obj.insert("self_healed".into(), Value::Bool(false));
            obj.insert("heal_deferred".into(), json!(reason));
        }
    }

    /// Merge the same fields into a usage `args_summary` string, so "did this query repair
    /// the cache" is measurable from the db alone. An untouched summary comes back unchanged.
    pub fn summarize(&self, summary: String) -> String {
        if !self.healed && self.deferred.is_none() {
            return summary;
        }
        let mut v: Value = serde_json::from_str(&summary).unwrap_or_else(|_| json!({}));
        if self.healed {
            v["self_healed"] = json!(true);
            v["heal_mode"] = json!(self.mode);
            v["heal_ms"] = json!(self.elapsed_ms);
        } else if let Some(reason) = self.deferred {
            v["self_healed"] = json!(false);
            v["heal_deferred"] = json!(reason);
        }
        v.to_string()
    }
}

/// Bring the project's index up to the tree before the caller answers from it.
///
/// Cheap by construction: no change is a metadata pass (23-37ms measured on the hook path),
/// one edited file is ~206ms, and only a superseded extractor/schema or an unnarrowable
/// candidate set pays the full 1-2s rebuild — inline, because this runs in a foreground
/// command the caller chose to run, not in a hook.
pub fn ensure_fresh(db: &mut Db, bin: &str, root: &Path, project_id: &str) -> HealOutcome {
    if heal_disabled() {
        return HealOutcome::default();
    }
    // Never create: an empty or schema-only database is not healed into an index.
    let indexed: i64 = match db.query_row("SELECT COUNT(*) FROM file_state", [], |r| r.get(0)) {
        Ok(n) => n,
        Err(_) => return HealOutcome::default(),
    };
    if indexed == 0 {
        // Never create still holds — but silence here is how a schema-only husk (a create
        // that died mid-run, measured 2026-09-24 on travel-2026: a sandboxed attempt left
        // one behind) reads as "heal never fired" on every query that touches it after.
        // The boundary names itself; the repair lever stays an explicit `cort index`.
        return deferred("empty_index_never_creates");
    }
    if indexed > heal_max_files() {
        return defer_to_background(root, project_id);
    }
    let Some(cache) = cache_dir() else {
        return HealOutcome::default();
    };
    // Non-blocking on purpose: an upgrade holding the locks is a reason to answer from the
    // index we have (with its own honest disclosure), not to stall a read behind a migration.
    let guard = match crate::upgrade::try_protected_entry(&cache) {
        Ok(g) => g,
        Err(_) => return deferred("upgrade_in_flight"),
    };
    let outcome = match incremental_index(db, bin, root, RebuildPolicy::Allow) {
        // "Healed" means work actually happened: an already-current index is the no-op the
        // payload must not mention (the same distinction the refresh census draws between
        // `refreshed` and `already_current`).
        Ok(stats)
            if stats.mode == "full" || stats.files_reindexed > 0 || stats.files_removed > 0 =>
        {
            HealOutcome {
                healed: true,
                mode: Some(stats.mode),
                elapsed_ms: stats.elapsed_ms,
                deferred: None,
            }
        }
        Ok(_) => HealOutcome::default(),
        Err(_) => deferred("heal_failed"),
    };
    drop(guard);
    outcome
}

/// The big-repo arm: answer now, repair behind us. Single-flight is the child's job — it takes
/// `.heal-<project>.lock` on start and exits quietly when someone else is already healing —
/// and the pre-check here only avoids the process churn of re-spawning a loser.
fn defer_to_background(root: &Path, project_id: &str) -> HealOutcome {
    let Some(cache) = cache_dir() else {
        return deferred("no_cache_dir");
    };
    let lock = cache.join(format!(".heal-{project_id}.lock"));
    if heal_in_flight(&lock) {
        return deferred("background_already_running");
    }
    let Ok(exe) = std::env::current_exe() else {
        return deferred("spawn_failed");
    };
    match spawn_background_healer(&exe, root) {
        Ok(()) => deferred("background_spawned"),
        Err(_) => deferred("spawn_failed"),
    }
}

/// Whether a background healer holds this project's heal lock right now. An unopenable lock
/// file is not evidence of a running healer (the machine is broken, not busy), so it reads as
/// "not in flight" and the spawn attempt gets to say the real error.
pub fn heal_in_flight(lock: &Path) -> bool {
    matches!(
        try_heal_lock(lock),
        Err(crate::upgrade::LockError::AdmissionBusy)
    )
}

/// Take the project's heal lock, or say it is busy. Same primitive and same release rules as
/// the upgrade locks: flock(2), released by close(2) via RAII, reclaimed by the kernel when a
/// holder dies — `a_killed_holder_releases_the_locks` (tests/upgrade.rs) pins that guarantee
/// for this primitive already, so there is no pid file and no TTL here either.
pub fn try_heal_lock(lock: &Path) -> Result<std::fs::File, crate::upgrade::LockError> {
    crate::upgrade::try_exclusive_lock(lock)
}

#[cfg(unix)]
extern "C" {
    /// Detach the spawned healer from the caller's session. Same minimal-FFI style as the
    /// `flock` extern in `upgrade.rs`: one function, no libc crate.
    fn setsid() -> i32;
}

/// Spawn `cort index --incremental --heal-background <root>` detached: null stdio, own
/// session, never waited on. The child records its own usage row (that is what the flag is
/// for), so the rebuild is attributable without the parent hanging around to say so.
#[cfg(unix)]
fn spawn_background_healer(exe: &Path, root: &Path) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    // SAFETY: pre_exec runs between fork and exec, in the child alone; setsid is an
    // async-signal-safe syscall with no preconditions on a fresh child. Its return is
    // deliberately ignored — a child that fails to detach still indexes correctly.
    unsafe {
        Command::new(exe)
            .args(["index", "--incremental", "--heal-background"])
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                setsid();
                Ok(())
            })
            .spawn()
            .map(|_| ())
    }
}

#[cfg(not(unix))]
fn spawn_background_healer(exe: &Path, root: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    Command::new(exe)
        .args(["index", "--incremental", "--heal-background"])
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_or_disabled_outcome_adds_no_keys() {
        let mut payload = json!({"symbol": "x"});
        HealOutcome::default().attach_to(&mut payload);
        assert!(
            payload.get("self_healed").is_none(),
            "the green path must stay byte-identical: {payload}"
        );
        assert_eq!(
            HealOutcome::default().summarize("{\"v\":1}".into()),
            "{\"v\":1}",
            "an untouched summary comes back unchanged"
        );
    }

    #[test]
    fn a_healed_outcome_names_what_it_did() {
        let outcome = HealOutcome {
            healed: true,
            mode: Some("full".into()),
            elapsed_ms: 1234,
            deferred: None,
        };
        let mut payload = json!({"symbol": "x"});
        outcome.attach_to(&mut payload);
        assert_eq!(payload["self_healed"], json!(true));
        assert_eq!(payload["heal_mode"], json!("full"));
        assert_eq!(payload["heal_ms"], json!(1234));
        let summary = outcome.summarize("{\"v\":1,\"symbol\":\"x\"}".into());
        let v: Value = serde_json::from_str(&summary).unwrap();
        assert_eq!(v["self_healed"], json!(true));
        assert_eq!(v["heal_mode"], json!("full"));
        assert_eq!(v["symbol"], json!("x"), "existing keys survive the merge");
    }

    #[test]
    fn a_deferred_outcome_says_why() {
        let outcome = deferred("background_spawned");
        let mut payload = json!({"symbol": "x"});
        outcome.attach_to(&mut payload);
        assert_eq!(payload["self_healed"], json!(false));
        assert_eq!(payload["heal_deferred"], json!("background_spawned"));
    }

    #[test]
    fn the_threshold_env_that_cannot_parse_keeps_the_default() {
        // Garbage in the variable must not invent a third policy; an explicit number wins;
        // zero defers everything (the knob the big-repo CLI test turns).
        assert_eq!(heal_max_files_from(None), DEFAULT_HEAL_MAX_FILES);
        assert_eq!(
            heal_max_files_from(Some("garbage".into())),
            DEFAULT_HEAL_MAX_FILES
        );
        assert_eq!(heal_max_files_from(Some(" 1 ".into())), 1);
        assert_eq!(heal_max_files_from(Some("0".into())), 0);
    }
}
