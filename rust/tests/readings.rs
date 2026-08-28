//! C3 readings — proposal §1–3 is the contract (supersedes JS / spec C3 rows on conflict).

use cort::db::{ensure_schema, open_db, project_id_for, Db};
use cort::errors::CortError;
use cort::readings::{
    classify_validation_failure, fragment_hash_prefix, parse_content_mode, read_fragment,
    read_fragment_with_fs, recall_readings, recall_readings_with_fs, ClassifiedFailure,
    ContentMode, FileMeta, NoteDisposition, OpenReadError, ReadPayload, RealFs, SourceFs,
    ValidationErrorDetail, DEFAULT_RECALL_LIMIT, RECALL_HEAD_LINES,
};
use rusqlite::params;
use serde_json::{json, Value};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

const BODY: &str = "first line\ndatabase lookup detail\nthird line\nfourth line\n";

struct Harness {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: Db,
    project_id: String,
}

fn setup() -> Harness { // db field is now mutable when full_index is used
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    std::fs::write(root.join("notes.txt"), BODY).unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/seed.ts"),
        "export function seed() { return 1; }\n",
    )
    .unwrap();
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(&root.to_string_lossy());
    db.execute(
        "INSERT INTO projects (project_id, name, path, last_indexed_at, extractor_version)
         VALUES (?1, ?2, ?3, 1, 'test')",
        params![project_id, "p", root.to_str().unwrap()],
    )
    .unwrap();
    Harness {
        _tmp: tmp,
        root,
        db,
        project_id,
    }
}

fn note_count(db: &Db) -> i64 {
    db.query_row("SELECT COUNT(*) FROM reading_notes", [], |r| r.get(0))
        .unwrap()
}

fn fts_count(db: &Db) -> i64 {
    db.query_row("SELECT COUNT(*) FROM reading_notes_fts", [], |r| r.get(0))
        .unwrap()
}

fn fts_match_database(db: &Db) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM reading_notes_fts WHERE reading_notes_fts MATCH 'database'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn json_keys_in_order(serialized: &str, keys: &[&str]) {
    let mut rest = serialized;
    for key in keys {
        let needle = format!("\"{key}\":");
        match rest.find(&needle) {
            Some(i) => rest = &rest[i + needle.len()..],
            None => panic!("expected key {key:?} in order in {serialized}"),
        }
    }
}

fn payload_json(p: &ReadPayload) -> String {
    serde_json::to_string(p).unwrap()
}

fn read_auto(
    db: &Db,
    root: &Path,
    project_id: &str,
    file_path: &str,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<ReadPayload, CortError> {
    read_fragment(
        db,
        root,
        project_id,
        file_path,
        start,
        end,
        ContentMode::Auto,
    )
}

struct CountingFs {
    inner: RealFs,
    metadata_calls: AtomicUsize,
    open_read_calls: AtomicUsize,
}

impl CountingFs {
    fn new() -> Self {
        Self {
            inner: RealFs,
            metadata_calls: AtomicUsize::new(0),
            open_read_calls: AtomicUsize::new(0),
        }
    }
    fn open_reads(&self) -> usize {
        self.open_read_calls.load(Ordering::SeqCst)
    }
}

impl SourceFs for CountingFs {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.canonicalize(path)
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        self.metadata_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.metadata(path)
    }
    fn open_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.open_read_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.open_read(path)
    }
}

struct FailFs {
    inner: RealFs,
    file_name: &'static str,
    on: &'static str,
    error: io::Error,
}

impl FailFs {
    fn raw(file_name: &'static str, on: &'static str, os_code: i32) -> Self {
        Self {
            inner: RealFs,
            file_name,
            on,
            error: io::Error::from_raw_os_error(os_code),
        }
    }
    fn kind(file_name: &'static str, on: &'static str, kind: ErrorKind, msg: &str) -> Self {
        Self {
            inner: RealFs,
            file_name,
            on,
            error: io::Error::new(kind, msg),
        }
    }
    fn matches(&self, path: &Path) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some(self.file_name)
    }
    fn clone_err(&self) -> io::Error {
        match self.error.raw_os_error() {
            Some(code) => io::Error::from_raw_os_error(code),
            None => io::Error::new(self.error.kind(), self.error.to_string()),
        }
    }
}

