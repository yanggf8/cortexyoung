# Cort-Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `cort-upgrade`, the repo-local binary that diagnoses an existing installation and brings every component to what the new source tree requires — under locks, with a verdict a user can act on.

**Architecture:** A third `[[bin]]` in the `rust/` crate (`src/bin/cort_upgrade.rs`) with shared logic in a new `rust/src/upgrade.rs` module. It runs the spec §5 sequence — diagnose, stage, validate, take two flocks, flip, rewiring, migrate indexes, single verdict — consuming the single-home facts plan 3b shipped (`install::render_shim`, `ast_grep_provenance`, `MANIFEST_KEYS`) so no inventory is restated. Component locks (admission + activity `flock`s over stable paths) coordinate with hooks; index migration rides the already-merged `rebuild_reasons` machinery.

**Tech Stack:** Rust (`rust/` crate — new module, new bin, small `main.rs` hook-plumbing addition), Bash (smoke-test additions only). File locking via `libc::flock` direct FFI — the repo already carries a `libc`-shaped `extern "C"` precedent (`ast_grep.rs::send_sigterm`), the crate is vendored offline, and one 3-line declaration beats a new dependency for a two-call site.

**Spec:** `docs/superpowers/specs/2026-09-06-cort-upgrade-design.md` (§2 diagnosis, §3 locks, §5 order, §6 verdict; §4 refusal/repayment and §9's five findings already shipped in plans 2 and 3a)

**Plan 3c of 3.** Plans 1 (drift diagnosis), 2 (refusal and repayment) and 3a (atomic publication) are on master. This plan's three predecessor facts: 3b put every inventory in `rust/src/install.rs`; 3a made `install.sh` atomic and gave the payload a generation-symlink layout; plan 2 made version drift a cause of `index_is_stale` with `rebuild_reasons` as the one shared reader.

## Global Constraints

- Repo is pure Rust; the only executable Bash is `install.sh` and `tests/install-smoke.sh`. `cort-upgrade` is a Rust `[[bin]]`, never installed — `tests/install-smoke.sh`'s payload assertion (only `cort` + pack ship) must stay green untouched.
- Run `bash -n install.sh && bash -n tests/install-smoke.sh`, then `bash tests/install-smoke.sh` (which needs a fresh `cargo build --release --locked --manifest-path rust/Cargo.toml` first — the merge lesson: a stale release binary against new pack files is exactly the mixed generation this plan exists to prevent), then in `rust/` AND `evals/`: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets`. Every command must exit 0 — report each exit code, never end a pipeline in `tail`.
- Storage failures are returned, never panicked on.
- The upgrader must not restate any inventory — it consumes `cort::install::*` and `HOOK_TARGETS` through the shipped binary's own verbs, never through restated tables.
- `unreadable` is never reported as `absent`, and never as a pass.
- No absolute developer paths anywhere, including fixtures.
- Every task's Step 5 deliberately breaks the implementation and confirms the new test goes red — and every break validation must assert the suite actually ran (exit code and result line), per the plan-3b execution record.

## The one scope decision to state up front

The spec's §5 lists eleven steps. Steps 2-3 (build and stage the new payload) are **`install.sh`'s existing job** — 3a already made them atomic, validated and generation-named. This plan does not re-implement a builder; `cort-upgrade` **invokes `install.sh` for the payload**, taking the locks *around* that invocation, exactly as §3(b) intended: the locks coordinate cort processes with the upgrade, and install.sh's own flock coordinates installer-with-installer. What this plan owns is everything install.sh was never meant to do: diagnose against the new tree's requirements, hold the cort-process locks, re-verify wiring and skills by content, migrate indexes, and produce the verdict.

---

## File Structure

- `rust/src/upgrade.rs` (NEW) — all logic: `Diagnosis`/`Component` types, `diagnose()`, `UpgradeLocks` (two-flock acquire/drop), `migrate_indexes()`, `Verdict`/exit-code mapping. No I/O decisions inline; every check is a pure function over paths it is handed.
- `rust/src/bin/cort_upgrade.rs` (NEW) — thin: arg parsing (`--ack`, `--keep-mine`, `--defer`, `--check`), calls `upgrade::*`, prints the verdict, exits with the classified code.
- `rust/Cargo.toml` — one `[[bin]]` entry, following the `fake_ast_grep` comment style.
- `rust/src/main.rs` — ONE addition: `hook-refresh`/`hook-suggest`/`cmd_index` entry paths take the shared activity lock (admission → activity → drop admission), via a helper in `upgrade.rs`. Three call sites, no policy duplication.
- `rust/tests/upgrade.rs` (NEW) — every property below, as end-to-end as the fixtures allow.
- `tests/install-smoke.sh` — one test: the payload assertion still holds with the third bin built.

---

### Task 1: diagnosis — every component, checked against the new tree

**Files:**
- Create: `rust/src/upgrade.rs` (diagnosis half only), `rust/tests/upgrade.rs`
- Modify: `rust/src/lib.rs` (one `pub mod upgrade;`)

**Interfaces:**
- Consumes: `cort::install::{ast_grep_provenance, MANIFEST_KEYS, render_shim}`, `cort::pack::extractor_version`, `cort::db::{SCHEMA_VERSION, list_projects, ProjectEntry}`, `cort::usage::USAGE_SCHEMA_VERSION`.
- Produces: `pub struct Component { pub name: &'static str, pub state: ComponentState, pub detail: String }`, `pub enum ComponentState { Current, Drifted, Unreadable, Absent, DeferredByUser }`, `pub fn diagnose(install_root: &Path, new_tree: &Path, acks: &[&str]) -> Vec<Component>`. `Unreadable` never maps to `Current`; `Absent` never fails.

The eight checked components (spec §2; `xg` explicitly out): payload identity (query the installed `cort` binary directly, through its manifest path — never the shim — for its extractor/schema pair; drift here is plan 1's `drifted`), shim content (`render_shim(cort_home)` vs the file at `manifest_get cort_bin`), ast-grep version (`ast_grep_provenance().version` vs the CLI on PATH), three skills (content-diff against `$new_tree/skills/...`, not stamp ownership), hook entries (shipped binary's `hook-install --all --status --lean`, compared against what this tree's `HOOK_TARGETS` would produce), manifest key-set (`internal-manifest-keys` output vs keys present in the live manifest), indexes (already served by `cort projects --verdict` on the installed binary), `usage.db` version.

- [ ] **Step 1: Write the failing test**

In `rust/tests/upgrade.rs`:

```rust
//! cort-upgrade properties. Every fixture here is end-to-end where the property demands it:
//! real directory layouts, real symlinks, real installed-shaped files. No toy renames.

use std::fs;
use std::path::Path;

/// A minimal installed-shape root: manifest with cort_bin, a real shim file, a pack dir.
/// Returns (root, cort_home, bin_dir). This is the one fixture every diagnosis test shares —
/// build ONCE per test via this fn so each test controls its own drift.
fn installed_root() -> ((tempfile::TempDir, tempfile::TempDir, tempfile::TempDir), std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let cort_home = home.path().join("cortexyoung/cort");
    fs::create_dir_all(cort_home.join("pack")).unwrap();
    let manifest_dir = home.path().join("cortexyoung");
    fs::write(
        manifest_dir.join("manifest"),
        format!("cort_bin:{}\n", bin.path().join("cort").display()),
    )
    .unwrap();
    fs::write(bin.path().join("cort"), "#!/bin/sh\nexit 0\n").unwrap();
    (root, home.path().to_path_buf(), cort_home, bin.path().to_path_buf())
}
```

The first real property — the one spec §7 names as the weak fixture for everyone else:

```rust
/// A lying shim must be caught by content, not existence. The shim is the component whose
/// "check" was pure theatre before 3b (the binary never answered for it).
#[test]
fn a_shim_that_differs_from_render_shim_is_drifted_not_current() {
    let (_g, home, cort_home, bin_dir) = installed_root();
    // The shim on disk carries a stale --version line; render_shim would produce something else.
    fs::write(
        bin_dir.join("cort"),
        "#!/usr/bin/env bash\nif [ \"$1\" = \"--version\" ]; then echo \"cort 0.0.9 (rust)\"; exit 0; fi\n",
    )
    .unwrap();
    let manifest = home.join("cortexyoung/manifest");
    // Point manifest cort_bin at our shim file (fixture already did), and render what THIS tree
    // would ship:
    let expected = cort::install::render_shim(&cort_home.to_string_lossy());
    let on_disk = fs::read_to_string(bin_dir.join("cort")).unwrap();
    assert_ne!(expected.trim_end(), on_disk.trim_end(), "fixture must be drifted");
    // The diagnosis must classify the shim component Drifted. (Direct unit call: the upgrade
    // module exposes per-component checks as pub fns so tests can call one component at a time.)
    let state = cort::upgrade::check_shim(&manifest, &cort_home, Path::new("/nonexistent-tree"));
    assert!(
        matches!(state, cort::upgrade::ComponentState::Drifted),
        "a content-diverged shim is Drifted even though the file exists: {state:?}"
    );
}
```

Second and third properties — unreadable ≠ absent, and the pack-byte fixture spec §7 demands:

```rust
/// A manifest that cannot be read is Unreadable, not Absent — the RootProbe::Unreadable
/// discipline one level up. Fixture: manifest path is a DIRECTORY, so reads fail rather than
/// return empty.
#[test]
fn an_unreadable_manifest_is_unreadable_not_absent() {
    let (_g, home, cort_home, _bin) = installed_root();
    let manifest = home.join("cortexyoung/manifest");
    fs::remove_file(&manifest).unwrap();
    fs::create_dir(&manifest).unwrap(); // reads now fail with EISDIR
    let state = cort::upgrade::check_shim(&manifest, &cort_home, Path::new("/nonexistent-tree"));
    assert!(matches!(state, cort::upgrade::ComponentState::Unreadable), "{state:?}");
}

/// The payload identity check runs the installed binary, not the shim. The fixture is two
/// payload dirs whose binaries agree on --version but whose packs differ by one byte — the
/// exact weak-fixture spec §7 calls impossible to catch via --version.
#[test]
fn payload_identity_is_decided_by_the_binaries_extractor_not_the_version_string() {
    // Two packs differing in one rule byte, two tiny "binaries" (shell scripts) that both
    // print the same version. Diagnosis must diff extractor hashes, so the drifted-pack side
    // is Drifted despite identical version strings.
    let pack_a = tempfile::tempdir().unwrap();
    let pack_b = tempfile::tempdir().unwrap();
    fs::write(pack_a.path().join("r.yml"), "id: a\nlanguage: ts\n").unwrap();
    fs::write(pack_b.path().join("r.yml"), "id: b\nlanguage: ts\n").unwrap();
    // The check under test takes (installed_extracted, new_extracted) pairs, extracted via
    // pack::pack_files over the given dir — so the unit seam is "hash this pack dir".
    let ha = cort::upgrade::pack_identity(pack_a.path()).unwrap();
    let hb = cort::upgrade::pack_identity(pack_b.path()).unwrap();
    assert_ne!(ha, hb, "one differing rule byte must change the identity");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade`
Expected: FAIL to compile — `no module named 'upgrade'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`:

```rust
//! cort-upgrade: diagnose an existing install, then bring it to what THIS tree requires.
//!
//! Repo-local binary, never installed. It runs the new tree's code, so it inherently knows what
//! the new release needs; its only question about the machine is "what's installed" (spec §1).
//! Every inventory it checks is consumed from `cort::install` or the shipped binary's own verbs
//! — restating one is the HOOK_TARGETS sin this module exists to end.

use std::fs;
use std::path::Path;

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
    pub name: &'static str,
    pub state: ComponentState,
    pub detail: String,
}

/// Pack identity for a directory: same construction as `pack::extractor_version` but over an
/// arbitrary dir, so tests can build two packs that differ by one byte. Sorted file list, hashed
/// contents — a shortened list or a substituted byte changes it.
pub fn pack_identity(dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<_> = walk_yaml(dir)?;
    files.sort();
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for f in files {
        h.update(&fs::read(&f)?);
    }
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
pub fn check_shim(manifest: &Path, cort_home: &Path, _new_tree: &Path) -> ComponentState {
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
```

Add `pub mod upgrade;` to `rust/src/lib.rs` (alphabetical, after `usage`). `sha2` is already a dependency. Do not add anything to `main.rs` in this task — the bin comes in Task 4; keeping the module bin-free lets tests exercise it without arg plumbing.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade`
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `check_shim` return `Current` whenever the file exists (ignore content). Expected: the
   drifted-shim test RED — this is spec §7's named weak fixture, and it must not be greenable.
2. Make `check_shim` map read errors to `Absent`. Expected: the unreadable-manifest test RED.
3. Make `pack_identity` hash only file NAMES, not contents. Expected: the pack-byte test RED —
   and note it is RED for the right reason (identities became equal), not a compile error.

- [ ] **Step 6: Verify everything**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/src/lib.rs rust/tests/upgrade.rs
git commit -m "feat(upgrade): per-component diagnosis against the new tree

Diagnosis runs the new tree's code, so it inherently knows what the release
requires; check_shim diffs the on-disk shim against render_shim by content
(the pre-3b check was pure theatre — the binary never answered for it), and
pack_identity hashes contents so a one-byte rule change is Drifted despite
identical version strings. Unreadable manifest is Unreadable, never Absent."
```

---

### Task 2: the two flocks — admission closes the gate, activity drains the room

**Files:**
- Modify: `rust/src/upgrade.rs`, `rust/src/main.rs` (three entry points take shared locks), `rust/tests/upgrade.rs`

**Interfaces:**
- Consumes: Task 1's module.
- Produces: `pub struct UpgradeLocks { _admission: File, _activity: File }`; `pub fn acquire_upgrade_locks(cache_dir: &Path, drain_timeout: Duration) -> Result<UpgradeLocks, LockError>`; `pub enum LockError { AdmissionBusy, DrainTimeout }`; and `pub fn with_protected_locks<T>(cache_dir: &Path, f: impl FnOnce() -> T) -> T` for the three cort entry points (admission shared → activity shared → drop admission → run f → drop activity).

**Paths.** Both lock files live in the stable cache dir (`cort::db::cache_dir()`), NOT the payload generation dir — the payload is what gets replaced; the locks must outlive any replacement. `.upgrade-admission.lock` and `.upgrade-activity.lock`.

**Mechanism** (spec §3b, verbatim): protected operation = flock(admission, LOCK_SH) → flock(activity, LOCK_SH) → funlock(admission) → work → funlock(activity). Upgrader = flock(admission, LOCK_EX) → flock(activity, LOCK_EX) **with deadline** → hold both through flip+migrate. Nonblocking first, then a bounded retry loop for the drain (100ms steps; `drain_timeout` default 30s). On drain timeout: abort the upgrade — never kill the foreground work (spec §3c).

**The honest boundary, stated in the doc comment and the verdict:** the FIRST upgrade cannot drain binaries old enough to predate these locks. They hold no lock we can wait on. SQLite's busy timeout excludes their writers; their WAL readers are excluded by nobody. §3b: claiming a drain from SQLite state alone is false, so the verdict labels the first upgrade `partial-drain` rather than implying exclusion.

- [ ] **Step 1: Write the failing test**

```rust
mod locks {
    use super::*;
    use std::time::Duration;

    /// A protected operation that started BEFORE the upgrader took admission must block the
    /// drain: the activity lock is still held shared, so the upgrader's exclusive activity
    /// attempt times out and the upgrade aborts — never kills the worker.
    #[test]
    fn a_worker_holding_activity_blocks_the_drain_and_the_upgrade_aborts() {
        let cache = tempfile::tempdir().unwrap();
        // Worker: admission(sh) -> activity(sh) -> release admission, hold activity 3s.
        let worker = std::thread::spawn(|| {
            let cache = cache.path().to_path_buf();
            cort::upgrade::with_protected_locks(&cache, || {
                std::thread::sleep(Duration::from_millis(2500));
                "worked"
            })
        });
        std::thread::sleep(Duration::from_millis(200)); // let the worker take both locks
        let result =
            cort::upgrade::acquire_upgrade_locks(cache.path(), Duration::from_millis(800));
        assert!(
            matches!(result, Err(cort::upgrade::LockError::DrainTimeout)),
            "a live worker must time the drain out: {result:?}"
        );
        assert_eq!(worker.join().unwrap(), "worked");
    }

    /// After the worker releases, the upgrader gets in. And once it holds, a NEW protected
    /// operation must stand down at admission — that is the gate closing, the reverse-notify
    /// of spec §3c.
    #[test]
    fn after_drain_the_upgrader_holds_and_new_workers_stand_down() {
        let cache = tempfile::tempdir().unwrap();
        let locks =
            cort::upgrade::acquire_upgrade_locks(cache.path(), Duration::from_secs(2)).unwrap();
        // A new worker must see admission BUSY (upgrader holds it exclusive).
        let stood_down =
            cort::upgrade::try_protected_entry(cache.path()).is_err();
        assert!(stood_down, "admission exclusive must turn new workers away");
        drop(locks);
        // After release, a worker gets in again.
        assert!(cort::upgrade::try_protected_entry(cache.path()).is_ok());
    }

    /// Crash safety is the OS's job: if the holder's pid dies, the kernel drops the flock and
    /// the next acquirer gets in. Fixture: fork-style — spawn a child that takes the locks and
    /// SIGKILLs itself; the parent must acquire within the timeout rather than seeing a stale
    /// lock. (This is the test spec §7 names: not "expired lease + dead pid", which any
    /// TTL-less impl passes — but an actual process death.)
    #[test]
    fn a_killed_holder_releases_the_locks() {
        let cache = tempfile::tempdir().unwrap();
        let cache_path = cache.path().to_path_buf();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("lock_holder_child") // test-internal dispatch, see bin's test hook
            .spawn()
            .expect("spawn holder child");
        std::thread::sleep(Duration::from_millis(400));
        child.kill().unwrap();
        child.wait().unwrap();
        let got =
            cort::upgrade::acquire_upgrade_locks(&cache_path, Duration::from_secs(2));
        assert!(got.is_ok(), "kernel must release flocks on process death: {got:?}");
    }
}
```

The child dispatch: `std::env::current_exe()` under `cargo test` is the test binary, so
`lock_holder_child` as argv[1] is dispatchable in the test crate's `main` — Rust test binaries
support a custom `main` via `#[start]`-free pattern: add at the bottom of `rust/tests/upgrade.rs`:

```rust
// Child dispatch for the killed-holder test: the test binary re-enters itself with argv[1]
// as the mode. This must live in the test crate because the holder must be the SAME
// executable that holds the locks (flock is per-open-file-description, not per-process,
// but re-using the binary keeps the fixture honest).
#[tokio::main]
```

Do NOT use `#[tokio::main]` — no tokio here. The test binary's `main` is generated; the
standard pattern is a `#[test]` that detects its own special argument:

```rust
#[test]
fn lock_holder_child() {
    if !std::env::args().any(|a| a == "lock_holder_child") {
        return; // real test run: no-op
    }
    let cache = std::env::var("UPGRADE_TEST_CACHE").unwrap();
    let _locks = cort::upgrade::acquire_upgrade_locks(
        std::path::Path::new(&cache), Duration::from_secs(60)).unwrap();
    std::thread::sleep(Duration::from_secs(60)); // parent will kill us
}
```

and the spawning test sets `UPGRADE_TEST_CACHE` in the child's env and filters: the spawning
test invokes `current_exe()` with args `["lock_holder_child"]`, the child's `#[test] fn` runs
all tests including itself — so guard the SPAWNING test with
`if std::env::args().any(|a| a == "lock_holder_child") { return; }` at its top. Ugly but
honest; a cleaner bin-based child would need the upgrade bin to grow a test-only mode, which
is production surface for a fixture.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade locks::`
Expected: FAIL to compile — `no function named 'acquire_upgrade_locks'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`. The FFI follows the `send_sigterm` precedent (`ast_grep.rs:226-238`) —
extern flock/funlock directly, no new crate:

```rust
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}
const LOCK_SH: i32 = 1;
const LOCK_EX: i32 = 2;
const LOCK_UN: i32 = 8;
const LOCK_NB: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    AdmissionBusy,
    DrainTimeout,
}

fn open_lock(path: &Path) -> File {
    OpenOptions::new().create(true).write(true).open(path).expect("open lock file")
}

fn flock_fd(file: &File, op: i32) -> bool {
    // SAFETY: fd is a live open file description we own; flock has no other preconditions.
    unsafe { flock(file.as_raw_fd(), op) == 0 }
}

/// Protected-operation entry: admission(sh) + activity(sh), then release admission. Returns
/// the activity guard. Err means admission busy — a new worker standing down (spec §3b).
pub fn try_protected_entry(cache: &Path) -> Result<ActivityGuard, LockError> {
    let adm = open_lock(&cache.join(".upgrade-admission.lock"));
    if !flock_fd(&adm, LOCK_SH | LOCK_NB) {
        return Err(LockError::AdmissionBusy);
    }
    let act = open_lock(&cache.join(".upgrade-activity.lock"));
    if !flock_fd(&act, LOCK_SH | LOCK_NB) {
        drop(adm);
        return Err(LockError::AdmissionBusy);
    }
    drop(adm); // release admission; activity carries the protection
    Ok(ActivityGuard { _file: act })
}

pub struct ActivityGuard {
    _file: File,
}

/// The three cort entry points run their bodies under this.
pub fn with_protected_locks<T>(cache: &Path, f: impl FnOnce() -> T) -> T {
    match try_protected_entry(cache) {
        Ok(_guard) => f(),
        // Admission busy = an upgrade holds it exclusive. Stand down quietly: the hook
        // contract is silence, and the upgrade will leave the system consistent.
        Err(_) => f(),
    }
}

/// NB: the body STILL RUNS on admission-busy. Read that again: for `hook-refresh` the
/// contract "silent and exit 0" means standing down IS running the quiet path — f() is the
/// quiet path there. For `cmd_index`, f() must first try the locks with a wait; see the
/// call-site notes in Task 4.
```

The upgrader side:

```rust
pub struct UpgradeLocks {
    _admission: File,
    _activity: File,
}

pub fn acquire_upgrade_locks(cache: &Path, drain_timeout: Duration) -> Result<UpgradeLocks, LockError> {
    let adm = open_lock(&cache.join(".upgrade-admission.lock"));
    if !flock_fd(&adm, LOCK_EX | LOCK_NB) {
        return Err(LockError::AdmissionBusy);
    }
    // Drain: wait for exclusive activity with a deadline. Every worker holding it shared
    // must let go before this succeeds.
    let act = open_lock(&cache.join(".upgrade-activity.lock"));
    let deadline = Instant::now() + drain_timeout;
    loop {
        if flock_fd(&act, LOCK_EX | LOCK_NB) {
            return Ok(UpgradeLocks { _admission: adm, _activity: act });
        }
        if Instant::now() >= deadline {
            return Err(LockError::DrainTimeout);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
```

(Absolute `$` paths: `open_lock` creates the file if missing — that is fine, both lock files
are in the stable cache dir which every cort process already requires.)

In `rust/src/main.rs`, wire the three entry points (this is the ONE main.rs change this task
makes; Task 4 does nothing here):

- `cmd_hook_refresh` (main.rs:815): wrap the `incremental_index` call in
  `upgrade::with_protected_locks(&cort::db::cache_dir(), || ...)`. On admission-busy the
  closure runs the existing quiet `db_unavailable` path — the contract already says give up
  rather than wait, and an upgrade in flight is exactly "rather than wait".
- `cmd_hook_suggest` (main.rs:964): wrap ONLY the evidence-query path (post shape-gate; spec
  §3b's cost note — the gate-rejected majority must not pay two opens). Same stand-down
  semantics: silent.
- `cmd_index` (main.rs:1653): unlike the hooks, a foreground index WAITS — spec §3c ranks
  killing user foreground work worse than deferring. Implement as a bounded
  `with_protected_locks` retry (same 30s deadline as the upgrader's drain), and on timeout
  proceed WITHOUT the lock but emit a warning field in the existing JSON payload:
  `"upgrade_in_flight": true`. Do not fail the user's index.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade locks::`
Expected: all three PASS. The killed-holder test takes ~1s (400ms hold + kill + acquire); if it
takes the full 60s child sleep, the child did not die — check the kill.

- [ ] **Step 5: Verify each test can actually fail**

1. Remove the LOCK_EX on admission in `acquire_upgrade_locks` (take it shared). Expected:
   `after_drain_the_upgrader_holds_and_new_workers_stand_down` RED — new workers get in during
   the upgrade, which is the gate failing open.
2. Make the drain loop `return Ok(...)` on first failure instead of retrying (no deadline).
   Expected: `a_worker_holding_activity_blocks_the_drain_and_the_upgrade_aborts` RED — the
   upgrade proceeds over a live worker.
3. In `try_protected_entry`, drop the activity lock acquisition (admission only). Expected:
   the worker-blocks-drain test RED in the opposite direction — workers never hold activity,
   so the drain never blocks and both tests lie. If it stays green, the drain test is not
   actually exercising the activity lock; fix the fixture.

- [ ] **Step 6: Verify everything**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cargo build --release --locked --manifest-path rust/Cargo.toml && bash tests/install-smoke.sh
```
Expected: all exit 0. The smoke suite exercises the REAL hook paths through the installed shim —
if the lock wiring broke `hook-refresh`, the suite's refresh assertions catch it.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/src/main.rs rust/tests/upgrade.rs
git commit -m "feat(upgrade): admission and activity flocks for upgrades and hooks

Two advisory flocks over stable cache-dir paths, not the payload dir: a
protected operation takes admission shared then activity shared and drops
admission; the upgrader takes admission exclusive (new workers stand down at
the gate) then drains activity exclusive with a deadline (in-flight workers
finish; a timeout aborts the upgrade rather than killing foreground work).
The kernel releases both on process death — no cleanup, no TTL, no
supervisor. hook-refresh and hook-suggest's evidence path pay two opens;
the shape-gated majority pays nothing. cmd_index waits with the same
deadline, then proceeds unlocked with upgrade_in_flight flagged — killing
user foreground work ranks worse than an unlocked index (spec §3c).

First-upgrade honesty: binaries old enough to predate these locks hold
nothing we can wait on. The verdict in Task 5 labels that case
partial-drain; claiming a drain from lock state alone would be false."
```

---

### Task 3: hook wiring and skills — re-verified by content, restored by the shipped verbs

**Files:**
- Modify: `rust/src/upgrade.rs`, `rust/tests/upgrade.rs`

**Interfaces:**
- Consumes: Task 1's `Component`/`ComponentState`; the shipped binary's `hook-install --all
  --status --lean` and `install.sh`'s skill-deploy steps (invoked, not restated).
- Produces: `pub fn check_hooks(managed_cort: &Path, new_tree: &Path) -> Component` and
  `pub fn check_skills(new_tree: &Path, home: &Path) -> Vec<Component>`.

**What "re-verified" means for hooks (spec §5 step 7).** Run the NEW tree's binary (the upgrader
is the new tree's code — so the upgrader's own `hook-install --all --status --lean`, not the
installed one). Compare each row's `command` against what `HOOK_TARGETS`-driven wiring would
produce for the installed shim path. Anything not matching → Drifted → repair by invoking the new
binary's `hook-install --all --command-prefix <shim>`, then RE-CHECK; a row still wrong after
repair is Drifted in the verdict with the row's own detail. **§9 finding 3 is why comparison must
be against expected shape, not the status answer alone.**

**Skills (spec §5 step 8).** Content-diff each of the three destinations against the new tree's
`skills/*/SKILL.md`. Diverged + managed → Drifted, repair by redeploying via `install.sh`'s
deploy path — UNLESS `--keep-mine`, in which case state `DeferredByUser` with detail "diverged,
kept per user". Diverged + unmanaged → Drifted, never auto-overwrite (adopting someone else's file
is install.sh's `--force` decision, not an upgrade's).

- [ ] **Step 1: Write the failing test**

```rust
/// A managed skill whose content diverged from the new tree is Drifted — NOT Current, which is
/// what the old stamp-ownership check answered. Spec §2 row 4: "old skill + valid current stamp
/// passes" was the defect.
#[test]
fn a_diverged_managed_skill_is_drifted_even_with_a_valid_stamp() {
    let new_tree = tempfile::tempdir().unwrap();
    let skill_dir = new_tree.path().join("skills/ast-grep");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "---\nname: ast-grep\n---\nnew body\n").unwrap();

    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join(".claude/skills/ast-grep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "---\nname: ast-grep\n---\nold body\n").unwrap();
    // A VALID stamp for the old body: the pre-3b check saw exactly this and said "managed, fine".
    let stamp_line = format!(
        "managed by cortexyoung install.sh\nskill_sha256:{:x}\n",
        Sha256::digest(b"---\nname: ast-grep\n---\nold body\n")
    );
    fs::write(dest.parent().unwrap().join(".cortexyoung-managed"), stamp_line).unwrap();

    let comps = cort::upgrade::check_skills(new_tree.path(), home.path());
    let ast = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(matches!(ast.state, cort::upgrade::ComponentState::Drifted), "{ast:?}");
}

/// --keep-mine turns the same divergence into DeferredByUser, and the file is untouched —
/// "repair" must never overwrite a user's edit silently.
#[test]
fn keep_mine_leaves_a_diverged_skill_alone() {
    // ... same fixture; check_skills_with_policy(..., keep_mine=true) ...
    // assert DeferredByUser and file bytes still == old body
}

/// Hook repair is re-verified after repair, not assumed. Fixture: a status whose command names
/// a foreign binary; after the repair callback (the test's fake for `hook-install --all`),
/// status returns the right command. The check must call repair, then re-run status, then say
/// Current. A check that repairs without re-verifying cannot detect a repair that did not take.
#[test]
fn hook_repair_is_followed_by_reverification() {
    // Fake managed_cort script: first --status call reports wired-to-foreign; after a marker
    // file exists (created by the fake "repair" callback), reports the expected command.
    // check_hooks(...) must: see Drifted, invoke repair_fn, re-check, and return Current only
    // because the second status agreed. Also: repair_fn NOT called when status already matches.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade skills\: hooks\:` (adjust filter names)
Expected: FAIL to compile — `no function named 'check_skills'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`. The stamp check must use the same hashing install.sh writes
(`skill_hash`: sha256 of the file, stamped as `skill_sha256:<hex>` beside it — read
`install.sh:189-207` for the exact contract and reuse the crate's `sha2`):

```rust
pub fn check_skills(new_tree: &Path, home: &Path) -> Vec<Component> {
    check_skills_with_policy(new_tree, home, /* keep_mine */ false)
}

pub fn check_skills_with_policy(new_tree: &Path, home: &Path, keep_mine: bool) -> Vec<Component> {
    // The three destinations, in install.sh's order: SKILL_DEST, AST_GREP_SKILL_DEST,
    // CODEX_SKILL_DEST. The paths come from env (CLAUDE_SKILL_HOME / CODEX_HOME) exactly as
    // install.sh resolves them — restating hardcoded homes here would be a second home for
    // "where skills live".
    let specs = [
        ("skill_xgrep", "skills/xgrep/SKILL.md", home.join(".claude/skills/xgrep/SKILL.md")),
        ("skill_ast_grep", "skills/ast-grep/SKILL.md", home.join(".claude/skills/ast-grep/SKILL.md")),
        ("skill_ast_grep_codex", "skills/ast-grep/SKILL.md", codex_skill_path()),
    ];
    // For each: absent source -> Absent (not a failure). Absent dest + present source ->
    // Drifted (missing). Present both: content equal -> Current; differ ->
    //   keep_mine ? DeferredByUser : Drifted(repairable).
    // Stamp mismatch + content match -> Current (3a made stamp writes atomic; a stale stamp
    // beside matching bytes is cosmetic, and this plan does not chase cosmetics).
    todo!()
}

fn codex_skill_path() -> std::path::PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"))
        .join("skills/ast-grep/SKILL.md")
}
```

`todo!()` marks the three check bodies — the tests above define the exact contract; write the
three loops. No new dependencies. Home-dir resolution: `cort::db::home_dir()` exists but is
private (`db.rs:82`) — make it `pub` in this task (one-line visibility change, justified by a
second caller); do NOT replicate the `HOME` read locally and do NOT invent a different
resolution.

For hooks, the check consumes the shipped binary as a subprocess:

```rust
pub fn check_hooks(managed_cort: &Path, expected_command: &str, repair: &dyn Fn()) -> Component {
    // 1. Run `<managed_cort> hook-install --all --status --lean`; non-zero or
    //    unknown_command -> Unreadable.
    // 2. Parse TSV rows (harness, ev, outcome, settings, detail, command) — the shape
    //    install.sh:525 already reads; same tab contract.
    // 3. Every row must have outcome==wired AND command starting with expected_command + " ".
    //    Any miss -> invoke repair() once, re-run steps 1-2, and re-judge. Return Drifted
    //    with the offending row's detail if it still misses; Current if all rows pass.
    todo!()
}
```

`expected_command` is `<shim path> hook-suggest --harness <name>` per-harness — derive from
`HOOK_TARGETS`'s harness names by reading the status rows' own `harness` field, keeping the
comparison to the command PREFIX (the shim path), which is what install.sh:537 checks. Do not
restate the per-harness command templates — the wired command already exists in the row; only
its prefix is ours to require.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade`
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `check_skills` treat stamp-validity as sufficient (skip content diff when the stamp
   matches). Expected: the diverged-with-valid-stamp test RED — this is spec §2 row 4's named
   defect and it must be greenable-by-accident-proof.
2. Make `keep_mine` still overwrite. Expected: keep-mine test RED, and the bytes assertion
   (file untouched) is the one that catches it.
3. Make `check_hooks` return Current after invoking repair WITHOUT re-running status. Expected:
   the re-verification test RED — specifically, fake the repair as a no-op and the test must
   still fail (proving re-verification is what catches a failed repair, not the repair call
   itself).

- [ ] **Step 6: Verify everything**

Same two-crate verification as Task 1. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/tests/upgrade.rs
git commit -m "feat(upgrade): hooks and skills re-verified by content, repair re-checked

Skills are content-diffed against the new tree, not stamp-checked — a valid
stamp over old bytes was the exact false-pass of spec §2 row 4. --keep-mine
downgrades divergence to DeferredByUser and leaves bytes untouched. Hook
diagnosis compares status rows against the expected shim prefix and repairs
via the shipped hook-install, then RE-RUNS status before believing the
repair took — a repair that did not take is Drifted with its row detail, not
a silent success."
```

---

### Task 4: index migration — eager for live directories, deferred for gone ones

**Files:**
- Modify: `rust/src/upgrade.rs`, `rust/tests/upgrade.rs`

**Interfaces:**
- Consumes: `cort::db::list_projects()` (with its `Unreadable` variant from plan 1), `cort::indexer::rebuild_reasons`, `cort::indexer::{incremental_index, full_index, RebuildPolicy}` via the project DB, plan 2's `FullRebuildRequired`.
- Produces: `pub fn migrate_indexes(defer: bool) -> Vec<Component>` — one `Component` per project, `name` = project path, state = `Current` (rebuilt or already-current) / `Drifted` (deferred: reasons recorded) / `Unreadable` (db would not open; NOT counted a failure) / `Absent` (directory gone; reasons recorded, never a failure).

**Policy (spec §4, the reviewed middle):** for each project from `list_projects`, read
`rebuild_reasons(db)` (already merged in plan 2). Empty reasons → Current, skip. Non-empty and
directory exists and `!defer` → run the rebuild (foreground policy: `AllowFullRebuild`), then
re-read reasons; still non-empty → Drifted with reasons (a rebuild that did not take). Non-empty
and (directory gone OR `defer`) → Drifted with reasons recorded, no rebuild. Every project keeps
its durable reasons either way — plan 2 already persists them, so nothing here writes new state.

**Flocks:** the whole migration runs inside the upgrader's held locks (Task 2) — hooks stand
down, no concurrent writer. A `FullRebuildRequired` here is a BUG (foreground policy forbids
nothing); map it to Drifted with reasons rather than panicking.

- [ ] **Step 1: Write the failing test**

```rust
/// An index whose stored extractor is superseded gets rebuilt (directory exists, no --defer),
/// and afterwards its reasons are empty. This is the spec §4 eager default, and it is the test
/// that proves the upgrader is the missing ACTOR for the debt plan 2 made visible.
#[test]
fn a_drifted_index_with_a_live_directory_is_rebuilt() {
    // Real end-to-end: build a tiny project, index it with THIS crate's indexer (the tests
    // already have ast-grep available via rust/tests/ fixtures — reuse rust/tests/graph.rs's
    // index_files helper pattern), then overwrite _cortex_meta's extractor_version with
    // "superseded". migrate_indexes(false) must flip it back to current.
    // Assert: reasons empty after; component state Current.
}

/// The same drift with the directory GONE must NOT attempt a rebuild — it records the debt and
/// moves on. Attempting would mean extract_all over a missing tree: either an error storm or,
/// worse, success against whatever partial state remains.
#[test]
fn a_drifted_index_whose_directory_is_gone_is_marked_not_rebuilt() {
    // Index a temp project, capture its db, delete the project dir, supersede the extractor
    // meta. migrate_indexes(false) must leave reasons non-empty and state Drifted (detail
    // names the reasons), and must not have created anything at the old path.
}

/// An unreadable index is reported Unreadable and is NOT a verdict failure (spec §6: unreadable
/// never passes, but gone never fails; unreadable is reported). The verdict aggregation in
/// Task 5 decides what it does to the exit code — here we only pin the component state.
#[test]
fn an_unreadable_index_is_unreadable_and_reported() {
    // A junk *.db in the cache dir; list_projects already reports it Unreadable (plan 1).
    // migrate_indexes must carry that through as ComponentState::Unreadable.
}

/// --defer records the debt without rebuilding, even when the directory is live.
#[test]
fn defer_marks_without_rebuilding() {
    // Same fixture as the first test but migrate_indexes(true): reasons stay non-empty,
    // state Drifted, detail names the reasons, and the index content is untouched.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade migrate\: defer\_`
Expected: FAIL to compile — `no function named 'migrate_indexes'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`:

```rust
pub fn migrate_indexes(defer: bool) -> Vec<Component> {
    let mut out = Vec::new();
    for entry in crate::db::list_projects() {
        match entry {
            crate::db::ProjectEntry::Unreadable { db_path, reason } => {
                out.push(Component {
                    name: "index_unreadable",
                    state: ComponentState::Unreadable,
                    detail: format!("{db_path}: {reason}"),
                });
                continue;
            }
            crate::db::ProjectEntry::Indexed(row) => {
                let gone = !Path::new(&row.path).is_dir();
                // Open WITHOUT migrating (open-project-unmigrated discipline — a structural
                // migration inside the upgrader is Task 3's job via install.sh, and doing it
                // here would hide it). Read reasons through the read-only open.
                let reasons = read_reasons_readonly(&row.db_path);
                let Some(reasons) = reasons else {
                    out.push(Component { name: "index_unreadable", state: ComponentState::Unreadable,
                        detail: format!("{}: metadata unreadable", row.db_path) });
                    continue;
                };
                if reasons.is_empty() {
                    out.push(Component { name: &row.path, state: ComponentState::Current, detail: String::new() });
                    continue;
                }
                if defer || gone {
                    out.push(Component { name: &row.path, state: ComponentState::Drifted,
                        detail: format!("deferred: {}", reasons.join(", ")) });
                    continue;
                }
                // Eager: foreground rebuild through the crate's own index path. Reuse
                // cmd_index's machinery by calling incremental_index directly with
                // RebuildPolicy::Allow — the upgrader IS a foreground actor.
                match rebuild_project(&row) {
                    Ok(()) => {
                        let after = read_reasons_readonly(&row.db_path).unwrap_or_default();
                        if after.is_empty() {
                            out.push(Component { name: &row.path, state: ComponentState::Current, detail: String::new() });
                        } else {
                            out.push(Component { name: &row.path, state: ComponentState::Drifted,
                                detail: format!("rebuild did not take: {}", after.join(", ")) });
                        }
                    }
                    Err(e) => out.push(Component { name: &row.path, state: ComponentState::Drifted,
                        detail: format!("rebuild failed: {e}") }),
                }
            }
        }
    }
    out
}
```

`read_reasons_readonly` opens the db read-only (no `ensure_schema` — the upgrader must not
migrate schema as a side effect of looking) and calls
`cort::indexer::rebuild_reasons(&db)`. `rebuild_project` opens read-write through
`open_db` + `ensure_schema` (the structural migration IS part of repaying `schema_changed`),
then `incremental_index(&mut db, &bin, &row.path, RebuildPolicy::Allow)` with the bin from
`resolve_ast_grep_bin`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade migrate\:` 
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `migrate_indexes` skip the rebuild unconditionally (always defer). Expected: the
   eager test RED — this is the "actor missing" regression, the exact failure mode that made
   the signal worthless.
2. Make it rebuild even when the directory is gone. Expected: the gone-directory test RED
   (rebuild errors or, if it "succeeds", the assertion that nothing was created at the old
   path catches it).
3. Make `read_reasons_readonly` swallow errors to empty reasons. Expected: the unreadable test
   RED — empty reasons would classify the junk db Current, which is the false-pass this plan
   keeps refusing.

- [ ] **Step 6: Verify everything**

Two-crate verification as before, plus `bash tests/install-smoke.sh`. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/tests/upgrade.rs
git commit -m "feat(upgrade): eager index migration for live projects, deferred for gone ones

rebuild_reasons (plan 2) is read through a read-only open — the upgrader
never migrates schema as a side effect of looking. Empty reasons: Current.
Live directory + reasons + no --defer: rebuild with AllowFullRebuild (the
upgrader IS the foreground actor plan 2's refusal was waiting for), then
RE-READ reasons before believing the rebuild took. Gone directory or
--defer: debt recorded, no rebuild. Unreadable: reported, never a pass."
```

---

### Task 5: the binary — verdict, exit taxonomy, escape hatches

**Files:**
- Create: `rust/src/bin/cort_upgrade.rs`
- Modify: `rust/Cargo.toml` (one `[[bin]]`), `rust/src/upgrade.rs` (verdict aggregation), `tests/install-smoke.sh` (one payload assertion still passes — no edit expected)

**Interfaces:**
- Consumes: Tasks 1-4 entirely.
- Produces: the `cort-upgrade` binary; `pub struct Verdict { pub components: Vec<Component>, pub exit: UpgradeExit }`; `pub enum UpgradeExit { Ok, Partial, Fatal }` mapping to 0/1/2.

**Verdict rules (spec §6, verbatim):** `Fatal` only for: locks unobtainable (DrainTimeout/AdmissionBusy), staging failure of the new payload. `Partial` for: any Drifted/Unreadable component not `--ack`ed. `Ok` when nothing drifted and nothing acked-into-silence was found. **Acked components become info lines, never failures.** Every line names the component and a next action (`run cort index`, `rerun with --force via install.sh`, `check your network for ast-grep`). The `--check` mode runs diagnosis only (no locks, no writes) and exits with the same taxonomy.

- [ ] **Step 1: Write the failing test**

```rust
/// The exit taxonomy is the spec §6 contract. A Drifted component is Partial (1), not Ok and
/// not Fatal — Fatal is reserved for "cannot proceed safely".
#[test]
fn drifted_components_are_partial_never_fatal() {
    let comps = vec![
        cort::upgrade::Component { name: "shim", state: cort::upgrade::ComponentState::Drifted, detail: "x".into() },
        cort::upgrade::Component { name: "skill_xgrep", state: cort::upgrade::ComponentState::Current, detail: String::new() },
    ];
    let v = cort::upgrade::verdict(comps.clone(), &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Partial));
    // An acked drift becomes info: still printed (visibility), but not a failure.
    let v = cort::upgrade::verdict(comps, &["shim"]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Ok), "acked drift must not fail the verdict");
}

/// A gone directory is NOT a failure (spec §6: gone never fails) even though its reasons are
/// recorded. This is the rule that keeps an ordinary machine with scratch indexes at exit 1
/// rather than 2.
#[test]
fn deferred_and_gone_indexes_do_not_fail_the_verdict() {
    let comps = vec![
        cort::upgrade::Component { name: "/gone/project", state: cort::upgrade::ComponentState::Absent, detail: "deferred: extractor_changed".into() },
    ];
    let v = cort::upgrade::verdict(comps, &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Ok));
}

/// Unreadable DOES count against the verdict (spec §6: unreadable never passes) — but as
/// Partial, not Fatal.
#[test]
fn unreadable_counts_as_partial() {
    let comps = vec![
        cort::upgrade::Component { name: "index_unreadable", state: cort::upgrade::ComponentState::Unreadable, detail: "junk".into() },
    ];
    let v = cort::upgrade::verdict(comps, &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Partial));
}
```

Plus a smoke addition in `tests/install-smoke.sh`: after the payload assertion, assert the
third bin exists in the build tree but NOT in the installed payload:

```bash
# cort-upgrade is repo-local, never installed — the same rule fake_ast_grep holds. Its presence
# in the payload would be a second executable nobody's package owns.
if [ -x "$REPO_ROOT/rust/target/debug/cort_upgrade" ] || [ -x "$REPO_ROOT/rust/target/release/cort_upgrade" ]; then
  if find "$HOME/.local/share/cortexyoung/cort" -name 'cort_upgrade' -print -quit | grep -q .; then
    fail "cort-upgrade stays out of the installed payload"
  else
    pass "cort-upgrade stays out of the installed payload"
  fi
fi
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade verdict\:`
Expected: FAIL to compile — `no function named 'verdict'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeExit { Ok, Partial, Fatal }

pub fn verdict(components: Vec<Component>, acks: &[&str]) -> Verdict {
    let mut exit = UpgradeExit::Ok;
    for c in &components {
        let acked = acks.contains(&c.name);
        match (c.state, acked) {
            (ComponentState::Unreadable, false) => exit = exit.max_partial(),
            (ComponentState::Drifted, false) => exit = exit.max_partial(),
            _ => {}
        }
    }
    Verdict { components, exit }
}

impl UpgradeExit {
    fn max_partial(self) -> Self {
        match self { UpgradeExit::Fatal => UpgradeExit::Fatal, _ => UpgradeExit::Partial }
    }
}
```

`Fatal` is set by the binary only, for the two conditions above (locks, staging) — it is never
derived from component states, and the type system makes that one-directional on purpose.

In `rust/src/bin/cort_upgrade.rs` — clap struct with `--ack <name>` (repeatable), `--keep-mine`,
`--defer`, `--check` (diagnose only). Sequence for the mutating run: diagnose (Task 1+3) → if
everything is already Current, print and exit 0 — **no locks taken, no install.sh run** (the
common steady state must be cheap and touch nothing). Otherwise: build the new payload via
`install.sh` (Task 2's locks are taken BEFORE the invocation and held through migration;
install.sh's own flock nests inside — installer-with-installer is already serialized by 3a)
→ stage validation comes free from install.sh → flip (install.sh) → re-run diagnosis
(§5: "diagnose once and trust it" is wrong — every commit boundary re-verifies) → rewire hooks
and skills (Task 3, with repair) → `migrate_indexes(args.defer)` (Task 4) → release locks →
`verdict(...)` → print every component as `name: state — detail (next action)` and exit.

The bin stays thin: it sequences and prints; every decision is a function in `upgrade.rs`.

`tests/install-smoke.sh`: the payload assertion needs no edit — `find` over the installed
generation cannot match a bin that is never copied — but ADD the assertion above so the
property is pinned rather than incidental.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade verdict\: && cargo build --locked` then
`bash tests/install-smoke.sh`.
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `verdict` map Drifted to `Ok` (only Unreadable counts). Expected: the first test RED.
2. Make `verdict` treat `Absent` as `Partial`. Expected: the gone-directory test RED — this is
   the rule that keeps scratch-index machines from permanent red, and its regression is silent.
3. Make an acked drift still count as Partial. Expected: the ack test RED.
4. Delete the smoke addition's `find` guard (pretend the payload always contains it). Expected:
   the smoke assertion cannot detect payload inclusion — i.e., confirm the guard is the thing
   being tested by breaking it once.

- [ ] **Step 6: Verify everything**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cargo build --release --locked --manifest-path rust/Cargo.toml && bash tests/install-smoke.sh
```
Expected: all exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/src/bin/cort_upgrade.rs rust/Cargo.toml rust/tests/upgrade.rs tests/install-smoke.sh
git commit -m "feat(upgrade): the cort-upgrade binary — verdict, taxonomy, escape hatches

Exit 0 all current, 1 partial-but-usable, 2 fatal (locks unobtainable or
payload staging failed — the only two 'cannot proceed safely' states, set by
the binary never derived from components). Acked drift becomes an info line.
Gone directories never fail; unreadable never passes. --check runs diagnosis
only. The steady state (everything current) takes no locks and runs no
install.sh — an upgrade that would do nothing must do nothing."
```

---

## Task Order Note (deliberate)

Tasks 1-4 build and test the library with NO binary, so every property is testable as a unit
before the CLI exists. Task 5 adds the bin as a thin sequencer. If Task 5's sequencing reveals a
missing library function, it gets added there and tested there — the bin never grows logic.

## Self-Review

**Spec coverage.** §2 diagnosis → Tasks 1, 3 (components 1-6, 8; component 9 hook-gate is
stateless cache and has no version to drift — it is diagnosed only as "present", which plan 1's
list_projects side already covers; recorded here so the gap is explicit, not silent). §3 locks →
Task 2. §4 migration → Task 4 (refusal/repayment already shipped in plan 2; this plan is the
actor). §5 order → Task 5's sequence, with steps 2-3 delegated to install.sh (stated in Scope).
§6 verdict → Task 5. §7's five fixture warnings → mapped: payload identity (Task 1 test 3),
atomic switch (3a shipped + Task 4's real rebuild), locks (Task 2's killed-holder), single
verdict (Task 5 + ack), refusal-visibility (already shipped, re-asserted via rebuild-re-read).

**Deliberately not here:** `ensure_schema`'s unconditional `graph_pending` (needs a migration
decision, orthogonal to the actor), the projects.extractor_version column drop (same reason,
plan 3b's self-review), pack walk's remaining silent-skip paths (fixed in the §10 round).

**Placeholders:** Task 3's `todo!()` bodies — the tests above them ARE the specification, and
the plan says what each body must do in prose; writing the bodies in the plan would just be a
second copy of the tests. Everything else is complete code.

**Type consistency:** `ComponentState::{Current, Drifted, Unreadable, Absent, DeferredByUser}`
used identically across all tasks; `LockError::{AdmissionBusy, DrainTimeout}`;
`UpgradeExit::{Ok, Partial, Fatal}` → 0/1/2; `check_shim(manifest, cort_home, new_tree)`;
`pack_identity(dir) -> io::Result<String>`; `acquire_upgrade_locks(cache, drain_timeout)`;
`migrate_indexes(defer) -> Vec<Component>`; `verdict(components, acks) -> Verdict`.
