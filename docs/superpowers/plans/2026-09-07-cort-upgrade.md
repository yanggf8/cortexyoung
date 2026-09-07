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
- The upgrader must not restate any inventory — it consumes `cort::install::*` and `cort::settings::HOOK_HARNESSES` directly (never through restated tables), and reaches install.sh only through the Task-0 modes.
- `unreadable` is never reported as `absent`, and never as a pass.
- No absolute developer paths anywhere, including fixtures.
- Every task's Step 5 deliberately breaks the implementation and confirms the new test goes red — and every break validation must assert the suite actually ran (exit code and result line), per the plan-3b execution record.

## The one scope decision to state up front

The spec's §5 lists eleven steps. Steps 2-3 (build and stage the new payload) are **`install.sh`'s existing job** — 3a already made them atomic, validated and generation-named. This plan does not re-implement a builder; `cort-upgrade` **invokes `install.sh` for the payload**, taking the locks *around* that invocation, exactly as §3(b) intended: the locks coordinate cort processes with the upgrade, and install.sh's own flock coordinates installer-with-installer. What this plan owns is everything install.sh was never meant to do: diagnose against the new tree's requirements, hold the cort-process locks, re-verify wiring and skills by content, migrate indexes, and produce the verdict.

---

## File Structure

- `rust/src/upgrade.rs` (NEW) — all logic: `Component`/`ComponentState` types, `diagnose()`, `UpgradeLocks` (two-flock acquire/drop), `migrate_indexes()`, `Verdict`/exit-code mapping. Checks never write (repair is separate); every check is a pure function over paths it is handed, plus injected subprocess/repair seams for hooks.
- `rust/src/bin/cort_upgrade.rs` (NEW) — thin: arg parsing (`--ack`, `--keep-mine`, `--defer`, `--check`), calls `upgrade::*`, prints the verdict, exits with the classified code.
- `rust/Cargo.toml` — one `[[bin]]` entry, following the `fake_ast_grep` comment style.
- `rust/src/main.rs` — ONE addition: the three entry points branch on `try_protected_entry` BEFORE any pack read or database open (admission → activity → drop admission), each with its own explicit quiet/blocking path. No shared "run either way" helper — a helper that runs the body on both branches cannot select the quiet path and was the critical defect of the first draft.
- `install.sh` — two modes for the upgrader plus existing-env deferral (Task 0). No other changes.
- `rust/tests/upgrade.rs` (NEW) — every property below, as end-to-end as the fixtures allow.
- `tests/install-smoke.sh` — payload assertion plus the Task 0/Task 5 additions named in those tasks.

---

### Task 0: install.sh learns stage-only, activate-only, and how to decline

**Files:**
- Modify: `install.sh`, `tests/install-smoke.sh`
- Test: `tests/install-smoke.sh` (new blocks)

**Interfaces:**
- Consumes: plan 3a's generation layout (generation dirs, symlink flip, validation rules).
- Produces: `install.sh --stage-only` (build + stage + validate, prints generation id, touches NOTHING live: no flip, no shim, no skills, no hooks, no manifest writes) and `install.sh --activate-only --gen <id>` (flip + shim + `cort_bin` manifest entry only, no skills, no hooks). Plus: `install.sh` with no mode flag on a machine that already has a manifest **declines** — prints "existing installation — run cort-upgrade" to stderr and exits 3 (distinct from the 0/1/2 verdict taxonomy, which belongs to cort-upgrade, not the installer).

**Why two modes instead of reusing the full install.** The upgrader must stage and validate BEFORE taking locks (spec §5 steps 2-3 precede step 4), hold locks across flip + rewire + migrate, and never trigger skill deployment (no `--keep-mine` exists in the installer, and adding one there would put upgrade policy in bash). A single `--payload-only` that flips would put activation before the locks; calling full `install.sh` would deploy skills before Task 3's policy runs. The split matches spec §5 exactly: stage (unlocked, invisible) → locks → re-verify → activate → rewire → migrate → release.

- [ ] **Step 1: Write the failing test**

In `tests/install-smoke.sh`, after the payload-generation block:

```bash
# --stage-only builds and validates without touching anything live. The proof is negative:
# the live symlink still points at the old generation afterwards, and no skill/hook/manifest
# row for the new generation exists.
```

Concretely: record `readlink $CORT_HOME` and the manifest's `cort_bin` before; run
`bash "$INSTALL_SH" --stage-only` capturing stdout as `$GEN`; assert exit 0; assert `$GEN`
matches `^cort-[0-9a-f]{12}$`; assert `readlink` unchanged; assert the manifest has no new
`cort_bin`; assert `$GEN/cort` is executable and `$GEN/pack/sgconfig.yml` exists. Then
`bash "$INSTALL_SH" --activate-only --gen "$GEN"`; assert the link now resolves to `$GEN`
and `cort --version` still answers through the shim. Then a second `--activate-only` with a
bogus id (`cort-000000000000`): must FAIL (nonzero) with "incomplete" on stderr, and the link
must be unchanged — activation validates before flipping, same rule as staging.

And the deferral test: with a manifest present, bare `bash "$INSTALL_SH"` (no mode) must exit
3 with "run cort-upgrade" on stderr and change NOTHING (manifest bytes identical before/after
— compare sha256, since "changed nothing" is the property, not "printed a line").

And the stage-takes-no-installer-lock test: hold `$MANIFEST_DIR/.install.lock` exclusive in
the background (`flock -x "$lock" sleep 30 &`), then run `--stage-only` — it must still exit
0 promptly (it builds into a fresh dir; serializing stagers against live installs was the
rejected design). Kill the holder afterwards. If stage ever takes the installer flock, this
hangs past the suite timeout instead of failing cleanly — run it with `timeout 120` so a
regression reads as an error, not a wedged CI leg.

- [ ] **Step 2: Run it to verify it fails**

Expected: FAIL — `install.sh: Unknown option` (the parser rejects both flags; deferral absent).

- [ ] **Step 3: Write minimal implementation**

`--stage-only` is the existing build+stage+validate code path with the flip, shim, manifest,
skill and hook steps skipped — factor by guarding, not by duplicating: the staging block stays
in one place, and a `STAGE_ONLY=1` early return (same shape as the existing `SOURCE_ONLY` seam)
exits after printing the generation id. `--stage-only` SKIPS the installer's own
`.install.lock` (install.sh:1115/1272): staging builds into a fresh dir and touches nothing
live, so serializing stagers would block fresh installs for a whole build for no safety gain
— two concurrent stagers just build two generations and the flip decides. `--activate-only`
KEEPS the installer flock (it mutates the live link + `cort_bin` entry; installer-vs-installer
must stay serialized). `--activate-only` runs ONLY flip (reusing `swap_symlink`
verbatim — not a second copy), shim write, and the single `cort_bin` manifest entry. Nesting
is deadlock-free in one direction only, documented in the comment above each mode: the
upgrader (holder of upgrade locks) may acquire the installer lock; install.sh never acquires
upgrade locks, so no cycle exists — a future reader must not "fix" this by adding upgrade-lock
handling inside install.sh, nor by making stage take the installer lock.

- [ ] **Step 4: Run it to verify it passes**

- [ ] **Step 5: Verify each test can actually fail**

1. Make `--stage-only` touch the live link (point it at a foreign target). Expected: the
   "link unchanged" assertion RED. (NOT "also flip to the staged gen": generations are
   content-addressed and the suite tree never changes mid-run, so a flip lands on the
   identical id and the link does not move — measured green against the flip break, which
   is the break observing nothing. The property is "stage touches nothing live", so the
   break must move the link somewhere observable.)
2. Make `--activate-only` skip the completeness check. Expected: the bogus-id flip succeeds
   (dangling link) and the NEXT assertion (`cort --version` answers through the shim) REDs —
   the pair proves the check is what stands between a typo and a dead install. If everything
   stays green, the test observes nothing; fix the fixture.
3. Remove the deferral branch. Expected: bare install on an installed machine proceeds (the
   sha256 comparison REDs because the manifest got rewritten).
4. Make `--stage-only` take the installer flock. Expected: the lock-held staging test times
   out (run with `timeout 120`; the timeout IS the red) — proving the test observes lock
   discipline, not just exit codes.

- [ ] **Step 6: Verify everything**

Bash syntax + full smoke + both Rust crates (unchanged code, but the suite is the gate).

- [ ] **Step 7: Commit**

```bash
git add install.sh tests/install-smoke.sh
git commit -m "feat(install): stage-only and activate-only modes for the upgrader

--stage-only builds, stages and validates a generation and prints its id
without touching anything live. --activate-only flips a validated generation
and writes the shim plus the single cort_bin manifest entry — no skills, no
hooks, so upgrade policy (keep-mine, hook shape) stays in the upgrader where
it belongs. Bare install.sh on an installed machine now declines with exit 3
and changes nothing, instead of reinstalling over a system it was never asked
to own."
```

---

### Task 1: diagnosis — every component, checked against the new tree

**Files:**
- Create: `rust/src/upgrade.rs` (diagnosis half only), `rust/tests/upgrade.rs`
- Modify: `rust/src/lib.rs` (one `pub mod upgrade;`), `rust/src/usage.rs` (one read-only version reader — the SQL stays where the table is defined)

**Interfaces:**
- Consumes: `cort::install::{MANIFEST_KEYS, MANIFEST_LEGACY_KEYS, render_shim, AST_GREP_PINNED}`, `cort::db::{list_projects, ProjectEntry}`, `cort::usage::{USAGE_SCHEMA_VERSION, usage_db_path}`, `cort::indexer::rebuild_reasons`, `cort::scan::SCAN_ENGINE`. (Deliberately NOT `pack::extractor_version` or `db::SCHEMA_VERSION`: `pack_identity` reimplements the construction over arbitrary dirs (that is its whole point), and the schema comparison lives inside `rebuild_reasons` — naming them here would promise a call that never happens.)
- Produces: `pub struct Component { pub name: String, pub state: ComponentState, pub detail: String }`, `pub enum ComponentState { Current, Drifted, Unreadable, Absent, DeferredByUser }`, `pub struct DiagnoseInputs<'a>` + `pub fn diagnose(inputs: &DiagnoseInputs) -> Vec<Component>`. `Unreadable` never maps to `Current`; `Absent` never fails. (Task 3 extends `DiagnoseInputs` with skill/hook fields and `diagnose` with its checks; the Task-1 version covers the Task-1 components plus per-project index reasons, read-only.)

