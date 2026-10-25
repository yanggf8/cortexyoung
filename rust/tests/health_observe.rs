//! D1 and D2 of `docs/superpowers/plans/2026-10-24-event-driven-collection.md`: the read-only
//! open an observation must use, the repair label lifted out of `impact`, the `fresh` label that
//! observation rows share with claudecat's long-term metric table, and the observation itself.
//!
//! `impact_repair_unchanged` is not a test in this file: it is the existing `impact` suite staying
//! green over the refactor (`rust/tests/impact.rs`, `the_packet_reports_index_staleness` and the
//! four `repair` assertions).

use cort::collect::CollectConfig;
use cort::db::{ensure_schema, open_db, open_db_readonly, project_id_for};
use cort::health::{bounded, fresh_label, observe, observe_segmented, Observation, Stage};
use cort::staleness::{repair_label, StaleReport};
use rusqlite::ErrorCode;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The `repair` vocabulary is defined once, so every case here is a case `impact` also answers.
fn report(stale: bool, reasons: &[&str], narrowed: bool) -> StaleReport {
    StaleReport {
        index_is_stale: stale,
        deleted_files: Vec::new(),
        changed_files: Vec::new(),
        rebuild_required: reasons.iter().map(|r| (*r).to_string()).collect(),
        candidates_narrowed: narrowed,
    }
}

fn reasons(names: &[&str]) -> Vec<String> {
    names.iter().map(|r| (*r).to_string()).collect()
}

#[test]
fn repair_label_matches_each_case() {
    // Nothing owed: a fresh index has no staleness to classify.
    assert_eq!(repair_label(&report(false, &[], true)), "none");
    // A dirty git tree the incremental path accepts; the PostToolUse hook heals it.
    assert_eq!(repair_label(&report(true, &[], true)), "refreshable");
    // A stored-version reason: the hook refuses and only a foreground `cort index` fixes it.
    assert_eq!(
        repair_label(&report(true, &["extractor_changed"], true)),
        "rebuild_required"
    );
    // Unnarrowed candidates refuse for a reason that is not in `rebuild_required` at all.
    assert_eq!(repair_label(&report(true, &[], false)), "rebuild_required");
}

#[test]
fn fresh_label_covers_every_label() {
    let none = reasons(&[]);
    // Readable graph state says fresh; unreadable says `fresh?` rather than claiming either.
    assert_eq!(
        fresh_label(Some("none"), &none, false, Some(false)),
        "fresh"
    );
    assert_eq!(fresh_label(Some("none"), &none, false, None), "fresh?");
    // Stale with nothing rebuild-shaped and no graph fact to sharpen it.
    assert_eq!(fresh_label(Some("none"), &none, true, None), "STALE");
    // A pending graph outranks plain staleness, as it does in `row_md`.
    assert_eq!(
        fresh_label(Some("none"), &none, true, Some(true)),
        "STALE/graph"
    );
    assert_eq!(
        fresh_label(Some("refreshable"), &none, true, Some(false)),
        "STALE/refresh"
    );
    assert_eq!(
        fresh_label(
            Some("rebuild_required"),
            &reasons(&["schema_changed"]),
            true,
            Some(false)
        ),
        "STALE/rebuild"
    );
    // A pending graph read from meta with no rebuild reasons in hand (they could not be computed)
    // still says graph, whatever the staleness verdict.
    assert_eq!(fresh_label(None, &none, false, Some(true)), "STALE/graph");
    // `no-index` is not a label here: the row's `status` carries it (design §3).
    assert_eq!(
        fresh_label(
            Some("rebuild_required"),
            &reasons(&["graph_incomplete"]),
            true,
            Some(true)
        ),
        "STALE/graph"
    );
}

#[test]
fn graph_reason_alone_is_stale_graph() {
    // `graph_pending` always adds `graph_incomplete` to the rebuild reasons, and any reason is
    // `rebuild_required` -- so a rebuild owed only for the graph must not read as a full rebuild.
    assert_eq!(
        fresh_label(
            Some("rebuild_required"),
            &reasons(&["graph_incomplete"]),
            true,
            Some(true)
        ),
        "STALE/graph"
    );
    // One other reason alongside it is a real rebuild: the graph reason cannot mask it.
    assert_eq!(
        fresh_label(
            Some("rebuild_required"),
            &reasons(&["graph_incomplete", "schema_changed"]),
            true,
            Some(true)
        ),
        "STALE/rebuild"
    );
}

