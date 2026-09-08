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
fn installed_root() -> (
    (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
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
    assert_ne!(
        expected.trim_end(),
        on_disk.trim_end(),
        "fixture must be drifted"
    );
    // The diagnosis must classify the shim component Drifted. (Direct unit call: the upgrade
    // module exposes per-component checks as pub fns so tests can call one component at a time.)
    let state = cort::upgrade::check_shim(&manifest, &cort_home);
    assert!(
        matches!(state, cort::upgrade::ComponentState::Drifted),
        "a content-diverged shim is Drifted even though the file exists: {state:?}"
    );
}

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
    assert!(
        matches!(state, cort::upgrade::ComponentState::Unreadable),
        "{state:?}"
    );
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
    assert_ne!(
        ha,
        format!("{:x}", raw.finalize()),
        "bytes-only hash must differ from the identity"
    );
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
        // No skills dir in this fixture: the skill components all read Absent, which is what
        // the pack assertions below ignore on purpose.
        new_tree: &_root,
        home: &home,
        keep_mine: false,
        new_binary: &_root.join("nonexistent-new-binary"),
    });
    let pack = comps.iter().find(|c| c.name == "pack").unwrap();
    assert!(matches!(pack.state, ComponentState::Drifted), "{pack:?}");
    // Mirror: same byte both sides → Current.
    fs::write(cort_home.join("pack/r.yml"), "id: a\nlanguage: ts\n").unwrap();
    let comps = cort::upgrade::diagnose(&DiagnoseInputs {
        install_root: &install_root,
        new_pack: new_pack.path(),
        installed_ast_grep_version: pin,
        new_tree: &_root,
        home: &home,
        keep_mine: false,
        new_binary: &_root.join("nonexistent-new-binary"),
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
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Drifted),
        "{c:?}"
    );
    assert!(
        c.detail.contains("0.44.0") && c.detail.contains("0.45.2"),
        "{c:?}"
    );
    // Unparseable installed output (empty string, "ast-grep bogus") → Unreadable, never
    // Current and never silently Drifted: we could not read it, so we claim nothing about it.
    let c = cort::upgrade::check_version_pin("", "0.45.2");
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Unreadable),
        "{c:?}"
    );
}

/// Manifest keys: the live manifest's key set diffed against the tree's authority
/// (`MANIFEST_KEYS` + `MANIFEST_LEGACY_KEYS`). Unknown keys are Drifted-with-detail, never
/// failures on their own — Task 5 decides what they do to the exit code.
#[test]
fn unknown_manifest_keys_are_drifted_with_names_not_silent() {
    let live = [
        "manifest_version",
        "cort_bin",
        "mystery_key_from_the_future",
    ];
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
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Drifted),
        "{c:?}"
    );
    let c = cort::upgrade::check_usage_schema(None, "1");
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Unreadable),
        "{c:?}"
    );
}

mod locks {
    // No `use super::*`: every name here is spelled in full or comes from std — an unused
    // glob import trips `-D warnings` (measured: the first draft carried one).
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
        let result = cort::upgrade::acquire_upgrade_locks(cache.path(), Duration::from_millis(800));
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
        let stood_down = cort::upgrade::try_protected_entry(cache.path()).is_err();
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
    /// (10s deadline) in a second thread; worker B arrives 200ms later and must get
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
    /// is SIGKILLed; the parent must acquire within the timeout rather than seeing a stale
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
        let got = cort::upgrade::acquire_upgrade_locks(&cache_path, Duration::from_secs(2));
        assert!(
            got.is_ok(),
            "kernel must release flocks on process death: {got:?}"
        );
    }
}

/// Child side of the killed-holder test. Runs ONLY when argv contains "lock_holder_child";
/// otherwise returns immediately. NOTE: this fn lives at the TOP level of the test crate,
/// outside `mod locks` — `Duration` is NOT imported here, so every duration uses the full
/// `std::time::Duration` path.
#[test]
fn lock_holder_child() {
    if !std::env::args().any(|a| a == "lock_holder_child") {
        return; // real test run: no-op
    }
    let cache = std::env::var("UPGRADE_TEST_CACHE").unwrap();
    let _locks = cort::upgrade::acquire_upgrade_locks(
        std::path::Path::new(&cache),
        std::time::Duration::from_secs(60),
    )
    .unwrap();
    // Signal AFTER acquiring: the parent must not kill us before we hold anything, or the
    // test proves nothing (a kill before acquisition passes against code that never locks).
    std::fs::write(std::path::Path::new(&cache).join(".holder-ready"), b"held").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60)); // parent will kill us
}

// ── Task 3: skills — content-diffed, not stamp-checked ─────────────