The eight checked components (spec §2; `xg` explicitly out): payload identity (query the installed `cort` binary directly, through its manifest path — never the shim — for its extractor/schema pair; drift here is plan 1's `drifted`), shim content (`render_shim(cort_home)` vs the file at `manifest_get cort_bin`), ast-grep version (`ast_grep_provenance().version` vs the CLI on PATH), three skills (content-diff against `$new_tree/skills/...`, not stamp ownership), hook entries (shipped binary's `hook-install --all --status --lean`, compared against what this tree's `HOOK_TARGETS` would produce), manifest key-set (`internal-manifest-keys` output vs keys present in the live manifest), indexes (already served by `cort projects --verdict` on the installed binary), `usage.db` version.

- [ ] **Step 1: Write the failing test**

In `rust/tests/upgrade.rs`:

```rust
//! cort-upgrade properties. Every fixture here is end-to-end where the property demands it:
//! real directory layouts, real symlinks, real installed-shaped files. No toy renames.

use std::fs;
// NOTE: no `use std::path::Path` here — no Task-1 test names bare `Path`, and an unused
// import trips `-D warnings`. A later task needing it adds the import exactly once, here.

/// A minimal installed-shape root: manifest with cort_bin, a real shim file, a pack dir.
/// Returns ((root, home, bin) TempDirs alive, root path, home path, cort_home, bin path).
/// The TempDir triple MUST be returned whole: `home` and `bin` dirs back the paths every
/// test writes through, and dropping them deletes the fixture mid-test (first-draft defect
/// caught in review — it returned one TempDir and dropped the other two, so every path
/// dangled). Callers bind `let ((_r, _h, _b), _root, home, cort_home, bin_dir)`.
fn installed_root() -> ((tempfile::TempDir, tempfile::TempDir, tempfile::TempDir), std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
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
    let root_p = root.path().to_path_buf();
    let home_p = home.path().to_path_buf();
    let bin_p = bin.path().to_path_buf();
    ((root, home, bin), root_p, home_p, cort_home, bin_p)
}
```

The first real property — the one spec §7 names as the weak fixture for everyone else:

```rust
/// A lying shim must be caught by content, not existence. The shim is the component whose
/// "check" was pure theatre before 3b (the binary never answered for it).
#[test]
fn a_shim_that_differs_from_render_shim_is_drifted_not_current() {
    let ((_r, _h, _b), _root, home, cort_home, bin_dir) = installed_root();
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
    let state = cort::upgrade::check_shim(&manifest, &cort_home);
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
    let ((_r, _h, _b), _root, home, cort_home, _bin) = installed_root();
    let manifest = home.join("cortexyoung/manifest");
    fs::remove_file(&manifest).unwrap();
    fs::create_dir(&manifest).unwrap(); // reads now fail with EISDIR
    let state = cort::upgrade::check_shim(&manifest, &cort_home);
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
    // The engine mix is load-bearing, not garnish: recompute the bytes-only hash inline and
    // demand difference. If the impl drops the SCAN_ENGINE mix, an engine-only bump (ast-grep
    // crate move, rules untouched) compares equal and the drift goes unseen — this assertion
    // is the only test that observes the mix.
    use sha2::{Digest, Sha256};
    let mut raw = Sha256::new();
    raw.update(b"id: a\nlanguage: ts\n");
    assert_ne!(ha, format!("{:x}", raw.finalize()), "bytes-only hash must differ from the identity");
}

/// diagnose() composes the checks: the test above proves the HASH moves, this one proves the
/// VERDICT moves. A diagnose() that always reports Current passes every test above — without
/// this test the whole component suite is theatre (review catch). Fixture: installed_root
/// whose pack holds r.yml "id: b", new-pack dir holding r.yml "id: a", installed ast-grep
/// reporting the tree pin. Expect: the pack component Drifted with both identities named.
/// Then the mirror: identical packs + matching shim → pack Current (the shim stays Drifted
/// in this fixture — bin/cort is "#!/bin/sh", not a real shim — and the test asserts ONLY
/// the pack component, so it cannot pass by accident of the shim).
#[test]
fn diagnose_reports_a_drifted_pack_and_a_current_one() {
    use cort::upgrade::{ComponentState, DiagnoseInputs};
    let ((_r, _h, _b), _root, home, cort_home, _bin) = installed_root();
    // install_root is the dir holding `manifest` + `cort/` (the fixture nests both under
    // home/cortexyoung); _root itself is an unrelated scratch dir.
    let install_root = home.join("cortexyoung");
    let new_pack = tempfile::tempdir().unwrap();
    fs::write(new_pack.path().join("r.yml"), "id: a\nlanguage: ts\n").unwrap();
    fs::write(cort_home.join("pack/r.yml"), "id: b\nlanguage: ts\n").unwrap();
    let pin = cort::install::AST_GREP_PINNED;
    let comps = cort::upgrade::diagnose(&DiagnoseInputs {
        install_root: &install_root,
        new_pack: new_pack.path(),
        installed_ast_grep_version: pin,
    });
    let pack = comps.iter().find(|c| c.name == "pack").unwrap();
    assert!(matches!(pack.state, ComponentState::Drifted), "{pack:?}");
    // Mirror: same byte both sides → Current.
    fs::write(cort_home.join("pack/r.yml"), "id: a\nlanguage: ts\n").unwrap();
    let comps = cort::upgrade::diagnose(&DiagnoseInputs {
        install_root: &install_root,
        new_pack: new_pack.path(),
        installed_ast_grep_version: pin,
    });
    let pack = comps.iter().find(|c| c.name == "pack").unwrap();
    assert!(matches!(pack.state, ComponentState::Current), "{pack:?}");
}

/// Every remaining component check returns a fully-formed `Component` (name included), not a
/// bare state — Task 5's verdict must never invent names for components it did not examine.
/// All three are pure comparisons: installed value(s) in, `Component` out. No subprocess, no
/// filesystem beyond what the caller hands in.
#[test]
fn version_pin_comparison_is_equality_not_presence() {
    // ast-grep: the installed CLI reports "0.45.2", the tree pins "0.45.2" → Current.
    let c = cort::upgrade::check_version_pin("0.45.2", "0.45.2");
    assert_eq!(c.name, "ast_grep");
    assert!(matches!(c.state, cort::upgrade::ComponentState::Current));
    // Installed CLI reports "0.44.0" → Drifted, and the detail names both sides: a bare
    // boolean is unactionable ("update what, to what?").
    let c = cort::upgrade::check_version_pin("0.44.0", "0.45.2");
    assert!(matches!(c.state, cort::upgrade::ComponentState::Drifted), "{c:?}");
    assert!(c.detail.contains("0.44.0") && c.detail.contains("0.45.2"), "{c:?}");
    // Unparseable installed output (empty string, "ast-grep bogus") → Unreadable, never
    // Current and never silently Drifted: we could not read it, so we claim nothing about it.
    let c = cort::upgrade::check_version_pin("", "0.45.2");
    assert!(matches!(c.state, cort::upgrade::ComponentState::Unreadable), "{c:?}");
}

/// Manifest keys: the live manifest's key set diffed against the tree's authority
/// (`MANIFEST_KEYS` + `MANIFEST_LEGACY_KEYS`). Unknown keys are Drifted-with-detail, never
/// failures on their own — Task 5 decides what they do to the exit code.
#[test]
fn unknown_manifest_keys_are_drifted_with_names_not_silent() {
    let live = ["manifest_version", "cort_bin", "mystery_key_from_the_future"];
    let c = cort::upgrade::check_manifest_keys(&live);
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Drifted),
        "{c:?}"
    );
    // And the detail must NAME the key: the whole point is telling the user what to look at.
    // A verdict naming "manifest" without the key sends the user grepping.
    assert!(
        c.detail.contains("mystery_key_from_the_future"),
        "detail must name the unknown key: {c:?}"
    );
}

/// usage.db schema: compare the stored `USAGE_SCHEMA_VERSION` against the tree constant.
/// Mismatch → Drifted (the recorder refuses to open it, so data collection is already degraded).
/// Unreadable file → Unreadable.
#[test]
fn usage_schema_mismatch_is_drifted() {
    let c = cort::upgrade::check_usage_schema(Some("1"), "1");
    assert_eq!(c.name, "usage_db");
    assert!(matches!(c.state, cort::upgrade::ComponentState::Current));
    let c = cort::upgrade::check_usage_schema(Some("0"), "1");
    assert!(matches!(c.state, cort::upgrade::ComponentState::Drifted), "{c:?}");
    let c = cort::upgrade::check_usage_schema(None, "1");
    assert!(matches!(c.state, cort::upgrade::ComponentState::Unreadable), "{c:?}");
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
    // Owned: per-project index components carry the project path, which is never 'static.
    // Fixed-name components use `"shim".to_string()` at construction — a `&'static str` field
    // compiles every literal but makes `migrate_indexes` unimplementable (first-draft defect:
    // the plan prescribed `name: &row.path` in four places, none of which compiles).
    pub name: String,
    pub state: ComponentState,
    pub detail: String,
}

/// Pack identity for a directory: same construction as `pack::extractor_version` but over an
/// arbitrary dir, so tests can build two packs that differ by one byte. Sorted file list, hashed
/// contents, MIXED WITH the scan engine identity — exactly like `extractor_version` does
/// (`pack.rs`: same bytes through a different engine are a different extractor, and the
/// first draft hashed bytes alone, so an engine-only bump — ast-grep crate move, rules
/// untouched — compared equal and the drift went unseen; review catch). Reuse
/// `crate::scan::SCAN_ENGINE` verbatim; restating the string here would be a second home.
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
        return Component { name, state: ComponentState::Unreadable, detail: "no version output to read".into() };
    }
    if installed == pinned {
        Component { name, state: ComponentState::Current, detail: String::new() }
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
        Component { name: "manifest_keys".to_string(), state: ComponentState::Current, detail: String::new() }
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
        None => Component { name: NAME.to_string(), state: ComponentState::Unreadable, detail: "usage.db unreadable or version key absent".into() },
        Some(v) if v == current => Component { name: NAME.to_string(), state: ComponentState::Current, detail: String::new() },
        Some(v) => Component {
            name: NAME.to_string(),
            state: ComponentState::Drifted,
            detail: format!("usage.db schema {v}, tree expects {current}"),
        },
    }
}