#[test]
fn open_db_readonly_creates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("nested").join("deeper").join("index.db");

    let err = open_db_readonly(&missing).expect_err("a missing index cannot be opened read-only");
    assert!(
        matches!(err, rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::CannotOpen),
        "expected SQLITE_CANTOPEN, got {err:?}"
    );
    assert!(!missing.exists(), "the read-only open created the file");
    assert!(
        !missing.parent().expect("parent").exists(),
        "the read-only open created the parent directory"
    );

    // A missing parent fails with or without `CREATE`, so it cannot tell the two apart. Only a
    // missing file in an existing directory does: with `CREATE` that opens as a new empty index.
    let beside = dir.path().join("index.db");
    let err = open_db_readonly(&beside).expect_err("a missing index beside others cannot open");
    assert!(
        matches!(err, rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::CannotOpen),
        "expected SQLITE_CANTOPEN, got {err:?}"
    );
    assert!(!beside.exists(), "the read-only open created the file");
}

#[test]
fn open_db_readonly_refuses_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("index.db");
    {
        let db = open_db(&path).expect("open_db");
        ensure_schema(&db).expect("ensure_schema");
    }
    let before = std::fs::read(&path).expect("read the index");

    let db = open_db_readonly(&path).expect("the existing index opens read-only");
    // Reading is what the observation does, and it must work through this handle.
    let rows: i64 = db
        .query_row("SELECT count(*) FROM _cortex_meta", [], |r| r.get(0))
        .expect("a read succeeds on the read-only handle");
    assert!(rows >= 1, "the schema's own meta rows are readable");

    let err = db
        .execute(
            "INSERT INTO _cortex_meta (key, value) VALUES ('health_probe', '1')",
            [],
        )
        .expect_err("a write through the read-only handle must be refused");
    assert!(
        matches!(err, rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ReadOnly),
        "expected SQLITE_READONLY, got {err:?}"
    );

    drop(db);
    assert_eq!(
        std::fs::read(&path).expect("read the index again"),
        before,
        "the read-only handle changed the index bytes"
    );
    // SQLite itself creates the WAL sidecars beside a WAL-mode database it opens, even read-only
    // (`db::open_db_readonly`'s doc says so); what must not happen is the *index* changing. The
    // observation path is expected to see this pair, so it is pinned here rather than left to be
    // discovered as a surprise in D2.
    let mut names = entry_names(dir.path());
    names.sort();
    assert_eq!(
        names,
        vec!["index.db", "index.db-shm", "index.db-wal"],
        "the read-only open left something other than SQLite's own WAL sidecars"
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("index.db-wal"))
            .expect("wal metadata")
            .len(),
        0,
        "the sidecar WAL carries bytes, so something wrote through the read-only handle"
    );
}

fn entry_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// D2: observing one index, read-only (design §3).
//
// `observe` runs inside this test process, so `CORT_CACHE_DIR` (where it looks the index up) has
// to be set here rather than only on a child process. The same binary's tests run in parallel
// threads and the environment is process-global, so every test that observes holds `ENV_LOCK`
// for its whole body: unserialised, two tests would point `observe` at one another's cache.
// ---------------------------------------------------------------------------------------------

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn cfg() -> CollectConfig {
    CollectConfig::from_lookup(|_| None)
}

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

const SAMPLE: &[(&str, &str)] = &[
    (
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    ),
    (
        "src/alpha.ts",
        "import { helper } from './helper';\n\
export function alpha(a: number) { return helper(a) + 1; }\n",
    ),
];

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-observe-")
        .tempdir()
        .expect("tempdir");
    for (rel, body) in SAMPLE {
        let abs = dir.path().join(rel);
        fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        fs::write(&abs, body).expect("write sample file");
    }
    let root = fs::canonicalize(dir.path()).expect("canonicalize the project");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "t@e.com"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    (dir, root)
}

