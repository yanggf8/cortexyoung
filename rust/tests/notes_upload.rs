use cort::memory_store;
use cort::notes::{ScanReport, ScanStatus, ScannedNote};
use cort::notes_upload::{self, ExistingRow};
use cort::remote::{Cell, RemoteDb, RemoteError, SqliteTransport, Transport};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

fn row(dir: &str, file: &str, status: ScanStatus, body: Option<&str>) -> ScannedNote {
    ScannedNote {
        encoded_dir_name: dir.into(),
        file_name: file.into(),
        status,
        bytes: body.map(|s| s.as_bytes().to_vec()),
        sha256: body.map(|s| format!("hash-{s}")),
    }
}

fn listed(dirs: &[&str]) -> BTreeSet<String> {
    dirs.iter().map(|dir| dir.to_string()).collect()
}

fn fixture() -> (tempfile::TempDir, BTreeSet<String>) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("project")).unwrap();
    (temp, BTreeSet::from(["p".into()]))
}

fn prepare(
    report: &ScanReport,
    root: &std::path::Path,
    opted: &BTreeSet<String>,
) -> notes_upload::Prepared {
    notes_upload::prepare(report, root, opted, |_| Ok("p".into()))
}

fn existing(dir: &str, file: &str, sha: &str, deleted: bool) -> ExistingRow {
    ExistingRow {
        project: "p".into(),
        source_dir: dir.into(),
        file_name: file.into(),
        sha256: sha.into(),
        deleted,
    }
}

struct CountingTransport {
    sqlite: SqliteTransport,
    posts: AtomicUsize,
    batches: AtomicUsize,
    selects: AtomicUsize,
    fail_batch: AtomicUsize,
    fail_select: AtomicBool,
}

impl CountingTransport {
    fn new() -> Self {
        Self {
            sqlite: SqliteTransport::new().unwrap(),
            posts: AtomicUsize::new(0),
            batches: AtomicUsize::new(0),
            selects: AtomicUsize::new(0),
            fail_batch: AtomicUsize::new(0),
            fail_select: AtomicBool::new(false),
        }
    }
}

struct Counted(Arc<CountingTransport>);

impl Transport for Counted {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        self.0.posts.fetch_add(1, Ordering::SeqCst);
        let request: Value = serde_json::from_slice(body).unwrap();
        let first = &request["requests"][0];
        if first["type"] == "batch" {
            let number = self.0.batches.fetch_add(1, Ordering::SeqCst) + 1;
            if number == self.0.fail_batch.load(Ordering::SeqCst) {
                self.0.sqlite.fail_next_status(503);
            }
        }
        if request["requests"].as_array().unwrap().iter().any(|r| {
            r["stmt"]["sql"].as_str().is_some_and(|s| {
                s.starts_with("SELECT project, source_dir, file_name, sha256, deleted")
            })
        }) {
            self.0.selects.fetch_add(1, Ordering::SeqCst);
            if self.0.fail_select.swap(false, Ordering::SeqCst) {
                self.0.sqlite.fail_next_status(503);
            }
        }
        self.0.sqlite.post(url, body, headers)
    }
}

fn remote(transport: &Arc<CountingTransport>) -> RemoteDb {
    RemoteDb::new(
        "turso://test.example",
        "test-token",
        Box::new(Counted(transport.clone())),
    )
    .unwrap()
}

fn upload(prepared: &notes_upload::Prepared, db: &RemoteDb, temp: &tempfile::TempDir) -> Value {
    memory_store::notes_upload_with(
        prepared,
        true,
        false,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        db,
        &temp.path().join("notes-uploaded.json"),
        false,
    )
    .unwrap()
}

fn upload_forced(
    prepared: &notes_upload::Prepared,
    db: &RemoteDb,
    temp: &tempfile::TempDir,
) -> Value {
    memory_store::notes_upload_with(
        prepared,
        true,
        false,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        db,
        &temp.path().join("notes-uploaded.json"),
        true,
    )
    .unwrap()
}