/// Build one diverged-managed-skill fixture: the new tree says "new body", the deployed file
/// says "old body", and the stamp beside the deployed file is VALID for the old body — the
/// exact state the pre-3b check read as "managed, fine" (spec §2 row 4's named defect).
fn diverged_skill_fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let new_tree = tempfile::tempdir().unwrap();
    let skill_dir = new_tree.path().join("skills/ast-grep");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: ast-grep\n---\nnew body\n",
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join(".claude/skills/ast-grep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let old_body = "---\nname: ast-grep\n---\nold body\n";
    fs::write(&dest, old_body).unwrap();
    (new_tree, home, dest, skill_dir.join("SKILL.md"))
}

/// A stamp for the old body, exactly the bytes install.sh's ensure_skill_stamp writes.
fn stamp_for(body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "managed by cortexyoung install.sh\nskill_sha256:{:x}\n",
        Sha256::digest(body)
    )
}

/// A managed skill whose content diverged from the new tree is Drifted — NOT Current, which is
/// what the old stamp-ownership check answered.
#[test]
fn a_diverged_managed_skill_is_drifted_even_with_a_valid_stamp() {
    let (new_tree, home, dest, source) = diverged_skill_fixture();
    let old_body = b"---\nname: ast-grep\n---\nold body\n";
    fs::write(
        dest.parent().unwrap().join(".cortexyoung-managed"),
        stamp_for(old_body),
    )
    .unwrap();

    // Call `_at` with explicit dests, NOT the env-reading wrapper: `set_var` inside parallel
    // tests is unsound, and a dev machine with CLAUDE_SKILL_HOME set would redirect the
    // wrapper elsewhere. The wrapper's env mirroring is review-verified, not test-pinned.
    let (xg, ast, cx, _km) = {
        let xg = home.path().join(".claude/skills/xgrep/SKILL.md");
        let cx = home.path().join(".codex/skills/ast-grep/SKILL.md");
        (xg, dest.clone(), cx, ())
    };
    let comps = cort::upgrade::check_skills_at(new_tree.path(), &xg, &ast, &cx, false);
    let ast_c = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(
        matches!(ast_c.state, cort::upgrade::ComponentState::Drifted),
        "{ast_c:?}"
    );
    assert_eq!(
        source.to_string_lossy(),
        new_tree
            .path()
            .join("skills/ast-grep/SKILL.md")
            .to_string_lossy()
    );
}

/// keep-mine=true turns the same divergence into DeferredByUser, and the file is untouched —
/// "repair" must never overwrite a user's edit silently.
#[test]
fn keep_mine_leaves_a_diverged_skill_alone() {
    let (new_tree, home, dest, _source) = diverged_skill_fixture();
    let old_body = b"---\nname: ast-grep\n---\nold body\n";
    fs::write(
        dest.parent().unwrap().join(".cortexyoung-managed"),
        stamp_for(old_body),
    )
    .unwrap();
    let xg = home.path().join(".claude/skills/xgrep/SKILL.md");
    let cx = home.path().join(".codex/skills/ast-grep/SKILL.md");
    let comps = cort::upgrade::check_skills_at(new_tree.path(), &xg, &dest, &cx, true);
    let ast_c = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(
        matches!(ast_c.state, cort::upgrade::ComponentState::DeferredByUser),
        "{ast_c:?}"
    );
    assert_eq!(
        fs::read_to_string(&dest).unwrap(),
        "---\nname: ast-grep\n---\nold body\n",
        "keep-mine must not touch bytes"
    );
}

/// keep-mine=false does NOT repair inside the check — checks never write (the `--check` mode
/// runs diagnosis only). Repair is a separate `repair_skill` step; this test pins the sequence
/// Task 5 runs: check (Drifted) → repair → re-check (Current, bytes == new tree).
#[test]
fn repair_redeploys_skill_then_recheck_says_current() {
    let (new_tree, home, dest, source) = diverged_skill_fixture();
    let xg = home.path().join(".claude/skills/xgrep/SKILL.md");
    let cx = home.path().join(".codex/skills/ast-grep/SKILL.md");
    let at = |keep_mine: bool| {
        cort::upgrade::check_skills_at(new_tree.path(), &xg, &dest, &cx, keep_mine)
    };
    let ast_c = at(false)
        .into_iter()
        .find(|c| c.name == "skill_ast_grep")
        .unwrap();
    assert!(
        matches!(ast_c.state, cort::upgrade::ComponentState::Drifted),
        "{ast_c:?}"
    );
    cort::upgrade::repair_skill(&source, &dest).unwrap();
    assert_eq!(
        fs::read_to_string(&dest).unwrap(),
        "---\nname: ast-grep\n---\nnew body\n",
        "repair writes the new bytes"
    );
    // The re-check must read the STAMP too: an old stamp beside new bytes is cosmetic, but a
    // repair that forgot the stamp would leave the file unmanaged — the next hand-edit would
    // be an unmanaged collision instead of a kept-mine.
    let stamp = fs::read_to_string(dest.parent().unwrap().join(".cortexyoung-managed")).unwrap();
    assert!(
        stamp.contains("skill_sha256:"),
        "repair claims the new bytes: {stamp}"
    );
    let ast_c = at(false)
        .into_iter()
        .find(|c| c.name == "skill_ast_grep")
        .unwrap();
    assert!(
        matches!(ast_c.state, cort::upgrade::ComponentState::Current),
        "{ast_c:?}"
    );
}

