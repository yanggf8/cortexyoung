//! SQLite open / schema v3 / cache helpers.

use crate::errors::CortError;
use rusqlite::{params, Connection, ErrorCode, OpenFlags};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCHEMA_VERSION: i64 = 3;

const SCHEMA_SQL: &str = include_str!("schema.sql");

pub type Db = Connection;

pub fn project_id_for(real_path: &str) -> String {
    format!("{:x}", Sha256::digest(real_path.as_bytes()))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME is required to resolve the default cache dir (JS os.homedir())")
}

pub fn cache_dir() -> PathBuf {
    // JS: process.env.CORT_CACHE_DIR ?? path.join(os.homedir(), '.cache', 'cortex-ng')
    // Empty string is not nullish in JS, so a set-but-empty env var wins.
    match std::env::var("CORT_CACHE_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => home_dir().join(".cache").join("cortex-ng"),
    }
}

pub fn db_path_for(real_path: &str) -> PathBuf {
    cache_dir().join(format!("{}.db", project_id_for(real_path)))
}

pub fn open_db(db_path: impl AsRef<Path>) -> rusqlite::Result<Db> {
    let db_path = db_path.as_ref();
    let is_memory = db_path.as_os_str() == std::ffi::OsStr::new(":memory:");
    if !is_memory {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).expect("mkdir cache dir");
            }
        }
    }
    let conn = if is_memory {
        Connection::open_in_memory()?
    } else {
        Connection::open(db_path)?
    };
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    if !is_memory {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(db_path)
                .expect("chmod: db file exists after open")
                .permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(db_path, perms).expect("chmod 0o600");
        }
    }
    Ok(conn)
}

pub fn get_meta(db: &Db, key: &str) -> rusqlite::Result<Option<String>> {
    let mut stmt = db.prepare("SELECT value FROM _cortex_meta WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

pub fn set_meta(db: &Db, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO _cortex_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn ensure_schema(db: &Db) -> Result<(), CortError> {
    db.execute_batch(SCHEMA_SQL)
        .expect("schema.sql (JS rethrows raw sqlite errors)");
    let existing = get_meta(db, "SCHEMA_VERSION").expect("getMeta");
    let expected = SCHEMA_VERSION.to_string();
    let upgrading = existing
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok())
        .is_some_and(|v| v < SCHEMA_VERSION);
    if existing.is_none() {
        set_meta(db, "SCHEMA_VERSION", &expected).expect("setMeta");
    } else if upgrading {
        // v3 adds `raw_edges`. An older database has chunks but no raw-edge layer, so a
        // rebuild would resolve zero edges and silently wipe the graph. Mark it pending:
        // `status` reports stale and the next incremental index falls back to a full one.
        set_meta(db, "graph_pending", "1").expect("setMeta");
        set_meta(db, "SCHEMA_VERSION", &expected).expect("setMeta");
    } else if existing.as_deref() != Some(expected.as_str()) {
        return Err(CortError::new(
            "schema_version_mismatch",
            json!({ "found": existing, "expected": SCHEMA_VERSION }),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectListRow {
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub git_head: Option<String>,
    pub last_indexed_at: Option<i64>,
    pub db_path: String,
}

pub fn list_projects() -> Vec<ProjectListRow> {
    let dir = cache_dir();
    if !dir.exists() {
        return Vec::new();
    }
    let mut names: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect(),
        Err(_) => return Vec::new(),
    };
    names.sort();
    let mut out = Vec::new();
    for name in names {
        if !name.ends_with(".db") {
            continue;
        }
        let db_path = dir.join(&name);
        let db_path_str = db_path.to_string_lossy().into_owned();
        let db = match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(db) => db,
            Err(_) => continue,
        };
        let row = db.query_row(
            "SELECT project_id, name, path, git_head, last_indexed_at FROM projects",
            [],
            |r| {
                Ok(ProjectListRow {
                    project_id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    git_head: r.get(3)?,
                    last_indexed_at: r.get(4)?,
                    db_path: db_path_str.clone(),
                })
            },
        );
        if let Ok(row) = row {
            out.push(row);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteResult {
    pub deleted: bool,
    pub db_path: String,
}

pub fn delete_project(real_path: &str) -> DeleteResult {
    let db_path = db_path_for(real_path);
    let db_path_str = db_path.to_string_lossy().into_owned();
    if !db_path.exists() {
        return DeleteResult {
            deleted: false,
            db_path: db_path_str,
        };
    }
    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{db_path_str}{suffix}"));
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
    DeleteResult {
        deleted: true,
        db_path: db_path_str,
    }
}

pub trait SqliteErrorCode {
    fn sqlite_code(&self) -> Option<&str>;
}

impl SqliteErrorCode for rusqlite::Error {
    fn sqlite_code(&self) -> Option<&'static str> {
        match self.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy) => Some("SQLITE_BUSY"),
            Some(ErrorCode::DiskFull) => Some("SQLITE_FULL"),
            Some(ErrorCode::DatabaseCorrupt) => Some("SQLITE_CORRUPT"),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum WithBusyRetryError<E> {
    Cort(CortError),
    Other(E),
}

pub fn with_busy_retry<T, E, F>(mut f: F) -> Result<T, WithBusyRetryError<E>>
where
    E: SqliteErrorCode + std::fmt::Display,
    F: FnMut() -> Result<T, E>,
{
    let mut last_err: Option<E> = None;
    for _attempt in 0..4 {
        match f() {
            Ok(v) => return Ok(v),
            Err(err) => match err.sqlite_code() {
                Some("SQLITE_BUSY") => {
                    last_err = Some(err);
                    continue;
                }
                Some(code @ ("SQLITE_FULL" | "SQLITE_CORRUPT")) => {
                    return Err(WithBusyRetryError::Cort(CortError::new(
                        "storage_full",
                        json!({ "sqlite_code": code }),
                    )));
                }
                _ => return Err(WithBusyRetryError::Other(err)),
            },
        }
    }
    let last = last_err.expect("busy loop always records last SQLITE_BUSY");
    Err(WithBusyRetryError::Cort(CortError::new(
        "storage_busy",
        json!({ "message": last.to_string() }),
    )))
}
