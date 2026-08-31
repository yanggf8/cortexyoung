//! Full index, file walk, git HEAD, status.
//!
//! C1 contract (spec §1.4 / §1.13 / §1.15), snake_case like Job B:
//! - `pack::extractor_version() -> String`
//! - `chunker::extract_file(ExtractFileArgs) -> Result<ExtractResult, CortError>`
//! - `graph::rebuild_relationships(db, project_id) -> rusqlite::Result<i64>` (from persisted
//!   chunks + `raw_edges`, because resolution spans files)
//!
//! Plan §7 B-gap: index/status entry points canonicalize `root` before
//! `cort::db::project_id_for`.

use crate::db::{set_meta, Db};
use crate::errors::CortError;
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const IGNORE_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    "target",
    "coverage",
    ".next",
    ".cache",
];

pub const SOURCE_EXT: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".py", ".rs"];

const INSERT_CHUNK: &str =
    "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
  start_line, end_line, content, content_hash, language, chunk_source)
  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

const INSERT_RAW_EDGE: &str = "INSERT INTO raw_edges
  (project_id, file_path, source_symbol, raw_target, rel_type, call_form, start_line)
  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
  ON CONFLICT(project_id, file_path, rel_type, raw_target, source_symbol, start_line)
  DO NOTHING";

