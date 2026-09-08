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

// ── Task 4: index migration — eager for live directories, deferred for gone ones ──

/// The eager/deferred/gone/unreadable policy of spec §4, one `Component` per project from
/// `crate::db::list_projects`. Runs INSIDE the upgrader's held locks (Task 2), so hooks are
/// stood down and there is no concurrent writer. Every project keeps its durable reasons —
/// plan 2 already persists them; nothing here writes new state.
///
/// States: Current (rebuilt or already-current) / Drifted (deferred, or a rebuild that did
/// not take, or a rebuild that errored) / Absent (directory gone; reasons recorded, never a
/// failure — Task 5 maps unacked Drifted to Partial, and "gone never fails" would be a lie
/// otherwise) / Unreadable (db would not open, or the post-rebuild re-read failed — NOT a
/// pass, never silently Current).
pub fn migrate_indexes(defer: bool) -> Vec<Component> {
    let mut out = Vec::new();
    for entry in crate::db::list_projects() {
        match entry {
            crate::db::ProjectEntry::Unreadable { db_path, reason } => {
                out.push(Component {
                    name: "index_unreadable".to_string(),
                    state: ComponentState::Unreadable,
                    detail: format!("{db_path}: {reason}"),
                });
            }
            crate::db::ProjectEntry::Indexed(row) => {
                let gone = !Path::new(&row.path).is_dir();
                let name = format!("index:{}", row.path);
                // Open WITHOUT migrating (open-project-unmigrated discipline — a structural
                // migration inside the upgrader is Task 0/5's install.sh path, and doing it
                // here would hide it). Read reasons through Task 1's read-only primitive.
                let reasons = match read_reasons_readonly(&row.db_path) {
                    Some(r) => r,
                    None => {
                        out.push(Component {
                            name,
                            state: ComponentState::Unreadable,
                            detail: format!("{}: metadata unreadable", row.db_path),
                        });
                        continue;
                    }
                };
                if reasons.is_empty() {
                    out.push(Component {
                        name,
                        state: ComponentState::Current,
                        detail: String::new(),
                    });
                    continue;
                }
                if gone {
                    out.push(Component {
                        name,
                        state: ComponentState::Absent,
                        detail: format!("directory gone, debt kept: {}", reasons.join(", ")),
                    });
                    continue;
                }
                if defer {
                    out.push(Component {
                        name,
                        state: ComponentState::Drifted,
                        detail: format!("deferred: {}", reasons.join(", ")),
                    });
                    continue;
                }
                // Eager: foreground rebuild through the crate's own index path — the upgrader
                // IS a foreground actor. `row` is owned; `name` was cloned from it up front.
                match rebuild_project(&row) {
                    Ok(()) => {
                        // Re-read, and a FAILED re-read is Unreadable ("re-read failed"),
                        // NEVER Current: an unreadable-between-rebuild-and-verify db must not
                        // read as empty reasons and a pass.
                        match read_reasons_readonly(&row.db_path) {
                            Some(after) if after.is_empty() => {
                                out.push(Component {
                                    name,
                                    state: ComponentState::Current,
                                    detail: String::new(),
                                });
                            }
                            Some(after) => {
                                out.push(Component {
                                    name,
                                    state: ComponentState::Drifted,
                                    detail: format!("rebuild did not take: {}", after.join(", ")),
                                });
                            }
                            None => {
                                out.push(Component {
                                    name,
                                    state: ComponentState::Unreadable,
                                    detail: format!(
                                        "{}: re-read failed after rebuild",
                                        row.db_path
                                    ),
                                });
                            }
                        }
                    }
                    Err(e) => out.push(Component {
                        name,
                        state: ComponentState::Drifted,
                        detail: format!("rebuild failed: {e}"),
                    }),
                }
            }
        }
    }
    out
}