impl SourceFs for FailFs {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.canonicalize(path)
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        if self.on == "metadata" && self.matches(path) {
            return Err(self.clone_err());
        }
        self.inner.metadata(path)
    }
    fn open_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        if self.matches(path) && (self.on == "open" || self.on == "read") {
            return Err(OpenReadError {
                operation: if self.on == "open" { "open" } else { "read" },
                error: self.clone_err(),
            });
        }
        self.inner.open_read(path)
    }
}

struct RaceFs {
    inner: RealFs,
    races_left: AtomicU32,
}

impl RaceFs {
    fn new(races: u32) -> Self {
        Self {
            inner: RealFs,
            races_left: AtomicU32::new(races),
        }
    }
}

impl SourceFs for RaceFs {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.canonicalize(path)
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        self.inner.metadata(path)
    }
    fn open_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        let bytes = self.inner.open_read(path)?;
        if self.races_left.load(Ordering::SeqCst) > 0 {
            self.races_left.fetch_sub(1, Ordering::SeqCst);
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| OpenReadError {
                    operation: "open",
                    error,
                })?;
            file.set_modified(SystemTime::now() + Duration::from_secs(120))
                .map_err(|error| OpenReadError {
                    operation: "read",
                    error,
                })?;
        }
        Ok(bytes)
    }
}

fn restore_mtime_after_equal_length_edit(path: &Path, new_body: &[u8]) {
    let meta = std::fs::metadata(path).unwrap();
    let modified = meta.modified().unwrap();
    assert_eq!(
        meta.len() as usize,
        new_body.len(),
        "edit must be equal-length"
    );
    std::fs::write(path, new_body).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(modified).unwrap();
    let after = std::fs::metadata(path).unwrap();
    assert_eq!(after.len(), meta.len());
    assert_eq!(after.modified().unwrap(), modified);
}

/// C3-1
#[test]
fn reading_notes_require_an_indexed_project() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("notes.txt"), BODY).unwrap();
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let err = read_auto(
        &db,
        root,
        &project_id_for(&root.to_string_lossy()),
        "notes.txt",
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "project_not_indexed");
}

/// C3-2 superseded: verified store hit defaults to receipt (no `content` field), not JS full body.
#[test]
fn first_auto_is_filesystem_full_second_auto_is_store_receipt() {
    let h = setup();
    let first = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(2), Some(3)).unwrap();
    let ReadPayload::Full(first) = first else {
        panic!("first auto must be full, got {}", payload_json(&first));
    };
    assert_eq!(first.source, "filesystem");
    assert_eq!(first.content_mode, "full");
    assert_eq!(first.content, "database lookup detail\nthird line");
    assert_eq!(first.read_count, 1);
    assert_eq!(
        first.content_hash_prefix,
        fragment_hash_prefix("database lookup detail\nthird line")
    );
    json_keys_in_order(
        &serde_json::to_string(&first).unwrap(),
        &[
            "file_path",
            "start_line",
            "end_line",
            "source",
            "read_count",
            "content_mode",
            "content_hash_prefix",
            "content",
        ],
    );

    let second = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(2), Some(3)).unwrap();
    let json = payload_json(&second);
    assert!(
        !json.contains("\"content\":"),
        "receipt must omit content field, got {json}"
    );
    let ReadPayload::Receipt(second) = second else {
        panic!("second auto must be receipt, got {json}");
    };
    assert_eq!(second.source, "store");
    assert_eq!(second.content_mode, "receipt");
    assert_eq!(second.read_count, 2);
    assert_eq!(second.file_path, "notes.txt");
    assert_eq!(second.start_line, 2);
    assert_eq!(second.end_line, 3);
    assert_eq!(
        second.content_hash_prefix,
        fragment_hash_prefix("database lookup detail\nthird line")
    );
    json_keys_in_order(
        &serde_json::to_string(&second).unwrap(),
        &[
            "file_path",
            "start_line",
            "end_line",
            "source",
            "read_count",
            "content_mode",
            "content_hash_prefix",
        ],
    );
    assert_eq!(fts_match_database(&h.db), 1);
}