/// Point the test process's cache and data dirs at fresh temporary directories, run `f`, then put
/// every value back — including on panic, so a failing test cannot poison the ones after it.
fn in_sandbox<R>(cache: &Path, data: &Path, extra: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
    let _guard = env_guard();
    let mut keys: Vec<String> = vec!["CORT_CACHE_DIR".into(), "CORT_MEMORY_DIR".into()];
    keys.extend(extra.iter().map(|(k, _)| (*k).to_string()));
    let prev: Vec<(String, Option<std::ffi::OsString>)> = keys
        .iter()
        .map(|k| (k.clone(), std::env::var_os(k)))
        .collect();
    // SAFETY (edition-2024 terms; in this 2021 crate `set_var` is still a safe fn): every test that
    // changes these variables holds ENV_LOCK for its whole body, and the only code that reads them
    // runs under that lock. Rust reads elsewhere in this binary -- `tempfile` reading TMPDIR,
    // `Command` copying the environment on spawn -- go through std's own environment lock and never
    // see a torn write. What no lock covers is foreign code calling `getenv` on another thread
    // (SQLite's unix VFS can, for temp files); that is the exposure every environment-setting test
    // in this repo already has, not one this file adds.
    unsafe {
        std::env::set_var("CORT_CACHE_DIR", cache);
        std::env::set_var("CORT_MEMORY_DIR", data);
        for (k, v) in extra {
            std::env::set_var(k, v);
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        for (k, v) in &prev {
            match v {
                Some(value) => std::env::set_var(k, value),
                None => std::env::remove_var(k),
            }
        }
    }
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// `cort index` in the sandbox, exactly as `tests/cli.rs` builds one: the child inherits the
/// cache directory this process set, so the binary and `observe` agree on where the index lives.
fn build_index(root: &Path) -> Output {
    let out = Command::new(cort_bin())
        .arg("index")
        .current_dir(root)
        .output()
        .expect("spawn cort index");
    assert!(
        out.status.success(),
        "cort index failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn index_path(cache: &Path, project_id: &str) -> PathBuf {
    cache.join(format!("{project_id}.db"))
}

fn ok_observation() -> Observation {
    Observation {
        status: "ok",
        error: None,
        fresh: Some("fresh".to_string()),
        chunks: Some(1),
        relationships: Some(0),
        not_chunked: Some(0),
        real_gap: Some(0),
        fts: Some("synced".to_string()),
    }
}

#[test]
fn fresh_index_observed_ok() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "ok", "{obs:?}");
        assert_eq!(obs.error, None, "{obs:?}");
        assert_eq!(obs.fresh.as_deref(), Some("fresh"), "{obs:?}");
        assert!(obs.chunks.unwrap_or(0) >= 1, "{obs:?}");
        assert!(obs.relationships.is_some(), "{obs:?}");
        // Both sample files are parsed, so nothing is "declared but not chunked" and the real gap
        // is zero -- 0 is a fact here, not a failed read.
        assert_eq!(obs.not_chunked, Some(0), "{obs:?}");
        assert_eq!(obs.real_gap, Some(0), "{obs:?}");
        assert_eq!(obs.fts.as_deref(), Some("synced"), "{obs:?}");
    });
}

#[test]
fn no_age_gate() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        {
            let db = open_db(&index).expect("open the index for the fixture write");
            db.execute(
                "UPDATE projects SET last_indexed_at = ?1",
                [1_000_000_000_000i64],
            )
            .expect("rewrite last_indexed_at");
        }
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.fresh.as_deref(), Some("fresh"), "{obs:?}");
    });
}

#[test]
fn edited_file_is_stale_refresh() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let edited = root.join("src/alpha.ts");
        let mut body = fs::read_to_string(&edited).expect("read alpha");
        body.push_str("export function gamma() { return 3; }\n");
        fs::write(&edited, body).expect("edit alpha");
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.fresh.as_deref(), Some("STALE/refresh"), "{obs:?}");
    });
}

#[test]
fn observation_never_writes_the_index() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        let bytes_before = fs::read(&index).expect("read the index");
        let mtime_before = fs::metadata(&index)
            .expect("stat the index")
            .modified()
            .expect("mtime");
        let names_before = entry_names(cache.path());

        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "ok", "{obs:?}");

        assert_eq!(
            fs::read(&index).expect("read the index again"),
            bytes_before,
            "the observation changed the index bytes"
        );
        assert_eq!(
            fs::metadata(&index)
                .expect("stat the index again")
                .modified()
                .expect("mtime"),
            mtime_before,
            "the observation touched the index's mtime"
        );

        // SQLite creates the wal-index beside a WAL database on open; nothing else may appear.
        let mut new_names: Vec<String> = entry_names(cache.path())
            .into_iter()
            .filter(|n| !names_before.contains(n))
            .collect();
        new_names.sort();
        let index_name = index
            .file_name()
            .expect("index name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            new_names,
            vec![format!("{index_name}-shm"), format!("{index_name}-wal")],
            "the observation created something other than SQLite's own WAL sidecars"
        );
        assert_eq!(
            fs::metadata(cache.path().join(format!("{index_name}-wal")))
                .expect("wal metadata")
                .len(),
            0,
            "a read-only observation left bytes in the WAL"
        );
    });
}

