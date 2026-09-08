//! cort-upgrade: diagnose an existing install, then bring it to what THIS tree requires.
//!
//! Repo-local binary, never installed. It runs the new tree's code, so it inherently knows what
//! the new release needs; its only question about the machine is "what's installed" (spec §1).
//! Every inventory it checks is consumed from `cort::install` or the shipped binary's own verbs
//! — restating one is the HOOK_TARGETS sin this module exists to end.

use std::fs;
use std::path::{Path, PathBuf};

// ── the two flocks: admission closes the gate, activity drains the room (spec §3b) ──
//
// The FFI follows the `send_sigterm` precedent (`ast_grep.rs`): a three-line extern, no new
// crate. Imports: this file already has `fs` and `Path` — add only what locking needs.

use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(unix)]
extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}
#[cfg(unix)]
const LOCK_SH: i32 = 1;
#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
const LOCK_NB: i32 = 4;
// No LOCK_UN: release is close(2) via RAII drop, which always releases flock(2). An explicit
// LOCK_UN constant with no user trips `-D warnings` and invites "unlock then keep using fd"
// shapes. No EINTR retry either, deliberately: a spurious EINTR collapses into contention,
// and every contention path here is the SAFE direction (a worker stands down, the upgrader
// aborts). A retry would need errno plumbing for a case that resolves to the same branch.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    AdmissionBusy,
    DrainTimeout,
    /// The lock file itself could not be opened (cache dir missing, unwritable, or a plain
    /// file where the dir belongs). Distinct from AdmissionBusy on purpose: contention means
    /// "someone is mid-upgrade", while this means "this machine's cache is broken" — and
    /// reporting the first for the second is the wrong-diagnosis sin (`cort index` must name
    /// the storage problem, not invent an upgrade). Workers stand down on it exactly as on
    /// AdmissionBusy — the quiet direction is safe either way.
    LockFileUnavailable,
    /// Non-unix platform: flock(2) does not exist. Workers run unguarded (today's behavior);
    /// the upgrader refuses (Fatal) rather than migrating unprotected. CI builds linux+macos
    /// only, so this arm is documentary — but an ungated `std::os::unix` import breaks the
    /// build for everyone else.
    Unsupported,
}

/// Open (creating) a lock file. Returns Err instead of panicking: `hook-refresh` promises
/// silence and exit 0 on every edit, and an unwritable cache dir must stand down, not panic.
#[cfg(unix)]
fn open_lock(path: &Path) -> std::io::Result<File> {
    // truncate(false) is explicit: a lock file must never be truncated — another process may
    // hold the flock on the same inode, and the file's bytes are irrelevant but its size is
    // not a license to reset anything.
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
}

#[cfg(unix)]
fn flock_fd(file: &File, op: i32) -> bool {
    // SAFETY: fd is a live open file description we own; flock has no other preconditions.
    unsafe { flock(file.as_raw_fd(), op) == 0 }
}

/// Protected-operation entry: admission(sh) + activity(sh), then release admission. Returns
/// the activity guard. Err means admission busy — the caller selects the quiet path, not this
/// function. There is deliberately NO "run the body either way" helper: a helper that runs
/// f() on both branches cannot select the quiet path, and the first draft's
/// `with_protected_locks` wrapped `incremental_index` so it indexed straight through an
/// upgrade holding exclusive locks (critical defect, caught in review).
#[cfg(unix)]
pub fn try_protected_entry(cache: &Path) -> Result<ActivityGuard, LockError> {
    let adm = open_lock(&cache.join(".upgrade-admission.lock"))
        .map_err(|_| LockError::LockFileUnavailable)?;
    if !flock_fd(&adm, LOCK_SH | LOCK_NB) {
        return Err(LockError::AdmissionBusy);
    }
    let act = match open_lock(&cache.join(".upgrade-activity.lock")) {
        Ok(a) => a,
        // Not a closure on purpose: the error arm must drop `adm` first, and a closure
        // would move it (`use of moved value` — measured).
        Err(_) => {
            drop(adm);
            return Err(LockError::LockFileUnavailable);
        }
    };
    if !flock_fd(&act, LOCK_SH | LOCK_NB) {
        drop(adm);
        return Err(LockError::AdmissionBusy);
    }
    drop(adm); // release admission; activity carries the protection
    Ok(ActivityGuard { _file: Some(act) })
}