/// C3-2 / proposal §1 TDD.3: receipt-vs-full compatibility.
#[test]
fn second_content_full_returns_body_byte_identical_to_first() {
    let h = setup();
    let first = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(3),
        ContentMode::Auto,
    )
    .unwrap();
    let first_body = match &first {
        ReadPayload::Full(f) => f.content.clone(),
        ReadPayload::Receipt(_) => panic!("first auto must include body"),
    };
    let second = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(3),
        ContentMode::Full,
    )
    .unwrap();
    let ReadPayload::Full(second) = second else {
        panic!("--content full must return body");
    };
    assert_eq!(second.source, "store");
    assert_eq!(second.content_mode, "full");
    assert_eq!(second.read_count, 2);
    assert_eq!(second.content.as_bytes(), first_body.as_bytes());
}

/// C3-3 superseded: whole-file note still serves a subrange, but only after hashing the whole source.
#[test]
fn whole_file_note_serves_subrange_after_hashing_whole_source() {
    let h = setup();
    let whole = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let ReadPayload::Full(whole) = whole else {
        panic!("first whole-file auto must be full");
    };
    assert_eq!(whole.content, BODY);
    assert_eq!(whole.content_hash_prefix, fragment_hash_prefix(BODY));

    let fs = CountingFs::new();
    let subset = read_fragment_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(2),
        ContentMode::Auto,
        &fs,
    )
    .unwrap();
    assert_eq!(
        fs.open_reads(),
        1,
        "must hash the whole source file, not skip I/O"
    );
    let json = payload_json(&subset);
    assert!(!json.contains("\"content\":"));
    let ReadPayload::Receipt(subset) = subset else {
        panic!("auto store hit is receipt");
    };
    assert_eq!(subset.source, "store");
    assert_eq!(
        subset.content_hash_prefix,
        fragment_hash_prefix("database lookup detail")
    );
    assert_ne!(
        subset.content_hash_prefix,
        fragment_hash_prefix(BODY),
        "subrange prefix must not reuse the whole-file prefix"
    );

    let full_subset = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(2),
        ContentMode::Full,
    )
    .unwrap();
    let ReadPayload::Full(full_subset) = full_subset else {
        panic!("full mode must return body");
    };
    assert_eq!(full_subset.content, "database lookup detail");
}

/// C3-4
#[test]
fn a_partial_note_never_masquerades_as_a_whole_file_cache_entry() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(2)).unwrap();
    let whole = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let ReadPayload::Full(whole) = whole else {
        panic!("partial note must not cover EOF; expected filesystem/full");
    };
    assert_eq!(whole.source, "filesystem");
    assert_eq!(whole.content, BODY);
}

/// C3-5
#[test]
fn an_omitted_end_line_caches_the_requested_start_through_eof() {
    let h = setup();
    let first = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(2), None).unwrap();
    let ReadPayload::Full(first) = first else {
        panic!("first omitted-end read must be full");
    };
    assert_eq!(first.source, "filesystem");
    assert!(first.content.starts_with("database lookup detail"));
    let second = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(3), None).unwrap();
    let ReadPayload::Receipt(second) = second else {
        panic!("EOF-covering note must serve later omitted-end as store/receipt");
    };
    assert_eq!(second.source, "store");
    let body = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(3),
        None,
        ContentMode::Full,
    )
    .unwrap();
    let ReadPayload::Full(body) = body else {
        panic!("full mode");
    };
    assert!(body.content.starts_with("third line"));
}