fn prepared_notes(
    temp: &tempfile::TempDir,
    opted: &BTreeSet<String>,
    rows: Vec<ScannedNote>,
) -> notes_upload::Prepared {
    prepare(
        &ScanReport {
            complete: true,
            listed_dirs: listed(&["-project"]),
            notes: rows,
        },
        temp.path(),
        opted,
    )
}

fn text_cell(row: &[Cell], index: usize) -> &str {
    match &row[index] {
        Cell::Text(s) => s,
        other => panic!("expected text: {other:?}"),
    }
}

#[test]
fn dispositions_and_safe_tombstone_scope() {
    let (temp, opted) = fixture();
    let report = ScanReport {
        complete: true,
        listed_dirs: listed(&["-project", "-missing"]),
        notes: vec![
            row(
                "-project",
                "ok.md",
                ScanStatus::Read,
                Some("---\nname: A\n---\nhello"),
            ),
            row("-project", "big.md", ScanStatus::SkippedSize, None),
            row(
                "-project",
                "bad.md",
                ScanStatus::SkippedUnreadable("bad".into()),
                None,
            ),
            row(
                "-project",
                "secret.md",
                ScanStatus::Read,
                Some("Authorization: Bearer abcd1234SECRET"),
            ),
            row("-missing", "x.md", ScanStatus::Read, Some("x")),
        ],
    };
    let prepared = prepare(&report, temp.path(), &opted);
    let c = &prepared.counts;
    assert_eq!(
        c.scanned,
        c.upload
            + c.unresolved
            + c.not_opted_in
            + c.skipped_size
            + c.skipped_other
            + c.skipped_secret
    );
    assert_eq!(
        (
            c.upload,
            c.unresolved,
            c.skipped_size,
            c.skipped_other,
            c.skipped_secret
        ),
        (1, 1, 1, 1, 1)
    );
    let plan = notes_upload::plan(
        &prepared,
        &[
            existing("-project", "big.md", "old", false),
            existing("-project", "secret.md", "old", false),
            existing("-project", "gone.md", "old", false),
            existing("-missing", "gone.md", "old", false),
        ],
    );
    assert_eq!(plan.tombstones.len(), 1);
    assert_eq!(plan.tombstones[0].file_name, "gone.md");
    assert!(plan
        .listing
        .contains("skipped_secret\tp\t-project/secret.md rule=authorization"));
    assert!(plan.listing.contains("unresolved\t?\t-missing/x.md"));
}

#[test]
fn unresolved_ambiguous_and_incomplete_never_tombstone() {
    let (temp, opted) = fixture();
    fs::create_dir_all(temp.path().join("a-b")).unwrap();
    fs::create_dir_all(temp.path().join("a/b")).unwrap();
    let report = ScanReport {
        complete: true,
        listed_dirs: listed(&["-a-b", "-unknown", "-project"]),
        notes: vec![
            row("-a-b", "a.md", ScanStatus::Read, Some("a")),
            row("-unknown", "b.md", ScanStatus::Read, Some("b")),
            row("-project", "c.md", ScanStatus::Read, Some("c")),
        ],
    };
    let prepared = notes_upload::prepare(&report, temp.path(), &opted, |path| {
        if path.ends_with("project") {
            Err("no origin".into())
        } else {
            Ok("p".into())
        }
    });
    assert_eq!(prepared.counts.unresolved, 3);
    assert!(prepared.tombstone_scope.is_empty());
    let incomplete = ScanReport {
        complete: false,
        listed_dirs: listed(&["-project"]),
        notes: vec![row("-project", "kept.md", ScanStatus::Read, Some("kept"))],
    };
    let plan = notes_upload::plan(
        &prepare(&incomplete, temp.path(), &opted),
        &[existing("-project", "gone.md", "old", false)],
    );
    assert!(plan.tombstones.is_empty());
    let not_opted = prepare(
        &ScanReport {
            complete: true,
            listed_dirs: listed(&["-project"]),
            notes: vec![row("-project", "a.md", ScanStatus::Read, Some("a"))],
        },
        temp.path(),
        &BTreeSet::new(),
    );
    assert_eq!(not_opted.counts.not_opted_in, 1);
}

