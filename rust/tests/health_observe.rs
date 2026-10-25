//! D1 of `docs/superpowers/plans/2026-10-24-event-driven-collection.md`: the read-only open an
//! observation must use, the repair label lifted out of `impact`, and the `fresh` label that
//! observation rows share with claudecat's long-term metric table.
//!
//! `impact_repair_unchanged` is not a test in this file: it is the existing `impact` suite staying
//! green over the refactor (`rust/tests/impact.rs`, `the_packet_reports_index_staleness` and the
//! four `repair` assertions).

use cort::db::{ensure_schema, open_db, open_db_readonly};
use cort::health::fresh_label;
use cort::staleness::{repair_label, StaleReport};
use rusqlite::ErrorCode;
use std::path::Path;

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