/// C3-6 — a REAL full reindex must not touch reading notes. JS parity:
/// `unchanged reading notes survive a full re-index` calls fullIndex, then reads from store.
#[test]
fn unchanged_reading_notes_survive_a_real_full_reindex() {
    let mut h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(2), Some(3)).unwrap();
    let bin = cort::ast_grep::resolve_ast_grep_bin().unwrap();
    cort::indexer::full_index(&mut h.db, &bin, &h.root).unwrap();
    assert_eq!(note_count(&h.db), 1);
    let second = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(2), Some(3)).unwrap();
    let ReadPayload::Receipt(second) = second else {
        panic!("notes must survive reindex as store/receipt");
    };
    assert_eq!(second.source, "store");
    assert_eq!(second.read_count, 2);
}

/// C3-7 superseded: drop only after successful-read hash mismatch (not mtime/size).
#[test]
fn fts_recall_returns_stored_readings_and_drops_them_after_hash_mismatch() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();
    let found = recall_readings(&h.db, &h.root, &h.project_id, "database", None, false).unwrap();
    assert_eq!(found.reading_count, 1);
    assert_eq!(found.readings[0].file_path, "notes.txt");
    assert!(found.readings[0].content.contains("database lookup detail"));
    json_keys_in_order(
        &serde_json::to_string(&found).unwrap(),
        &["query", "readings", "reading_count", "truncated_query"],
    );
    json_keys_in_order(
        &serde_json::to_string(&found.readings[0]).unwrap(),
        &[
            "file_path",
            "start_line",
            "end_line",
            "content",
            "content_truncated",
            "read_count",
            "last_read_at",
        ],
    );

    std::fs::write(h.root.join("notes.txt"), format!("{BODY}changed\n")).unwrap();
    let stale = recall_readings(&h.db, &h.root, &h.project_id, "database", None, false).unwrap();
    assert_eq!(stale.reading_count, 0);
    assert_eq!(stale.readings.len(), 0);
    assert_eq!(note_count(&h.db), 0);
    assert_eq!(fts_count(&h.db), 0);
}

/// C3-8
#[test]
fn reading_rejects_paths_outside_the_indexed_project_and_invalid_ranges() {
    let h = setup();
    let err = read_auto(&h.db, &h.root, &h.project_id, "../outside", None, None).unwrap_err();
    assert!(
        err.code == "file_not_found" || err.code == "path_outside_project",
        "got {}",
        err.code
    );
    let err = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(3), Some(2)).unwrap_err();
    assert_eq!(err.code, "invalid_line_range");
    let err = read_auto(&h.db, &h.root, &h.project_id, "", None, None).unwrap_err();
    assert_eq!(err.code, "missing_file");
    std::fs::create_dir(h.root.join("adir")).unwrap();
    let err = read_auto(&h.db, &h.root, &h.project_id, "adir", None, None).unwrap_err();
    assert_eq!(err.code, "not_a_file");
}

/// Proposal named regression: equal-length edit + restored mtime must not serve stale.
#[test]
fn equal_length_edit_with_restored_mtime_must_not_serve_stale() {
    let h = setup();
    let first = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let ReadPayload::Full(first) = first else {
        panic!("first read full");
    };
    assert_eq!(first.content, BODY);

    let mutated = BODY.replacen("first line", "FIRST LINE", 1);
    assert_eq!(mutated.len(), BODY.len());
    restore_mtime_after_equal_length_edit(&h.root.join("notes.txt"), mutated.as_bytes());

    let second = read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let ReadPayload::Full(second) = second else {
        panic!(
            "stale store receipt is forbidden; must re-serve filesystem/full, got {}",
            payload_json(&second)
        );
    };
    assert_eq!(second.source, "filesystem");
    assert_eq!(second.content, mutated);
    assert_ne!(second.content, BODY);
    assert_eq!(second.read_count, 1);
}