/// The guard a protected operation holds across its body. `None` on non-unix: no flock(2)
/// there, so exclusion is unavailable and workers run exactly as today — the UPGRADER still
/// refuses (see `acquire_upgrade_locks`), so the unsafe direction stays closed everywhere.
#[derive(Debug)]
pub struct ActivityGuard {
    _file: Option<File>,
}

#[cfg(not(unix))]
pub fn try_protected_entry(_cache: &Path) -> Result<ActivityGuard, LockError> {
    Ok(ActivityGuard { _file: None })
}

#[derive(Debug)]
pub struct UpgradeLocks {
    _admission: File,
    _activity: File,
}
// Debug is load-bearing, not vanity: the killed-holder test asserts
// `acquire_upgrade_locks(...).is_ok()` with `{got:?}` on failure, and without this derive
// that assertion does not compile.

#[cfg(unix)]
pub fn acquire_upgrade_locks(
    cache: &Path,
    drain_timeout: Duration,
) -> Result<UpgradeLocks, LockError> {
    let adm = open_lock(&cache.join(".upgrade-admission.lock"))
        .map_err(|_| LockError::LockFileUnavailable)?;
    if !flock_fd(&adm, LOCK_EX | LOCK_NB) {
        return Err(LockError::AdmissionBusy);
    }
    // Drain: wait for exclusive activity with a deadline. Every worker holding it shared
    // must let go before this succeeds.
    let act = match open_lock(&cache.join(".upgrade-activity.lock")) {
        Ok(a) => a,
        Err(_) => {
            drop(adm);
            return Err(LockError::LockFileUnavailable);
        }
    };
    let deadline = Instant::now() + drain_timeout;
    loop {
        if flock_fd(&act, LOCK_EX | LOCK_NB) {
            return Ok(UpgradeLocks {
                _admission: adm,
                _activity: act,
            });
        }
        if Instant::now() >= deadline {
            return Err(LockError::DrainTimeout);
        }
        // 50ms poll: overshoot past the deadline is bounded by one interval, documented
        // here so nobody "fixes" it into a busy loop.
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(not(unix))]
pub fn acquire_upgrade_locks(
    _cache: &Path,
    _drain_timeout: Duration,
) -> Result<UpgradeLocks, LockError> {
    Err(LockError::Unsupported)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentState {
    Current,
    Drifted,
    Unreadable,
    Absent,
    DeferredByUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    // Owned: per-project index components carry the project path, which is never 'static.
    pub name: String,
    pub state: ComponentState,
    pub detail: String,
}

/// Pack identity for a directory: same construction as `pack::extractor_version` but over an
/// arbitrary dir, so tests can build two packs that differ by one byte. Sorted file list, hashed
/// contents, MIXED WITH the scan engine identity — exactly like `extractor_version` does:
/// the same bytes through a different engine are a different extractor.
pub fn pack_identity(dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<_> = walk_yaml(dir)?;
    files.sort();
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for f in files {
        h.update(&fs::read(&f)?);
    }
    h.update(crate::scan::SCAN_ENGINE.as_bytes());
    Ok(format!("{:x}", h.finalize()))
}

fn walk_yaml(dir: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk_yaml(&p)?);
        } else if p.extension().map(|e| e == "yml").unwrap_or(false) {
            out.push(p);
        }
    }
    Ok(out)
}

/// The shim check: read the manifest, resolve cort_bin, diff against render_shim. Unreadable
/// manifest (read ERROR, not empty) is Unreadable — never Absent, never Current.
pub fn check_shim(manifest: &Path, cort_home: &Path) -> ComponentState {
    let contents = match fs::read_to_string(manifest) {
        Ok(c) => c,
        Err(_) => return ComponentState::Unreadable,
    };
    let Some(bin_line) = contents.lines().find(|l| l.starts_with("cort_bin:")) else {
        return ComponentState::Absent;
    };
    let bin_path = Path::new(bin_line.trim_start_matches("cort_bin:"));
    let on_disk = match fs::read_to_string(bin_path) {
        Ok(s) => s,
        Err(_) => return ComponentState::Unreadable,
    };
    let expected = crate::install::render_shim(&cort_home.to_string_lossy());
    if on_disk.trim_end() == expected.trim_end() {
        ComponentState::Current
    } else {
        ComponentState::Drifted
    }
}