#[test]
fn unchanged_and_deleted_rows() {
    let (temp, opted) = fixture();
    let report = ScanReport {
        complete: true,
        listed_dirs: listed(&["-project"]),
        notes: vec![row("-project", "a.md", ScanStatus::Read, Some("a"))],
    };
    let prepared = prepare(&report, temp.path(), &opted);
    let unchanged = notes_upload::plan(&prepared, &[existing("-project", "a.md", "hash-a", false)]);
    assert_eq!(unchanged.counts.unchanged, 1);
    assert!(unchanged.upserts.is_empty());
    let resurrect = notes_upload::plan(&prepared, &[existing("-project", "a.md", "hash-a", true)]);
    assert_eq!(resurrect.upserts.len(), 1);
    assert_eq!(
        notes_upload::note_id("p", "m", "h", "-project", "a.md"),
        notes_upload::note_id("p", "m", "h", "-project", "a.md")
    );
    assert_eq!(
        notes_upload::note_id("p", "m", "h", "-project", "a.md").len(),
        12
    );
}

#[test]
fn opt_in_file_is_local_and_sorted() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state/notes-opt-in");
    assert!(notes_upload::read_opt_in(&path).unwrap().is_empty());
    let set = BTreeSet::from(["z".into(), "a".into()]);
    notes_upload::write_opt_in(&path, &set).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "a\nz\n");
    assert_eq!(notes_upload::read_opt_in(&path).unwrap(), set);
}
#[test]
fn sql_round_trip_tombstone_and_read_paths() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let initial = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
    );
    let first = upload(&initial, &db, &temp);
    assert_eq!(first["counts"]["upserts"], 1);
    assert_eq!(first["state"], "rebuilt");
    let state_path = temp.path().join("notes-uploaded.json");
    let state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state["version"], 1);
    assert_eq!(state["machine"], "machine1");
    assert_eq!(
        state["notes"],
        json!([{"project":"p","harness":"claude-code","source_dir":"-project","file_name":"a.md","sha256":"hash-hello"}])
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&state_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let before = transport.posts.load(Ordering::SeqCst);
    let second = upload(&initial, &db, &temp);
    assert_eq!(second["counts"]["unchanged"], 1);
    assert_eq!(transport.posts.load(Ordering::SeqCst), before);
    let edited = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("edited"))],
    );
    assert_eq!(upload(&edited, &db, &temp)["counts"]["upserts"], 1);
    let id = notes_upload::note_id("p", "machine1", "claude-code", "-project", "a.md");
    assert_eq!(
        memory_store::notes_show_with(&db, &id).unwrap()["body"],
        "edited"
    );
    assert_eq!(
        memory_store::notes_list_with(&db, "p").unwrap()["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let old_state = fs::read(&state_path).unwrap();
    let gone = prepared_notes(&temp, &opted, vec![]);
    let deleted = upload(&gone, &db, &temp);
    assert_eq!(deleted["counts"]["tombstones"], 1);
    let raw = db
        .execute(
            "SELECT deleted, body, sha256 FROM harness_notes WHERE note_id=?",
            vec![Cell::Text(id.clone())],
        )
        .unwrap();
    assert_eq!(raw.rows[0][0], Cell::Int(1));
    assert_eq!(text_cell(&raw.rows[0], 1), "");
    assert_eq!(text_cell(&raw.rows[0], 2), "hash-edited");
    assert!(memory_store::notes_show_with(&db, &id).is_err());
    assert!(memory_store::notes_list_with(&db, "p").unwrap()["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    fs::write(&state_path, old_state).unwrap();
    assert_eq!(upload(&gone, &db, &temp)["tombstone_skipped"], 0);
    let third = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "b.md", ScanStatus::Read, Some("body"))],
    );
    upload(&third, &db, &temp);
    db.execute(
        "UPDATE harness_notes SET sha256='other' WHERE file_name='b.md'",
        vec![],
    )
    .unwrap();
    let skipped = upload(&gone, &db, &temp);
    assert_eq!(skipped["tombstone_skipped"], 1);
    let state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert!(state["notes"].as_array().unwrap().is_empty());
}

