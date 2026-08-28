//! Readings: `read_fragment` / `recall_readings`.
//! Contract: docs/superpowers/plans/2026-08-28-codex-fix-proposal.md §1–3
//! (supersedes JS and spec C3 rows wherever they conflict).

use crate::db::Db;
use crate::errors::CortError;
use crate::fts::sanitize_fts_query;
use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_RECALL_LIMIT: i64 = 5;
pub const RECALL_HEAD_LINES: usize = 12;

const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentMode {
    Auto,
    Receipt,
    Full,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadReceipt {
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub source: String,
    pub read_count: i64,
    pub content_mode: String,
    pub content_hash_prefix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadFull {
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub source: String,
    pub read_count: i64,
    pub content_mode: String,
    pub content_hash_prefix: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ReadPayload {
    Receipt(ReadReceipt),
    Full(ReadFull),
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallReading {
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub content: String,
    pub content_truncated: bool,
    pub read_count: i64,
    pub last_read_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallPayload {
    pub query: String,
    pub readings: Vec<RecallReading>,
    pub reading_count: i64,
    pub truncated_query: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationErrorDetail {
    pub command: String,
    pub file_path: String,
    pub operation: String,
    pub errno: Option<String>,
    pub os_code: Option<i32>,
    pub retryable: bool,
    pub note_action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteDisposition {
    Prune,
    Retain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedFailure {
    pub disposition: NoteDisposition,
    pub retryable: bool,
    pub errno: Option<String>,
    pub os_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileMeta {
    pub size: u64,
    pub mtime_ms: f64,
    pub dev: u64,
    pub ino: u64,
    pub is_file: bool,
}

#[derive(Debug)]
pub struct OpenReadError {
    pub operation: &'static str,
    pub error: io::Error,
}

pub trait SourceFs {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn metadata(&self, path: &Path) -> io::Result<FileMeta>;
    fn open_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError>;
}

pub struct RealFs;

impl SourceFs for RealFs {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        Ok(file_meta(&std::fs::metadata(path)?))
    }

    fn open_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        let mut file = std::fs::File::open(path).map_err(|error| OpenReadError {
            operation: "open",
            error,
        })?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|error| OpenReadError {
            operation: "read",
            error,
        })?;
        Ok(buf)
    }
}

pub fn parse_content_mode(provided: Option<&str>) -> Result<ContentMode, CortError> {
    match provided {
        None => Ok(ContentMode::Auto),
        Some("auto") => Ok(ContentMode::Auto),
        Some("receipt") => Ok(ContentMode::Receipt),
        Some("full") => Ok(ContentMode::Full),
        Some(other) => Err(CortError::new(
            "invalid_content_mode",
            json!({
                "provided": other,
                "allowed": ["auto", "receipt", "full"],
            }),
        )),
    }
}

pub fn fragment_hash_prefix(content: &str) -> String {
    let hex = sha256_hex(content.as_bytes());
    hex[..12].to_string()
}

pub fn classify_validation_failure(
    raw_os_error: Option<i32>,
    operation: &str,
) -> ClassifiedFailure {
    let _ = operation;
    match raw_os_error {
        Some(code) => classify_os_code(code),
        None => ClassifiedFailure {
            disposition: NoteDisposition::Retain,
            retryable: false,
            errno: None,
            os_code: None,
        },
    }
}

pub fn read_fragment(
    db: &Db,
    root: &Path,
    project_id: &str,
    file_path: &str,
    start_line: Option<i64>,
    end_line: Option<i64>,
    content_mode: ContentMode,
) -> Result<ReadPayload, CortError> {
    read_fragment_with_fs(
        db,
        root,
        project_id,
        file_path,
        start_line,
        end_line,
        content_mode,
        &RealFs,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn read_fragment_with_fs(
    db: &Db,
    root: &Path,
    project_id: &str,
    file_path: &str,
    start_line: Option<i64>,
    end_line: Option<i64>,
    content_mode: ContentMode,
    fs: &dyn SourceFs,
) -> Result<ReadPayload, CortError> {
    require_indexed(db, project_id)?;
    let resolved = resolve_project_file(fs, root, file_path)?;
    let start = match start_line {
        None => 1,
        Some(v) => require_positive_line(v, "start")?,
    };
    let requested_end = match end_line {
        None => None,
        Some(v) => Some(require_positive_line(v, "end")?),
    };
    if let Some(end) = requested_end {
        if end < start {
            return Err(CortError::new(
                "invalid_line_range",
                json!({ "start": start, "end": end }),
            ));
        }
    }

    let notes = load_notes(db, project_id, &resolved.rel);
    let covering = notes.iter().find(|n| is_covering(n, start, requested_end));

    let validated = match read_stable(fs, &resolved.abs, "read", &resolved.rel) {
        StableRead::Bytes(v) => v,
        StableRead::Prune => {
            prune_notes_for_file(db, project_id, &resolved.rel);
            return Err(CortError::new(
                "file_not_found",
                json!({ "file_path": file_path }),
            ));
        }
        StableRead::Fail(detail) => return Err(cort_validation(&detail)),
    };

    if let Some(note) = covering {
        if note.source_hash == validated.hash {
            let end = requested_end.unwrap_or(note.end_line);
            let content = slice_stored(&note.content, note.start_line, start, end);
            let read_count = commit_store_hit(
                db,
                project_id,
                &resolved.rel,
                note.reading_id,
                &validated.meta,
            );
            return Ok(build_read_payload(
                resolved.rel,
                start,
                end,
                "store",
                read_count,
                content_mode,
                content,
            ));
        }
    }

    let hash_matches_existing =
        !notes.is_empty() && notes.iter().all(|n| n.source_hash == validated.hash);
    if !notes.is_empty() && !hash_matches_existing {
        return persist_from_bytes(
            db,
            project_id,
            &resolved.rel,
            start,
            requested_end,
            content_mode,
            &validated,
            true,
        );
    }

    if hash_matches_existing {
        update_file_metadata(db, project_id, &resolved.rel, &validated.meta);
    }
    persist_from_bytes(
        db,
        project_id,
        &resolved.rel,
        start,
        requested_end,
        content_mode,
        &validated,
        false,
    )
}

pub fn recall_readings(
    db: &Db,
    root: &Path,
    project_id: &str,
    query: &str,
    limit: Option<i64>,
    full_content: bool,
) -> Result<RecallPayload, CortError> {
    recall_readings_with_fs(db, root, project_id, query, limit, full_content, &RealFs)
}

pub fn recall_readings_with_fs(
    db: &Db,
    root: &Path,
    project_id: &str,
    query: &str,
    limit: Option<i64>,
    full_content: bool,
    fs: &dyn SourceFs,
) -> Result<RecallPayload, CortError> {
    require_indexed(db, project_id)?;
    let parsed_limit = match limit {
        None => DEFAULT_RECALL_LIMIT,
        Some(v) => v,
    };
    if !(1..=100).contains(&parsed_limit) {
        return Err(CortError::new(
            "invalid_limit",
            json!({ "limit": parsed_limit }),
        ));
    }
    let sanitized = sanitize_fts_query(query)?;
    let fetch_limit = parsed_limit * 4;
    let candidates = match load_recall_candidates(db, project_id, &sanitized.query, fetch_limit) {
        Ok(rows) => rows,
        Err(err) => {
            return Err(CortError::new(
                "fts_query_failed",
                json!({
                    "query": sanitized.query,
                    "message": err.to_string(),
                }),
            ));
        }
    };

    let mut checked: HashMap<String, FileCheck> = HashMap::new();
    let mut results: Vec<RecallReading> = Vec::new();
    for row in candidates {
        if results.len() as i64 >= parsed_limit {
            break;
        }
        if matches!(checked.get(&row.file_path), Some(FileCheck::Pruned)) {
            continue;
        }
        if !checked.contains_key(&row.file_path) {
            let abs = root.join(&row.file_path);
            let check = match read_stable(fs, &abs, "recall", &row.file_path) {
                StableRead::Bytes(v) => FileCheck::Valid {
                    hash: v.hash,
                    meta: v.meta,
                },
                StableRead::Prune => {
                    prune_notes_for_file(db, project_id, &row.file_path);
                    FileCheck::Pruned
                }
                StableRead::Fail(detail) => FileCheck::Failed(detail),
            };
            checked.insert(row.file_path.clone(), check);
        }
        let check = checked
            .get(&row.file_path)
            .cloned()
            .expect("file check inserted above");
        match check {
            FileCheck::Failed(detail) => return Err(cort_validation(&detail)),
            FileCheck::Pruned => continue,
            FileCheck::Valid { hash, meta } => {
                if hash != row.source_hash {
                    prune_notes_for_file(db, project_id, &row.file_path);
                    checked.insert(row.file_path.clone(), FileCheck::Pruned);
                    continue;
                }
                update_file_metadata(db, project_id, &row.file_path, &meta);
                results.push(trim_content(&row, full_content));
            }
        }
    }

    Ok(RecallPayload {
        query: query.to_string(),
        reading_count: results.len() as i64,
        readings: results,
        truncated_query: sanitized.truncated_query,
    })
}

fn classify_os_code(code: i32) -> ClassifiedFailure {
    // Linux errno numbers. Proposal requires raw errno, not ErrorKind.
    let (errno, retryable, prune) = match code {
        2 => (Some("ENOENT"), false, true),
        5 => (Some("EIO"), true, false),
        24 => (Some("EMFILE"), true, false),
        23 => (Some("ENFILE"), true, false),
        4 => (Some("EINTR"), true, false),
        11 => (Some("EAGAIN"), true, false),
        16 => (Some("EBUSY"), true, false),
        110 => (Some("ETIMEDOUT"), true, false),
        116 => (Some("ESTALE"), true, false),
        13 => (Some("EACCES"), false, false),
        1 => (Some("EPERM"), false, false),
        40 => (Some("ELOOP"), false, false),
        36 => (Some("ENAMETOOLONG"), false, false),
        20 => (Some("ENOTDIR"), false, false),
        22 => (Some("EINVAL"), false, false),
        _ => (None, false, false),
    };
    ClassifiedFailure {
        disposition: if prune {
            NoteDisposition::Prune
        } else {
            NoteDisposition::Retain
        },
        retryable,
        errno: errno.map(str::to_string),
        os_code: Some(code),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_meta(meta: &std::fs::Metadata) -> FileMeta {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    #[cfg(unix)]
    let (dev, ino) = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let (dev, ino) = (0, 0);
    FileMeta {
        size: meta.len(),
        mtime_ms,
        dev,
        ino,
        is_file: meta.is_file(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn cort_validation(detail: &ValidationErrorDetail) -> CortError {
    CortError::new(
        "validation_error",
        serde_json::to_value(detail).expect("ValidationErrorDetail serializes"),
    )
}

fn require_indexed(db: &Db, project_id: &str) -> Result<(), CortError> {
    let mut stmt = db
        .prepare("SELECT 1 FROM projects WHERE project_id = ?1 AND last_indexed_at IS NOT NULL")
        .expect("prepare require_indexed");
    let exists = stmt
        .exists(params![project_id])
        .expect("query require_indexed");
    if exists {
        Ok(())
    } else {
        Err(CortError::new(
            "project_not_indexed",
            json!({ "hint": "run cort index first" }),
        ))
    }
}

fn require_positive_line(value: i64, name: &str) -> Result<i64, CortError> {
    if (1..=JS_MAX_SAFE_INTEGER).contains(&value) {
        Ok(value)
    } else {
        let mut map = serde_json::Map::new();
        map.insert(name.to_string(), json!(value));
        Err(CortError::new("invalid_line_range", Value::Object(map)))
    }
}

struct ResolvedPath {
    abs: PathBuf,
    rel: String,
}

fn resolve_project_file(
    fs: &dyn SourceFs,
    root: &Path,
    requested: &str,
) -> Result<ResolvedPath, CortError> {
    if requested.is_empty() {
        return Err(CortError::new(
            "missing_file",
            json!({ "hint": "cort read <file> [--start <line>] [--end <line>]" }),
        ));
    }
    let candidate = root.join(requested);
    let abs = fs
        .canonicalize(&candidate)
        .map_err(|_| CortError::new("file_not_found", json!({ "file_path": requested })))?;
    let root_abs = fs.canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let rel = match inside_project(&root_abs, &abs) {
        Some(rel) => rel,
        None => {
            return Err(CortError::new(
                "path_outside_project",
                json!({ "file_path": requested }),
            ));
        }
    };
    let meta = fs
        .metadata(&abs)
        .map_err(|_| CortError::new("file_not_found", json!({ "file_path": requested })))?;
    if !meta.is_file {
        return Err(CortError::new(
            "not_a_file",
            json!({ "file_path": requested }),
        ));
    }
    Ok(ResolvedPath { abs, rel })
}

fn inside_project(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    Some(rel.to_string_lossy().replace('\\', "/"))
}

struct NoteRow {
    reading_id: i64,
    start_line: i64,
    end_line: i64,
    ends_at_eof: i64,
    content: String,
    source_hash: String,
}

fn load_notes(db: &Db, project_id: &str, file_path: &str) -> Vec<NoteRow> {
    let mut stmt = db
        .prepare(
            "SELECT reading_id, start_line, end_line, ends_at_eof, content, source_hash
               FROM reading_notes
              WHERE project_id = ?1 AND file_path = ?2
              ORDER BY (end_line - start_line), start_line",
        )
        .expect("prepare load_notes");
    stmt.query_map(params![project_id, file_path], |row| {
        Ok(NoteRow {
            reading_id: row.get(0)?,
            start_line: row.get(1)?,
            end_line: row.get(2)?,
            ends_at_eof: row.get(3)?,
            content: row.get(4)?,
            source_hash: row.get(5)?,
        })
    })
    .expect("query load_notes")
    .map(|r| r.expect("note row"))
    .collect()
}

fn is_covering(note: &NoteRow, start: i64, requested_end: Option<i64>) -> bool {
    if note.start_line > start {
        return false;
    }
    match requested_end {
        None => note.ends_at_eof == 1,
        Some(end) => note.end_line >= end,
    }
}

struct ValidatedSource {
    bytes: Vec<u8>,
    hash: String,
    meta: FileMeta,
}

enum StableRead {
    Bytes(ValidatedSource),
    Prune,
    Fail(ValidationErrorDetail),
}

#[derive(Clone)]
enum FileCheck {
    Valid { hash: String, meta: FileMeta },
    Pruned,
    Failed(ValidationErrorDetail),
}

fn same_identity(a: &FileMeta, b: &FileMeta) -> bool {
    a.dev == b.dev && a.ino == b.ino && a.size == b.size && a.mtime_ms == b.mtime_ms
}

fn io_outcome(command: &str, file_path: &str, operation: &str, err: io::Error) -> StableRead {
    let classified = classify_validation_failure(err.raw_os_error(), operation);
    if classified.disposition == NoteDisposition::Prune {
        return StableRead::Prune;
    }
    StableRead::Fail(ValidationErrorDetail {
        command: command.to_string(),
        file_path: file_path.to_string(),
        operation: operation.to_string(),
        errno: classified.errno,
        os_code: classified.os_code,
        retryable: classified.retryable,
        note_action: "retained".to_string(),
    })
}

fn retained(command: &str, file_path: &str, operation: &str) -> ValidationErrorDetail {
    ValidationErrorDetail {
        command: command.to_string(),
        file_path: file_path.to_string(),
        operation: operation.to_string(),
        errno: None,
        os_code: None,
        retryable: false,
        note_action: "retained".to_string(),
    }
}

fn read_stable(fs: &dyn SourceFs, path: &Path, command: &str, rel: &str) -> StableRead {
    for attempt in 0..2 {
        let pre = match fs.metadata(path) {
            Ok(m) => m,
            Err(e) => return io_outcome(command, rel, "metadata", e),
        };
        if !pre.is_file {
            return StableRead::Fail(retained(command, rel, "not_regular_file"));
        }
        let bytes = match fs.open_read(path) {
            Ok(b) => b,
            Err(e) => return io_outcome(command, rel, e.operation, e.error),
        };
        let post = match fs.metadata(path) {
            Ok(m) => m,
            Err(e) => return io_outcome(command, rel, "metadata", e),
        };
        if !post.is_file {
            return StableRead::Fail(retained(command, rel, "not_regular_file"));
        }
        if same_identity(&pre, &post) {
            return StableRead::Bytes(ValidatedSource {
                hash: sha256_hex(&bytes),
                bytes,
                meta: post,
            });
        }
        if attempt == 1 {
            return StableRead::Fail(retained(command, rel, "source_changed_during_validation"));
        }
    }
    unreachable!("race detector retries at most once")
}

pub fn prune_notes_for_file(db: &Db, project_id: &str, file_path: &str) {
    db.execute(
        "DELETE FROM reading_notes WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )
    .expect("prune reading_notes");
}

fn update_file_metadata(db: &Db, project_id: &str, file_path: &str, meta: &FileMeta) {
    db.execute(
        "UPDATE reading_notes SET source_mtime_ms = ?1, source_size = ?2
          WHERE project_id = ?3 AND file_path = ?4",
        params![meta.mtime_ms, meta.size as i64, project_id, file_path],
    )
    .expect("update reading_notes metadata");
}

fn commit_store_hit(
    db: &Db,
    project_id: &str,
    file_path: &str,
    reading_id: i64,
    meta: &FileMeta,
) -> i64 {
    let tx = db.unchecked_transaction().expect("begin store-hit tx");
    tx.execute(
        "UPDATE reading_notes SET source_mtime_ms = ?1, source_size = ?2
          WHERE project_id = ?3 AND file_path = ?4",
        params![meta.mtime_ms, meta.size as i64, project_id, file_path],
    )
    .expect("update metadata on store hit");
    let now = now_ms();
    tx.execute(
        "UPDATE reading_notes SET read_count = read_count + 1, last_read_at = ?1
          WHERE reading_id = ?2",
        params![now, reading_id],
    )
    .expect("increment read_count");
    let read_count: i64 = tx
        .query_row(
            "SELECT read_count FROM reading_notes WHERE reading_id = ?1",
            params![reading_id],
            |r| r.get(0),
        )
        .expect("fetch read_count");
    tx.commit().expect("commit store-hit tx");
    read_count
}

fn slice_stored(content: &str, note_start: i64, start: i64, end: i64) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    let from = (start - note_start) as usize;
    let to = (end - note_start + 1) as usize;
    let to = to.min(lines.len());
    let from = from.min(to);
    lines[from..to].join("\n")
}

fn slice_lines(text: &str, start: i64, end: i64) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    lines[(start as usize - 1)..(end as usize)].join("\n")
}

fn effective_mode(mode: ContentMode, source: &str) -> &'static str {
    match mode {
        ContentMode::Receipt => "receipt",
        ContentMode::Full => "full",
        ContentMode::Auto => {
            if source == "store" {
                "receipt"
            } else {
                "full"
            }
        }
    }
}

fn build_read_payload(
    file_path: String,
    start_line: i64,
    end_line: i64,
    source: &str,
    read_count: i64,
    mode: ContentMode,
    content: String,
) -> ReadPayload {
    let content_hash_prefix = fragment_hash_prefix(&content);
    if effective_mode(mode, source) == "receipt" {
        ReadPayload::Receipt(ReadReceipt {
            file_path,
            start_line,
            end_line,
            source: source.to_string(),
            read_count,
            content_mode: "receipt".to_string(),
            content_hash_prefix,
        })
    } else {
        ReadPayload::Full(ReadFull {
            file_path,
            start_line,
            end_line,
            source: source.to_string(),
            read_count,
            content_mode: "full".to_string(),
            content_hash_prefix,
            content,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_from_bytes(
    db: &Db,
    project_id: &str,
    rel: &str,
    start: i64,
    requested_end: Option<i64>,
    content_mode: ContentMode,
    validated: &ValidatedSource,
    delete_first: bool,
) -> Result<ReadPayload, CortError> {
    let text = String::from_utf8_lossy(&validated.bytes);
    let line_count = text.split('\n').count() as i64;
    let end = requested_end.unwrap_or(line_count);
    if start > line_count || end > line_count {
        if delete_first {
            prune_notes_for_file(db, project_id, rel);
        }
        return Err(CortError::new(
            "invalid_line_range",
            json!({ "start": start, "end": end, "file_lines": line_count }),
        ));
    }
    let content = slice_lines(&text, start, end);
    let ends_at_eof: i64 = if requested_end.is_none() { 1 } else { 0 };
    let now = now_ms();
    let tx = db.unchecked_transaction().expect("begin persist tx");
    if delete_first {
        tx.execute(
            "DELETE FROM reading_notes WHERE project_id = ?1 AND file_path = ?2",
            params![project_id, rel],
        )
        .expect("evict notes on hash mismatch");
    }
    tx.execute(
        "INSERT INTO reading_notes
            (project_id, file_path, start_line, end_line, ends_at_eof, content, source_hash,
             source_mtime_ms, source_size, read_count, first_read_at, last_read_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)
         ON CONFLICT(project_id, file_path, start_line, end_line) DO UPDATE SET
            ends_at_eof = excluded.ends_at_eof,
            content = excluded.content,
            source_hash = excluded.source_hash,
            source_mtime_ms = excluded.source_mtime_ms,
            source_size = excluded.source_size,
            read_count = reading_notes.read_count + 1,
            last_read_at = excluded.last_read_at",
        params![
            project_id,
            rel,
            start,
            end,
            ends_at_eof,
            content,
            validated.hash,
            validated.meta.mtime_ms,
            validated.meta.size as i64,
            now,
        ],
    )
    .expect("insert reading_notes");
    let read_count: i64 = tx
        .query_row(
            "SELECT read_count FROM reading_notes
              WHERE project_id = ?1 AND file_path = ?2 AND start_line = ?3 AND end_line = ?4",
            params![project_id, rel, start, end],
            |r| r.get(0),
        )
        .expect("fetch inserted read_count");
    tx.commit().expect("commit persist tx");
    Ok(build_read_payload(
        rel.to_string(),
        start,
        end,
        "filesystem",
        read_count,
        content_mode,
        content,
    ))
}

struct RecallRow {
    file_path: String,
    start_line: i64,
    end_line: i64,
    content: String,
    source_hash: String,
    read_count: i64,
    last_read_at: i64,
}

fn load_recall_candidates(
    db: &Db,
    project_id: &str,
    fts_query: &str,
    limit: i64,
) -> rusqlite::Result<Vec<RecallRow>> {
    let mut stmt = db.prepare(
        "SELECT n.file_path, n.start_line, n.end_line, n.content, n.source_hash,
                n.read_count, n.last_read_at
           FROM reading_notes_fts
           JOIN reading_notes n ON n.reading_id = reading_notes_fts.rowid
          WHERE reading_notes_fts MATCH ?1 AND n.project_id = ?2
          ORDER BY bm25(reading_notes_fts)
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![fts_query, project_id, limit], |row| {
        Ok(RecallRow {
            file_path: row.get(0)?,
            start_line: row.get(1)?,
            end_line: row.get(2)?,
            content: row.get(3)?,
            source_hash: row.get(4)?,
            read_count: row.get(5)?,
            last_read_at: row.get(6)?,
        })
    })?;
    rows.collect()
}

fn trim_content(row: &RecallRow, full_content: bool) -> RecallReading {
    let lines: Vec<&str> = row.content.split('\n').collect();
    let truncated = !full_content && lines.len() > RECALL_HEAD_LINES;
    let content = if truncated {
        format!("{}\n…", lines[..RECALL_HEAD_LINES].join("\n"))
    } else {
        row.content.clone()
    };
    RecallReading {
        file_path: row.file_path.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
        content,
        content_truncated: truncated,
        read_count: row.read_count,
        last_read_at: row.last_read_at,
    }
}