/// Proposal named regression: metadata-identical + hash-identical still hashes before store hit.
#[test]
fn metadata_identical_and_hash_identical_still_hashes_before_store_hit() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let fs = CountingFs::new();
    let second = read_fragment_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        None,
        None,
        ContentMode::Auto,
        &fs,
    )
    .unwrap();
    assert_eq!(
        fs.open_reads(),
        1,
        "no stat fast-path: must open/read/hash even when metadata matches"
    );
    let ReadPayload::Receipt(second) = second else {
        panic!("verified hit is receipt");
    };
    assert_eq!(second.source, "store");
    assert_eq!(second.read_count, 2);
}

/// Proposal named regression: subrange from a whole-file note hashes the whole source first.
#[test]
fn subrange_from_whole_file_note_hashes_the_whole_source_not_the_subrange() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let mutated = BODY.replacen("first line", "FIRST LINE", 1);
    restore_mtime_after_equal_length_edit(&h.root.join("notes.txt"), mutated.as_bytes());
    let subset = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(2),
        ContentMode::Full,
    )
    .unwrap();
    let ReadPayload::Full(subset) = subset else {
        panic!("hash mismatch must rebuild from filesystem bytes");
    };
    assert_eq!(
        subset.source, "filesystem",
        "a subrange-only hasher would miss an equal-length edit outside the range"
    );
    assert_eq!(subset.content, "database lookup detail");
}

/// Proposal §1: `--content receipt` persists body but never returns it.
#[test]
fn content_receipt_on_first_miss_persists_and_returns_filesystem_receipt() {
    let h = setup();
    let first = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(3),
        ContentMode::Receipt,
    )
    .unwrap();
    let json = payload_json(&first);
    assert!(!json.contains("\"content\":"));
    let ReadPayload::Receipt(first) = first else {
        panic!("receipt mode never returns body");
    };
    assert_eq!(first.source, "filesystem");
    assert_eq!(first.content_mode, "receipt");
    assert_eq!(note_count(&h.db), 1);
    let stored: String =
        h.db.query_row("SELECT content FROM reading_notes", [], |r| r.get(0))
            .unwrap();
    assert_eq!(stored, "database lookup detail\nthird line");

    let later = read_fragment(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        Some(2),
        Some(3),
        ContentMode::Full,
    )
    .unwrap();
    let ReadPayload::Full(later) = later else {
        panic!("later full must return persisted body");
    };
    assert_eq!(later.source, "store");
    assert_eq!(later.content, stored);
}

#[test]
fn invalid_content_mode_reports_provided_and_allowed() {
    assert_eq!(parse_content_mode(None).unwrap(), ContentMode::Auto);
    assert_eq!(parse_content_mode(Some("auto")).unwrap(), ContentMode::Auto);
    assert_eq!(
        parse_content_mode(Some("receipt")).unwrap(),
        ContentMode::Receipt
    );
    assert_eq!(parse_content_mode(Some("full")).unwrap(), ContentMode::Full);
    let err = parse_content_mode(Some("bogus")).unwrap_err();
    assert_eq!(err.code, "invalid_content_mode");
    assert_eq!(
        err.detail,
        json!({
            "provided": "bogus",
            "allowed": ["auto", "receipt", "full"],
        })
    );
    let err = parse_content_mode(Some("")).unwrap_err();
    assert_eq!(err.code, "invalid_content_mode");
    assert_eq!(err.detail["provided"], json!(""));
}

