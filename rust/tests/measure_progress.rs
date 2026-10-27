use cort::audit_import::{ImportedUsage, ImportedUsageOutcome};
use cort::measure::{record_with_scan, RecordInputs, RecordOutcome};
use cort::measure_progress::{self, Load};
use cort::measure_store::{CompactionRow, MachineHost, MeasureStore, SqliteStore};
use cort::telemetry::{
    command_log_columns, row_fingerprint, TelemetryEvent, UploadInputs, SOURCE_COMMAND,
    SOURCE_HEALTH,
};
use cort::usage::{insert_health_row_at, record_command_at, CommandRecord, HealthRow};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MACHINE: &str = "progress-test";

struct Counted {
    inner: SqliteStore,
    reads: usize,
    fail_owners: bool,
    fail_watermarks: bool,
}

macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) -> $ret { self.inner.$name($($arg),*) })*
    };
}
macro_rules! read {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) -> $ret {
            self.reads += 1;
            self.inner.$name($($arg),*)
        })*
    };
}

impl MeasureStore for Counted {
    fn compaction_owners(&mut self, machine_id: &str) -> Result<Vec<(String, String)>, String> {
        self.reads += 1;
        if self.fail_owners {
            Err("owners unavailable".into())
        } else {
            self.inner.compaction_owners(machine_id)
        }
    }
    fn event_watermark(
        &mut self,
        machine_id: &str,
        source: &str,
    ) -> Result<Option<(i64, String)>, String> {
        self.reads += 1;
        if self.fail_watermarks {
            Err("watermarks unavailable".into())
        } else {
            self.inner.event_watermark(machine_id, source)
        }
    }
    forward! {
        upsert_compaction(row: &CompactionRow) -> Result<(), String>;
        upsert_machine_host(machine_id: &str, host: &str, seen_ms: i64) -> Result<(), String>;
        upsert_imported_usage(rows: &[ImportedUsage]) -> Result<ImportedUsageOutcome, String>;
        insert_events(events: &[TelemetryEvent]) -> Result<usize, String>;
        upsert_aliases(rows: &[(String, String, String)]) -> Result<(), String>;
        push() -> Result<(), String>;
        pull() -> Result<(), String>;
    }
    read! {
        compactions_since(since_ms: i64) -> Result<Vec<CompactionRow>, String>;
        machine_hosts() -> Result<Vec<MachineHost>, String>;
        imported_usage() -> Result<Vec<ImportedUsage>, String>;
        remote_ids(machine_id: &str, source: &str) -> Result<HashMap<i64, String>, String>;
        remote_payload(machine_id: &str, source: &str, event_id: &str) -> Result<Option<String>, String>;
        aliases() -> Result<Vec<(String, String, String)>, String>;
        events_page(after: Option<(String, String)>, limit: usize) -> Result<Vec<TelemetryEvent>, String>;
        events_in_key_range(first: &(String, String), last: &(String, String)) -> Result<Vec<TelemetryEvent>, String>;
        event_counts() -> Result<Vec<(String, String, usize)>, String>;
    }
}

fn store(dir: &Path) -> Counted {
    Counted {
        inner: SqliteStore::open(&dir.join("store.db")).unwrap(),
        reads: 0,
        fail_owners: false,
        fail_watermarks: false,
    }
}