/// An absent skill source is Absent — a release may legitimately drop a skill; that is not a
/// failure and never a repair target.
#[test]
fn an_absent_skill_source_is_absent_not_a_failure() {
    let empty_tree = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join(".claude/skills/ast-grep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "---\nname: ast-grep\n---\nsome old body\n").unwrap();
    let xg = home.path().join(".claude/skills/xgrep/SKILL.md");
    let cx = home.path().join(".codex/skills/ast-grep/SKILL.md");
    let comps = cort::upgrade::check_skills_at(empty_tree.path(), &xg, &dest, &cx, false);
    let ast_c = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(
        matches!(ast_c.state, cort::upgrade::ComponentState::Absent),
        "{ast_c:?}"
    );
}

// ── Task 3: hooks — expected-shape comparison, repair re-verified ──

/// Six wired rows produced by the crate's OWN installers into per-dialect temp files, in the
/// exact TSV shape `hook-install --all --status --lean` emits (harness, event, outcome,
/// settings, detail, command). Building fixtures with anything else would test judge_hooks
/// against a fantasy of what install_hook writes.
fn wired_fixture(dir: &std::path::Path, shim: &std::path::Path) -> String {
    let json_path = dir.join("settings.json");
    let toml_path = dir.join("codex.toml");
    let kimi_path = dir.join("kimi.toml");
    for harness in cort::settings::HOOK_HARNESSES {
        for event in cort::settings::EVENTS {
            let command = format!(
                "{} {} --harness {}",
                shim.display(),
                event.subcommand(),
                harness
            );
            // Three dialects, three Result types — assert is_ok per arm rather than unifying.
            let ok = match harness {
                "claude-code" => cort::settings::install_hook(&json_path, &command, event).is_ok(),
                "codex" => cort::settings_toml::install_hook(&toml_path, &command, event).is_ok(),
                "kimi-code" => {
                    cort::settings_kimi::install_hook(&kimi_path, &command, event).is_ok()
                }
                other => unreachable!("unknown harness fixture {other}"),
            };
            assert!(ok, "fixture install failed for {harness}/{event:?}");
        }
    }
    let mut rows = Vec::new();
    for harness in cort::settings::HOOK_HARNESSES {
        for event in cort::settings::EVENTS {
            let (settings, command) = match harness {
                "claude-code" => (
                    json_path.clone(),
                    cort::settings::installed_command(&json_path, event).unwrap(),
                ),
                "codex" => (
                    toml_path.clone(),
                    cort::settings_toml::installed_command(&toml_path, event).unwrap(),
                ),
                _ => (
                    kimi_path.clone(),
                    cort::settings_kimi::installed_command(&kimi_path, event).unwrap(),
                ),
            };
            rows.push(format!(
                "{}\t{}\twired\t{}\t-\t{}",
                harness,
                event.flag_name(),
                settings.display(),
                command
            ));
        }
    }
    let mut tsv = rows.join("\n");
    tsv.push('\n');
    tsv
}

/// Hook repair is re-verified after repair, not assumed. `run_status` is injected (same seam
/// style as the `repair` callback): the tests never spawn subprocesses — Task 5 passes the
/// real `<bin> hook-install --all --status --lean` runner with its deadline; the tests pass
/// fakes.
#[test]
fn hook_repair_is_followed_by_reverification() {
    use std::cell::Cell;
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("cort");
    let good = wired_fixture(dir.path(), &shim);
    let drifted_row = format!("{} hook-suggest --harness codex", shim.display());
    let bad = good.replace(&drifted_row, "/foreign hook-suggest --harness codex");
    assert_ne!(good, bad, "fixture must be drifted");
    let repaired = Cell::new(false);
    let calls = Cell::new(0);
    let run_status = || {
        calls.set(calls.get() + 1);
        Ok(if repaired.get() {
            good.clone()
        } else {
            bad.clone()
        })
    };
    let repair = || repaired.set(true);
    let c = cort::upgrade::check_hooks(&shim, &run_status, &repair);
    // Current ONLY because the second status agreed; one repair; exactly two status calls.
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Current),
        "{c:?}"
    );
    assert_eq!(calls.get(), 2);
    assert!(repaired.get());
}

/// A repair that does not take must stay Drifted — returning Current after invoking repair
/// WITHOUT re-running status would pass the test above while proving nothing.
#[test]
fn a_noop_hook_repair_stays_drifted() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("cort");
    let good = wired_fixture(dir.path(), &shim);
    let bad = good.replace(
        &format!("{} hook-suggest --harness codex", shim.display()),
        "/foreign hook-suggest --harness codex",
    );
    let c = cort::upgrade::check_hooks(&shim, &|| Ok(bad.clone()), &|| {});
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Drifted),
        "{c:?}"
    );
    assert!(
        c.detail.contains("codex"),
        "detail names the offending row: {c:?}"
    );
}