#[test]
fn missing_index_is_no_index() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "no_index", "{obs:?}");
        assert_eq!(obs.fresh, None, "{obs:?}");
        assert_eq!(obs.chunks, None, "{obs:?}");
    });
}

#[test]
fn missing_dir_is_missing_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let gone = dir.path().join("deleted-project");
    let project_id = project_id_for(gone.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        // The directory is checked before the index: when both are gone, the directory's absence
        // is the fact that explains the other one (`health::observe`'s doc says so).
        let obs = observe(&gone, &project_id, &cfg());
        assert_eq!(obs.status, "missing_dir", "{obs:?}");
        assert_eq!(obs.fresh, None, "{obs:?}");
        assert_eq!(obs.chunks, None, "{obs:?}");
    });
}

#[test]
fn unreadable_index_records_error() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        fs::write(
            index_path(cache.path(), &project_id),
            b"not a database at all",
        )
        .expect("write garbage");
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert!(
            obs.error.as_deref().is_some_and(|e| !e.is_empty()),
            "an unreadable index must say why: {obs:?}"
        );
        assert_eq!(obs.fresh, None, "{obs:?}");
        assert_eq!(obs.chunks, None, "{obs:?}");
        assert_eq!(obs.relationships, None, "{obs:?}");
        assert_eq!(obs.not_chunked, None, "{obs:?}");
        assert_eq!(obs.real_gap, None, "{obs:?}");
        assert_eq!(obs.fts, None, "{obs:?}");
    });
}

#[test]
fn large_tree_skips_staleness() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(
        cache.path(),
        data.path(),
        &[("CORT_HEAL_MAX_FILES", "0")],
        || {
            build_index(&root);
            let obs = observe(&root, &project_id, &cfg());
            assert_eq!(obs.status, "ok", "{obs:?}");
            assert_eq!(obs.fresh, None, "{obs:?}");
            assert_eq!(
                obs.error.as_deref(),
                Some("staleness_skipped_large_tree"),
                "{obs:?}"
            );
            // The skipped phase must not cost the metrics that were readable anyway.
            assert!(obs.chunks.unwrap_or(0) >= 1, "{obs:?}");
            assert_eq!(obs.fts.as_deref(), Some("synced"), "{obs:?}");
        },
    );
}

#[test]
fn graph_pending_index_is_stale_graph() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        {
            // The stored fact the observation reads (`_cortex_meta.graph_pending`, the key
            // `read_meta` reads); written with the ordinary writable open and closed before the
            // observation so no writer is alive while it looks.
            let db = open_db(index_path(cache.path(), &project_id)).expect("open the index");
            cort::db::set_meta(&db, "graph_pending", "1").expect("set graph_pending");
        }
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.fresh.as_deref(), Some("STALE/graph"), "{obs:?}");
    });
}

#[test]
fn bounded_times_out() {
    let started = Instant::now();
    let obs = bounded(
        || {
            std::thread::sleep(Duration::from_secs(2));
            ok_observation()
        },
        Duration::from_millis(100),
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a timed-out observation returned after {:?}",
        started.elapsed()
    );
    assert_eq!(obs.status, "unreadable", "{obs:?}");
    assert_eq!(obs.error.as_deref(), Some("timeout"), "{obs:?}");
    assert_eq!(obs.fresh, None, "{obs:?}");
    assert_eq!(obs.chunks, None, "{obs:?}");
}

#[test]
fn index_deleted_mid_observation_is_recorded() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        // The injection point is the segmented observation's callback, which runs after the chunks
        // have been read -- the index disappears under a reader that already holds a handle.
        let obs = observe_segmented(&root, &project_id, &cfg(), |stage| {
            if stage != Stage::AfterChunks {
                return;
            }
            fs::remove_file(&index).expect("delete the index mid-observation");
        });
        assert!(
            obs.status == "unreadable" || obs.status == "no_index",
            "a deleted index must be recorded, not read as healthy: {obs:?}"
        );
        assert_ne!(obs.status, "ok", "{obs:?}");
    });
}