fn command(path: &Path) {
    record_command_at(
        path,
        &CommandRecord {
            now_ms: 1_800_000_000_000,
            project_id: None,
            command: "impact".into(),
            args_summary: "{}".into(),
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
}

fn health(path: &Path) {
    insert_health_row_at(
        path,
        &HealthRow {
            id: 0,
            ts: 1_800_000_000_000,
            machine_id: MACHINE.into(),
            machine_source: "test".into(),
            root: None,
            local_project_id: "local".into(),
            project_id: None,
            status: "ok".into(),
            error: None,
            fresh: None,
            chunks: None,
            relationships: None,
            not_chunked: None,
            real_gap: None,
            fts: None,
        },
    )
    .unwrap();
}

fn inputs(dir: &Path, usage_db: PathBuf) -> RecordInputs {
    RecordInputs {
        projects_dir: dir.join("projects"),
        marks_path: dir.join("measure-transcripts.json"),
        machine_id: MACHINE.into(),
        host: "host".into(),
        now_ms: 1_800_000_000_000,
        events: None,
        health: Some(usage_db),
    }
}

fn transcript(dir: &Path, session: &str) {
    let project = dir.join("projects/p");
    fs::create_dir_all(&project).unwrap();
    let line = serde_json::json!({"type":"system","subtype":"compact_boundary","uuid":"boundary",
        "sessionId":session,"timestamp":"2026-10-22T23:37:47.345Z",
        "compactMetadata":{"trigger":"auto","preTokens":100,"postTokens":50}});
    fs::write(project.join("s.jsonl"), format!("{line}\n")).unwrap();
}

#[test]
fn loaded_progress_collects_with_zero_store_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = tmp.path().join("usage.db");
    health(&usage);
    transcript(tmp.path(), "session");
    let mut store = store(tmp.path());
    let input = inputs(tmp.path(), usage);
    let (first, _) = record_with_scan(&mut store, &input).unwrap();
    assert_eq!(first.progress.as_deref(), Some("rebuilt"));
    store.reads = 0;
    let (second, outcome) = record_with_scan(&mut store, &input).unwrap();
    assert_eq!(second.progress.as_deref(), Some("loaded"));
    assert_eq!(outcome, RecordOutcome::Pushed);
    assert_eq!(store.reads, 0);
}

#[test]
fn rebuild_uses_only_matching_legacy_watermark() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = tmp.path().join("usage.db");
    command(&usage);
    let fingerprint = row_fingerprint(&usage, 1).unwrap().unwrap();
    let mut store = store(tmp.path());
    let conn = rusqlite::Connection::open(tmp.path().join("store.db")).unwrap();
    conn.execute(
        "INSERT INTO event_watermarks VALUES (?1, ?2, 1, ?3)",
        (MACHINE, SOURCE_COMMAND, &fingerprint),
    )
    .unwrap();
    let path = tmp.path().join("measure-progress.json");
    let (progress, label) =
        measure_progress::open(&mut store, &path, MACHINE, Some(&usage)).unwrap();
    assert_eq!(label, "rebuilt");
    assert_eq!(
        progress.watermark(MACHINE, SOURCE_COMMAND).unwrap().rowid,
        1
    );
    assert!(progress.watermark(MACHINE, SOURCE_HEALTH).is_none());
    fs::remove_file(&path).unwrap();
    conn.execute("UPDATE event_watermarks SET row_fingerprint = 'wrong'", [])
        .unwrap();
    let (progress, _) = measure_progress::open(&mut store, &path, MACHINE, Some(&usage)).unwrap();
    assert!(progress.watermark(MACHINE, SOURCE_COMMAND).is_none());
    fs::remove_file(&path).unwrap();
    store.fail_watermarks = true;
    let (progress, _) = measure_progress::open(&mut store, &path, MACHINE, Some(&usage)).unwrap();
    assert!(progress.watermark(MACHINE, SOURCE_COMMAND).is_none());
}

#[test]
fn missing_owner_map_pauses_scan_then_retries_only_that_read() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = tmp.path().join("usage.db");
    health(&usage);
    transcript(tmp.path(), "session");
    let input = inputs(tmp.path(), usage);
    fs::write(&input.marks_path, "{}").unwrap();
    let mut store = store(tmp.path());
    store.fail_owners = true;
    let (scan, _) = record_with_scan(&mut store, &input).unwrap();
    assert_eq!(scan.progress.as_deref(), Some("rebuilt_partial"));
    assert_eq!(scan.compactions, 0);
    assert_eq!(fs::read_to_string(&input.marks_path).unwrap(), "{}");
    store.reads = 0;
    store.fail_owners = false;
    let (scan, _) = record_with_scan(&mut store, &input).unwrap();
    assert_eq!(scan.progress.as_deref(), Some("rebuilt"));
    assert_eq!(scan.compactions, 1);
    assert_eq!(store.reads, 1);
    assert!(input.marks_path.exists());
}

#[test]
fn corrupt_progress_rebuilds_and_private_atomic_save_replaces_file() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("measure-progress.json");
    fs::write(&path, "not json").unwrap();
    assert!(matches!(measure_progress::load(&path), Load::Corrupt));
    let mut store = store(tmp.path());
    let (progress, label) = measure_progress::open(&mut store, &path, MACHINE, None).unwrap();
    assert_eq!(label, "rebuilt");
    // Without a usage.db the watermarks stay unchecked for a later run to seed.
    assert!(!progress.watermarks_checked);
    assert!(progress.compactions_complete);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let inode = fs::metadata(&path).unwrap().ino();
    progress.save(&path).unwrap();
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count(),
        0
    );
    fs::write(
        &path,
        r#"{"version":99,"watermarks":{},"compactions":{},"compactions_complete":true}"#,
    )
    .unwrap();
    assert!(matches!(measure_progress::load(&path), Load::Corrupt));
}