/// Clean status never calls repair: a check that "repairs" unconditionally would redeploy on
/// every upgrade run, churning backups and trust stamps for nothing. (The missing-row case —
/// five rows present and matching, one target absent — is Drifted WITH repair called: the fix
/// for an absent row is installing it. Both directions pinned, neither collapsible.)
#[test]
fn clean_hook_status_never_calls_repair() {
    use std::cell::Cell;
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("cort");
    let good = wired_fixture(dir.path(), &shim);
    let called = Cell::new(false);
    let c = cort::upgrade::check_hooks(&shim, &|| Ok(good.clone()), &|| called.set(true));
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Current),
        "{c:?}"
    );
    assert!(!called.get(), "matching status must not trigger a repair");
}

/// A missing row is Drifted, not silently ok: judging only returned rows would pass a settings
/// file that lost an event. (Rows are complete BY CONSTRUCTION today — hook_install_all always
/// emits all six — so this test pins the set comparison, not a case anyone has seen.)
#[test]
fn a_missing_hook_row_is_drifted() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("cort");
    let good = wired_fixture(dir.path(), &shim);
    let one_row: String = good.lines().next().unwrap().to_string() + "\n";
    let c = cort::upgrade::check_hooks(&shim, &|| Ok(one_row.clone()), &|| {});
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Drifted),
        "{c:?}"
    );
    assert!(c.detail.contains("missing"), "{c:?}");
}

/// A correct command with an obsolete matcher still passes `--status` — the exact false-pass
/// CLAUDE.md §12-13 records (a matcher-only rewrite left the command byte-identical). The
/// shape check exists for this row and only this row proves it.
#[test]
fn a_matcher_rewrite_with_identical_command_is_drifted() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("cort");
    let good = wired_fixture(dir.path(), &shim);
    let json_path = dir.path().join("settings.json");
    let on_disk = fs::read_to_string(&json_path).unwrap();
    // serde_json pretty-prints with `": "` — the closing quote keeps the rewrite off the
    // refresh rows (`"Bash|Edit|..."` does not contain `"Bash"` with a quote after Bash).
    let rewritten = on_disk
        .replace(r#""matcher": "Bash""#, r#""matcher": "exec_command""#)
        .replace(r#""matcher":"Bash""#, r#""matcher":"exec_command""#);
    assert_ne!(on_disk, rewritten, "fixture rewrite must change the file");
    fs::write(&json_path, rewritten).unwrap();
    let c = cort::upgrade::judge_hooks(&shim, &good);
    assert!(
        matches!(c.state, cort::upgrade::ComponentState::Drifted),
        "{c:?}"
    );
    assert!(
        c.detail.contains("shape"),
        "detail names the shape drift: {c:?}"
    );
}

// ── Task 4: index migration — eager for live directories, deferred for gone ones ──
//
// Env isolation: CORT_CACHE_DIR is process-global and migrate_indexes reads it in-process,
// so these tests serialize on a lock. Copy of rust/tests/context.rs's ENV_LOCK + with_vars
// (cross-crate import is impossible — each integration target is its own crate — and a
// `set_var` without the lock flakes under parallel threads).

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_vars(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
    let _g = env_guard();
    let prev: Vec<(String, Option<String>)> = pairs
        .iter()
        .map(|(k, _)| ((*k).to_string(), std::env::var(k).ok()))
        .collect();
    unsafe {
        for (k, val) in pairs {
            match val {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        for (k, old) in prev {
            match old {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// Real indexed project in a TEMP cache dir: lib-level full_index into db_path_for(root),
/// so list_projects (which reads CORT_CACHE_DIR) finds exactly this one project. The `cache`
/// argument is documentation only — the function reads the dir from CORT_CACHE_DIR, set by
/// the caller's `with_vars`.
fn indexed_project_in(_cache: &std::path::Path) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::Builder::new()
        .prefix("cort-upg-proj-")
        .tempdir()
        .unwrap();
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
    with_vars(
        &[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))],
        || {
            let (_dir, root, db_path) = indexed_project_in(cache.path());
            // Supersede the extractor stamp, then prove the precondition (reasons non-empty) —
            // without it the test passes on an index that was never drifted.
            let db = cort::db::open_db(&db_path).unwrap();
            cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
            drop(db);
            let db = cort::db::open_db(&db_path).unwrap();
            assert!(
                !cort::indexer::rebuild_reasons(&db).unwrap().is_empty(),
                "precondition: drifted"
            );
            drop(db);
            let comps = cort::upgrade::migrate_indexes(false);
            let c = comps
                .iter()
                .find(|c| c.name == format!("index:{}", root.display()))
                .unwrap();
            assert!(
                matches!(c.state, cort::upgrade::ComponentState::Current),
                "{c:?}"
            );
            let db = cort::db::open_db(&db_path).unwrap();
            assert!(
                cort::indexer::rebuild_reasons(&db).unwrap().is_empty(),
                "rebuilt means no debt left"
            );
        },
    );
}

/// The same drift with the directory GONE must NOT attempt a rebuild — it records the debt and
/// moves on as Absent (never Drifted: Task 5 maps unacked Drifted to Partial and "gone never
/// fails"). Attempting would mean extraction over a missing tree.
#[test]
fn a_drifted_index_whose_directory_is_gone_is_marked_not_rebuilt() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(
        &[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))],
        || {
            let (dir, root, db_path) = indexed_project_in(cache.path());
            let db = cort::db::open_db(&db_path).unwrap();
            cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
            drop(db);
            let root_str = root.to_string_lossy().into_owned();
            drop(dir); // the directory is gone now
            assert!(
                !std::path::Path::new(&root_str).exists(),
                "precondition: gone"
            );
            let comps = cort::upgrade::migrate_indexes(false);
            let c = comps
                .iter()
                .find(|c| c.name == format!("index:{root_str}"))
                .unwrap();
            assert!(
                matches!(c.state, cort::upgrade::ComponentState::Absent),
                "{c:?}"
            );
            assert!(
                c.detail.contains("extractor"),
                "debt recorded, not dropped: {c:?}"
            );
            assert!(
                !std::path::Path::new(&root_str).exists(),
                "no rebuild recreated the tree"
            );
        },
    );
}

/// An unreadable index is reported Unreadable and is NOT a verdict failure (spec §6: unreadable
/// never passes, but gone never fails; unreadable is reported). The verdict aggregation in
/// Task 5 decides what it does to the exit code — here we only pin the component state.
#[test]
fn an_unreadable_index_is_unreadable_and_reported() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(
        &[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))],
        || {
            // A junk *.db in the cache dir; list_projects already reports it Unreadable (plan 1).
            fs::write(cache.path().join("junk.db"), b"not a database").unwrap();
            let comps = cort::upgrade::migrate_indexes(false);
            let c = comps.iter().find(|c| c.name == "index_unreadable").unwrap();
            assert!(
                matches!(c.state, cort::upgrade::ComponentState::Unreadable),
                "{c:?}"
            );
        },
    );
}

