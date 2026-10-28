use cort::measure_progress::{self, Load};
use cort::measure_store::{MeasureStore, SqliteStore};
use cort::telemetry::{
    health_event_id, health_events, upload_events, upload_health_events, EventsReport,
    UploadInputs, SOURCE_HEALTH,
};
use cort::usage::{insert_health_row_at, record_command_at, CommandRecord, HealthRow};
use serde_json::Value;
use std::path::{Path, PathBuf};

const DAY: i64 = 1_793_145_600_000;
const MACHINE: &str = "health-machine";

fn row(ts: i64, status: &str) -> HealthRow {
    HealthRow {
        id: 0,
        ts,
        machine_id: MACHINE.into(),
        machine_source: "test".into(),
        root: Some("repo".into()),
        local_project_id: "local".into(),
        project_id: Some("shared".into()),
        status: "ok".into(),
        error: if status == "ok" {
            None
        } else {
            Some(status.into())
        },
        fresh: Some("fresh".into()),
        chunks: Some(3),
        relationships: Some(2),
        not_chunked: Some(0),
        real_gap: Some(0),
        fts: Some("ok".into()),
    }
}

fn db(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn put(path: &Path, ts: i64, status: &str) {
    insert_health_row_at(path, &row(ts, status)).unwrap();
}

fn store(dir: &tempfile::TempDir) -> SqliteStore {
    SqliteStore::open(&db(dir, "store.db")).unwrap()
}

#[test]
fn health_boundary_filters_before_paging_and_upload() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    put(&path, cort::telemetry::MEASURE_START_MS - 1, "old");
    put(&path, cort::telemetry::MEASURE_START_MS, "new");
    let events = health_events(&path, 0, 1).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].time_ms, Some(cort::telemetry::MEASURE_START_MS));
    let mut remote = store(&dir);
    uploaded(
        upload_health_events(&mut remote, &path, MACHINE, &db(&dir, "progress.json")),
        1,
        1,
        true,
        None,
        events[0].rowid.unwrap(),
    );
    assert_eq!(health_count(&mut remote), 1);
}

#[test]
fn all_old_health_rows_advance_watermark() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    put(&path, cort::telemetry::MEASURE_START_MS - 1, "old");
    let mut remote = store(&dir);
    let progress_path = db(&dir, "progress.json");
    let report = upload_health_events(&mut remote, &path, MACHINE, &progress_path);
    assert!(
        matches!(
            report,
            EventsReport::Uploaded {
                scanned: 0,
                inserted: 0,
                watermark: Some(_),
                ..
            }
        ),
        "{report:?}"
    );
    assert_eq!(health_count(&mut remote), 0);
    put(&path, cort::telemetry::MEASURE_START_MS, "new");
    assert!(matches!(
        upload_health_events(&mut remote, &path, MACHINE, &progress_path),
        EventsReport::Uploaded { inserted: 1, .. }
    ));
}

fn health_count(s: &mut SqliteStore) -> usize {
    s.event_count(MACHINE, SOURCE_HEALTH).unwrap()
}

fn uploaded(
    report: EventsReport,
    scanned: usize,
    inserted: usize,
    full_range: bool,
    reason: Option<&str>,
    watermark: i64,
) {
    assert_eq!(
        report,
        EventsReport::Uploaded {
            scanned,
            inserted,
            aliases: 0,
            full_range,
            reason: reason.map(str::to_string),
            watermark: Some(watermark),
        }
    );
}

#[test]
fn health_event_id_is_content_keyed() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    put(&path, DAY, "ok");
    put(&path, DAY + 1000, "ok");
    put(&path, DAY + 2000, "bad");
    put(&path, DAY + 86_400_000, "ok");
    let e = health_events(&path, 0, 10).unwrap();
    assert_eq!(e[0].event_id, e[1].event_id);
    assert_ne!(e[0].event_id, e[2].event_id);
    assert_ne!(e[0].event_id, e[3].event_id);
    assert_ne!(
        health_event_id(MACHINE, "2025-10-09", "repo", "{}"),
        health_event_id(MACHINE, "2025-10-10", "repo", "{}")
    );
    assert_eq!(e[0].machine_id, MACHINE);
    assert_eq!(e[0].time_ms, Some(DAY));
    assert_eq!(e[0].project_id.as_deref(), Some("shared"));
    assert_eq!(e[0].rowid, Some(1));
}