/// Installed ast-grep CLI output vs the tree pin. Pure string comparison: the caller (Task 5
/// sequencing the installed binary's `--version`) owns subprocess policy; this function only
/// judges. Empty or unparseable installed output is Unreadable — an empty string never equals
/// the pin, so without this arm a missing binary would read as Drifted and demand an update to
/// a parser that was never there.
pub fn check_version_pin(installed: &str, pinned: &str) -> Component {
    let name = "ast_grep".to_string();
    let installed = installed.trim();
    if installed.is_empty() {
        return Component {
            name,
            state: ComponentState::Unreadable,
            detail: "no version output to read".into(),
        };
    }
    if installed == pinned {
        Component {
            name,
            state: ComponentState::Current,
            detail: String::new(),
        }
    } else {
        Component {
            name,
            state: ComponentState::Drifted,
            detail: format!("installed reports {installed}, tree pins {pinned}"),
        }
    }
}

/// Live manifest key set vs the tree authority. `live` is every `xxx:` key prefix present in the
/// manifest file (the caller reads the file; this function diffs). Unknown keys are Drifted with
/// names, never failures here — Task 5 maps states to exit codes.
pub fn check_manifest_keys(live: &[&str]) -> Component {
    let unknown: Vec<&&str> = live
        .iter()
        .filter(|k| {
            !crate::install::MANIFEST_KEYS.contains(k)
                && !crate::install::MANIFEST_LEGACY_KEYS.contains(k)
        })
        .collect();
    if unknown.is_empty() {
        Component {
            name: "manifest_keys".to_string(),
            state: ComponentState::Current,
            detail: String::new(),
        }
    } else {
        let names: Vec<String> = unknown.iter().map(|k| k.to_string()).collect();
        Component {
            name: "manifest_keys".to_string(),
            state: ComponentState::Drifted,
            detail: format!("keys no release knows: {}", names.join(", ")),
        }
    }
}

/// Stored usage-schema version vs the tree constant. `None` (file unreadable or key absent)
/// is Unreadable: absence of evidence, reported, never passed and never silently drifted.
pub fn check_usage_schema(stored: Option<&str>, current: &str) -> Component {
    const NAME: &str = "usage_db";
    match stored {
        None => Component {
            name: NAME.to_string(),
            state: ComponentState::Unreadable,
            detail: "usage.db unreadable or version key absent".into(),
        },
        Some(v) if v == current => Component {
            name: NAME.to_string(),
            state: ComponentState::Current,
            detail: String::new(),
        },
        Some(v) => Component {
            name: NAME.to_string(),
            state: ComponentState::Drifted,
            detail: format!("usage.db schema {v}, tree expects {current}"),
        },
    }
}

/// Read-only reasons for one project db: open WITHOUT migrating (no `ensure_schema` — the
/// upgrader must not migrate schema as a side effect of looking) and run plan 2's shared
/// reader. `None` = could not read = Unreadable downstream, never empty-debt.
pub fn read_reasons_readonly(db_path: &str) -> Option<Vec<String>> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    // Spec §10 item 1: the scan connection used to be opened with no busy timeout, so one
    // transient SQLITE_BUSY under a concurrent refresh-hook write reported a healthy index
    // Unreadable. Same 5s the rest of db.rs uses; contention now waits instead of lying.
    conn.busy_timeout(std::time::Duration::from_secs(5)).ok()?;
    crate::indexer::rebuild_reasons(&conn).ok()
}

pub struct DiagnoseInputs<'a> {
    /// Dir holding `manifest` + `cort/` (cort_home = install_root/cort, pack = cort_home/pack).
    pub install_root: &'a Path,
    /// The staged new generation's pack dir (Task 0 produces it).
    pub new_pack: &'a Path,
    /// Installed ast-grep `--version` stdout, gathered by the CALLER (Task 5 owns subprocess
    /// policy incl. deadlines; this function only judges — same seam as check_version_pin).
    pub installed_ast_grep_version: &'a str,
    /// Source tree (`skills/*/SKILL.md` live here).
    pub new_tree: &'a Path,
    /// Agent home — the skill-destination defaults root here exactly as install.sh resolves
    /// them (`$HOME/.claude`, `${CLAUDE_SKILL_HOME:-...}`, `${CODEX_HOME:-...}`).
    pub home: &'a Path,
    pub keep_mine: bool,
}