/// --defer records the debt without rebuilding, even when the directory is live.
#[test]
fn defer_marks_without_rebuilding() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(
        &[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))],
        || {
            let (_dir, root, db_path) = indexed_project_in(cache.path());
            let db = cort::db::open_db(&db_path).unwrap();
            cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
            drop(db);
            let comps = cort::upgrade::migrate_indexes(true);
            let c = comps
                .iter()
                .find(|c| c.name == format!("index:{}", root.display()))
                .unwrap();
            assert!(
                matches!(c.state, cort::upgrade::ComponentState::Drifted),
                "{c:?}"
            );
            assert!(c.detail.contains("extractor"), "{c:?}");
            let db = cort::db::open_db(&db_path).unwrap();
            assert!(
                !cort::indexer::rebuild_reasons(&db).unwrap().is_empty(),
                "deferred means debt kept"
            );
        },
    );
}

/// A database that becomes unreadable BETWEEN rebuild and re-read must NOT read Current.
/// The deterministic seam is the primitive itself: a path that was readable and is now
/// garbage reads None, never Some(vec![]).
#[test]
fn a_post_rebuild_unreadable_db_is_not_empty_reasons() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(
        &[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))],
        || {
            let (_dir, _root, db_path) = indexed_project_in(cache.path());
            assert!(
                cort::upgrade::read_reasons_readonly(&db_path).is_some(),
                "precondition: readable"
            );
            fs::write(&db_path, b"garbage").unwrap();
            assert!(
                cort::upgrade::read_reasons_readonly(&db_path).is_none(),
                "garbage is not empty debt"
            );
        },
    );
}

// ── Task 5: the binary's library half — verdict, acks, marker, deadline, check path ──

fn comp(name: &str, state: cort::upgrade::ComponentState) -> cort::upgrade::Component {
    cort::upgrade::Component {
        name: name.to_string(),
        state,
        detail: "x".into(),
    }
}

/// The exit taxonomy is the spec §6 contract. A Drifted component is Partial (1), not Ok and
/// not Fatal — Fatal is reserved for "cannot proceed safely".
#[test]
fn drifted_components_are_partial_never_fatal() {
    let comps = vec![
        comp("shim", cort::upgrade::ComponentState::Drifted),
        comp("skill_xgrep", cort::upgrade::ComponentState::Current),
    ];
    let v = cort::upgrade::verdict(comps.clone(), &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Partial));
    // An acked drift becomes info: still printed (visibility), but not a failure.
    let v = cort::upgrade::verdict(comps, &["shim"]);
    assert!(
        matches!(v.exit, cort::upgrade::UpgradeExit::Ok),
        "acked drift must not fail the verdict"
    );
}