/// Read-only reasons for one project db: open WITHOUT migrating (no `ensure_schema` — the
/// upgrader must not migrate schema as a side effect of looking) and run plan 2's shared
/// reader. `None` = could not read = Unreadable downstream, never empty-debt. Defined HERE
/// because diagnosis owns it; Task 4's migration only calls it.
pub fn read_reasons_readonly(db_path: &str) -> Option<Vec<String>> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    // Spec §10 item 1 lands here, not as its own task: the scan connection used to be opened
    // with no busy timeout, so one transient SQLITE_BUSY under a concurrent refresh-hook write
    // reported a healthy index Unreadable. Same 5s the rest of db.rs uses; contention now
    // waits instead of lying. (No dedicated timing test — timing tests flake; the direction
    // is wait-instead-of-Unreadable and the existing Unreadable tests still pin the true
    // unreadable path.)
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
    out.push(match (
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
    });
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
            let live: Vec<&str> =
                contents.lines().filter_map(|l| l.split(':').next()).collect();
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
```

In `rust/src/usage.rs`, one small reader — the SQL stays where the table is defined, so
`upgrade.rs` never restates usage's layout:

```rust
/// Stored usage-schema version, read-only. `None` = file missing, unreadable, or key absent;
/// the CALLER maps missing-file to Absent and the rest to Unreadable (that distinction lives
/// in diagnosis, not here).
pub fn read_schema_version(path: &Path) -> Option<String> {
    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT value FROM _usage_meta WHERE key = ?1",
        params![VERSION_KEY],
        |r| r.get(0),
    )
    .ok()?
}
```
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
4. Make `check_version_pin` return `Current` on empty input. Expected: the version test RED —
   an unreadable parser output must never read as a passing pin.
5. Make `check_manifest_keys` drop unknown keys silently (return Current always). Expected: the
   manifest test RED — and specifically on the detail assertion, which is the only line that
   observes the names rather than the verdict.
6. Make `diagnose` return all-`Current` regardless of input. Expected: the e2e verdict test
   (`diagnose_reports_a_drifted_pack_and_a_current_one`) RED — this is the break that proves
   the composition observes its inputs. Without it every unit test above can pass around a
   diagnose that claims everything is fine.
7. Make `pack_identity` drop the `SCAN_ENGINE` mix (bytes only). Expected: the pack-byte test
   RED at the engine-mix assertion while the ha!=hb assertion stays GREEN — proving the two
   assertions observe different properties. An engine-only bump with this break in place
   compares equal and drifts unseen.

- [ ] **Step 6: Verify everything**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/src/lib.rs rust/src/usage.rs rust/tests/upgrade.rs
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
- Produces: `pub struct UpgradeLocks` (Debug); `pub fn acquire_upgrade_locks(cache_dir: &Path, drain_timeout: Duration) -> Result<UpgradeLocks, LockError>`; `pub enum LockError { AdmissionBusy, DrainTimeout, Unsupported }`; and `pub fn try_protected_entry(cache_dir: &Path) -> Result<ActivityGuard, LockError>` for the three cort entry points (admission shared → activity shared → drop admission → guard held across the work). The caller branches on the Result — there is NO run-either-way helper (see Step 3).

**Paths.** Both lock files live in the stable cache dir (`cort::db::cache_dir()`), NOT the payload generation dir — the payload is what gets replaced; the locks must outlive any replacement. `.upgrade-admission.lock` and `.upgrade-activity.lock`.

**Mechanism** (spec §3b, verbatim): protected operation = flock(admission, LOCK_SH) → flock(activity, LOCK_SH) → funlock(admission) → work → funlock(activity). Upgrader = flock(admission, LOCK_EX) → flock(activity, LOCK_EX) **with deadline** → hold both through flip+migrate. Nonblocking first, then a bounded retry loop for the drain (50ms steps, overshoot bounded by one interval; `drain_timeout` default 30s). On drain timeout: abort the upgrade — never kill the foreground work (spec §3c).

**First-upgrade detection (spec §3b option A, implemented — not just labeled).** After a
successful upgrade the binary writes a `.upgraded_once` marker beside the lock files. When
`acquire_upgrade_locks` succeeds and the marker is ABSENT, this is the first upgrade: the
drain still ran bounded (old binaries holding activity get waited on, then abort on timeout),
but even success proves nothing about pre-lock binaries — so Task 5's verdict appends a
`partial_drain_first_upgrade` info component naming the WAL-reader risk, and writes the
marker on release. Marker present → normal verdict. Test (Task 5): marker absent → info
present; marker present → absent. The spec's option B (migrate a copied generation) stays
unbuilt deliberately: option A is the bounded-abort-plus-named-risk the spec accepts, and a
copy-migrate doubles disk and still cannot exclude a WAL reader.

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
        // Worker: admission(sh) -> activity(sh) -> release admission, hold activity 2.5s.
        // The closure must own its path (spawned closures are 'static) — borrow `cache`
        // directly and this fails to compile. The guard is held across the sleep: dropping
        // it early would release activity and the drain would succeed, passing for nothing.
        let worker_cache = cache.path().to_path_buf();
        let worker = std::thread::spawn(move || {
            let _guard = cort::upgrade::try_protected_entry(&worker_cache).unwrap();
            std::thread::sleep(Duration::from_millis(2500));
            "worked"
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

    /// A worker arriving MID-DRAIN stands down at admission instead of joining activity.
    /// Without the admission acquisition in `try_protected_entry`, the newcomer takes activity
    /// shared and every arrival extends the drain — the upgrade still aborts safely, but
    /// liveness dies under load, and no other test observes the admission half of the worker
    /// protocol at all. Fixture: worker A holds activity (2s); the upgrader starts a drain
    /// (3s deadline) in a second thread; worker B arrives 200ms later and must get
    /// AdmissionBusy, while the drain still completes once A exits.
    #[test]
    fn a_worker_arriving_mid_drain_stands_down_at_admission() {
        let cache = tempfile::tempdir().unwrap();
        let a_cache = cache.path().to_path_buf();
        let worker_a = std::thread::spawn(move || {
            let _guard = cort::upgrade::try_protected_entry(&a_cache).unwrap();
            std::thread::sleep(Duration::from_millis(2000));
        });
        std::thread::sleep(Duration::from_millis(200)); // A holds activity
        let d_cache = cache.path().to_path_buf();
        let drain = std::thread::spawn(move || {
            cort::upgrade::acquire_upgrade_locks(&d_cache, Duration::from_secs(10))
        });
        std::thread::sleep(Duration::from_millis(200)); // upgrader holds admission exclusive
        let b_result = cort::upgrade::try_protected_entry(cache.path());
        assert!(
            matches!(b_result, Err(cort::upgrade::LockError::AdmissionBusy)),
            "a mid-drain arrival must stand down at the gate, not join activity: {b_result:?}"
        );
        let locks = drain.join().unwrap().expect("drain completes once A exits");
        drop(locks);
        worker_a.join().unwrap();
    }

    /// Crash safety is the OS's job: if the holder's pid dies, the kernel drops the flock and
    /// the next acquirer gets in. Fixture: fork-style — spawn a child that takes the locks and
    /// SIGKILLs itself; the parent must acquire within the timeout rather than seeing a stale
    /// lock. (This is the test spec §7 names: not "expired lease + dead pid", which any
    /// TTL-less impl passes — but an actual process death.)
    #[test]
    fn a_killed_holder_releases_the_locks() {
        if std::env::args().any(|a| a == "lock_holder_child") {
            return; // child re-entry: the child fn below does the holding
        }
        let cache = tempfile::tempdir().unwrap();
        let cache_path = cache.path().to_path_buf();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("lock_holder_child")
            .env("UPGRADE_TEST_CACHE", &cache_path)
            .spawn()
            .expect("spawn holder child");
        // Readiness handshake, bounded: poll for the sentinel the child writes AFTER acquiring.
        // Killing before it holds anything would pass against code that never locks.
        let sentinel = cache_path.join(".holder-ready");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !sentinel.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "holder child never acquired the locks"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let got =
            cort::upgrade::acquire_upgrade_locks(&cache_path, Duration::from_secs(2));
        assert!(got.is_ok(), "kernel must release flocks on process death: {got:?}");
    }
}
```

The child dispatch: `std::env::current_exe()` under `cargo test` is the test binary, so
`lock_holder_child` as argv[1] re-enters it. The test harness runs every `#[test]` fn in the
child too — including the spawner itself — so BOTH the spawner and the child fn start with the
same guard, branching on argv. Add at the bottom of `rust/tests/upgrade.rs`:

```rust
/// Child side of the killed-holder test. Runs ONLY when argv contains "lock_holder_child";
/// otherwise returns immediately (it is still enumerated as a test in every run, real or child).
/// NOTE: this fn lives at the TOP level of the test crate, outside `mod locks` — `Duration`
/// is NOT imported here, so every duration uses the full `std::time::Duration` path. Writing
/// a bare `Duration` compiles inside `mod locks` and fails here (review catch).
#[test]
fn lock_holder_child() {
    if !std::env::args().any(|a| a == "lock_holder_child") {
        return; // real test run: no-op
    }
    let cache = std::env::var("UPGRADE_TEST_CACHE").unwrap();
    let _locks = cort::upgrade::acquire_upgrade_locks(
        std::path::Path::new(&cache), std::time::Duration::from_secs(60)).unwrap();
    // Signal AFTER acquiring: the parent must not kill us before we hold anything, or the
    // test proves nothing (a kill before acquisition passes against code that never locks).
    std::fs::write(std::path::Path::new(&cache).join(".holder-ready"), b"held").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60)); // parent will kill us
}
```

And the spawner (already written with its guard inline above) sets the env, waits for the
sentinel, kills, waits, and only then asserts acquisition succeeds. A cleaner bin-based child
would need the upgrade bin to grow a test-only mode, which is production surface for a fixture.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade locks::`
Expected: FAIL to compile — `no function named 'acquire_upgrade_locks'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`. The FFI follows the `send_sigterm` precedent (`ast_grep.rs:226-238`) —
extern flock directly, no new crate. Imports: Task 1 already has `use std::fs;` and
`use std::path::Path;` — do NOT repeat them (a second `use std::path::Path` is a duplicate
import); add only what Task 2 needs:

```rust
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
// shapes. (First-draft defect, caught in review.)
// No EINTR retry either, deliberately: a spurious EINTR collapses into contention, and every
// contention path here is the SAFE direction (a worker stands down, the upgrader aborts). A
// retry would need errno plumbing for a case that resolves to the same branch.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    AdmissionBusy,
    DrainTimeout,
    /// Non-unix platform: flock(2) does not exist. Workers run unguarded (today's behavior);
    /// the upgrader refuses (Fatal) rather than migrating unprotected. CI builds linux+macos
    /// only, so this arm is documentary — but an ungated `std::os::unix` import breaks the
    /// build for everyone else, which is how the first draft shipped.
    Unsupported,
}

/// Open (creating) a lock file. Returns Err instead of panicking: `hook-refresh` promises
/// silence and exit 0 on every edit, and an unwritable cache dir must stand down, not panic
/// (first-draft `.expect()` broke that promise).
#[cfg(unix)]
fn open_lock(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).write(true).open(path)
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
    let adm = open_lock(&cache.join(".upgrade-admission.lock")).map_err(|_| LockError::AdmissionBusy)?;
    if !flock_fd(&adm, LOCK_SH | LOCK_NB) {
        return Err(LockError::AdmissionBusy);
    }
    let act = open_lock(&cache.join(".upgrade-activity.lock")).map_err(|_| LockError::AdmissionBusy)?;
    if !flock_fd(&act, LOCK_SH | LOCK_NB) {
        drop(adm);
        return Err(LockError::AdmissionBusy);
    }
    drop(adm); // release admission; activity carries the protection
    Ok(ActivityGuard { _file: Some(act) })
}

#[cfg(not(unix))]
pub fn try_protected_entry(_cache: &Path) -> Result<ActivityGuard, LockError> {
    // No flock(2) here: exclusion is unavailable, so the guard is a no-op and workers run
    // exactly as today. The UPGRADER still refuses (see acquire_upgrade_locks) rather than
    // migrating unprotected — the unsafe direction stays closed on every platform.
    Ok(ActivityGuard { _file: None })
}

The upgrader side:

```rust
#[derive(Debug)]
pub struct UpgradeLocks {
    _admission: File,
    _activity: File,
}
// Debug is load-bearing, not vanity: the killed-holder test asserts
// `acquire_upgrade_locks(...).is_ok()` with `{got:?}` on failure, and without this derive
// that assertion does not compile (review catch).

#[cfg(unix)]
pub fn acquire_upgrade_locks(cache: &Path, drain_timeout: Duration) -> Result<UpgradeLocks, LockError> {
    let adm = open_lock(&cache.join(".upgrade-admission.lock")).map_err(|_| LockError::AdmissionBusy)?;
    if !flock_fd(&adm, LOCK_EX | LOCK_NB) {
        return Err(LockError::AdmissionBusy);
    }
    // Drain: wait for exclusive activity with a deadline. Every worker holding it shared
    // must let go before this succeeds.
    let act = open_lock(&cache.join(".upgrade-activity.lock")).map_err(|_| LockError::AdmissionBusy)?;
    let deadline = Instant::now() + drain_timeout;
    loop {
        if flock_fd(&act, LOCK_EX | LOCK_NB) {
            return Ok(UpgradeLocks { _admission: adm, _activity: act });
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
pub fn acquire_upgrade_locks(_cache: &Path, _drain_timeout: Duration) -> Result<UpgradeLocks, LockError> {
    Err(LockError::Unsupported)
}
```