/// The foreground rebuild for one project row: open read-write through `open_db` +
/// `ensure_schema` (the structural migration IS part of repaying `schema_changed`), then
/// `incremental_index` with `RebuildPolicy::Allow`. A `FullRebuildRequired` here is a BUG
/// (foreground policy forbids nothing); it surfaces as the Err string and reads Drifted
/// rather than panicking, same as any other failure.
fn rebuild_project(row: &crate::db::ProjectListRow) -> Result<(), crate::indexer::IndexError> {
    let mut db = crate::db::open_db(&row.db_path)?;
    crate::db::ensure_schema(&db)?;
    let bin = crate::ast_grep::resolve_ast_grep_bin()?;
    crate::incremental::incremental_index(
        &mut db,
        &bin,
        &row.path,
        crate::incremental::RebuildPolicy::Allow,
    )
    .map(|_| ())
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
    /// The new tree's cort binary. Hook copy, judgement and fixes live in the binary, not
    /// in the pack or the shim — without this component a code-only change produced a new
    /// binary while diagnose read everything Current and the upgrade did nothing (found
    /// deploying the P2/P4 hook copy; Kimi/Codex rounds missed it, the machine caught it).
    pub new_binary: &'a Path,
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
    // Binary content: the pack can be identical and the shim pristine while the CODE changed
    // (hook copy, judgement, fixes all live in the cort binary). Hash both sides — same
    // construction as the shim's content check, one level down.
    out.push(
        match (
            fs::read(cort_home.join("cort")),
            fs::read(inputs.new_binary),
        ) {
            (Ok(a), Ok(b)) if a == b => Component {
                name: "binary".into(),
                state: ComponentState::Current,
                detail: String::new(),
            },
            (Ok(a), Ok(b)) => {
                use sha2::{Digest, Sha256};
                Component {
                    name: "binary".into(),
                    state: ComponentState::Drifted,
                    detail: format!(
                        "installed binary {:x}, new binary {:x}",
                        Sha256::digest(&a),
                        Sha256::digest(&b)
                    ),
                }
            }
            (Err(e), _) | (_, Err(e)) => Component {
                name: "binary".into(),
                state: ComponentState::Unreadable,
                detail: format!("a binary could not be read: {e}"),
            },
        },
    );
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
                    Some(reasons) => {
                        // Same gone policy as `migrate_indexes`, because `--check` reads
                        // through THIS path and spec §6's "gone never fails" has no
                        // mutating-route exception: a directory that no longer exists
                        // reads Absent with its debt recorded, never Drifted — otherwise
                        // `--check` would fail every machine that ever deleted an indexed
                        // project (review catch, Grok round).
                        if !Path::new(&row.path).is_dir() {
                            out.push(Component {
                                name,
                                state: ComponentState::Absent,
                                detail: format!(
                                    "directory gone, debt kept: {}",
                                    reasons.join(", ")
                                ),
                            });
                        } else {
                            out.push(Component {
                                name,
                                state: ComponentState::Drifted,
                                detail: format!("needs: {}", reasons.join(", ")),
                            });
                        }
                    }
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
    let specs = skill_paths(new_tree, home);
    check_skills_at(new_tree, &specs[0].2, &specs[1].2, &specs[2].2, keep_mine)
}

/// Skill (name, source, destination) triples, resolved exactly as install.sh resolves them —
/// the same env overrides `check_skills` reads. Repair MUST write the destination diagnosis
/// looked at: a hard-coded default home misses the real (overridden) copy and can overwrite
/// an unrelated file at the default path (Codex review round).
pub fn skill_paths(new_tree: &Path, home: &Path) -> Vec<(&'static str, PathBuf, PathBuf)> {
    let claude_home = std::env::var_os("CLAUDE_SKILL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    vec![
        (
            "skill_xgrep",
            new_tree.join("skills/xgrep/SKILL.md"),
            home.join(".claude/skills/xgrep/SKILL.md"),
        ),
        (
            "skill_ast_grep",
            new_tree.join("skills/ast-grep/SKILL.md"),
            claude_home.join("skills/ast-grep/SKILL.md"),
        ),
        (
            "skill_ast_grep_codex",
            new_tree.join("skills/ast-grep/SKILL.md"),
            codex_home.join("skills/ast-grep/SKILL.md"),
        ),
    ]
}

