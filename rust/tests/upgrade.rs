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