(Absolute `$` paths: `open_lock` creates the file if missing — that is fine, both lock files
are in the stable cache dir which every cort process already requires.)

In `rust/src/main.rs`, wire the three entry points (this is the ONE main.rs change this task
makes; Task 4 does nothing here). The branch happens at the TOP of each entry function,
BEFORE `pin_bin` and before any database open: the review caught the first draft wrapping
the `incremental_index` call (main.rs:911), by which point the pack was already read and the
db already opened through a tree the upgrader may be mid-migration on. `cache_dir()` needs
only env/HOME — no db — so entry costs two opens, nothing else.

- `cmd_hook_refresh` (main.rs:815): first line `let _guard = match
  upgrade::try_protected_entry(&cort::db::cache_dir()) { Ok(g) => g, Err(_) => return quiet
  db_unavailable-equivalent }`. On admission-busy it runs the EXISTING quiet path — the
  contract already says give up rather than wait, and an upgrade in flight is exactly
  "rather than wait". `Unsupported` (non-unix) returns Ok(no-op guard): workers run as today.
- `cmd_hook_suggest` (main.rs:964): same branch, but ONLY on the evidence-query path (post
  shape-gate; spec §3b's cost note — the gate-rejected majority must not pay two opens).
  Same stand-down semantics: silent, no usage row beyond what silence already records.
- `cmd_index` (main.rs:1653): unlike the hooks, a foreground index WAITS — spec §3(c) ranks
  killing user foreground work worse than deferring ("前景動作等鎖,並告知原因"). Bounded
  retry of `try_protected_entry` with the same 30s deadline as the upgrader's drain; on
  success run normally. On timeout DO NOT proceed unlocked — the first draft did
  ("proceed WITHOUT the lock with upgrade_in_flight flagged"), which invalidates the
  exclusion precisely when migration is slowest, and spec §3(c) orders the opposite outcome:
  the timeout aborts the UPGRADE, not the foreground work. So cmd_index on timeout returns
  a structured error naming `upgrade_in_flight` with a retry-later next action, and the
  upgrader's drain timeout (Task 5 maps DrainTimeout → Fatal/exit 2) is what yields. The
  two deadlines race; whoever times out first is the one that stands down, and both
  directions are safe.

No signature change to `incremental_index`: the 21 existing call sites (17 in
`rust/tests/incremental.rs`, one each in `context.rs:987` and `hook.rs:539`, two in
`main.rs:911,1661` — counted, not estimated) are untouched. Only the two main.rs bodies get
the entry branch around them.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade locks::`
Expected: all four PASS. The drain test takes ~3s (2.5s hold), the mid-drain test ~2s,
the killed-holder ~1s (sentinel handshake + kill + acquire); if the killed-holder takes
the full 60s child sleep, the child did not die — check the kill. (Four, not three: the
mid-drain arrival test below is part of this task's gate.)

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
4. In `try_protected_entry`, drop the ADMISSION lock acquisition (activity only). Expected:
   `a_worker_arriving_mid_drain_stands_down_at_admission` RED — the newcomer joins activity
   instead of standing down. Breaks 1-3 all mutate the UPGRADER side or the activity half;
   without this break nothing observes the worker-side admission acquisition, and deleting
   it passes the whole suite (review catch — the first draft's breaks were not independent
   in exactly this direction).

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
supervisor. hook-refresh and hook-suggest's evidence path branch at fn top
before any pack read or db open; the shape-gated majority pays nothing.
cmd_index waits boundedly, then refuses with upgrade_in_flight rather than
indexing through a migration — the timeout aborts the upgrade, never the
user's foreground work (spec §3c).

First-upgrade honesty: binaries old enough to predate these locks hold
nothing we can wait on. The verdict in Task 5 labels that case
partial-drain; claiming a drain from lock state alone would be false."
```

---

### Task 3: hook wiring and skills — re-verified by content, restored by the shipped verbs

**Files:**
- Modify: `rust/src/upgrade.rs`, `rust/tests/upgrade.rs`, `rust/src/settings.rs` (+`HOOK_HARNESSES`, +`entry_shape_ok`), `rust/src/settings_toml.rs` (+`entry_shape_ok`), `rust/src/settings_kimi.rs` (+`entry_shape_ok`), `rust/src/main.rs` (build `HOOK_TARGETS` from the const — one line)
- Test: `rust/tests/upgrade.rs` (hook/skill properties), the dialect test files (shape-fn properties, where real-shape fixtures live)

**Interfaces:**
- Consumes: Task 1's `Component`/`ComponentState`; status TSV through an injected runner (never a subprocess in tests); `cort::settings::{HOOK_HARNESSES, EVENTS, HookEvent}`; the shipped binary's `hook-install --all` for repair (invoked, not restated).
- Produces: `pub fn check_hooks(shim, run_status, repair) -> Component` (= `judge_hooks` + repair-once + re-judge), `pub fn judge_hooks(shim, status_tsv) -> Component` (pure, reused by `--check`), `pub fn check_skills_at(...) -> Vec<Component>` + `pub fn repair_skill(source, dest) -> std::io::Result<()>`, per-dialect `pub fn entry_shape_ok(...) -> bool`, and the additive `DiagnoseInputs::{new_tree, home, keep_mine}` extension + `diagnose()` appending skill components. Task 5 extends Task 1's `diagnose` with these; the Task-1 version is untouched by this task.

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
    use sha2::{Digest, Sha256}; // fn-level import: sha2 is a main dependency, usable from
    // integration tests, but the crate root keeps one `use std::fs;` — a second site importing
    // sha2 at top level would be the duplicate-import defect Task 2's Step 3 warns about.
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

    // Call `_at` with explicit dests, NOT the env-reading wrapper: `set_var` inside parallel
    // tests is unsound, and a dev machine with CLAUDE_SKILL_HOME set would redirect the
    // wrapper elsewhere. The wrapper's 6 lines of env mirroring are review-verified, not
    // test-pinned — stated here so nobody "covers" it with an env-mutating test.
    let comps = cort::upgrade::check_skills_at(
        new_tree.path(),
        &home.path().join(".claude/skills/xgrep/SKILL.md"),
        &dest,
        &home.path().join(".codex/skills/ast-grep/SKILL.md"),
        false,
    );
    let ast = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(matches!(ast.state, cort::upgrade::ComponentState::Drifted), "{ast:?}");
}

/// --keep-mine turns the same divergence into DeferredByUser, and the file is untouched —
/// "repair" must never overwrite a user's edit silently.
#[test]
fn keep_mine_leaves_a_diverged_skill_alone() {
    use sha2::{Digest, Sha256};
    let new_tree = tempfile::tempdir().unwrap();
    let skill_dir = new_tree.path().join("skills/ast-grep");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "---\nname: ast-grep\n---\nnew body\n").unwrap();
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join(".claude/skills/ast-grep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let old_body = "---\nname: ast-grep\n---\nold body\n";
    fs::write(&dest, old_body).unwrap();
    // Managed (valid stamp for the old body) AND user-diverged: keep-mine must NOT redeploy.
    let stamp_line = format!(
        "managed by cortexyoung install.sh\nskill_sha256:{:x}\n",
        Sha256::digest(old_body.as_bytes())
    );
    fs::write(dest.parent().unwrap().join(".cortexyoung-managed"), stamp_line).unwrap();
    let comps = cort::upgrade::check_skills_at(
        new_tree.path(),
        &home.path().join(".claude/skills/xgrep/SKILL.md"),
        &dest,
        &home.path().join(".codex/skills/ast-grep/SKILL.md"),
        true,
    );
    let ast = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(matches!(ast.state, cort::upgrade::ComponentState::DeferredByUser), "{ast:?}");
    assert_eq!(fs::read_to_string(&dest).unwrap(), old_body, "keep-mine must not touch bytes");
}

