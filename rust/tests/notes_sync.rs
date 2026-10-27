use cort::notes::{ScanReport, ScanStatus, ScannedNote};
use cort::notes_sync::{self, ExistingRow};
use std::collections::BTreeSet;
use std::fs;

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
) -> notes_sync::Prepared {
    notes_sync::prepare(report, root, opted, |_| Ok("p".into()))
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
    let plan = notes_sync::plan(
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
    let prepared = notes_sync::prepare(&report, temp.path(), &opted, |path| {
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
    let plan = notes_sync::plan(
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
    let unchanged = notes_sync::plan(&prepared, &[existing("-project", "a.md", "hash-a", false)]);
    assert_eq!(unchanged.counts.unchanged, 1);
    assert!(unchanged.upserts.is_empty());
    let resurrect = notes_sync::plan(&prepared, &[existing("-project", "a.md", "hash-a", true)]);
    assert_eq!(resurrect.upserts.len(), 1);
    assert_eq!(
        notes_sync::note_id("p", "m", "h", "-project", "a.md"),
        notes_sync::note_id("p", "m", "h", "-project", "a.md")
    );
    assert_eq!(
        notes_sync::note_id("p", "m", "h", "-project", "a.md").len(),
        12
    );
}

#[test]
fn opt_in_file_is_local_and_sorted() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state/notes-opt-in");
    assert!(notes_sync::read_opt_in(&path).unwrap().is_empty());
    let set = BTreeSet::from(["z".into(), "a".into()]);
    notes_sync::write_opt_in(&path, &set).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "a\nz\n");
    assert_eq!(notes_sync::read_opt_in(&path).unwrap(), set);
}

#[test]
fn sql_round_trip_tombstone_and_read_paths() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        assert!(notes_sync::list(&conn, "p").await.unwrap().is_empty());
        assert!(notes_sync::show(&conn, "id").await.unwrap().is_none());
        assert!(notes_sync::existing_rows(&conn, "m", "h")
            .await
            .unwrap()
            .is_empty());
        notes_sync::ensure_notes_schema(&conn).await.unwrap();
        let (temp, opted) = fixture();
        let report = ScanReport {
            complete: true,
            listed_dirs: listed(&["-project"]),
            notes: vec![row("-project", "a.md", ScanStatus::Read, Some("hello"))],
        };
        let prepared = prepare(&report, temp.path(), &opted);
        let plan = notes_sync::plan(&prepared, &[]);
        notes_sync::apply(&conn, "machine1", "claude", "2026-01-01T00:00:00Z", &plan)
            .await
            .unwrap();
        let rows = notes_sync::existing_rows(&conn, "machine1", "claude")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sha256, "hash-hello");
        let id = notes_sync::note_id("p", "machine1", "claude", "-project", "a.md");
        assert_eq!(
            notes_sync::show(&conn, &id).await.unwrap().unwrap().body,
            "hello"
        );
        notes_sync::apply(&conn, "machine2", "claude", "2026-01-02T00:00:00Z", &plan)
            .await
            .unwrap();
        assert_eq!(notes_sync::list(&conn, "p").await.unwrap().len(), 2);
        let absent = ScanReport {
            complete: true,
            listed_dirs: listed(&["-project"]),
            notes: vec![row("-project", "other.md", ScanStatus::SkippedSize, None)],
        };
        let tombstone = notes_sync::plan(&prepare(&absent, temp.path(), &opted), &rows);
        notes_sync::apply(
            &conn,
            "machine1",
            "claude",
            "2026-01-03T00:00:00Z",
            &tombstone,
        )
        .await
        .unwrap();
        assert!(notes_sync::show(&conn, &id).await.unwrap().is_none());
        assert_eq!(notes_sync::list(&conn, "p").await.unwrap().len(), 1);
        let mut raw = conn
            .query(
                "SELECT body, sha256, title FROM harness_notes WHERE note_id=?",
                (id.as_str(),),
            )
            .await
            .unwrap();
        let row = raw.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "");
        assert_eq!(row.get::<String>(1).unwrap(), "");
        assert_eq!(row.get::<Option<String>>(2).unwrap(), None);
    });
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
    let plan = notes_sync::plan(
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