/// A gone directory is NOT a failure (spec §6: gone never fails) even though its reasons are
/// recorded. This test feeds Absent BY HAND — and it is a real assertion (not the vacuous one
/// the first draft shipped) because Task 4's gone-directory test proves the pipeline really
/// emits Absent for gone dirs; the two tests compose.
#[test]
fn deferred_and_gone_indexes_do_not_fail_the_verdict() {
    let comps = vec![comp(
        "index:/gone/project",
        cort::upgrade::ComponentState::Absent,
    )];
    let v = cort::upgrade::verdict(comps, &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Ok));
}

/// Unreadable DOES count against the verdict (spec §6: unreadable never passes) — but as
/// Partial, not Fatal.
#[test]
fn unreadable_counts_as_partial() {
    let comps = vec![comp(
        "index_unreadable",
        cort::upgrade::ComponentState::Unreadable,
    )];
    let v = cort::upgrade::verdict(comps, &[]);
    assert!(matches!(v.exit, cort::upgrade::UpgradeExit::Partial));
}

/// Ack persistence (spec §6: "該元件之後降為資訊列" — *afterwards*). `--ack shim` must work
/// on the NEXT invocation too, not just the current call. The store file is
/// `<cache>/.upgrade-acks`, one name per line — beside the lock files, never in the manifest
/// (manifest keys belong to install.sh) and never in usage.db (different owner).
#[test]
fn an_ack_survives_to_the_next_invocation() {
    let cache = tempfile::tempdir().unwrap();
    assert!(
        cort::upgrade::load_acks(cache.path()).is_empty(),
        "fresh cache acks nothing"
    );
    cort::upgrade::save_ack(cache.path(), "shim").unwrap();
    cort::upgrade::save_ack(cache.path(), "shim").unwrap(); // idempotent
    let acks = cort::upgrade::load_acks(cache.path());
    assert_eq!(acks, vec!["shim".to_string()]);
    let comps = vec![comp("shim", cort::upgrade::ComponentState::Drifted)];
    let borrowed: Vec<&str> = acks.iter().map(String::as_str).collect();
    let v = cort::upgrade::verdict(comps, &borrowed);
    assert!(
        matches!(v.exit, cort::upgrade::UpgradeExit::Ok),
        "a persisted ack quiets the next run"
    );
    // Corrupt ack store reads as EMPTY, never as a pass and never as a crash: an unreadable
    // memory must not silence real drift.
    fs::write(
        cache.path().join(".upgrade-acks"),
        b"\xff\xfe garbage \x00\n",
    )
    .unwrap();
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
    assert!(
        note.detail.contains("WAL"),
        "the risk must be named, not implied: {note:?}"
    );
    cort::upgrade::write_first_upgrade_marker(cache.path()).unwrap();
    assert!(cort::upgrade::first_upgrade_note(cache.path()).is_none());
}

/// The subprocess runner has a deadline: against a binary that sleeps 30s it returns Err
/// within 2s, not after the sleep. The plan's draft aimed this at `run_status_with_deadline`
/// with `/bin/sleep` "ignoring its argv" — measured false: `sleep hook-install` exits 1
/// immediately, so the old shape passed in 0.00s with no deadline in play at all (the
/// nonzero exit produced the Err, not the timeout). The deadline lives in
/// `run_capture_with_deadline`, so that is what gets the genuinely-hanging fixture:
/// `/bin/sleep 30` really does hang.
#[test]
fn a_hanging_status_subprocess_hits_the_deadline() {
    let start = std::time::Instant::now();
    let r = cort::upgrade::run_capture_with_deadline(
        std::path::Path::new("/bin/sleep"),
        &["30"],
        std::time::Duration::from_secs(2),
    );
    assert!(r.is_err(), "a 30s sleep must not survive a 2s deadline");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "deadline, not patience: {:?}",
        start.elapsed()
    );
}

/// `--check` never repairs, even when drifted — by CONSTRUCTION: `diagnose_for_check` takes
/// no repair callback, so there is nothing to call. What the test pins is the other half of
/// that property: a check that cannot repair must still REPORT (a blind check is worse than
/// none — it would print Current over drift).
#[test]
fn check_mode_reports_drift_it_cannot_repair() {
    let bad =
        "claude-code\tpre\twired\t/s\ttrusted=true\t/foreign hook-suggest --harness claude-code\n";
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
            new_binary: &install_root.path().join("nonexistent-new-binary"),
        },
        std::path::Path::new("/shim"),
        &|| Ok(bad.to_string()),
    );
    assert!(comps
        .iter()
        .any(|c| c.name == "hooks" && matches!(c.state, cort::upgrade::ComponentState::Drifted)));
}