/// The one repair target an upgrade may act on: a skill that is Drifted AND owned — the
/// installer's stamp beside the destination. An unmanaged divergence is install.sh --force's
/// decision (spec: adopting it is never an upgrade's), and the decision reads the stamp on
/// disk, never detail prose: the first draft's `contains("managed")` also matched
/// "unmanaged" and would overwrite a user's own unstamped skill (Codex review round).
///
/// Deliberate consequence of reading the stamp: a DELETED skill whose stamp survives also
/// reads Drifted ("missing (the new tree ships it)") and IS repaired — the file is
/// re-created. A managed asset the user removed is drift, and repair rewrites the stamp in
/// the installer's format, so ownership is preserved (Kimi review round, behavior note).
pub fn skill_repair_target(
    comp: &Component,
    new_tree: &Path,
    home: &Path,
) -> Option<(PathBuf, PathBuf)> {
    if !matches!(comp.state, ComponentState::Drifted) {
        return None;
    }
    let (_name, src, dest) = skill_paths(new_tree, home)
        .into_iter()
        .find(|(n, _, _)| *n == comp.name)?;
    let managed = dest.parent()?.join(MANAGED_STAMP_NAME).exists();
    managed.then_some((src, dest))
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

// ── Task 5: the binary's library half — verdict, acks, marker, deadline, check path ──
//
// The bin sequences and prints; every decision here is a function, so the tests above hold
// without spawning a process.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeExit {
    Ok,
    Partial,
    Fatal,
}

#[derive(Debug)]
pub struct Verdict {
    pub components: Vec<Component>,
    pub exit: UpgradeExit,
}

/// Spec §6, verbatim: Partial for any Drifted/Unreadable component not acked; Ok otherwise.
/// `Absent` never counts (gone never fails — the verdict test feeds Absent by hand, and Task
/// 4's gone-directory test proves the pipeline really emits it). `Fatal` is never derived
/// here: it is set by the binary only, for locks-unobtainable and staging-failed, and the
/// type system keeps that one-directional on purpose. Acked components stay in the list —
/// they print as info lines, they just stop failing the exit.
pub fn verdict(components: Vec<Component>, acks: &[&str]) -> Verdict {
    let mut exit = UpgradeExit::Ok;
    for c in &components {
        let acked = acks.contains(&c.name.as_str());
        if acked {
            continue;
        }
        if matches!(
            c.state,
            ComponentState::Drifted | ComponentState::Unreadable
        ) {
            exit = UpgradeExit::Partial;
        }
    }
    Verdict { components, exit }
}