/// A rebuild that puts a new file at the index path while the observation still reads the old one
/// is recorded, not read as a healthy snapshot of a database nobody uses (design Review Focus 4).
#[cfg(unix)]
#[test]
fn index_replaced_mid_observation_is_unreadable() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        let obs = observe_segmented(&root, &project_id, &cfg(), |stage| {
            if stage != Stage::AfterChunks {
                return;
            }
            // Same bytes, new file: only the identity changes, which is the case a path check
            // alone cannot see.
            let copy = index.with_extension("rebuilt");
            fs::copy(&index, &copy).expect("copy the index");
            fs::rename(&copy, &index).expect("replace the index");
        });
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert_eq!(
            obs.error.as_deref(),
            Some("index_replaced_during_observation"),
            "{obs:?}"
        );
    });
}

/// An empty file is a valid empty SQLite database, so it passes the probe; it is still not an
/// index, and eight NULLs under `ok` would hide that.
#[test]
fn an_empty_file_is_not_an_index() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        fs::write(index_path(cache.path(), &project_id), b"").expect("write an empty file");
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert!(
            obs.error
                .as_deref()
                .is_some_and(|e| e.starts_with("chunks_unreadable")),
            "{obs:?}"
        );
        assert_eq!(obs.chunks, None, "{obs:?}");
        assert_eq!(obs.fts, None, "{obs:?}");
    });
}

/// A staleness phase that fails while walking the tree is a read error on a tree that moved
/// (design §3: `unreadable` with the reason); the cells read before it were real and stay.
#[cfg(unix)]
#[test]
fn a_staleness_failure_is_unreadable_and_keeps_what_was_read() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        // An edit makes the file a staleness candidate; removing read permission makes reading it
        // fail, the way a file being rewritten or re-permissioned mid-walk does.
        let edited = root.join("src/alpha.ts");
        fs::write(&edited, "export function alpha() { return 0; }\n").expect("edit alpha");
        fs::set_permissions(&edited, fs::Permissions::from_mode(0o000)).expect("chmod 000");
        if fs::read(&edited).is_ok() {
            eprintln!("SKIP: running with privileges that read a mode-000 file");
            return;
        }
        let obs = observe(&root, &project_id, &cfg());
        fs::set_permissions(&edited, fs::Permissions::from_mode(0o644)).expect("chmod back");
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert!(
            obs.error
                .as_deref()
                .is_some_and(|e| e.starts_with("staleness_failed")),
            "{obs:?}"
        );
        assert_eq!(obs.fresh, None, "{obs:?}");
        assert!(obs.chunks.unwrap_or(0) >= 1, "{obs:?}");
        assert_eq!(obs.fts.as_deref(), Some("synced"), "{obs:?}");
    });
}

/// The read-only open is the observation's only open. On a WAL index the writable `open_db` leaves
/// no trace a byte comparison can see, so this fixture is a rollback-journal index: `open_db`
/// would rewrite its header to WAL, and the bytes would change.
#[test]
fn observation_does_not_convert_a_rollback_journal_index() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        {
            let db = rusqlite::Connection::open(&index).expect("open the fixture");
            let mode: String = db
                .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
                .expect("switch to a rollback journal");
            assert_eq!(mode, "delete", "the fixture must leave WAL mode");
        }
        let before = fs::read(&index).expect("read the index");
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "ok", "{obs:?}");
        assert_eq!(
            fs::read(&index).expect("read the index again"),
            before,
            "the observation rewrote a rollback-journal index"
        );
    });
}

/// The second move check: a rebuild that lands during the staleness walk -- the longest stretch of
/// an observation -- is caught before the answer, not only one that lands right after the chunks.
#[cfg(unix)]
#[test]
fn index_replaced_before_the_answer_is_unreadable() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        let obs = observe_segmented(&root, &project_id, &cfg(), |stage| {
            if stage != Stage::BeforeAnswer {
                return;
            }
            let copy = index.with_extension("rebuilt");
            fs::copy(&index, &copy).expect("copy the index");
            fs::rename(&copy, &index).expect("replace the index");
        });
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert_eq!(
            obs.error.as_deref(),
            Some("index_replaced_during_observation"),
            "{obs:?}"
        );
        assert_eq!(
            obs.chunks, None,
            "nothing read from a replaced index is carried: {obs:?}"
        );
    });
}