/// Compose the Task-1 components. Task 3 extends this fn with skill/hook components; the
/// per-component fns above stay the unit seams (the e2e test calls this, the break tests call
/// those — both levels observe something the other cannot).
pub fn diagnose(inputs: &DiagnoseInputs) -> Vec<Component> {
    let mut out = Vec::new();
    let manifest = inputs.install_root.join("manifest");
    let cort_home = inputs.install_root.join("cort");
    out.push(Component {
        name: "shim".to_string(),
        state: check_shim(&manifest, &cort_home),
        detail: String::new(),
    });
    out.push(
        match (
            pack_identity(&cort_home.join("pack")),
            pack_identity(inputs.new_pack),
        ) {
            (Ok(a), Ok(b)) if a == b => Component {
                name: "pack".into(),
                state: ComponentState::Current,
                detail: String::new(),
            },
            (Ok(a), Ok(b)) => Component {
                name: "pack".into(),
                state: ComponentState::Drifted,
                detail: format!("installed pack {a}, new pack {b}"),
            },
            _ => Component {
                name: "pack".into(),
                state: ComponentState::Unreadable,
                detail: "a pack dir could not be hashed".into(),
            },
        },
    );
    out.push(check_version_pin(
        inputs.installed_ast_grep_version,
        crate::install::AST_GREP_PINNED,
    ));
    // Manifest keys: every `xxx:` prefix in the live file. An unreadable manifest is its own
    // Unreadable component — the key-set cannot be diffed from bytes we could not read, and
    // empty-keys-would-read-Current is exactly the false-pass this plan keeps refusing.
    out.push(match fs::read_to_string(&manifest) {
        Err(_) => Component {
            name: "manifest_keys".into(),
            state: ComponentState::Unreadable,
            detail: "manifest unreadable".into(),
        },
        Ok(contents) => {
            let live: Vec<&str> = contents
                .lines()
                .filter_map(|l| l.split(':').next())
                .collect();
            check_manifest_keys(&live)
        }
    });
    // usage.db: no file yet → Absent (fresh machine, never a failure — Task 5's verdict
    // keeps that promise); present-but-unreadable → Unreadable.
    out.push(match crate::usage::usage_db_path() {
        Some(p) if p.exists() => check_usage_schema(
            crate::usage::read_schema_version(&p).as_deref(),
            &crate::usage::USAGE_SCHEMA_VERSION.to_string(),
        ),
        _ => Component {
            name: "usage_db".into(),
            state: ComponentState::Absent,
            detail: "no usage.db yet".into(),
        },
    });
    // Indexes, read-only: empty reasons → Current; non-empty → Drifted WITH the reasons named
    // (Task 4 repays them; diagnosis only names them). Unreadable db → Unreadable.
    out.extend(check_skills(inputs.new_tree, inputs.home, inputs.keep_mine));
    for entry in crate::db::list_projects() {
        match entry {
            crate::db::ProjectEntry::Unreadable { db_path, reason } => {
                out.push(Component {
                    name: "index_unreadable".into(),
                    state: ComponentState::Unreadable,
                    detail: format!("{db_path}: {reason}"),
                });
            }
            crate::db::ProjectEntry::Indexed(row) => {
                let name = format!("index:{}", row.path);
                match read_reasons_readonly(&row.db_path) {
                    None => out.push(Component {
                        name,
                        state: ComponentState::Unreadable,
                        detail: format!("{}: metadata unreadable", row.db_path),
                    }),
                    Some(reasons) if reasons.is_empty() => out.push(Component {
                        name,
                        state: ComponentState::Current,
                        detail: String::new(),
                    }),
                    Some(reasons) => out.push(Component {
                        name,
                        state: ComponentState::Drifted,
                        detail: format!("needs: {}", reasons.join(", ")),
                    }),
                }
            }
        }
    }
    out
}