#[test]
fn classify_validation_failure_table() {
    fn check(code: i32, disposition: NoteDisposition, retryable: bool, errno: &str) {
        let got = classify_validation_failure(Some(code), "read");
        assert_eq!(
            got,
            ClassifiedFailure {
                disposition,
                retryable,
                errno: Some(errno.to_string()),
                os_code: Some(code),
            },
            "os_code={code}"
        );
    }
    check(2, NoteDisposition::Prune, false, "ENOENT");
    check(20, NoteDisposition::Retain, false, "ENOTDIR");
    check(5, NoteDisposition::Retain, true, "EIO");
    check(24, NoteDisposition::Retain, true, "EMFILE");
    check(23, NoteDisposition::Retain, true, "ENFILE");
    check(4, NoteDisposition::Retain, true, "EINTR");
    check(11, NoteDisposition::Retain, true, "EAGAIN");
    check(16, NoteDisposition::Retain, true, "EBUSY");
    check(110, NoteDisposition::Retain, true, "ETIMEDOUT");
    check(116, NoteDisposition::Retain, true, "ESTALE");
    check(13, NoteDisposition::Retain, false, "EACCES");
    check(1, NoteDisposition::Retain, false, "EPERM");
    check(40, NoteDisposition::Retain, false, "ELOOP");
    check(36, NoteDisposition::Retain, false, "ENAMETOOLONG");
    check(22, NoteDisposition::Retain, false, "EINVAL");

    let unknown = classify_validation_failure(Some(999), "read");
    assert_eq!(unknown.disposition, NoteDisposition::Retain);
    assert!(!unknown.retryable);
    assert_eq!(unknown.errno, None);
    assert_eq!(unknown.os_code, Some(999));

    let none = classify_validation_failure(None, "read");
    assert_eq!(none.disposition, NoteDisposition::Retain);
    assert!(!none.retryable);
    assert_eq!(none.errno, None);
    assert_eq!(none.os_code, None);
}

#[test]
fn recall_enoent_prunes_notes_and_fts_and_succeeds_empty() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();
    std::fs::remove_file(h.root.join("notes.txt")).unwrap();
    let found = recall_readings(&h.db, &h.root, &h.project_id, "database", None, false).unwrap();
    assert_eq!(found.reading_count, 0);
    assert_eq!(found.query, "database");
    assert_eq!(note_count(&h.db), 0);
    assert_eq!(fts_count(&h.db), 0);
}

/// Proposal named regression: EIO keeps notes and emits exact validation_error fields.
#[test]
fn recall_eio_keeps_notes_and_emits_exact_validation_error_fields() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();
    let notes_before = note_count(&h.db);
    let fts_before = fts_count(&h.db);
    let fs = FailFs::raw("notes.txt", "read", 5);
    let err = recall_readings_with_fs(&h.db, &h.root, &h.project_id, "database", None, false, &fs)
        .unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(
        err.detail,
        json!({
            "command": "recall",
            "file_path": "notes.txt",
            "operation": "read",
            "errno": "EIO",
            "os_code": 5,
            "retryable": true,
            "note_action": "retained",
        })
    );
    assert_eq!(note_count(&h.db), notes_before);
    assert_eq!(fts_count(&h.db), fts_before);

    let detail = ValidationErrorDetail {
        command: "recall".into(),
        file_path: "notes.txt".into(),
        operation: "read".into(),
        errno: Some("EIO".into()),
        os_code: Some(5),
        retryable: true,
        note_action: "retained".into(),
    };
    json_keys_in_order(
        &serde_json::to_string(&detail).unwrap(),
        &[
            "command",
            "file_path",
            "operation",
            "errno",
            "os_code",
            "retryable",
            "note_action",
        ],
    );
}

/// Proposal named regression: ENOTDIR retains.
#[test]
fn recall_enotdir_retains() {
    let h = setup();
    h.db.execute(
        "INSERT INTO reading_notes
            (project_id, file_path, start_line, end_line, ends_at_eof, content, source_hash,
             source_mtime_ms, source_size, read_count, first_read_at, last_read_at)
         VALUES (?1, 'notes.txt/ghost', 1, 1, 0, 'database lookup', 'abcd', 0, 0, 1, 0, 0)",
        params![h.project_id],
    )
    .unwrap();
    assert_eq!(note_count(&h.db), 1);
    let err = recall_readings(&h.db, &h.root, &h.project_id, "database", None, false).unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(err.detail["command"], json!("recall"));
    assert_eq!(err.detail["file_path"], json!("notes.txt/ghost"));
    assert_eq!(err.detail["errno"], json!("ENOTDIR"));
    assert_eq!(err.detail["os_code"], json!(20));
    assert_eq!(err.detail["retryable"], json!(false));
    assert_eq!(err.detail["note_action"], json!("retained"));
    assert_eq!(note_count(&h.db), 1);
    assert_eq!(fts_count(&h.db), 1);
}