#[test]
fn index_deleted_before_the_answer_is_no_index() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        let obs = observe_segmented(&root, &project_id, &cfg(), |stage| {
            if stage == Stage::BeforeAnswer {
                fs::remove_file(&index).expect("delete the index before the answer");
            }
        });
        assert_eq!(obs.status, "no_index", "{obs:?}");
        assert_eq!(
            obs.error.as_deref(),
            Some("index_deleted_during_observation"),
            "{obs:?}"
        );
        assert_eq!(obs.chunks, None, "{obs:?}");
    });
}

/// Design §3: a NULL is "could not read it" with the reason in `status`/`error`. A cell that fails
/// on its own leaves the observation `ok` and is named, so it cannot pass for any other NULL.
#[test]
fn a_cell_that_fails_alone_is_named() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        {
            let db = open_db(index_path(cache.path(), &project_id)).expect("open the index");
            db.execute_batch("DROP TABLE chunks_fts")
                .expect("drop the FTS table");
        }
        let obs = observe(&root, &project_id, &cfg());
        assert_eq!(obs.status, "ok", "{obs:?}");
        assert_eq!(obs.fts, None, "{obs:?}");
        assert!(obs.chunks.unwrap_or(0) >= 1, "{obs:?}");
        assert_eq!(
            obs.error.as_deref(),
            Some("cells_unreadable: fts"),
            "{obs:?}"
        );
    });
}

/// "Could not look" is not "not there": a project directory that cannot be stat'ed is recorded as
/// unreadable, not as a project that went away.
#[cfg(unix)]
#[test]
fn an_unstatable_root_is_unreadable_not_missing() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().expect("parent");
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let root = parent.path().join("project");
    fs::create_dir(&root).expect("create the project");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o000)).expect("chmod 000");
    if fs::metadata(&root).is_ok() {
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o755)).expect("chmod back");
        eprintln!("SKIP: running with privileges that stat through a mode-000 directory");
        return;
    }
    let obs = in_sandbox(cache.path(), data.path(), &[], || {
        observe(&root, &project_id, &cfg())
    });
    fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o755)).expect("chmod back");
    assert_eq!(obs.status, "unreadable", "{obs:?}");
    assert!(
        obs.error
            .as_deref()
            .is_some_and(|e| e.starts_with("root_unstatable")),
        "{obs:?}"
    );
}

/// `cort index` rebuilds in place -- one transaction on the same file, so the inode never changes.
/// A rebuild that commits while the observation reads is recorded, not mixed into an `ok` row.
#[test]
fn an_in_place_rebuild_mid_observation_is_unreadable() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let obs = observe_segmented(&root, &project_id, &cfg(), |stage| {
            if stage != Stage::AfterChunks {
                return;
            }
            // A real edit and a real `cort index`, the way an agent's session produces one.
            let edited = root.join("src/helper.ts");
            fs::write(
                &edited,
                "export function helper(n: number) { return n * 3; }\n",
            )
            .expect("edit helper");
            build_index(&root);
        });
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert_eq!(
            obs.error.as_deref(),
            Some("index_written_during_observation"),
            "{obs:?}"
        );
    });
}

/// Any other connection's commit counts, not only a rebuild: the edit hook's incremental is the
/// common one, and it moves the index the staleness verdict was judging.
#[test]
fn a_commit_by_another_connection_mid_observation_is_unreadable() {
    let (_dir, root) = project();
    let cache = tempfile::tempdir().expect("cache");
    let data = tempfile::tempdir().expect("data");
    let project_id = project_id_for(root.to_str().expect("utf8 root"));
    in_sandbox(cache.path(), data.path(), &[], || {
        build_index(&root);
        let index = index_path(cache.path(), &project_id);
        let obs = observe_segmented(&root, &project_id, &cfg(), |stage| {
            if stage == Stage::BeforeAnswer {
                let db = open_db(&index).expect("open a second connection");
                cort::db::set_meta(&db, "health_probe", "1").expect("commit from it");
            }
        });
        assert_eq!(obs.status, "unreadable", "{obs:?}");
        assert_eq!(
            obs.error.as_deref(),
            Some("index_written_during_observation"),
            "{obs:?}"
        );
    });
}