#[test]
fn rebuilt_owner_skips_a_different_session() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = tmp.path().join("usage.db");
    health(&usage);
    transcript(tmp.path(), "second");
    let mut store = store(tmp.path());
    let conn = rusqlite::Connection::open(tmp.path().join("store.db")).unwrap();
    conn.execute("INSERT INTO compactions VALUES (?1, 'boundary', 'first', 1, NULL, 'auto', NULL, NULL, 0,0,0,0,0,0,0,0,0,0,0,0,1)", [MACHINE]).unwrap();
    let (scan, _) = record_with_scan(&mut store, &inputs(tmp.path(), usage)).unwrap();
    assert_eq!(scan.compactions, 0);
    assert_eq!(
        store.inner.compaction_owners(MACHINE).unwrap(),
        vec![("boundary".into(), "first".into())]
    );
}

#[test]
fn unchecked_watermarks_are_retried_until_a_read_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = tmp.path().join("usage.db");
    command(&usage);
    let fingerprint = row_fingerprint(&usage, 1).unwrap().unwrap();
    let mut store = store(tmp.path());
    rusqlite::Connection::open(tmp.path().join("store.db"))
        .unwrap()
        .execute(
            "INSERT INTO event_watermarks VALUES (?1, ?2, 1, ?3)",
            (MACHINE, SOURCE_COMMAND, &fingerprint),
        )
        .unwrap();
    let path = tmp.path().join("measure-progress.json");
    // A rebuild with no usage.db to check against settles nothing about watermarks.
    let (progress, label) = measure_progress::open(&mut store, &path, MACHINE, None).unwrap();
    assert_eq!(label, "rebuilt");
    assert!(!progress.watermarks_checked);
    // Nor does a local fingerprint that cannot be read (a directory is no database).
    let (progress, label) =
        measure_progress::open(&mut store, &path, MACHINE, Some(tmp.path())).unwrap();
    assert_eq!(label, "rebuilt_partial");
    assert!(!progress.watermarks_checked);
    // Nor does a failed read.
    store.fail_watermarks = true;
    let (progress, label) =
        measure_progress::open(&mut store, &path, MACHINE, Some(&usage)).unwrap();
    assert_eq!(label, "rebuilt_partial");
    assert!(progress.watermark(MACHINE, SOURCE_COMMAND).is_none());
    // The first successful read seeds it, and later runs no longer ask.
    store.fail_watermarks = false;
    let (progress, label) =
        measure_progress::open(&mut store, &path, MACHINE, Some(&usage)).unwrap();
    assert_eq!(label, "rebuilt");
    assert_eq!(
        progress.watermark(MACHINE, SOURCE_COMMAND).unwrap().rowid,
        1
    );
    store.reads = 0;
    let (_, label) = measure_progress::open(&mut store, &path, MACHINE, Some(&usage)).unwrap();
    assert_eq!(label, "loaded");
    assert_eq!(store.reads, 0);
}

#[test]
fn loaded_progress_uploads_command_events_with_zero_store_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let usage = tmp.path().join("usage.db");
    command(&usage);
    health(&usage);
    let marker = tmp.path().join("measure-events-verified.json");
    let columns = command_log_columns(&usage).unwrap();
    fs::write(
        &marker,
        serde_json::json!({"machine_id": MACHINE, "columns": columns}).to_string(),
    )
    .unwrap();
    let mut store = store(tmp.path());
    let input = RecordInputs {
        events: Some(UploadInputs {
            usage_db: usage.clone(),
            cost_dir: tmp.path().join("cost"),
            marker_path: marker,
            machine_id: MACHINE.into(),
        }),
        ..inputs(tmp.path(), usage.clone())
    };
    let (first, _) = record_with_scan(&mut store, &input).unwrap();
    assert_eq!(first.events.as_ref().unwrap()["status"], "uploaded");
    command(&usage);
    store.reads = 0;
    let (second, _) = record_with_scan(&mut store, &input).unwrap();
    assert_eq!(second.progress.as_deref(), Some("loaded"));
    let events = second.events.unwrap();
    assert_eq!(events["status"], "uploaded");
    assert_eq!(events["inserted"], 1);
    assert_eq!(events["full_range"], false);
    assert_eq!(store.reads, 0);
}