/// keep-mine=false does NOT repair inside the check — checks never write (the `--check`
/// mode runs diagnosis only). Repair is a separate `repair_skill` step; this test pins the
/// sequence Task 5 runs: check (Drifted) → repair → re-check (Current, bytes == new tree).
/// A repair that writes bytes but no stamp — or reports Current without writing — fails the
/// re-check, which is the assertion that matters.
#[test]
fn repair_redeploys_skill_then_recheck_says_current() {
    let new_tree = tempfile::tempdir().unwrap();
    let skill_dir = new_tree.path().join("skills/ast-grep");
    fs::create_dir_all(&skill_dir).unwrap();
    let new_body = "---\nname: ast-grep\n---\nnew body\n";
    let source = skill_dir.join("SKILL.md");
    fs::write(&source, new_body).unwrap();
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join(".claude/skills/ast-grep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "---\nname: ast-grep\n---\nold body\n").unwrap();
    let at = |keep_mine: bool| {
        cort::upgrade::check_skills_at(
            new_tree.path(),
            &home.path().join(".claude/skills/xgrep/SKILL.md"),
            &dest,
            &home.path().join(".codex/skills/ast-grep/SKILL.md"),
            keep_mine,
        )
    };
    let ast = at(false).into_iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(matches!(ast.state, cort::upgrade::ComponentState::Drifted), "{ast:?}");
    cort::upgrade::repair_skill(&source, &dest).unwrap();
    assert_eq!(fs::read_to_string(&dest).unwrap(), new_body);
    let ast = at(false).into_iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(matches!(ast.state, cort::upgrade::ComponentState::Current), "{ast:?}");
}

/// Hook repair is re-verified after repair, not assumed. `run_status` is injected (same seam
/// style as the `repair` callback): the tests never spawn subprocesses, so no shell-script
/// fixtures in a pure-Rust repo — Task 5 passes the real `<bin> hook-install --all --status
/// --lean` runner with its deadline; the tests pass fakes.
#[test]
fn hook_repair_is_followed_by_reverification() {
    use std::cell::Cell;
    // Status TSV rows: harness, event(pre/post), outcome, settings, detail, command.
    let good = "claude-code\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness claude-code\n\
                claude-code\tpost\twired\t/s\ttrusted=true\t/shim hook-refresh --harness claude-code\n\
                codex\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness codex\n\
                codex\tpost\twired\t/s\ttrusted=true\t/shim hook-refresh --harness codex\n\
                kimi-code\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness kimi-code\n\
                kimi-code\tpost\twired\t/s\ttrusted=true\t/shim hook-refresh --harness kimi-code\n";
    let bad = good.replace("/shim hook-suggest", "/foreign hook-suggest");
    let calls = Cell::new(0);
    let repaired = Cell::new(false);
    let run_status = || {
        calls.set(calls.get() + 1);
        Ok(if repaired.get() { good.to_string() } else { bad.clone() })
    };
    let repair = || repaired.set(true);
    let c = cort::upgrade::check_hooks(
        std::path::Path::new("/shim"), &run_status, &repair,
    );
    // Drifted first, repair invoked exactly once, re-check ran (2 status calls), Current only
    // because the SECOND status agreed.
    assert!(matches!(c.state, cort::upgrade::ComponentState::Current), "{c:?}");
    assert_eq!(calls.get(), 2);
    assert!(repaired.get());
}

/// A repair that does not take must stay Drifted — returning Current after invoking repair
/// WITHOUT re-running status would pass the test above while proving nothing.
#[test]
fn a_noop_hook_repair_stays_drifted() {
    let bad = "claude-code\tpre\twired\t/s\ttrusted=true\t/foreign hook-suggest --harness claude-code\n";
    let c = cort::upgrade::check_hooks(
        std::path::Path::new("/shim"),
        &|| Ok(bad.to_string()),
        &|| {}, // repair that repairs nothing
    );
    assert!(matches!(c.state, cort::upgrade::ComponentState::Drifted), "{c:?}");
    assert!(c.detail.contains("claude-code"), "detail names the offending row: {c:?}");
}

/// Clean status never calls repair: a check that "repairs" unconditionally would redeploy on
/// every upgrade run, churning backups and trust stamps for nothing. (The missing-row case —
/// five rows present and matching, one target absent — is Drifted WITH repair called: the fix
/// for an absent row is installing it. Both directions pinned, neither collapsible.)
#[test]
fn clean_hook_status_never_calls_repair() {
    use std::cell::Cell;
    let good = "claude-code\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness claude-code\n\
                claude-code\tpost\twired\t/s\ttrusted=true\t/shim hook-refresh --harness claude-code\n\
                codex\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness codex\n\
                codex\tpost\twired\t/s\ttrusted=true\t/shim hook-refresh --harness codex\n\
                kimi-code\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness kimi-code\n\
                kimi-code\tpost\twired\t/s\ttrusted=true\t/shim hook-refresh --harness kimi-code\n";
    let called = Cell::new(false);
    let c = cort::upgrade::check_hooks(
        std::path::Path::new("/shim"),
        &|| Ok(good.to_string()),
        &|| called.set(true),
    );
    assert!(matches!(c.state, cort::upgrade::ComponentState::Current), "{c:?}");
    assert!(!called.get(), "matching status must not trigger a repair");
}

/// A missing row is Drifted, not silently ok: judging only returned rows would pass a settings
/// file that lost an event. (Rows are complete BY CONSTRUCTION today — hook_install_all always
/// emits all six — so this test pins the set comparison, not a case anyone has seen.)
#[test]
fn a_missing_hook_row_is_drifted() {
    let five = "claude-code\tpre\twired\t/s\ttrusted=true\t/shim hook-suggest --harness claude-code\n";
    let c = cort::upgrade::check_hooks(
        std::path::Path::new("/shim"),
        &|| Ok(five.to_string()),
        &|| {},
    );
    assert!(matches!(c.state, cort::upgrade::ComponentState::Drifted), "{c:?}");
    assert!(c.detail.contains("kimi-code") || c.detail.contains("missing"), "{c:?}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade -- skill hook`
Expected: FAIL to compile — `no function named 'check_skills_at'`. (One filter position only:
`cargo test --test upgrade A B` is a cargo arg error, not a test run — every multi-word
filter in this plan passes after `--`, and a zero-test "pass" is the false-green the Task-5
filter caught.)

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`. The stamp check must use the same hashing install.sh writes
(`skill_hash`: sha256 of the file, stamped as `skill_sha256:<hex>` beside it — read
`install.sh:189-207` for the exact contract and reuse the crate's `sha2`):

```rust
pub fn check_skills(new_tree: &Path, home: &Path) -> Vec<Component> {
    check_skills_with_policy(new_tree, home, /* keep_mine */ false)
}

pub fn check_skills_with_policy(new_tree: &Path, home: &Path, keep_mine: bool) -> Vec<Component> {
    // Env resolution mirrors install.sh EXACTLY (review catch — the first draft hardcoded
    // `home.join(".claude/...")` for all three dests, ignoring CLAUDE_SKILL_HOME that
    // install.sh:25 honors for the ast-grep skill):
    // - xgrep skill: `$HOME/.claude/skills/xgrep/SKILL.md`, NO env override (install.sh:22
    //   hardcodes it — do not invent one here);
    // - ast-grep skill: `${CLAUDE_SKILL_HOME:-$HOME/.claude}/skills/ast-grep/SKILL.md`;
    // - codex skill: `${CODEX_HOME:-$HOME/.codex}/skills/ast-grep/SKILL.md`.
    // In production `home` IS $HOME so the defaults coincide; in tests `home` is a tempdir.
    let claude_home = std::env::var_os("CLAUDE_SKILL_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    check_skills_at(
        new_tree,
        &home.join(".claude/skills/xgrep/SKILL.md"),
        &claude_home.join("skills/ast-grep/SKILL.md"),
        &codex_home.join("skills/ast-grep/SKILL.md"),
        keep_mine,
    )
}

/// The testable core: all paths explicit, ZERO env dependence. `pub` because tests call it
/// (the wrapper above is the only other caller).
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
    // For each: absent source -> Absent (not a failure). Absent dest + present source ->
    // Drifted (missing). Present both: content equal -> Current (a stale stamp beside
    // matching bytes is cosmetic — 3a made stamp writes atomic, this plan does not chase
    // cosmetics). Content differ ->
    //   keep_mine ? DeferredByUser (bytes untouched, always)
    //   : Drifted with detail naming the divergence and whether the dest is managed
    //     ("repairable" iff our stamp is present; an unmanaged divergent dest is NEVER
    //     auto-overwritten — adopting someone else's file is install.sh's --force decision,
    //     not an upgrade's. The first draft repaired unconditionally, review catch).
    // Unreadable dest or source (fs error, not absent) -> Unreadable, never Current: the
    // first draft's precedence list had no such arm and an EACCES dest read as Current.
    // CHECKS NEVER WRITE (the --check mode runs diagnosis only): repair is `repair_skill`
    // below, sequenced by Task 5 as check → repair → re-check.
    todo!()
}

/// Redeploy one skill: copy bytes, write a valid managed stamp per the install.sh:189-207
/// contract. The stamp FORMAT is read from install.sh, not restated — the test above pins
/// bytes + re-check, so a format drift fails loudly at the re-check, not silently here.
pub fn repair_skill(source: &Path, dest: &Path) -> std::io::Result<()> {
    todo!()
}
```

`todo!()` marks the check body — the tests above define the exact contract; write the loop.
No new dependencies. No `home_dir()` call anywhere in this task (the wrapper roots every
default at the passed-in `home`): `cort::db::home_dir()` stays private, and Task 3's commit
touches only `upgrade.rs` + tests — a `db.rs` visibility change with no caller would be the
stale instruction the review caught.

For hooks, the check consumes status TSV through an injected runner (tests pass fakes; Task 5
passes the real `<new binary> hook-install --all --status --lean` with a subprocess deadline —
that deadline is what makes `--check` nonblocking, pinned by a Task-5 test holding the locks
and asserting prompt return):

```rust
pub fn check_hooks(
    shim: &Path,
    run_status: &dyn Fn() -> Result<String, String>,
    repair: &dyn Fn(),
) -> Component {
    // 1. run_status() → TSV rows (harness, event[pre|post], outcome, settings, detail,
    //    command); Err (nonzero, unparsable) → Unreadable, never a pass.
    // 2. Build the EXPECTED set from `cort::settings::HOOK_HARNESSES × cort::settings::EVENTS`:
    //    per row, expected command = `{shim} {subcommand} --harness {harness}` where the
    //    subcommand comes from the row's OWN event field via `HookEvent::parse` +
    //    `HookEvent::subcommand` (Suggest → hook-suggest, Refresh → hook-refresh). The first
    //    draft expected hook-suggest on Refresh rows (wrong for half the set, review catch)
    //    and required `starts_with(expected + " ")`, which false-fails the exact command the
    //    installer writes (real commands carry no trailing args — the "repair" would loop
    //    forever on correct wiring, review catch). Equality per row, missing/extra rows →
    //    Drifted with detail (rows are complete by construction today, so the set comparison
    //    is a pin, not a prediction).
    // 3. Per row, ALSO verify entry shape via the dialect's own `entry_shape_ok(path, event,
    //    command)` — `--status` reports command+trust only, so a correct command with an
    //    obsolete matcher reads wired (the exact false-pass CLAUDE.md §12-13 records: a
    //    matcher-only rewrite kept the command byte-identical). The shape constants stay
    //    where they already live (settings_toml.rs MATCHER/matcher_for, and the equivalents
    //    in settings.rs/settings_kimi.rs); the new fns reuse each module's own parser —
    //    restating a matcher table here would be the HOOK_TARGETS sin.
    // 4. Any miss → repair() once, re-run 1-3, re-judge. Still missing → Drifted with the
    //    offending row's detail; all pass → Current. repair() is NOT called when the first
    //    pass is fully Current.
    todo!()
}
```

`entry_shape_ok` (one per dialect module, additive, no behavior change to existing paths):
`pub fn entry_shape_ok(path: &Path, event: HookEvent, expected_command: &str) -> bool` in
`settings.rs` / `settings_toml.rs` / `settings_kimi.rs` — true iff the installed entry's
command equals `expected_command` AND its matcher/timeout/group shape equals what that
module installs today. Where a dialect genuinely has no shape beyond the command, say so
explicitly in a comment and degrade to command equality — an unchecked shape must be
declared, never silently absent. Tests live in the dialect test files (they own real-shape
fixtures): a matcher-rewritten-but-command-identical entry → false; the canonical entry →
true. Without these, the command-equality above re-enacts the §12-13 false-pass.

`HOOK_HARNESSES` (single home for the harness list): add
`pub const HOOK_HARNESSES: [&str; 3] = ["claude-code", "codex", "kimi-code"];` to
`cort::settings`, and build main.rs's `HOOK_TARGETS` from it
(`[(Json, HOOK_HARNESSES[0]), (CodexToml, HOOK_HARNESSES[1]), (KimiToml, HOOK_HARNESSES[2])]`
— const-indexing keeps one name list; the format mapping stays where `SettingsFormat`
lives). Task 3's commit therefore touches `main.rs` (one const) + `settings*.rs` + tests —
listed in Files below, not smuggled.

`check_hooks` is the judging half plus the repair loop, split so `--check` (Task 5) can reuse
the judgment with no repair path at all:

```rust
/// Pure judgment over one `--status --lean` TSV: set-compare against the
/// HOOK_HARNESSES × EVENTS expectation (per-row command equality, missing/extra rows →
/// Drifted) plus per-row `entry_shape_ok`. No subprocess, no repair — `--check` calls this
/// through `run_status_with_deadline` and CANNOT repair (no callback exists).
pub fn judge_hooks(shim: &Path, status_tsv: &str) -> Component { todo!() }

pub fn check_hooks(
    shim: &Path,
    run_status: &dyn Fn() -> Result<String, String>,
    repair: &dyn Fn(),
) -> Component {
    // run → judge; all-Current → return it (repair NOT called — pinned by
    // clean_hook_status_never_calls_repair). Else repair() once, re-run, re-judge; still
    // missing → Drifted with the offending row's detail. run Err → Unreadable.
    todo!()
}
```

Task 1's `diagnose` grows here (additive — Task-1 tests keep passing untouched):

```rust
// DiagnoseInputs gains:
pub new_tree: &'a Path,  // source tree (skills/*/SKILL.md live here)
pub home: &'a Path,      // agent home (skill-dest defaults root here)
pub keep_mine: bool,
// diagnose() appends: check_skills_with_policy(new_tree, home, keep_mine).
// Hooks are NOT inside diagnose(): the hook seam needs run_status/repair closures, and
// closures in the inputs struct are lifetime noise — the bin (Task 5) appends check_hooks
// (mutating) or judge_hooks via diagnose_for_check (--check) itself.
//
// MECHANICAL FALLOUT, do not miss: extending the struct breaks Task-1's e2e test literals
// (`diagnose_reports_a_drifted_pack_and_a_current_one` constructs DiagnoseInputs without the
// new fields). Update both literals in this task's commit with
// `new_tree: <tmp>, home: <tmp>, keep_mine: false` — a struct extension that leaves an
// earlier test uncompilable is a red Task-3 gate, not a Task-1 regression.
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade`
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `check_skills` treat stamp-validity as sufficient (skip content diff when the stamp
   matches). Expected: the diverged-with-valid-stamp test RED — this is spec §2 row 4's named
   defect and it must be greenable-by-accident-proof.
2. Make `keep_mine=true` report Drifted instead of DeferredByUser. Expected: the keep-mine
   test RED at the state assertion. (The bytes-untouched assertion passes regardless — checks
   never write — and stays as a pin against a future repair-inside-check, not as the break's
   tripwire. The first draft's break said "make keep_mine still overwrite", which no longer
   typechecks now that repair lives in `repair_skill`; review catch.)
3. Make `check_hooks` return Current after invoking repair WITHOUT re-running status. Expected:
   the re-verification test RED — specifically, the no-op-repair test must still fail (proving
   re-verification is what catches a failed repair, not the repair call itself).
4. Rewrite one dialect fixture's matcher (command byte-identical) in its own test file.
   Expected: that dialect's `entry_shape_ok` test RED — and the upgrade-level command-equality
   tests stay GREEN, proving the shape check observes what command comparison cannot.
5. Make `repair_skill` write the stamp but NOT the bytes. Expected: the repair test RED at
   the re-check (still Drifted) — proving the re-check observes bytes, not the stamp. (Note
   the asymmetry: the reverse break — bytes without stamp — PASSES, because content-equal
   reads Current regardless of stamp per the cosmetic rule. That asymmetry is the point: the
   check must be byte-theatre-proof in the direction that matters.)

- [ ] **Step 6: Verify everything**

Same two-crate verification as Task 1. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/src/settings.rs rust/src/settings_toml.rs rust/src/settings_kimi.rs rust/src/main.rs rust/tests/upgrade.rs rust/tests/settings.rs rust/tests/settings_toml.rs
git commit -m "feat(upgrade): hooks and skills re-verified by content, repair re-checked

Skills are content-diffed against the new tree, not stamp-checked — a valid
stamp over old bytes was the exact false-pass of spec §2 row 4. --keep-mine
downgrades divergence to DeferredByUser and leaves bytes untouched; repair is
a separate step, never inside the check (so --check writes nothing). Hook
diagnosis compares each status row against the HOOK_TARGETS-derived expected
command for its own (harness, event) — Refresh rows expect hook-refresh, not
hook-suggest — plus per-dialect entry-shape checks, because --status cannot
see a matcher-only rewrite. Repair runs once, then status RE-RUNS before
anyone believes it took."
```

---

### Task 4: index migration — eager for live directories, deferred for gone ones

**Files:**
- Modify: `rust/src/upgrade.rs`, `rust/tests/upgrade.rs`, `rust/src/db.rs` (one line: 5s busy timeout on the `list_projects` scan open — spec §10 item 1; see Step 3)

**Interfaces:**
- Consumes: `cort::db::list_projects()` (with its `Unreadable` variant from plan 1), Task 1's `read_reasons_readonly`, `cort::incremental::{incremental_index, RebuildPolicy}` (NOT `cort::indexer::` — the first draft imported both from `indexer`; `full_index` does live in `indexer` but migration never calls it directly: `incremental_index` with `Allow` performs whatever rebuild the reasons require, review catch), plan 2's `FullRebuildRequired`.
- Produces: `pub fn migrate_indexes(defer: bool) -> Vec<Component>` — one `Component` per project, `name` = owned project path (`String`; the first draft wrote `name: &row.path` against a `&'static str` field in five places, none of which compiles), state = `Current` (rebuilt or already-current) / `Drifted` (deferred, or rebuild did not take: reasons recorded) / `Absent` (directory gone; reasons recorded, never a failure) / `Unreadable` (db would not open, or the post-rebuild re-read failed; NOT counted a pass, never silently Current).

**Policy (spec §4, the reviewed middle):** for each project from `list_projects`, read
reasons through Task 1's `read_reasons_readonly` (this task defines no new reader). Empty
reasons → Current, skip. Non-empty and directory exists and `!defer` → run the rebuild
(foreground policy: `RebuildPolicy::Allow` — the upgrader IS a foreground actor), then
re-read reasons; still non-empty → Drifted with reasons (a rebuild that did not take).
Non-empty and `defer` → Drifted with reasons recorded, no rebuild. Non-empty and directory
GONE → `Absent` with reasons recorded, no rebuild — never Drifted, because Task 5's verdict
maps unacked Drifted to Partial and "gone never fails" (spec §6) would be a lie otherwise.
The first draft collapsed `defer || gone` into Drifted while its own Produces line promised
Absent-for-gone and its own verdict test fed Absent by hand — the real pipeline never
produced what the test asserted (review catch). Every project keeps its durable reasons
either way — plan 2 already persists them, so nothing here writes new state.

**Flocks:** the whole migration runs inside the upgrader's held locks (Task 2) — hooks stand
down, no concurrent writer. A `FullRebuildRequired` here is a BUG (foreground policy forbids
nothing); map it to Drifted with reasons rather than panicking.

- [ ] **Step 1: Write the failing test**

```rust
// Env isolation: CORT_CACHE_DIR is process-global and migrate_indexes reads it in-process,
// so these tests serialize on a lock. Copy of rust/tests/context.rs's ENV_LOCK + with_vars
// (cross-crate import is impossible — each integration target is its own crate — and a
// `set_var` without the lock flakes under parallel threads; the first draft said nothing
// and every cache assertion would have been order-dependent).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_vars(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) { /* verbatim copy of context.rs */ }

/// Real indexed project in a TEMP cache dir: lib-level full_index into db_path_for(root),
/// so list_projects (which reads CORT_CACHE_DIR) finds exactly this one project.
fn indexed_project_in(cache: &std::path::Path) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::Builder::new().prefix("cort-upg-proj-").tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::write(root.join("a.ts"), "export function aaa() { return 1; }\n").unwrap();
    let bin = cort::ast_grep::resolve_ast_grep_bin().expect("ast-grep on PATH");
    let db_path = cort::db::db_path_for(root.to_str().unwrap());
    let mut db = cort::db::open_db(&db_path).unwrap();
    cort::db::ensure_schema(&db).unwrap();
    cort::indexer::full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db_path.to_string_lossy().into_owned())
}

/// An index whose stored extractor is superseded gets rebuilt (directory exists, no --defer),
/// and afterwards its reasons are empty. This is the spec §4 eager default, and it is the test
/// that proves the upgrader is the missing ACTOR for the debt plan 2 made visible.
#[test]
fn a_drifted_index_with_a_live_directory_is_rebuilt() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(&[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))], || {
        let (_dir, root, db_path) = indexed_project_in(cache.path());
        // Supersede the extractor stamp, then prove the precondition (reasons non-empty) —
        // without it the test passes on an index that was never drifted.
        let db = cort::db::open_db(&db_path).unwrap();
        cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
        drop(db);
        let db = cort::db::open_db(&db_path).unwrap();
        assert!(!cort::indexer::rebuild_reasons(&db).unwrap().is_empty(), "precondition: drifted");
        drop(db);
        let comps = cort::upgrade::migrate_indexes(false);
        let c = comps.iter().find(|c| c.name == format!("index:{}", root.display())).unwrap();
        assert!(matches!(c.state, cort::upgrade::ComponentState::Current), "{c:?}");
        let db = cort::db::open_db(&db_path).unwrap();
        assert!(cort::indexer::rebuild_reasons(&db).unwrap().is_empty(), "rebuilt means no debt left");
    });
}

/// The same drift with the directory GONE must NOT attempt a rebuild — it records the debt and
/// moves on as Absent (never Drifted: Task 5 maps unacked Drifted to Partial and "gone never
/// fails"). Attempting would mean extraction over a missing tree.
#[test]
fn a_drifted_index_whose_directory_is_gone_is_marked_not_rebuilt() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(&[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))], || {
        let (dir, root, db_path) = indexed_project_in(cache.path());
        let db = cort::db::open_db(&db_path).unwrap();
        cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
        drop(db);
        let root_str = root.to_string_lossy().into_owned();
        drop(dir); // the directory is gone now
        assert!(!std::path::Path::new(&root_str).exists(), "precondition: gone");
        let comps = cort::upgrade::migrate_indexes(false);
        let c = comps.iter().find(|c| c.name == format!("index:{root_str}")).unwrap();
        assert!(matches!(c.state, cort::upgrade::ComponentState::Absent), "{c:?}");
        assert!(c.detail.contains("extractor"), "debt recorded, not dropped: {c:?}");
        assert!(!std::path::Path::new(&root_str).exists(), "no rebuild recreated the tree");
    });
}

/// An unreadable index is reported Unreadable and is NOT a verdict failure (spec §6: unreadable
/// never passes, but gone never fails; unreadable is reported). The verdict aggregation in
/// Task 5 decides what it does to the exit code — here we only pin the component state.
#[test]
fn an_unreadable_index_is_unreadable_and_reported() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(&[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))], || {
        // A junk *.db in the cache dir; list_projects already reports it Unreadable (plan 1).
        fs::write(cache.path().join("junk.db"), b"not a database").unwrap();
        let comps = cort::upgrade::migrate_indexes(false);
        let c = comps.iter().find(|c| c.name == "index_unreadable").unwrap();
        assert!(matches!(c.state, cort::upgrade::ComponentState::Unreadable), "{c:?}");
    });
}

/// --defer records the debt without rebuilding, even when the directory is live.
#[test]
fn defer_marks_without_rebuilding() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(&[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))], || {
        let (_dir, root, db_path) = indexed_project_in(cache.path());
        let db = cort::db::open_db(&db_path).unwrap();
        cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
        drop(db);
        let comps = cort::upgrade::migrate_indexes(true);
        let c = comps.iter().find(|c| c.name == format!("index:{}", root.display())).unwrap();
        assert!(matches!(c.state, cort::upgrade::ComponentState::Drifted), "{c:?}");
        assert!(c.detail.contains("extractor"), "{c:?}");
        let db = cort::db::open_db(&db_path).unwrap();
        assert!(!cort::indexer::rebuild_reasons(&db).unwrap().is_empty(), "deferred means debt kept");
    });
}

/// A database that becomes unreadable BETWEEN rebuild and re-read must NOT read Current: the
/// first draft's `unwrap_or_default()` turned that window into a pass (review catch).
/// Fixture: live drifted project; sabotage the db file after the rebuild would have run is
/// timing-dependent, so the deterministic seam is smaller — Task 5's sequencing calls
/// re-read through the same `read_reasons_readonly`, and THIS test pins the primitive:
/// a path that was readable and is now garbage reads None, never Some(vec![]).
#[test]
fn a_post_rebuild_unreadable_db_is_not_empty_reasons() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(&[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))], || {
        let (_dir, _root, db_path) = indexed_project_in(cache.path());
        assert!(cort::upgrade::read_reasons_readonly(&db_path).is_some(), "precondition: readable");
        fs::write(&db_path, b"garbage").unwrap();
        assert!(cort::upgrade::read_reasons_readonly(&db_path).is_none(), "garbage is not empty debt");
    });
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade -- drifted defer unreadable`
Expected: FAIL to compile — `no function named 'migrate_indexes'`. (Filters after `--`;
`drifted` catches both rebuild tests, `defer` the defer test, `unreadable` both unreadable
tests. A filter matching zero tests exits 0 without running anything — every filter in this
plan was checked against the names it must catch.)

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`:

```rust
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
                continue;
            }
            crate::db::ProjectEntry::Indexed(row) => {
                let gone = !Path::new(&row.path).is_dir();
                // Open WITHOUT migrating (open-project-unmigrated discipline — a structural
                // migration inside the upgrader is Task 0/5's install.sh path, and doing it
                // here would hide it). Read reasons through Task 1's read-only primitive.
                let name = format!("index:{}", row.path);
                let reasons = match read_reasons_readonly(&row.db_path) {
                    Some(r) => r,
                    None => {
                        out.push(Component { name, state: ComponentState::Unreadable,
                            detail: format!("{}: metadata unreadable", row.db_path) });
                        continue;
                    }
                };
                if reasons.is_empty() {
                    out.push(Component { name, state: ComponentState::Current, detail: String::new() });
                    continue;
                }
                if gone {
                    // Absent, NOT Drifted — see Policy above. `row` is owned and dropped at
                    // the iteration end, so `name` was cloned up front (the first draft's
                    // `name: &row.path` never compiles against a String field).
                    out.push(Component { name, state: ComponentState::Absent,
                        detail: format!("directory gone, debt kept: {}", reasons.join(", ")) });
                    continue;
                }
                if defer {
                    out.push(Component { name, state: ComponentState::Drifted,
                        detail: format!("deferred: {}", reasons.join(", ")) });
                    continue;
                }
                // Eager: foreground rebuild through the crate's own index path. Reuse
                // cmd_index's machinery by calling incremental_index directly with
                // RebuildPolicy::Allow — the upgrader IS a foreground actor.
                match rebuild_project(&row) {
                    Ok(()) => {
                        // Re-read, and a FAILED re-read is Drifted ("re-read failed"), NEVER
                        // Current: the first draft's `unwrap_or_default()` turned an
                        // unreadable-between-rebuild-and-verify db into empty reasons and a
                        // pass (review catch).
                        match read_reasons_readonly(&row.db_path) {
                            Some(after) if after.is_empty() => {
                                out.push(Component { name, state: ComponentState::Current, detail: String::new() });
                            }
                            Some(after) => {
                                out.push(Component { name, state: ComponentState::Drifted,
                                    detail: format!("rebuild did not take: {}", after.join(", ")) });
                            }
                            None => {
                                out.push(Component { name, state: ComponentState::Unreadable,
                                    detail: format!("{}: re-read failed after rebuild", row.db_path) });
                            }
                        }
                    }
                    Err(e) => out.push(Component { name, state: ComponentState::Drifted,
                        detail: format!("rebuild failed: {e}") }),
                }
            }
        }
    }
    out
}
```

`rebuild_project` opens read-write through `open_db` + `ensure_schema` (the structural
migration IS part of repaying `schema_changed`), then
`cort::incremental::incremental_index(&mut db, &bin, &row.path, RebuildPolicy::Allow)` with
the bin from `resolve_ast_grep_bin`. A `FullRebuildRequired` here is a BUG (foreground
policy forbids nothing); map it to Drifted with reasons rather than panicking. (`Path` here
is Task 1's `use std::path::Path` — still the single import; do not re-add it.)

Plus the one-line `db.rs` change (spec §10 item 1, second half): in `list_projects`, after
the `SQLITE_OPEN_READ_ONLY` open succeeds, `db.busy_timeout(Duration::from_secs(5))` — the
same 5s the rest of the module uses. The scan runs under a concurrent refresh-hook writer on
every edit; without this, one transient `SQLITE_BUSY` reports a healthy index `Unreadable`.
No dedicated timing test (flaky by nature); the direction is wait-instead-of-Unreadable and
every existing Unreadable test still pins the true-unreadable path. This lands in Task 4's
commit, not Task 1's — it changes existing behavior, and Task 4 is the task that reads
through it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade -- drifted defer unreadable`
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `migrate_indexes` skip the rebuild unconditionally (always defer). Expected: the
   eager test RED — this is the "actor missing" regression, the exact failure mode that made
   the signal worthless.