#[test]
fn recall_emfile_retryable_true_eacces_retryable_false() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();

    let emfile = FailFs::raw("notes.txt", "open", 24);
    let err = recall_readings_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "database",
        None,
        false,
        &emfile,
    )
    .unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(err.detail["errno"], json!("EMFILE"));
    assert_eq!(err.detail["retryable"], json!(true));
    assert_eq!(err.detail["note_action"], json!("retained"));
    assert_eq!(note_count(&h.db), 1);

    let eacces = FailFs::raw("notes.txt", "metadata", 13);
    let err = recall_readings_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "database",
        None,
        false,
        &eacces,
    )
    .unwrap_err();
    assert_eq!(err.detail["errno"], json!("EACCES"));
    assert_eq!(err.detail["operation"], json!("metadata"));
    assert_eq!(err.detail["retryable"], json!(false));
    assert_eq!(note_count(&h.db), 1);
}

#[test]
fn recall_language_not_found_without_raw_enoent_retains() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();
    let fs = FailFs::kind(
        "notes.txt",
        "metadata",
        ErrorKind::NotFound,
        "generic not found",
    );
    let err = recall_readings_with_fs(&h.db, &h.root, &h.project_id, "database", None, false, &fs)
        .unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(err.detail["errno"], Value::Null);
    assert_eq!(err.detail["os_code"], Value::Null);
    assert_eq!(err.detail["retryable"], json!(false));
    assert_eq!(err.detail["note_action"], json!("retained"));
    assert_eq!(note_count(&h.db), 1);
}

#[test]
fn recall_multi_candidate_eio_fail_closed_no_partial_results() {
    let h = setup();
    std::fs::write(h.root.join("other.txt"), "database also lives here\n").unwrap();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();
    read_auto(&h.db, &h.root, &h.project_id, "other.txt", None, None).unwrap();
    assert_eq!(note_count(&h.db), 2);
    let fs = FailFs::raw("notes.txt", "read", 5);
    let err = recall_readings_with_fs(&h.db, &h.root, &h.project_id, "database", None, false, &fs)
        .unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(err.detail["file_path"], json!("notes.txt"));
    assert_eq!(err.detail["errno"], json!("EIO"));
    assert_eq!(note_count(&h.db), 2, "uncertifiable notes must be retained");
}

#[test]
fn hash_mismatch_rebuild_uses_already_read_bytes_no_second_read() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let mutated = BODY.replacen("first line", "FIRST LINE", 1);
    std::fs::write(h.root.join("notes.txt"), &mutated).unwrap();
    let fs = CountingFs::new();
    let second = read_fragment_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        None,
        None,
        ContentMode::Full,
        &fs,
    )
    .unwrap();
    assert_eq!(
        fs.open_reads(),
        1,
        "mismatch rebuild must reuse the already-read bytes"
    );
    let ReadPayload::Full(second) = second else {
        panic!("mismatch serves filesystem/full");
    };
    assert_eq!(second.source, "filesystem");
    assert_eq!(second.content, mutated);
    let stored: String =
        h.db.query_row("SELECT content FROM reading_notes", [], |r| r.get(0))
            .unwrap();
    assert_eq!(stored, mutated);
    assert_eq!(note_count(&h.db), 1);
}