/// Load persisted acks from `<cache>/.upgrade-acks` (one name per line). Missing file →
/// empty. Garbage → empty (never a crash, never a pass — an unreadable memory must not
/// silence real drift, and must not wedge the run either).
pub fn load_acks(cache: &Path) -> Vec<String> {
    let Ok(raw) = fs::read_to_string(cache.join(".upgrade-acks")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let name = line.trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Persist one ack (idempotent — acking twice is not two acks). The bin `save_ack`s each
/// `--ack <name>` BEFORE applying it, so the next invocation already knows.
pub fn save_ack(cache: &Path, name: &str) -> std::io::Result<()> {
    let path = cache.join(".upgrade-acks");
    let mut names = load_acks(cache);
    if names.iter().any(|n| n == name) {
        return Ok(());
    }
    names.push(name.to_string());
    let mut body = String::new();
    for n in &names {
        body.push_str(n);
        body.push('\n');
    }
    fs::write(path, body)
}

/// First-upgrade note: `None` when `.upgraded_once` exists beside the lock files, else an
/// INFO component naming the WAL-reader risk. It rides the verdict's component list so it is
/// printed, never smuggled — and its Current state can never fail the exit. The bin writes
/// the marker only after a FULLY successful run.
pub fn first_upgrade_note(cache: &Path) -> Option<Component> {
    if cache.join(".upgraded_once").exists() {
        return None;
    }
    Some(Component {
        name: "partial_drain_first_upgrade".to_string(),
        state: ComponentState::Current,
        detail: "first upgrade under the new locking: the 30s drain cannot prove exclusion \
                 of workers running pre-lock cort binaries, so a reader mid-WAL-write when \
                 the payload flipped could have been reading a database being migrated \
                 beneath it. Future upgrades drain against lock-aware workers; this note \
                 does not recur."
            .to_string(),
    })
}

pub fn write_first_upgrade_marker(cache: &Path) -> std::io::Result<()> {
    fs::write(cache.join(".upgraded_once"), b"")
}

/// Run `<bin> <args>` capturing stdout with a deadline; SIGTERM on timeout. The one
/// subprocess runner every upgrader invocation shares — `--check` and the mutating run
/// alike — so a hanging child can wedge a status poll but never the upgrade.
pub fn run_capture_with_deadline(
    bin: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<(String, String), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::thread;

    let mut child = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    let pid = child.id();
    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let out_h = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let err_h = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    let drain = |out_h: thread::JoinHandle<Vec<u8>>,
                 err_h: thread::JoinHandle<Vec<u8>>|
     -> (String, String) {
        let stdout = String::from_utf8_lossy(&out_h.join().unwrap_or_default()).into_owned();
        let stderr = String::from_utf8_lossy(&err_h.join().unwrap_or_default()).into_owned();
        (stdout, stderr)
    };

    match rx.recv_timeout(timeout) {
        Ok(Ok(status)) => {
            let (stdout, stderr) = drain(out_h, err_h);
            if !status.success() {
                return Err(format!(
                    "{} {:?} exited {}: {}",
                    bin.display(),
                    args,
                    status.code().unwrap_or(-1),
                    stderr.trim()
                ));
            }
            Ok((stdout, stderr))
        }
        Ok(Err(e)) => Err(format!("{}: {e}", bin.display())),
        Err(_) => {
            #[cfg(unix)]
            crate::ast_grep::send_sigterm(pid);
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let _ = drain(out_h, err_h);
            Err(format!(
                "{} {:?} timed out after {}ms",
                bin.display(),
                args,
                timeout.as_millis()
            ))
        }
    }
}

/// Run `<bin> hook-install --all --status --lean` with a deadline. `Err` on spawn failure,
/// nonzero exit, output that is not the 6-field TSV, or timeout (the `--check` nonblocking
/// pin).
pub fn run_status_with_deadline(bin: &Path, timeout: Duration) -> Result<String, String> {
    let (stdout, stderr) = run_capture_with_deadline(
        bin,
        &["hook-install", "--all", "--status", "--lean"],
        timeout,
    )?;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        if line.split('\t').count() < 6 {
            return Err(format!("status output unparsable: {stderr}"));
        }
    }
    Ok(stdout)
}

/// The `--check` diagnosis: Task-1 `diagnose` inputs the caller already gathered, PLUS the
/// hook judgment — with NO repair callback in the signature. `--check` cannot repair because
/// there is nothing to call: `judge_hooks` is the pure judgment half of `check_hooks`, and
/// the check path is `run_status_with_deadline` → `judge_hooks`. The mutating run uses
/// `check_hooks` (judge → repair → re-judge). "Diagnose only" is a wiring fact, not prose.
pub fn diagnose_for_check(
    inputs: &DiagnoseInputs,
    shim: &Path,
    run_status: &dyn Fn() -> Result<String, String>,
) -> Vec<Component> {
    let mut comps = diagnose(inputs);
    match run_status() {
        Err(e) => comps.push(Component {
            name: "hooks".to_string(),
            state: ComponentState::Unreadable,
            detail: format!("status failed: {e}"),
        }),
        Ok(tsv) => comps.push(judge_hooks(shim, &tsv)),
    }
    comps
}