#[test]
fn rebuild_read_and_failure_retry() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let initial = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
    );
    upload(&initial, &db, &temp);
    let state_path = temp.path().join("notes-uploaded.json");
    fs::remove_file(&state_path).unwrap();
    let gone = prepared_notes(&temp, &opted, vec![]);
    let reads = transport.selects.load(Ordering::SeqCst);
    assert_eq!(upload(&gone, &db, &temp)["counts"]["tombstones"], 1);
    assert_eq!(transport.selects.load(Ordering::SeqCst), reads + 1);
    fs::remove_file(&state_path).unwrap();
    transport.fail_select.store(true, Ordering::SeqCst);
    let retry = upload(&initial, &db, &temp);
    assert_eq!(retry["tombstones_unknown"], true);
    assert_eq!(retry["counts"]["upserts"], 1);
    let state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state["tombstones_unknown"], true);
    assert_eq!(upload(&initial, &db, &temp)["tombstones_unknown"], false);
    let mut wrong: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    wrong["database"] = json!("another-database");
    fs::write(&state_path, wrong.to_string()).unwrap();
    let reads = transport.selects.load(Ordering::SeqCst);
    assert_eq!(upload(&initial, &db, &temp)["state"], "rebuilt");
    assert_eq!(transport.selects.load(Ordering::SeqCst), reads + 1);
}

#[test]
fn dry_run_reads_state_without_writing_and_reports_fallback() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let prepared = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
    );
    upload(&prepared, &db, &temp);
    let state_path = temp.path().join("notes-uploaded.json");
    let before = fs::read(&state_path).unwrap();
    let posts = transport.posts.load(Ordering::SeqCst);
    let dry = memory_store::notes_upload_with(
        &prepared,
        true,
        true,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        &db,
        &state_path,
        false,
    )
    .unwrap();
    assert_eq!(dry["uploaded"], false);
    assert_eq!(dry["planned_from"], "local_state");
    assert_eq!(transport.posts.load(Ordering::SeqCst), posts);
    assert_eq!(fs::read(&state_path).unwrap(), before);
    fs::remove_file(&state_path).unwrap();
    let dry = memory_store::notes_upload_with(
        &prepared,
        true,
        true,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        &db,
        &state_path,
        false,
    )
    .unwrap();
    assert_eq!(dry["planned_from"], "remote");
    assert!(!state_path.exists());
    transport.fail_select.store(true, Ordering::SeqCst);
    let dry = memory_store::notes_upload_with(
        &prepared,
        true,
        true,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        &db,
        &state_path,
        false,
    )
    .unwrap();
    assert_eq!(dry["planned_from"], "none");
    assert_eq!(dry["remote"], "unreachable");
    assert!(!state_path.exists());
}

#[cfg(unix)]
#[test]
fn state_symlink_is_never_followed() {
    use std::os::unix::fs::symlink;
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let prepared = prepared_notes(&temp, &opted, vec![]);
    let target = temp.path().join("target");
    fs::write(&target, "sentinel").unwrap();
    let state = temp.path().join("notes-uploaded.json");
    symlink(&target, &state).unwrap();
    let result = memory_store::notes_upload_with(
        &prepared,
        true,
        false,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        &db,
        &state,
        false,
    );
    assert!(result.unwrap_err().contains("must not be a symlink"));
    assert_eq!(fs::read_to_string(target).unwrap(), "sentinel");
}