#[test]
fn uploads_without_dry_run_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    put(&path, DAY, "ok");
    let mut s = store(&dir);
    uploaded(
        upload_health_events(
            &mut s,
            &path,
            MACHINE,
            &path.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        true,
        None,
        1,
    );
    assert_eq!(health_count(&mut s), 1);
    assert_eq!(
        match measure_progress::load(&path.with_file_name("measure-progress.json")) {
            Load::Loaded(p) => p
                .watermark(MACHINE, SOURCE_HEALTH)
                .map(|w| (w.rowid, w.fingerprint.clone())),
            _ => None,
        },
        Some((1, format!("{DAY}|repo")))
    );
}

#[test]
fn command_and_health_upload_without_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    record_command_at(
        &path,
        &CommandRecord {
            now_ms: DAY,
            project_id: Some("shared".into()),
            command: "impact".into(),
            args_summary: "x".into(),
            status: "ok".into(),
            error_code: None,
            read_source: None,
            requested_content_mode: None,
            effective_content_mode: None,
            receipt_hit: None,
            index_stale: None,
            bytes_out: 0,
            saved_bytes: 0,
        },
    );
    put(&path, DAY, "ok");
    let mut s = store(&dir);
    let input = UploadInputs {
        usage_db: path.clone(),
        cost_dir: db(&dir, "cost"),
        progress_path: db(&dir, "measure-progress.json"),
        machine_id: MACHINE.into(),
    };
    assert!(matches!(
        upload_events(&mut s, &input),
        EventsReport::Uploaded { inserted: 1, .. }
    ));
    uploaded(
        upload_health_events(
            &mut s,
            &path,
            MACHINE,
            &path.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        true,
        None,
        1,
    );
}