#[test]
fn pre_post_metadata_race_retries_once_then_validation_error() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let notes_before = note_count(&h.db);

    let once = RaceFs::new(1);
    let ok = read_fragment_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        None,
        None,
        ContentMode::Auto,
        &once,
    )
    .unwrap();
    match ok {
        ReadPayload::Receipt(r) => assert_eq!(r.source, "store"),
        ReadPayload::Full(f) => assert_eq!(f.source, "store"),
    }
    assert_eq!(note_count(&h.db), notes_before);

    let twice = RaceFs::new(2);
    let err = read_fragment_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        None,
        None,
        ContentMode::Auto,
        &twice,
    )
    .unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(err.detail["command"], json!("read"));
    assert_eq!(err.detail["file_path"], json!("notes.txt"));
    assert_eq!(
        err.detail["operation"],
        json!("source_changed_during_validation")
    );
    assert_eq!(err.detail["errno"], Value::Null);
    assert_eq!(err.detail["os_code"], Value::Null);
    assert_eq!(err.detail["retryable"], json!(false));
    assert_eq!(err.detail["note_action"], json!("retained"));
    assert_eq!(note_count(&h.db), notes_before);
}

#[test]
fn recall_not_regular_file_retains() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", Some(1), Some(3)).unwrap();
    std::fs::remove_file(h.root.join("notes.txt")).unwrap();
    std::fs::create_dir(h.root.join("notes.txt")).unwrap();
    let err = recall_readings(&h.db, &h.root, &h.project_id, "database", None, false).unwrap_err();
    assert_eq!(err.code, "validation_error");
    assert_eq!(err.detail["operation"], json!("not_regular_file"));
    assert_eq!(err.detail["retryable"], json!(false));
    assert_eq!(err.detail["note_action"], json!("retained"));
    assert_eq!(note_count(&h.db), 1);
}

#[test]
fn recall_trims_to_head_lines_unless_full_content() {
    let h = setup();
    let long: String = (1..=20).map(|i| format!("database line {i}\n")).collect();
    std::fs::write(h.root.join("notes.txt"), &long).unwrap();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let trimmed = recall_readings(&h.db, &h.root, &h.project_id, "database", None, false).unwrap();
    assert!(trimmed.readings[0].content_truncated);
    let expected: String = (1..=RECALL_HEAD_LINES)
        .map(|i| format!("database line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n…";
    assert_eq!(trimmed.readings[0].content, expected);
    let full = recall_readings(&h.db, &h.root, &h.project_id, "database", None, true).unwrap();
    assert!(!full.readings[0].content_truncated);
    assert_eq!(full.readings[0].content, long);
}

#[test]
fn recall_rejects_invalid_limit_and_empty_query() {
    let h = setup();
    let err =
        recall_readings(&h.db, &h.root, &h.project_id, "database", Some(0), false).unwrap_err();
    assert_eq!(err.code, "invalid_limit");
    let err =
        recall_readings(&h.db, &h.root, &h.project_id, "database", Some(101), false).unwrap_err();
    assert_eq!(err.code, "invalid_limit");
    let err = recall_readings(&h.db, &h.root, &h.project_id, "   ", None, false).unwrap_err();
    assert_eq!(err.code, "empty_query");
    assert_eq!(DEFAULT_RECALL_LIMIT, 5);
}

#[test]
fn read_count_does_not_increment_before_verification() {
    let h = setup();
    read_auto(&h.db, &h.root, &h.project_id, "notes.txt", None, None).unwrap();
    let count_before: i64 =
        h.db.query_row("SELECT read_count FROM reading_notes", [], |r| r.get(0))
            .unwrap();
    let fs = FailFs::raw("notes.txt", "read", 5);
    let err = read_fragment_with_fs(
        &h.db,
        &h.root,
        &h.project_id,
        "notes.txt",
        None,
        None,
        ContentMode::Auto,
        &fs,
    )
    .unwrap_err();
    assert_eq!(err.code, "validation_error");
    let count_after: i64 =
        h.db.query_row("SELECT read_count FROM reading_notes", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count_after, count_before);
}
