//! The heal lock is the single-flight gate for background self-healing: two stale queries on a
//! big repo must not spawn two rebuilders.
//!
//! Death-release is deliberately not re-proven here. The lock is the same flock(2) primitive
//! as `upgrade.rs`, released by close(2) via RAII and reclaimed by the kernel when a holder
//! dies; `a_killed_holder_releases_the_locks` (tests/upgrade.rs) pins exactly that guarantee
//! for this primitive, and a second copy of the test would prove the kernel twice.

use std::path::PathBuf;

fn lock_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(".heal-fixture.lock")
}

#[test]
fn a_held_heal_lock_excludes_until_released() {
    let dir = tempfile::tempdir().unwrap();
    let path = lock_path(&dir);

    let guard = cort::heal::try_heal_lock(&path).expect("the first caller takes the lock");
    assert!(
        cort::heal::try_heal_lock(&path).is_err(),
        "a second caller while held must be excluded"
    );
    assert!(
        cort::heal::heal_in_flight(&path),
        "the probe must see the holder"
    );
    drop(guard);

    assert!(
        !cort::heal::heal_in_flight(&path),
        "release is close(2) via RAII drop: {path:?}"
    );
    cort::heal::try_heal_lock(&path).expect("the lock is reusable after the holder releases it");
}

#[test]
fn a_lock_that_cannot_be_created_is_reported_not_silent() {
    // A plain file where the directory belongs is the machine-is-broken case, and the error
    // must say so rather than read as contention -- the same wrong-diagnosis split
    // `LockError::LockFileUnavailable` exists for on the upgrade side.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let path = blocker.join(".heal.lock");
    match cort::heal::try_heal_lock(&path) {
        Err(cort::upgrade::LockError::LockFileUnavailable) => {}
        other => panic!("expected LockFileUnavailable, got {other:?}"),
    }
    assert!(
        !cort::heal::heal_in_flight(&path),
        "an unopenable lock file is not evidence of a running healer"
    );
}