#[test]
fn chunks_save_only_committed_progress() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let names: Vec<String> = (0..120).map(|i| format!("{i:03}.md")).collect();
    let rows = names
        .iter()
        .map(|name| row("-project", name, ScanStatus::Read, Some("body")))
        .collect();
    let prepared = prepared_notes(&temp, &opted, rows);
    transport.fail_batch.store(2, Ordering::SeqCst);
    let result = memory_store::notes_upload_with(
        &prepared,
        true,
        false,
        false,
        "turso://test.example",
        "machine1",
        "NUC11i5",
        &db,
        &temp.path().join("notes-uploaded.json"),
        false,
    );
    assert!(result.is_err());
    let state: Value =
        serde_json::from_slice(&fs::read(temp.path().join("notes-uploaded.json")).unwrap())
            .unwrap();
    // The first batch carries the six schema statements and the host row ahead of 43 notes.
    assert_eq!(state["notes"].as_array().unwrap().len(), 43);
    let rows = db
        .execute("SELECT COUNT(*) FROM harness_notes", vec![])
        .unwrap();
    assert_eq!(rows.rows[0][0], Cell::Int(43));
    transport.fail_batch.store(0, Ordering::SeqCst);
    assert_eq!(upload(&prepared, &db, &temp)["counts"]["upserts"], 77);
    assert!(transport.batches.load(Ordering::SeqCst) >= 4);
    let state: Value =
        serde_json::from_slice(&fs::read(temp.path().join("notes-uploaded.json")).unwrap())
            .unwrap();
    assert_eq!(state["notes"].as_array().unwrap().len(), 120);
}

#[test]
fn notes_indexes_support_read_paths() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    for sql in [
        notes_upload::CREATE_TABLE,
        notes_upload::CREATE_MACHINES,
        notes_upload::CREATE_INDEX,
        notes_upload::CREATE_MACHINE_INDEX,
        notes_upload::CREATE_ID_INDEX,
        notes_upload::CREATE_PROJECT_TIME_INDEX,
    ] {
        db.execute(sql, []).unwrap();
    }
    for (sql, expected) in [
        ("SELECT project, source_dir, file_name, sha256, deleted FROM harness_notes WHERE machine=? AND harness=?", "harness_notes_by_machine"),
        ("SELECT n.body FROM harness_notes n LEFT JOIN harness_machines m ON m.machine=n.machine WHERE n.note_id=? AND n.deleted=0", "harness_notes_by_id"),
        ("SELECT n.note_id FROM harness_notes n LEFT JOIN harness_machines m ON m.machine=n.machine WHERE n.project=? AND n.deleted=0 ORDER BY n.updated_at DESC", "harness_notes_by_project_time"),
    ] {
        let mut plan = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let details: Vec<String> = if sql.contains("machine=? AND harness=?") {
            plan.query_map(["x", "x"], |row| row.get(3)).unwrap().map(Result::unwrap).collect()
        } else {
            plan.query_map(["x"], |row| row.get(3)).unwrap().map(Result::unwrap).collect()
        };
        assert!(details.iter().any(|detail| detail.contains(expected)), "{details:?}");
    }
}

#[test]
fn notes_reads_never_create_the_schema() {
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    assert!(matches!(
        memory_store::notes_list_with(&db, "p"),
        Err(memory_store::LedgerError::Remote(
            cort::remote::ErrorKind::Sql
        ))
    ));
    assert!(matches!(
        memory_store::notes_show_with(&db, "missing"),
        Err(memory_store::LedgerError::Remote(
            cort::remote::ErrorKind::Sql
        ))
    ));
    let tables = db
        .execute(
            "SELECT name FROM sqlite_master WHERE name='harness_notes' OR name='harness_machines'",
            vec![],
        )
        .unwrap();
    assert!(tables.rows.is_empty());
}

/// A directory whose every note was deleted has no rows, and still proves they are gone; a
/// directory that was a symlink or failed to list is absent from `listed_dirs` and proves nothing.
#[test]
fn tombstones_follow_listed_directories_not_rows() {
    let (temp, opted) = fixture();
    fs::create_dir_all(temp.path().join("linked")).unwrap();
    let report = ScanReport {
        complete: true,
        listed_dirs: listed(&["-project"]),
        notes: vec![row("-linked", "", ScanStatus::SkippedSymlink, None)],
    };
    let plan = notes_upload::plan(
        &prepare(&report, temp.path(), &opted),
        &[
            existing("-project", "emptied.md", "old", false),
            existing("-linked", "kept.md", "old", false),
        ],
    );
    assert!(plan.listing.contains("tombstone\tp\t-project/emptied.md"));
    let gone: Vec<_> = plan
        .tombstones
        .iter()
        .map(|t| format!("{}/{}", t.source_dir, t.file_name))
        .collect();
    assert_eq!(gone, ["-project/emptied.md"]);
}