#[test]
fn watermark_skips_uploaded_health_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    put(&path, DAY, "ok");
    let mut s = store(&dir);
    uploaded(
        upload_health_events(
            &mut s,
            &path,
            MACHINE,
            &path.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        true,
        None,
        1,
    );
    let floor: String = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT value FROM _usage_meta WHERE key='uploaded_health_rowid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(floor, "1");
    uploaded(
        upload_health_events(
            &mut s,
            &path,
            MACHINE,
            &path.with_file_name("measure-progress.json"),
        ),
        0,
        0,
        false,
        None,
        1,
    );
    put(&path, DAY + 1, "bad");
    uploaded(
        upload_health_events(
            &mut s,
            &path,
            MACHINE,
            &path.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        false,
        None,
        2,
    );
}

#[test]
fn reobservation_after_replacement_is_one_event() {
    let dir = tempfile::tempdir().unwrap();
    let old = db(&dir, "old.db");
    let new = db(&dir, "new.db");
    put(&old, DAY, "ok");
    put(&old, DAY + 1, "ok");
    let mut s = store(&dir);
    uploaded(
        upload_health_events(
            &mut s,
            &old,
            MACHINE,
            &old.with_file_name("measure-progress.json"),
        ),
        2,
        1,
        true,
        None,
        2,
    );
    put(&new, DAY + 2, "ok");
    uploaded(
        upload_health_events(
            &mut s,
            &new,
            MACHINE,
            &new.with_file_name("measure-progress.json"),
        ),
        1,
        0,
        true,
        Some("watermark_row_missing"),
        1,
    );
    assert_eq!(health_count(&mut s), 1);
}

#[test]
fn replacement_below_old_watermark_still_uploads() {
    let dir = tempfile::tempdir().unwrap();
    let old = db(&dir, "old.db");
    let new = db(&dir, "new.db");
    for i in 0..5 {
        put(&old, DAY + i, &format!("old{i}"));
    }
    let mut s = store(&dir);
    uploaded(
        upload_health_events(
            &mut s,
            &old,
            MACHINE,
            &old.with_file_name("measure-progress.json"),
        ),
        5,
        5,
        true,
        None,
        5,
    );
    for i in 0..3 {
        put(&new, DAY + 100 + i, &format!("new{i}"));
    }
    uploaded(
        upload_health_events(
            &mut s,
            &new,
            MACHINE,
            &new.with_file_name("measure-progress.json"),
        ),
        3,
        3,
        true,
        Some("watermark_row_missing"),
        3,
    );
    assert_eq!(health_count(&mut s), 8);
}

#[test]
fn changed_row_at_watermark_reads_full_range() {
    let dir = tempfile::tempdir().unwrap();
    let old = db(&dir, "old.db");
    let new = db(&dir, "new.db");
    put(&old, DAY, "old");
    let mut s = store(&dir);
    uploaded(
        upload_health_events(
            &mut s,
            &old,
            MACHINE,
            &old.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        true,
        None,
        1,
    );
    put(&new, DAY + 100, "new");
    uploaded(
        upload_health_events(
            &mut s,
            &new,
            MACHINE,
            &new.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        true,
        Some("watermark_row_changed"),
        1,
    );
    assert_eq!(health_count(&mut s), 2);
}

#[test]
fn null_root_is_the_empty_string() {
    let dir = tempfile::tempdir().unwrap();
    let old = db(&dir, "old.db");
    let new = db(&dir, "new.db");
    let rootless = HealthRow {
        root: None,
        ..row(DAY, "unreadable")
    };
    insert_health_row_at(&old, &rootless).unwrap();
    let event = health_events(&old, 0, 1).unwrap().remove(0);
    let mut value: Value = serde_json::from_str(&event.payload).unwrap();
    assert_eq!(value["root"], Value::Null);
    let fields = value.as_object_mut().unwrap();
    fields.remove("id");
    fields.remove("ts");
    assert_eq!(
        event.event_id,
        health_event_id(MACHINE, "2026-10-28", "", &value.to_string())
    );
    let mut s = store(&dir);
    uploaded(
        upload_health_events(
            &mut s,
            &old,
            MACHINE,
            &old.with_file_name("measure-progress.json"),
        ),
        1,
        1,
        true,
        None,
        1,
    );
    assert_eq!(
        match measure_progress::load(&old.with_file_name("measure-progress.json")) {
            Load::Loaded(p) => p
                .watermark(MACHINE, SOURCE_HEALTH)
                .map(|w| (w.rowid, w.fingerprint.clone())),
            _ => None,
        },
        Some((1, format!("{DAY}|")))
    );
    // A rerun with the row unchanged verifies the stored `ts|` fingerprint and reads nothing.
    uploaded(
        upload_health_events(
            &mut s,
            &old,
            MACHINE,
            &old.with_file_name("measure-progress.json"),
        ),
        0,
        0,
        false,
        None,
        1,
    );
    // The same rootless state re-observed later that day in a replaced database is one event.
    insert_health_row_at(
        &new,
        &HealthRow {
            root: None,
            ..row(DAY + 5000, "unreadable")
        },
    )
    .unwrap();
    uploaded(
        upload_health_events(
            &mut s,
            &new,
            MACHINE,
            &new.with_file_name("measure-progress.json"),
        ),
        1,
        0,
        true,
        Some("watermark_row_changed"),
        1,
    );
    assert_eq!(health_count(&mut s), 1);
}

#[test]
fn payload_keys_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(&dir, "usage.db");
    put(&path, DAY, "ok");
    let event = health_events(&path, 0, 1).unwrap().remove(0);
    assert!(event
        .payload
        .starts_with("{\"chunks\":3,\"error\":null,\"fresh\":\"fresh\","));
    let value: Value = serde_json::from_str(&event.payload).unwrap();
    assert_eq!(value["id"], 1);
    assert_eq!(value["ts"], DAY);
    assert_eq!(value.as_object().unwrap().len(), 15);
    assert!(!event.payload.contains(' '));
}