/// `--check` completes while an upgrade holds the locks exclusive (diagnosis takes no locks
/// by construction). Passes trivially today — it pins the property against a future edit
/// that adds locking to the diagnose path, which would wedge every --check behind a drain.
#[test]
fn check_mode_completes_while_an_upgrade_holds_the_locks() {
    let cache = tempfile::tempdir().unwrap();
    let locks =
        cort::upgrade::acquire_upgrade_locks(cache.path(), std::time::Duration::from_secs(5))
            .unwrap();
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
            new_binary: &install_root.path().join("nonexistent-new-binary"),
        },
        std::path::Path::new("/shim"),
        &|| Ok(String::new()),
    );
    assert!(!comps.is_empty());
    drop(locks);
}

/// `diagnose()` (the `--check` path) applies the same gone policy as `migrate_indexes`:
/// a drifted index whose directory no longer exists reads Absent with the debt recorded,
/// never Drifted — spec §6's "gone never fails" has no mutating-route exception. A
/// diagnose that classified gone as Drifted would fail `--check` on every machine that
/// ever deleted an indexed project (Grok review round).
#[test]
fn a_gone_index_reads_absent_in_diagnose_too() {
    let cache = tempfile::tempdir().unwrap();
    with_vars(
        &[("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()))],
        || {
            let (dir, root, db_path) = indexed_project_in(cache.path());
            let db = cort::db::open_db(&db_path).unwrap();
            cort::db::set_meta(&db, "extractor_version", "superseded").unwrap();
            drop(db);
            let root_str = root.to_string_lossy().into_owned();
            drop(dir); // the directory is gone now
                       // DiagnoseInputs need paths for the OTHER components; those components are not
                       // what this test observes — only the per-project index entry.
            let scratch = tempfile::tempdir().unwrap();
            let comps = cort::upgrade::diagnose(&cort::upgrade::DiagnoseInputs {
                install_root: scratch.path(),
                new_pack: scratch.path(),
                installed_ast_grep_version: cort::install::AST_GREP_PINNED,
                new_tree: scratch.path(),
                home: scratch.path(),
                keep_mine: false,
                new_binary: &scratch.path().join("nonexistent-new-binary"),
            });
            let c = comps
                .iter()
                .find(|c| c.name == format!("index:{root_str}"))
                .unwrap();
            assert!(
                matches!(c.state, cort::upgrade::ComponentState::Absent),
                "gone must not fail --check either: {c:?}"
            );
            assert!(c.detail.contains("extractor"), "debt recorded: {c:?}");
        },
    );
}

// ── Codex review round: repair targets the OWNED skill at the path diagnosis looked ──

/// An unmanaged divergence is install.sh --force's decision, never an upgrade's. The first
/// draft decided this from detail prose — `contains("managed")` also matches "unmanaged", so
/// a user's own diverged, unstamped skill would be overwritten AND claimed by a stamp. The
/// decision reads the stamp on disk now.
#[test]
fn an_unmanaged_diverged_skill_has_no_repair_target() {
    let tree = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(tree.path().join("skills/ast-grep")).unwrap();
    fs::write(tree.path().join("skills/ast-grep/SKILL.md"), "new bytes\n").unwrap();
    let dest = home.path().join(".claude/skills/ast-grep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "the user's own diverged skill\n").unwrap();
    // No stamp: unmanaged.
    let comps = cort::upgrade::check_skills_at(tree.path(), &dest, &dest, &dest, false);
    let c = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
    assert!(matches!(c.state, cort::upgrade::ComponentState::Drifted));
    assert!(
        cort::upgrade::skill_repair_target(c, tree.path(), home.path()).is_none(),
        "unmanaged drift is --force's decision, not an upgrade's: {c:?}"
    );
}

/// A managed drifted skill repairs at the exact destination diagnosis read — same source,
/// same dest.
#[test]
fn a_managed_diverged_skill_repairs_at_the_destination_diagnosis_read() {
    let tree = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(tree.path().join("skills/xgrep")).unwrap();
    fs::write(tree.path().join("skills/xgrep/SKILL.md"), "new bytes\n").unwrap();
    let dest = home.path().join(".claude/skills/xgrep/SKILL.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "old managed bytes\n").unwrap();
    fs::write(dest.parent().unwrap().join(".cortexyoung-managed"), b"").unwrap();
    let comps = cort::upgrade::check_skills_at(
        tree.path(),
        &home.path().join(".claude/skills/xgrep/SKILL.md"),
        &dest,
        &dest,
        false,
    );
    let c = comps.iter().find(|c| c.name == "skill_xgrep").unwrap();
    let (src, got_dest) = cort::upgrade::skill_repair_target(c, tree.path(), home.path())
        .expect("managed drift IS repairable");
    assert_eq!(src, tree.path().join("skills/xgrep/SKILL.md"));
    assert_eq!(got_dest, dest);
}

/// Repair must write where diagnosis looked: check_skills honours CLAUDE_SKILL_HOME and
/// CODEX_HOME; a repair hard-coding the default home would miss the real (overridden) copy —
/// and could overwrite an unrelated file at the default path (Codex round).
#[test]
fn skill_repair_honours_the_skill_home_overrides() {
    let tree = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let claude_skill_home = tempfile::tempdir().unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    fs::create_dir_all(tree.path().join("skills/ast-grep")).unwrap();
    fs::write(tree.path().join("skills/ast-grep/SKILL.md"), "new bytes\n").unwrap();
    let overridden = claude_skill_home.path().join("skills/ast-grep/SKILL.md");
    fs::create_dir_all(overridden.parent().unwrap()).unwrap();
    fs::write(&overridden, "old managed bytes\n").unwrap();
    fs::write(
        overridden.parent().unwrap().join(".cortexyoung-managed"),
        b"",
    )
    .unwrap();
    with_vars(
        &[
            (
                "CLAUDE_SKILL_HOME",
                Some(claude_skill_home.path().to_str().unwrap()),
            ),
            ("CODEX_HOME", Some(codex_home.path().to_str().unwrap())),
        ],
        || {
            let comps = cort::upgrade::check_skills(tree.path(), home.path(), false);
            let c = comps.iter().find(|c| c.name == "skill_ast_grep").unwrap();
            assert!(
                matches!(c.state, cort::upgrade::ComponentState::Drifted),
                "diagnosis reads the overridden home: {c:?}"
            );
            let (_src, dest) = cort::upgrade::skill_repair_target(c, tree.path(), home.path())
                .expect("the overridden copy is managed and drifted");
            assert_eq!(
                dest, overridden,
                "repair must target the overridden path diagnosis read"
            );
        },
    );
}

// ── binary content: the component that notices when the PAYLOAD CODE changed ──

/// Hook copy lives in the cort binary, not in the pack or the shim. Without this component a
/// code-only change (copy, judgement, bugfix) produced a new tree binary while diagnose read
/// shim/pack/version all Current and exited 0 - the new copy could never reach the machine
/// (found deploying the P2/P4 hook copy: generation did not flip, installed and tree binaries
/// hashed differently, and the upgrade declared everything current).
#[test]
fn a_binary_content_drift_is_drifted_even_when_shim_and_pack_match() {
    use cort::upgrade::{ComponentState, DiagnoseInputs};
    let ((_r, _h, _b), _root, home, cort_home, _bin) = installed_root();
    let install_root = home.join("cortexyoung");
    // Pack identical both sides; only the binary bytes differ.
    fs::write(cort_home.join("pack/r.yml"), "id: a\nlanguage: ts\n").unwrap();
    let new_pack = tempfile::tempdir().unwrap();
    fs::write(new_pack.path().join("r.yml"), "id: a\nlanguage: ts\n").unwrap();
    fs::write(cort_home.join("cort"), "#!/bin/sh\nold payload\n").unwrap();
    let new_bin = tempfile::tempdir().unwrap();
    fs::write(new_bin.path().join("cort"), "#!/bin/sh\nnew payload\n").unwrap();
    let comps = cort::upgrade::diagnose(&DiagnoseInputs {
        install_root: &install_root,
        new_pack: new_pack.path(),
        installed_ast_grep_version: cort::install::AST_GREP_PINNED,
        new_tree: &_root,
        home: &home,
        keep_mine: false,
        new_binary: new_bin.path().join("cort").as_path(),
    });
    let b = comps.iter().find(|c| c.name == "binary").unwrap();
    assert!(
        matches!(b.state, ComponentState::Drifted),
        "different binary bytes must be Drifted: {b:?}"
    );
    assert!(b.detail.contains("installed"), "{b:?}");
}

/// The mirror: identical bytes read Current, so the steady state keeps its right to do
/// nothing.
#[test]
fn an_identical_binary_is_current() {
    use cort::upgrade::{ComponentState, DiagnoseInputs};
    let ((_r, _h, _b), _root, home, cort_home, _bin) = installed_root();
    let install_root = home.join("cortexyoung");
    fs::write(cort_home.join("pack/r.yml"), "id: a\nlanguage: ts\n").unwrap();
    let new_pack = tempfile::tempdir().unwrap();
    fs::write(new_pack.path().join("r.yml"), "id: a\nlanguage: ts\n").unwrap();
    fs::write(cort_home.join("cort"), "#!/bin/sh\nsame payload\n").unwrap();
    let new_bin = tempfile::tempdir().unwrap();
    fs::write(new_bin.path().join("cort"), "#!/bin/sh\nsame payload\n").unwrap();
    let comps = cort::upgrade::diagnose(&DiagnoseInputs {
        install_root: &install_root,
        new_pack: new_pack.path(),
        installed_ast_grep_version: cort::install::AST_GREP_PINNED,
        new_tree: &_root,
        home: &home,
        keep_mine: false,
        new_binary: new_bin.path().join("cort").as_path(),
    });
    let b = comps.iter().find(|c| c.name == "binary").unwrap();
    assert!(matches!(b.state, ComponentState::Current), "{b:?}");
}