// ── Task 3: skills — content-diffed, not stamp-checked ─────────────

const MANAGED_SIGNATURE: &str = "managed by cortexyoung install.sh";
const MANAGED_STAMP_NAME: &str = ".cortexyoung-managed";

/// Skills, resolved the way install.sh resolves them. The env-reading wrapper; tests call
/// `check_skills_at` (all paths explicit) because mutating process env inside parallel tests
/// is unsound. Env mirroring:
/// - xgrep skill: `$HOME/.claude/skills/xgrep/SKILL.md`, NO override (install.sh hardcodes it);
/// - ast-grep skill: `${CLAUDE_SKILL_HOME:-$HOME/.claude}/skills/ast-grep/SKILL.md`;
/// - codex skill: `${CODEX_HOME:-$HOME/.codex}/skills/ast-grep/SKILL.md`.
pub fn check_skills(new_tree: &Path, home: &Path, keep_mine: bool) -> Vec<Component> {
    let claude_home = std::env::var_os("CLAUDE_SKILL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    check_skills_at(
        new_tree,
        &home.join(".claude/skills/xgrep/SKILL.md"),
        &claude_home.join("skills/ast-grep/SKILL.md"),
        &codex_home.join("skills/ast-grep/SKILL.md"),
        keep_mine,
    )
}

/// The testable core: all paths explicit, ZERO env dependence. Checks never write — repair
/// is `repair_skill`, sequenced by the binary as check → repair → re-check, so `--check`
/// (which runs this with no repair step at all) holds its diagnose-only promise by
/// construction.
pub fn check_skills_at(
    new_tree: &Path,
    xgrep_dest: &Path,
    ast_grep_dest: &Path,
    codex_dest: &Path,
    keep_mine: bool,
) -> Vec<Component> {
    let source = new_tree.join("skills/xgrep/SKILL.md");
    let ast_source = new_tree.join("skills/ast-grep/SKILL.md");
    let specs = [
        ("skill_xgrep", source.as_path(), xgrep_dest),
        ("skill_ast_grep", ast_source.as_path(), ast_grep_dest),
        ("skill_ast_grep_codex", ast_source.as_path(), codex_dest),
    ];
    let mut out = Vec::new();
    for (name, src, dest) in specs {
        let stamp_path = dest.parent().map(|p| p.join(MANAGED_STAMP_NAME));
        let read = |p: &Path| fs::read_to_string(p);
        let comp = if !src.exists() {
            // A release may legitimately drop a skill; its deployed copy is a leftover, not
            // a failure and not a repair target.
            Component {
                name: name.to_string(),
                state: ComponentState::Absent,
                detail: "no skill source in the new tree".into(),
            }
        } else if !dest.exists() {
            Component {
                name: name.to_string(),
                state: ComponentState::Drifted,
                detail: "missing (the new tree ships it)".into(),
            }
        } else {
            match (read(src), read(dest)) {
                (Err(_), _) | (_, Err(_)) => Component {
                    name: name.to_string(),
                    state: ComponentState::Unreadable,
                    detail: "skill source or destination unreadable".into(),
                },
                (Ok(s), Ok(d)) if s == d => {
                    // A stale stamp beside matching bytes is cosmetic — install.sh made stamp
                    // writes atomic, this check does not chase cosmetics.
                    Component {
                        name: name.to_string(),
                        state: ComponentState::Current,
                        detail: String::new(),
                    }
                }
                (Ok(_), Ok(_)) => {
                    let managed = stamp_path.as_deref().is_some_and(|p| p.exists());
                    let detail = if managed {
                        format!(
                            "diverged from the new tree (managed, repairable{})",
                            if keep_mine { ", kept per user" } else { "" }
                        )
                    } else {
                        "diverged from the new tree (unmanaged — adopting it is install.sh's \
--force decision, never an upgrade's)"
                            .to_string()
                    };
                    let state = if keep_mine && managed {
                        ComponentState::DeferredByUser
                    } else {
                        ComponentState::Drifted
                    };
                    Component {
                        name: name.to_string(),
                        state,
                        detail,
                    }
                }
            }
        };
        out.push(comp);
    }
    out
}