#[derive(Debug)]
pub enum IndexError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Cort(CortError),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Sqlite(e) => write!(f, "{e}"),
            Self::Cort(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<io::Error> for IndexError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<rusqlite::Error> for IndexError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<CortError> for IndexError {
    fn from(e: CortError) -> Self {
        Self::Cort(e)
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalRoot {
    pub path: PathBuf,
    pub path_str: String,
    pub project_id: String,
}

/// Canonicalize then hash. Plan §7: must call `project_id_for` only after canonicalize.
pub fn canonicalize_root(root: impl AsRef<Path>) -> Result<CanonicalRoot, IndexError> {
    let path = fs::canonicalize(root)?;
    let path_str = path_to_utf8(&path);
    let project_id = crate::db::project_id_for(&path_str);
    Ok(CanonicalRoot {
        path,
        path_str,
        project_id,
    })
}

pub fn project_id_for_root(root: impl AsRef<Path>) -> Result<String, IndexError> {
    Ok(canonicalize_root(root)?.project_id)
}

pub(crate) fn path_to_utf8(p: &Path) -> String {
    p.to_str()
        .expect("project path must be valid UTF-8 (JS realpath is a string)")
        .to_string()
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}

fn basename(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

fn is_ignore_dir(name: &str) -> bool {
    IGNORE_DIRS.contains(&name)
}

fn ext_of(name: &std::ffi::OsStr) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default()
}

fn is_source_ext(ext: &str) -> bool {
    SOURCE_EXT.contains(&ext)
}

pub fn walk_files(root: impl AsRef<Path>) -> Vec<String> {
    let root = root.as_ref();
    let mut out = Vec::new();
    walk_dir(root, root, &mut out);
    out.sort();
    out
}

fn walk_dir(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)
        .expect("walkFiles readdir")
        .map(|e| e.expect("walkFiles dirent"))
        .collect();
    entries.sort_by_key(|a| a.file_name());
    for entry in entries {
        let ft = entry.file_type().expect("walkFiles file_type");
        if ft.is_symlink() {
            continue;
        }
        let abs = entry.path();
        if ft.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !is_ignore_dir(&name) {
                walk_dir(&abs, root, out);
            }
        } else if ft.is_file() && is_source_ext(&ext_of(&entry.file_name())) {
            let rel = abs.strip_prefix(root).expect("walk path under root");
            let posix = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(posix);
        }
    }
}

pub fn git_head_of(root: impl AsRef<Path>) -> Option<String> {
    let r = Command::new("git")
        .arg("-C")
        .arg(root.as_ref())
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !r.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&r.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[derive(Debug, Clone)]
pub struct ExtractedFile {
    pub rel: String,
    pub result: crate::chunker::ExtractResult,
}

pub(crate) fn extract_one(
    bin: &str,
    project_id: &str,
    file_path: &str,
    abs_path: &Path,
    source: &str,
) -> Result<crate::chunker::ExtractResult, IndexError> {
    let abs_s = path_to_utf8(abs_path);
    crate::chunker::extract_file(crate::chunker::ExtractFileArgs {
        bin,
        project_id,
        file_path,
        abs_path: &abs_s,
        source,
        timeout_ms: None,
    })
    .map_err(IndexError::from)
}

pub fn extract_all(
    bin: &str,
    root: &Path,
    project_id: &str,
    files: &[String],
) -> Result<Vec<ExtractedFile>, IndexError> {
    let mut out = Vec::with_capacity(files.len());
    for rel in files {
        let abs = root.join(rel);
        let source = fs::read_to_string(&abs)?;
        let result = extract_one(bin, project_id, rel, &abs, &source)?;
        out.push(ExtractedFile {
            rel: rel.clone(),
            result,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FullIndexStats {
    pub files: i64,
    pub chunks: i64,
    pub unparsed: i64,
    pub relationships: i64,
    pub elapsed_ms: i64,
}

pub(crate) fn insert_chunk(conn: &Connection, c: &crate::chunker::Chunk) -> rusqlite::Result<()> {
    conn.execute(
        INSERT_CHUNK,
        params![
            c.chunk_id,
            c.project_id,
            c.file_path,
            c.symbol_name,
            c.chunk_type,
            c.start_line,
            c.end_line,
            c.content,
            c.content_hash,
            c.language,
            c.chunk_source,
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_raw_edge(
    conn: &Connection,
    project_id: &str,
    file_path: &str,
    edge: &crate::chunker::Edge,
) -> rusqlite::Result<()> {
    conn.execute(
        INSERT_RAW_EDGE,
        params![
            project_id,
            file_path,
            edge.source_symbol.clone().unwrap_or_default(),
            edge.raw_target,
            edge.rel_type,
            edge.call_form.as_str(),
            edge.start_line,
        ],
    )?;
    Ok(())
}

/// File-level and symbol-scoped matches both live in `raw_edges`; the empty string stands for
/// "no enclosing symbol" so the primary key can deduplicate repeated imports.
pub(crate) fn replace_file_raw_edges(
    conn: &Connection,
    project_id: &str,
    file_path: &str,
    edges: &[crate::chunker::Edge],
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM raw_edges WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )?;
    // `raw_edges` keys on (file, rel_type, target, source, line) without the call form, so two forms
    // of the same call on one line are a single row and the insert order decides which one survives.
    // Make that order a rule instead of a coincidence of subprocess output.
    let mut ordered: Vec<&crate::chunker::Edge> = edges.iter().collect();
    ordered.sort_by_key(|e| {
        (
            e.rel_type.clone(),
            e.raw_target.clone(),
            e.source_symbol.clone().unwrap_or_default(),
            e.start_line,
            e.call_form.insertion_rank(),
        )
    });
    for edge in ordered {
        insert_raw_edge(conn, project_id, file_path, edge)?;
    }
    Ok(())
}

/// Entry: canonicalize root, then `project_id_for`.
pub fn full_index(
    db: &mut Db,
    bin: &str,
    root: impl AsRef<Path>,
) -> Result<FullIndexStats, IndexError> {
    let started = Instant::now();
    let canon = canonicalize_root(root)?;
    let files = walk_files(&canon.path);
    let version = crate::pack::extractor_version();
    let head = git_head_of(&canon.path);

    // Extraction runs outside the transaction: subprocesses must not hold a write lock.
    let extracted = extract_all(bin, &canon.path, &canon.project_id, &files)?;

    let mut chunk_count = 0i64;
    let mut unparsed_count = 0i64;

    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO projects (project_id, name, path, git_head, last_indexed_at, extractor_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_id) DO UPDATE SET
           name = excluded.name, path = excluded.path, git_head = excluded.git_head,
           last_indexed_at = excluded.last_indexed_at, extractor_version = excluded.extractor_version",
        params![
            canon.project_id,
            basename(&canon.path),
            canon.path_str,
            head,
            now_ms(),
            version,
        ],
    )?;

    tx.execute(
        "DELETE FROM chunks WHERE project_id = ?1",
        params![canon.project_id],
    )?;
    tx.execute(
        "DELETE FROM file_state WHERE project_id = ?1",
        params![canon.project_id],
    )?;
    tx.execute(
        "DELETE FROM raw_edges WHERE project_id = ?1",
        params![canon.project_id],
    )?;

    for extracted_file in &extracted {
        if extracted_file.result.unparsed {
            unparsed_count += 1;
        }
        replace_file_raw_edges(
            &tx,
            &canon.project_id,
            &extracted_file.rel,
            &extracted_file.result.edges,
        )?;
        for c in &extracted_file.result.chunks {
            insert_chunk(&tx, c)?;
            chunk_count += 1;
        }
        tx.execute(
            "INSERT INTO file_state (project_id, file_path, file_content_hash)
             VALUES (?1, ?2, ?3) ON CONFLICT(project_id, file_path)
             DO UPDATE SET file_content_hash = excluded.file_content_hash, updated_at = datetime('now')",
            params![
                canon.project_id,
                extracted_file.rel,
                extracted_file.result.file_content_hash,
            ],
        )?;
    }

    // Every chunk and raw edge for this run is in place, so the derived graph can be
    // recomputed as one unit. This replaces the old per-file pass, which could only see the
    // re-indexed file's own edges (audit F-01).
    let rel_count = crate::graph::rebuild_relationships(&tx, &canon.project_id)?;

    set_meta(&tx, "extractor_version", &version)?;
    set_meta(&tx, "graph_pending", "0")?;
    tx.commit()?;

    Ok(FullIndexStats {
        files: files.len() as i64,
        chunks: chunk_count,
        unparsed: unparsed_count,
        relationships: rel_count,
        elapsed_ms: started.elapsed().as_millis() as i64,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub project_id: String,
    pub path: String,
    pub indexed: bool,
    pub files: i64,
    pub chunks: i64,
    pub readings: i64,
    pub relationships: i64,
    pub extractor_version: String,
    pub git_head: Option<String>,
    pub last_indexed_at: Option<i64>,
}

/// Entry: canonicalize root, then `project_id_for`. Does not call ast-grep.
pub fn status_of(db: &Db, root: impl AsRef<Path>) -> Result<Status, IndexError> {
    let canon = canonicalize_root(root)?;
    let proj = db
        .query_row(
            "SELECT path, extractor_version, git_head, last_indexed_at FROM projects WHERE project_id = ?1",
            params![canon.project_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((path, extractor_version, git_head, last_indexed_at)) = proj else {
        return Ok(Status {
            project_id: canon.project_id,
            path: canon.path_str,
            indexed: false,
            files: 0,
            chunks: 0,
            readings: 0,
            relationships: 0,
            extractor_version: String::new(),
            git_head: None,
            last_indexed_at: None,
        });
    };
    let files: i64 = db.query_row(
        "SELECT COUNT(*) FROM file_state WHERE project_id = ?1",
        params![canon.project_id],
        |r| r.get(0),
    )?;
    let chunks: i64 = db.query_row(
        "SELECT COUNT(*) FROM chunks WHERE project_id = ?1",
        params![canon.project_id],
        |r| r.get(0),
    )?;
    let readings: i64 = db.query_row(
        "SELECT COUNT(*) FROM reading_notes WHERE project_id = ?1",
        params![canon.project_id],
        |r| r.get(0),
    )?;
    let relationships: i64 = db.query_row(
        "SELECT COUNT(*) FROM relationships r
           JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?1",
        params![canon.project_id],
        |r| r.get(0),
    )?;
    Ok(Status {
        project_id: canon.project_id,
        path,
        indexed: true,
        files,
        chunks,
        readings,
        relationships,
        extractor_version,
        git_head,
        last_indexed_at,
    })
}