2. Make it rebuild even when the directory is gone. Expected: the gone-directory test RED
   (rebuild errors or, if it "succeeds", the assertion that nothing was created at the old
   path catches it).
3. Make `read_reasons_readonly` swallow errors to `Some(vec![])`. Expected: the
   post-rebuild-unreadable test RED — and NOTE the junk-db unreadable test stays GREEN, which
   is exactly why the first draft's break ("expect the unreadable test RED") was vacuous: the
   junk fixture takes the `ProjectEntry::Unreadable` branch before any read, so it cannot
   observe the read primitive at all (review catch). Only the garbage-overwrites-db test
   exercises this line.

- [ ] **Step 6: Verify everything**

Two-crate verification as before, plus `bash tests/install-smoke.sh`. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/upgrade.rs rust/src/db.rs rust/tests/upgrade.rs
git commit -m "feat(upgrade): eager index migration for live projects, deferred for gone ones

rebuild_reasons (plan 2) is read through a read-only open — the upgrader
never migrates schema as a side effect of looking. Empty reasons: Current.
Live directory + reasons + no --defer: rebuild with RebuildPolicy::Allow (the
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
        cort::upgrade::Component { name: "shim".to_string(), state: cort::upgrade::ComponentState::Drifted, detail: "x".into() },
        cort::upgrade::Component { name: "skill_xgrep".to_string(), state: cort::upgrade::ComponentState::Current, detail: String::new() },
    ];
    let v = cort::upgrade::verdict(comps.clone(), &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Partial));
    // An acked drift becomes info: still printed (visibility), but not a failure.
    let v = cort::upgrade::verdict(comps, &["shim"]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Ok), "acked drift must not fail the verdict");
}