/// Redeploy one skill: the new bytes, published by staged rename (a half-written skill would
/// hash to something the stamp does not name — install.sh's `write_skill` discipline), then
/// claimed in the stamp beside it, in the exact format install.sh's `ensure_skill_stamp`
/// writes. The FORMAT is a contract pinned by `repair_redeploys_skill_then_recheck_says_current`,
/// not restated as code that parses install.sh.
pub fn repair_skill(source: &Path, dest: &Path) -> std::io::Result<()> {
    let bytes = fs::read(source)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_file_name(format!(
        ".{}.cort-upgrade-tmp",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("skill")
    ));
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, dest)?;
    use sha2::{Digest, Sha256};
    let stamp = format!(
        "{}\nskill_sha256:{:x}\n",
        MANAGED_SIGNATURE,
        Sha256::digest(&bytes)
    );
    let stamp_path = dest
        .parent()
        .ok_or_else(|| std::io::Error::other("skill dest has no parent directory"))?
        .join(MANAGED_STAMP_NAME);
    fs::write(stamp_path, stamp)
}

// ── Task 3: hooks — expected-shape comparison, repair re-verified ──

/// Pure judgment over one `--status --lean` TSV: the expected set is
/// `HOOK_HARNESSES × EVENTS` with per-row command EQUALITY (`{shim} {subcommand} --harness
/// {harness}`, the subcommand from the row's OWN event — expecting hook-suggest on a Refresh
/// row was the first draft's bug) plus per-row `entry_shape_ok` on the settings file the row
/// names. No subprocess, no repair: `--check` runs exactly this and cannot fix anything.
pub fn judge_hooks(shim: &Path, status_tsv: &str) -> Component {
    let name = "hooks".to_string();
    let rows: Vec<Vec<&str>> = status_tsv
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').collect())
        .collect();
    let mut misses: Vec<String> = Vec::new();
    for harness in crate::settings::HOOK_HARNESSES {
        for event in crate::settings::EVENTS {
            let expected = format!(
                "{} {} --harness {}",
                shim.display(),
                event.subcommand(),
                harness
            );
            let row = rows
                .iter()
                .find(|r| r.len() >= 6 && r[0] == harness && r[1] == event.flag_name());
            let label = format!("{harness}/{}", event.flag_name());
            match row {
                None => misses.push(format!("{label}: missing")),
                Some(r) if r[2] != "wired" => {
                    misses.push(format!("{label}: {}", r[2]));
                }
                Some(r) if r[5] != expected => {
                    misses.push(format!("{label}: wired to `{}`", r[5]));
                }
                Some(r) => {
                    // The command matches; the row's settings file decides the shape. A path
                    // of `-` (no settings recorded) or one that fails the dialect's shape
                    // check is a miss — this is the arm that catches a matcher-only rewrite.
                    if r[3] == "-"
                        || !crate::settings::entry_shape_ok(
                            harness,
                            Path::new(r[3]),
                            event,
                            &expected,
                        )
                    {
                        misses.push(format!("{label}: command matches but entry shape drifted"));
                    }
                }
            }
        }
    }
    if misses.is_empty() {
        Component {
            name,
            state: ComponentState::Current,
            detail: String::new(),
        }
    } else {
        Component {
            name,
            state: ComponentState::Drifted,
            detail: misses.join("; "),
        }
    }
}

/// The judging half plus the repair loop: judge → all-Current? return (repair NOT called) →
/// else repair() once, re-run status, re-judge. Still wrong → Drifted with the row detail;
/// run_status Err → Unreadable, never a pass.
pub fn check_hooks(
    shim: &Path,
    run_status: &dyn Fn() -> Result<String, String>,
    repair: &dyn Fn(),
) -> Component {
    let first = match run_status() {
        Ok(t) => t,
        Err(e) => {
            return Component {
                name: "hooks".to_string(),
                state: ComponentState::Unreadable,
                detail: format!("status failed: {e}"),
            };
        }
    };
    let judged = judge_hooks(shim, &first);
    if matches!(judged.state, ComponentState::Current) {
        return judged;
    }
    repair();
    match run_status() {
        Ok(second) => judge_hooks(shim, &second),
        Err(e) => Component {
            name: "hooks".to_string(),
            state: ComponentState::Unreadable,
            detail: format!("status failed after repair: {e}"),
        },
    }
}