/// While an old replica directory exists, an older cort may write this machine's rows without
/// touching the state file; a forced rebuild read must undo both of its effects.
#[test]
fn legacy_replica_writes_are_reconciled_by_a_forced_rebuild() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let note = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
    );
    upload(&note, &db, &temp);
    // An old-version tombstone clears sha256; the file is still on disk with the same bytes.
    db.execute(
        "UPDATE harness_notes SET deleted=1, body='', sha256='' WHERE file_name='a.md'",
        vec![],
    )
    .unwrap();
    assert_eq!(upload(&note, &db, &temp)["counts"]["unchanged"], 1);
    let forced = upload_forced(&note, &db, &temp);
    assert_eq!(forced["state"], "rebuilt_legacy_replica");
    assert_eq!(forced["counts"]["upserts"], 1);
    let live = db
        .execute(
            "SELECT deleted, body FROM harness_notes WHERE file_name='a.md'",
            vec![],
        )
        .unwrap();
    assert_eq!(live.rows[0][0], Cell::Int(0));
    assert_eq!(text_cell(&live.rows[0], 1), "hello");
    // An old-version upsert of other bytes, then the file is deleted: the tombstone must carry
    // the remote hash, not the state's.
    db.execute(
        "UPDATE harness_notes SET sha256='hash-other', body='other' WHERE file_name='a.md'",
        vec![],
    )
    .unwrap();
    let gone = prepared_notes(&temp, &opted, vec![]);
    let buried = upload_forced(&gone, &db, &temp);
    assert_eq!(buried["counts"]["tombstones"], 1);
    assert_eq!(buried["tombstone_skipped"], 0);
    let dead = db
        .execute(
            "SELECT deleted, sha256 FROM harness_notes WHERE file_name='a.md'",
            vec![],
        )
        .unwrap();
    assert_eq!(dead.rows[0][0], Cell::Int(1));
    assert_eq!(text_cell(&dead.rows[0], 1), "hash-other");
}

/// No marker remembers the schema: a database recreated at the same URL gets its tables back on
/// the next write, and a run with nothing to write still sends nothing.
#[test]
fn a_recreated_database_gets_its_schema_back_on_the_next_write() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let note = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
    );
    upload(&note, &db, &temp);
    db.execute("DROP TABLE harness_notes", vec![]).unwrap();
    db.execute("DROP TABLE harness_machines", vec![]).unwrap();
    let posts = transport.posts.load(Ordering::SeqCst);
    assert_eq!(upload(&note, &db, &temp)["counts"]["unchanged"], 1);
    assert_eq!(transport.posts.load(Ordering::SeqCst), posts);
    let edited = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("edited"))],
    );
    assert_eq!(upload(&edited, &db, &temp)["counts"]["upserts"], 1);
    let id = notes_upload::note_id("p", "machine1", "claude-code", "-project", "a.md");
    assert_eq!(
        memory_store::notes_show_with(&db, &id).unwrap()["body"],
        "edited"
    );
}

#[test]
fn a_state_of_another_version_or_machine_is_rebuilt() {
    let (temp, opted) = fixture();
    let transport = Arc::new(CountingTransport::new());
    let db = remote(&transport);
    let note = prepared_notes(
        &temp,
        &opted,
        vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
    );
    upload(&note, &db, &temp);
    let state_path = temp.path().join("notes-uploaded.json");
    let good: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    for (field, value) in [("version", json!(2)), ("machine", json!("machine2"))] {
        let mut wrong = good.clone();
        wrong[field] = value;
        fs::write(&state_path, wrong.to_string()).unwrap();
        let reads = transport.selects.load(Ordering::SeqCst);
        let result = upload(&note, &db, &temp);
        assert_eq!(result["state"], "rebuilt", "{field}");
        assert_eq!(result["counts"]["unchanged"], 1, "{field}");
        assert_eq!(
            transport.selects.load(Ordering::SeqCst),
            reads + 1,
            "{field}"
        );
    }
}