/// A gone directory is NOT a failure (spec §6: gone never fails) even though its reasons are
/// recorded. This test feeds Absent BY HAND — and it is a real assertion (not the vacuous one
/// the first draft shipped) because Task 4's gone-directory test proves the pipeline really
/// emits Absent for gone dirs; the two tests compose.
#[test]
fn deferred_and_gone_indexes_do_not_fail_the_verdict() {
    let comps = vec![
        cort::upgrade::Component { name: "/gone/project".to_string(), state: cort::upgrade::ComponentState::Absent, detail: "deferred: extractor_changed".into() },
    ];
    let v = cort::upgrade::verdict(comps, &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Ok));
}

/// Unreadable DOES count against the verdict (spec §6: unreadable never passes) — but as
/// Partial, not Fatal.
#[test]
fn unreadable_counts_as_partial() {
    let comps = vec![
        cort::upgrade::Component { name: "index_unreadable".to_string(), state: cort::upgrade::ComponentState::Unreadable, detail: "junk".into() },
    ];
    let v = cort::upgrade::verdict(comps, &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Partial));
}

/// Ack persistence (spec §6: "該元件之後降為資訊列" — *afterwards*). `--ack shim` must work
/// on the NEXT invocation too, not just the current call: the drifted-test above proves the
/// single-call semantics, THIS test proves the store. `save_ack` appends one name (idempotent —
/// acking twice is not two acks); `load_acks` reads the set back; a verdict over loaded acks
/// is Ok. No subprocess, no bin: second-invocation semantics at the lib seam. The store file
/// is `<cache>/.upgrade-acks`, one name per line — beside the lock files, never in the
/// manifest (manifest keys belong to install.sh) and never in usage.db (different owner).
#[test]
fn an_ack_survives_to_the_next_invocation() {
    let cache = tempfile::tempdir().unwrap();
    assert!(cort::upgrade::load_acks(cache.path()).is_empty(), "fresh cache acks nothing");
    cort::upgrade::save_ack(cache.path(), "shim").unwrap();
    cort::upgrade::save_ack(cache.path(), "shim").unwrap(); // idempotent
    let acks = cort::upgrade::load_acks(cache.path());
    assert_eq!(acks, vec!["shim".to_string()]);
    let comps = vec![
        cort::upgrade::Component { name: "shim".to_string(), state: cort::upgrade::ComponentState::Drifted, detail: "x".into() },
    ];
    let borrowed: Vec<&str> = acks.iter().map(String::as_str).collect();
    let v = cort::upgrade::verdict(comps, &borrowed);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Ok), "a persisted ack quiets the next run");
    // Corrupt ack store reads as EMPTY, never as a pass and never as a crash: an unreadable
    // memory must not silence real drift.
    fs::write(cache.path().join(".upgrade-acks"), b"\xff\xfe garbage \x00\n").unwrap();
    assert!(cort::upgrade::load_acks(cache.path()).is_empty());
}

/// First-upgrade detection (Task 2's marker): no marker → the verdict gains a
/// `partial_drain_first_upgrade` info component naming the WAL-reader risk; marker present →
/// no such component. The drain still ran bounded either way — the marker records only that
/// *this* upgrade cannot prove exclusion of pre-lock binaries (spec §3b option A).
#[test]
fn the_first_upgrade_labels_partial_drain() {
    let cache = tempfile::tempdir().unwrap();
    let note = cort::upgrade::first_upgrade_note(cache.path()).unwrap();
    assert!(note.detail.contains("WAL"), "the risk must be named, not implied: {note:?}");
    cort::upgrade::write_first_upgrade_marker(cache.path()).unwrap();
    assert!(cort::upgrade::first_upgrade_note(cache.path()).is_none());
}

/// The `--check` status runner has a deadline: against a binary that sleeps 30s it returns
/// Err within 2s, not after the sleep. No fixture binary needed — `/bin/sleep` ignores its
/// argv and sleeps, which is exactly the hanging-subprocess shape. (Copy of the
/// `recv_timeout` discipline in `ast_grep.rs::exec_ast_grep`, tested here instead of there.)
#[test]
fn a_hanging_status_subprocess_hits_the_deadline() {
    let start = std::time::Instant::now();
    let r = cort::upgrade::run_status_with_deadline(
        std::path::Path::new("/bin/sleep"),
        std::time::Duration::from_secs(2),
    );
    assert!(r.is_err(), "a 30s sleep must not survive a 2s deadline");
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "deadline, not patience: {:?}", start.elapsed());
}

/// `--check` never repairs, even when drifted — by CONSTRUCTION: `diagnose_for_check` takes
/// no repair callback, so there is nothing to call. What the test pins is the other half of
/// that property: a check that cannot repair must still REPORT (a blind check is worse than
/// none — it would print Current over drift). (Review catch — the first draft stated
/// "diagnose only" while passing a repair callback, pinning nothing.)
#[test]
fn check_mode_reports_drift_it_cannot_repair() {
    let bad = "claude-code\tpre\twired\t/s\ttrusted=true\t/foreign hook-suggest --harness claude-code\n";
    let install_root = tempfile::tempdir().unwrap();
    let new_pack = tempfile::tempdir().unwrap();
    let comps = cort::upgrade::diagnose_for_check(
        &cort::upgrade::DiagnoseInputs {
            install_root: install_root.path(),
            new_pack: new_pack.path(),
            installed_ast_grep_version: cort::install::AST_GREP_PINNED,
            new_tree: install_root.path(),
            home: install_root.path(),
            keep_mine: false,
        },
        std::path::Path::new("/shim"),
        &|| Ok(bad.to_string()),
    );
    assert!(comps.iter().any(|c| c.name == "hooks" && matches!(c.state, cort::upgrade::ComponentState::Drifted)));
}

/// `--check` completes while an upgrade holds the locks exclusive (diagnosis takes no locks
/// by construction). Passes trivially today — it pins the property against a future edit
/// that adds locking to the diagnose path, which would wedge every --check behind a drain.
#[test]
fn check_mode_completes_while_an_upgrade_holds_the_locks() {
    let cache = tempfile::tempdir().unwrap();
    let locks = cort::upgrade::acquire_upgrade_locks(cache.path(), std::time::Duration::from_secs(5)).unwrap();
    let install_root = tempfile::tempdir().unwrap();
    let new_pack = tempfile::tempdir().unwrap();
    let comps = cort::upgrade::diagnose_for_check(
        &cort::upgrade::DiagnoseInputs {
            install_root: install_root.path(),
            new_pack: new_pack.path(),
            installed_ast_grep_version: cort::install::AST_GREP_PINNED,
            new_tree: install_root.path(),
            home: install_root.path(),
            keep_mine: false,
        },
        std::path::Path::new("/shim"),
        &|| Ok(String::new()),
    );
    assert!(!comps.is_empty());
    drop(locks);
}
```

Plus a smoke addition in `tests/install-smoke.sh`, right after the fake_ast_grep payload
assertion — which gets the same one-line repair in this task (drive-by with cause: it uses the
identical `find`-on-symlink shape, so it has the identical hole):

```bash
# cort-upgrade is repo-local, never installed — the same rule fake_ast_grep holds. Its presence
# in the payload would be a second executable nobody's package owns.
#
# Assert DIRECTLY through the generation symlink (`test -e` follows it). `find <symlink>
# -name ...` does NOT descend (measured: empty result on a populated tree), so the old shape
# passed whether or not the file was there — a guard that cannot fail is not a guard. The
# same repair applies to the fake_ast_grep assertion above.
test -x "$REPO_ROOT/rust/target/release/cort_upgrade" \
  || fail "release build must produce cort_upgrade (smoke Step 6 builds release first)"
if [ -e "$CORT_HOME_PATH/cort_upgrade" ]; then
  fail "cort-upgrade stays out of the installed payload"
else
  pass "cort-upgrade stays out of the installed payload"
fi
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test upgrade -- drifted gone unreadable ack first_upgrade hanging check_mode`
Expected: FAIL to compile — `no function named 'verdict'` (plus `load_acks`, `save_ack`,
`first_upgrade_note`, `write_first_upgrade_marker`, `run_status_with_deadline`,
`diagnose_for_check` — the first missing name fails first; all arrive in this task). Every
token above matches ≥1 test name by substring (`drifted` catches the verdict + check tests);
a token matching zero tests exits 0 having run nothing — the false-green this plan refuses.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/upgrade.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeExit { Ok, Partial, Fatal }

pub fn verdict(components: Vec<Component>, acks: &[&str]) -> Verdict {
    let mut exit = UpgradeExit::Ok;
    for c in &components {
        // `c` is borrowed: match the state BY REFERENCE (first draft wrote
        // `match (c.state, acked)`, moving a non-Copy enum out of a borrow — review catch).
        let acked = acks.contains(&c.name.as_str());
        match (&c.state, acked) {
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

Ack store + first-upgrade marker (lib fns, tested above — the bin only calls them):

```rust
/// Load persisted acks from `<cache>/.upgrade-acks` (one name per line). Missing file →
/// empty. Garbage → empty (never a crash, never a pass — see the corruption test).
pub fn load_acks(cache: &Path) -> Vec<String> { todo!() }
/// Persist one ack (idempotent). `--ack <name>` writes through this BEFORE applying, so the
/// next invocation already knows.
pub fn save_ack(cache: &Path, name: &str) -> std::io::Result<()> { todo!() }
/// First-upgrade note: `None` when `.upgraded_once` exists beside the lock files, else
/// `Some(Component { name: "partial_drain_first_upgrade", state: Current, detail: <WAL-reader
/// risk text> })`. An INFO component, never a failure — it rides the verdict's component list
/// so it is printed, not smuggled. The bin writes the marker after a successful release.
pub fn first_upgrade_note(cache: &Path) -> Option<Component> { todo!() }
pub fn write_first_upgrade_marker(cache: &Path) -> std::io::Result<()> { todo!() }

/// Run `<bin> hook-install --all --status --lean` with a deadline. `Err` on nonzero exit,
/// unparsable output, or timeout (the `--check` nonblocking pin). `--check` and the mutating
/// run share this runner — the deadline is a property of the invocation, not the mode.
pub fn run_status_with_deadline(bin: &Path, timeout: Duration) -> Result<String, String> { todo!() }

/// The `--check` diagnosis: Task-1 `diagnose` inputs the caller already gathered, PLUS the
/// hook judgment — with NO repair callback in the signature. `--check` cannot repair because
/// there is nothing to call: `judge_hooks` is the pure judgment half of `check_hooks`
/// (parse + set-compare + shape-verify, no subprocess, no repair), and the check path is
/// `run_status_with_deadline` → `judge_hooks`. The mutating run uses `check_hooks`
/// (judge → repair → re-judge). "Diagnose only" is therefore a wiring fact, not prose —
/// review catch (the first draft passed a stub repair callback into the same `check_hooks`,
/// pinning nothing).
pub fn judge_hooks(shim: &Path, status_tsv: &str) -> Component { todo!() }

pub fn diagnose_for_check(
    inputs: &DiagnoseInputs, // Task-3-extended (new_tree/home/keep_mine included)
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
```

In `rust/src/bin/cort_upgrade.rs` — clap struct with `--ack <name>` (repeatable; each one is
`save_ack`ed immediately, then applied this run too), `--keep-mine`, `--defer`, `--check`
(diagnose only). Sequence for the mutating run:

1. `diagnose` (Tasks 1+3, extended with hook/skill components) → if everything already
   Current, print and exit 0 — **no locks taken, no install.sh run** (the steady state must
   be cheap and touch nothing).
2. Otherwise: `install.sh --stage-only` (Task 0; unlocked, invisible, validated; takes NO
   installer lock) → capture the generation id.
3. Take Task-2 locks (`acquire_upgrade_locks`, 30s drain; timeout → Fatal/exit 2, killing
   nothing).
4. `install.sh --activate-only --gen <id>` (flip + shim + `cort_bin` only — NO skills, NO
   hooks, so Task 3's policy (keep-mine, hook shape) can never be pre-empted by the installer;
   the first draft invoked full `install.sh` here, which deploys skills before the policy runs
   and holds no `--keep-mine` — review catch). The installer flock nests inside the upgrade
   locks (leaf-ward, no cycle — Task 0 documents the direction).
5. Re-run diagnosis (§5: "diagnose once and trust it" is wrong — every commit boundary
   re-verifies) → rewire hooks and skills (Task 3 checks; repair via `hook-install --all
   --command-prefix` + `repair_skill`, then RE-CHECK each) → `migrate_indexes(args.defer)`
   (Task 4, which re-reads reasons post-rebuild) → append `first_upgrade_note` →
   release locks → write the first-upgrade marker (only after a FULLY successful run —
   verdict computed and exit still to print; a failed upgrade must NOT mark itself done) →
6. `verdict(components, &load_acks_plus_cli_acks)` → print every component as
   `name: state — detail (next action)` and exit 0/1/2.

`--check` runs step 1 only (diagnose + print + taxonomy), with three pinned properties:
(a) the status runner it passes has a deadline (`run_status_with_deadline` above; test: runner
against a 30s-sleeping fake binary returns Err within 2s);
(b) the check path contains no repair call — `diagnose_for_check` takes no repair callback, so
the property holds by construction, and `check_mode_reports_drift_it_cannot_repair` pins the
companion risk (a check that cannot repair going blind);
(c) the whole check completes while another thread holds the upgrade locks exclusive
(diagnosis takes no locks by construction; the test pins that against future edits).

No bin-level chaos test (sabotage injected between diagnose and repair at the process level):
deliberately out of scope — the three commit-boundary re-verifications (post-flip diagnosis,
post-repair hook re-check, post-rebuild reason re-read) are each specified and break-tested in
their own task, and a cross-process fault-injection harness would be the first bash-orchestrated
test in a pure-Rust gate. Stated here so the omission is explicit, not silent.

The bin stays thin: it sequences and prints; every decision is a function in `upgrade.rs`.

`tests/install-smoke.sh`: the payload assertion for fake_ast_grep gets the one-line direct-test
repair (same hole), plus the new cort_upgrade assertion above.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test upgrade -- drifted gone unreadable ack first_upgrade hanging check_mode && cargo build --locked` then
`bash tests/install-smoke.sh`.
Expected: all PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Make `verdict` map Drifted to `Ok` (only Unreadable counts). Expected: the first test RED.
2. Make `verdict` treat `Absent` as `Partial`. Expected: the gone-directory test RED — this is
   the rule that keeps scratch-index machines from permanent red, and its regression is silent.
3. Make an acked drift still count as Partial. Expected: the ack test RED.
4. Make `load_acks` return a hardcoded `vec!["shim"]`. Expected: the ack-persistence test RED
   at the FRESH-cache assertion (acks nothing) — proving the test observes the store, not the
   verdict. Then make `save_ack` a no-op (return Ok without writing). Expected: RED at the
   roundtrip assertion — proving persistence is a write, not a wish.
5. Delete the `.upgraded_once` write from `write_first_upgrade_marker` (keep Ok). Expected:
   the first-upgrade test RED at the is_none assertion — the marker file is the property, not
   the return code.
6. Make `run_status_with_deadline` wait without a deadline (plain `output()`). Expected: the
   hanging-subprocess test RED by taking 30s (it times the suite, not just fails — run this
   break with `timeout 60` and read the timeout as the signal).
7. Copy a `cort_upgrade` file into the installed payload fixture and re-run the smoke block.
   Expected: FAIL naming the payload. Then delete the release binary and re-run: Expected:
   FAIL naming the build (fail, never skip — the old `[ -x ... ] ||` skip-guard is gone).

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
the binary never derived from components). Acked drift becomes an info line,
and --ack persists to the next run via the cache-dir store (a memory that
forgets is not an escape hatch). Gone directories never fail; unreadable
never passes. --check runs diagnosis only: judged, never repaired, deadline
on every subprocess, no locks taken. The steady state (everything current)
takes no locks and runs no install.sh — an upgrade that would do nothing
must do nothing. Staging is lock-free and invisible (--stage-only skips the
installer flock); only activation flips under both locks."
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
Task 2 (incl. the first-upgrade marker + partial-drain info: option A implemented, option B
explicitly declined with cause). §4 migration → Task 4 (refusal/repayment already shipped in
plan 2; this plan is the actor). §5 order → Task 5's sequence over Task-0 modes
(stage unlocked/invisible → locks → activate → re-verify → rewire → migrate → release; every
commit boundary re-verifies). §6 verdict → Task 5 (taxonomy + PERSISTED acks + never-fail
gone + never-pass unreadable). §7's five fixture warnings → mapped: payload identity (Task 1
e2e verdict test, not just the hash test), atomic switch (3a shipped + Task 4's real rebuild),
locks (Task 2's killed-holder + mid-drain arrival), single verdict (Task 5 + ack store),
refusal-visibility (already shipped, re-asserted via rebuild-re-read). §10 item 1 (scan busy
timeout) → Task 1's `read_reasons_readonly` AND the same 5s on `list_projects`' scan open
(Task 4's one-line db.rs change — the upgrader reads through both, so both stop lying).

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
