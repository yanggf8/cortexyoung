//! SQLite open / schema v4 / cache helpers.

use crate::errors::CortError;
use rusqlite::{params, Connection, ErrorCode, OpenFlags};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCHEMA_VERSION: i64 = 5;

/// Every `TEXT NOT NULL DEFAULT` column an `ALTER TABLE ADD COLUMN` has to supply, per table.
/// Kept as one list so the upgrade path and `schema.sql` cannot drift: a column that exists in the
/// fresh schema but not here produces a database that only fails at first insert.
const V4_ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    (
        "raw_edges",
        "call_form",
        "TEXT NOT NULL DEFAULT 'bare' CHECK(call_form IN ('bare','receiver','scoped'))",
    ),
    ("relationships", "call_site_line", "INTEGER"),
    (
        "relationships",
        "call_form",
        "TEXT NOT NULL DEFAULT 'bare' CHECK(call_form IN ('bare','receiver','scoped'))",
    ),
];

/// The v5 rebuild for `relationships`: `(table, CREATE, explicit column list)`.
///
/// The column list is written out because it is used on BOTH sides of the INSERT. `SELECT *` is
/// wrong here and the reason is not obvious: `migrate_v4` appends its columns with `ALTER TABLE ADD
/// COLUMN`, so a v3-then-v4 database carries `call_site_line`/`call_form` at the END while a fresh
/// one from `schema.sql` carries them in the MIDDLE. Five of eight columns differ, and a positional
/// copy would put `confidence` into `call_site_line`.
///
/// The body is `schema.sql`'s, suffixed `__v5`. It is a third copy of that text and nothing but
/// `migrating_a_real_v3_database_to_v5_preserves_and_aligns_every_row` checks that it has not
/// drifted -- that test compares the migrated column order against a fresh database for exactly
/// this reason.
const V5_RELATIONSHIPS: (&str, &str, &str) = (
    "relationships",
    "CREATE TABLE relationships__v5 (
       source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
       target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
       rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls','references')),
       call_site_line INTEGER,
       call_form TEXT NOT NULL DEFAULT 'bare'
         CHECK(call_form IN ('bare','receiver','scoped','type')),
       confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
       confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
       confidence_reasoning TEXT,
       PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type))",
    "source_chunk_id, target_chunk_id, rel_type, call_site_line, call_form,
     confidence, confidence_score, confidence_reasoning",
);

/// The v5 rebuild for `raw_edges`. Same contract as [`V5_RELATIONSHIPS`].
const V5_RAW_EDGES: (&str, &str, &str) = (
    "raw_edges",
    "CREATE TABLE raw_edges__v5 (
       project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
       file_path TEXT NOT NULL,
       source_symbol TEXT NOT NULL DEFAULT '',
       raw_target TEXT NOT NULL,
       rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls','references')),
       call_form TEXT NOT NULL DEFAULT 'bare'
         CHECK(call_form IN ('bare','receiver','scoped','type')),
       start_line INTEGER NOT NULL,
       PRIMARY KEY (project_id, file_path, rel_type, raw_target, source_symbol, start_line))",
    "project_id, file_path, source_symbol, raw_target, rel_type, call_form, start_line",
);

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

/// An OS-level failure while preparing the database file, as a sqlite error.
///
/// `open_db` returns `rusqlite::Result`, and these are `io::Error`s, so they need a shape the
/// caller already handles. `SQLITE_CANTOPEN` is the honest one: whatever went wrong, the outcome is
/// that this file could not be made usable. `classify_sqlite` folds it into `storage_busy`, which
/// is exactly how a caller should treat a disk that is not cooperating.
fn cantopen(what: &str, e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(format!("{what}: {e}")),
    )
}

pub fn open_db(db_path: impl AsRef<Path>) -> rusqlite::Result<Db> {
    let db_path = db_path.as_ref();
    let is_memory = db_path.as_os_str() == std::ffi::OsStr::new(":memory:");
    if !is_memory {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| cantopen("could not create the cache directory", e))?;
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
            // 0600 is a privacy property, not a nicety: this file holds the contents of the user's
            // source. A failure to set it is returned rather than ignored -- degrading quietly here
            // would leave a world-readable index that nobody was told about -- and returned rather
            // than panicked on, because every caller of `open_db` already has an error path and one
            // of them is a hook that must never do anything but exit 0.
            let mut perms = std::fs::metadata(db_path)
                .map_err(|e| cantopen("could not stat the database file", e))?
                .permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(db_path, perms)
                .map_err(|e| cantopen("could not restrict the database file to 0600", e))?;
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

/// The head the index was actually built from. Staleness and the incremental candidate set both
/// need it, and neither can get it from the working tree: after a `git pull` the tree agrees with
/// the *new* head while every chunk still describes the old one.
pub fn indexed_head(db: &Db, project_id: &str) -> rusqlite::Result<Option<String>> {
    let mut stmt = db.prepare("SELECT git_head FROM projects WHERE project_id = ?1")?;
    let mut rows = stmt.query(params![project_id])?;
    match rows.next()? {
        Some(row) => row.get(0),
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

fn column_exists(db: &Db, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let found = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .any(|r| r.map(|n: String| n == column).unwrap_or(false));
    Ok(found)
}

/// Add the v4 columns to tables that predate them.
///
/// `CREATE TABLE IF NOT EXISTS` never adds a column to a table that already exists, so an upgrade
/// has to say so explicitly or every later insert into `raw_edges` fails on an unknown column.
/// Order matters: this runs *before* `SCHEMA_VERSION` is written, so a failed `ALTER` leaves the
/// database at its old version and the next run retries instead of trusting a half-migrated file.
/// The rows themselves need no back-fill -- `raw_edges` is rewritten by the full re-index that
/// `graph_pending` forces, and `relationships` is derived state rebuilt from it.
fn migrate_v4(db: &Db) -> Result<(), CortError> {
    for (table, column, spec) in V4_ADDED_COLUMNS {
        let exists = column_exists(db, table, column).map_err(|e| {
            CortError::new(
                "schema_migration_failed",
                json!({ "table": table, "column": column, "message": e.to_string() }),
            )
        })?;
        if exists {
            continue;
        }
        db.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {spec}"),
            [],
        )
        .map_err(|e| {
            CortError::new(
                "schema_migration_failed",
                json!({ "table": table, "column": column, "message": e.to_string() }),
            )
        })?;
    }
    Ok(())
}

/// Widen the v5 CHECK constraints on `relationships` and `raw_edges`.
///
/// SQLite cannot alter a CHECK in place, so this is the documented rebuild. Four properties, each of
/// which was wrong in a draft and is now pinned by a test:
///
/// * Columns are named on both sides of the INSERT -- see [`V5_RELATIONSHIPS`] for why `SELECT *`
///   silently misaligns a real upgraded database.
/// * Each table's rebuild runs inside its own transaction, so a mid-rebuild failure rolls back
///   instead of leaving a half-copied table behind.
/// * No `PRAGMA foreign_keys` dance. Nothing in `schema.sql` references `relationships` or
///   `raw_edges` -- only their own indexes do, and those drop and recreate with the table -- and
///   dropping a table is never an FK violation. A draft turned enforcement off and restored it with
///   `let _ =`, which would have left foreign keys silently OFF for the rest of the process if the
///   restore failed. The pragma protected against a hazard this schema does not have.
/// * The error path does not delete `{table}__v5`. The only state in which a populated temporary
///   table outlives this batch is one where the transaction never committed, and if the ROLLBACK
///   itself failed -- the transient IOERR class that already killed eight tests on a CI runner --
///   that table may hold the only surviving copy of the rows. The retry clears it, because the batch
///   opens with `DROP TABLE IF EXISTS`. Cleaning up here can only destroy data, never save any.
///
/// Rows are copied rather than re-derived because README's upgrade note promises that `impact` keeps
/// answering from the pre-upgrade graph until the forced re-index runs. `chunks` is untouched: Rust
/// type declarations are stored as `chunk:class`, a value its CHECK already allows, so the largest
/// table and its external-content FTS mirror never move.
///
/// Runs before `SCHEMA_VERSION` is written, so a failure leaves the database at its old version and
/// the next open retries. Every sqlite error is returned, never panicked on -- `hook-refresh` reaches
/// this path on every edit and promises to be silent and exit 0.
///
/// One behaviour worth knowing before diagnosing it twice: this is the first *write transaction* on
/// the open path (v4's were quick ALTERs). Between deploying this binary and the first successful
/// migration, a concurrent open serialises on `BEGIN IMMEDIATE` for up to the busy timeout set in
/// `open_db`. A racing `hook-refresh` blocks rather than failing fast, then lands on its quiet
/// `db_unavailable` path. The race itself is safe: the loser re-runs the rebuild, which is idempotent
/// and lossless precisely because both sides of the INSERT name their columns.
fn migrate_v5(db: &Db) -> Result<(), CortError> {
    let fail = |stage: &str, e: rusqlite::Error| {
        CortError::new(
            "schema_migration_failed",
            json!({ "version": 5, "stage": stage, "message": e.to_string() }),
        )
    };
    for (table, create, columns) in [V5_RELATIONSHIPS, V5_RAW_EDGES] {
        db.execute_batch(&format!(
            "BEGIN IMMEDIATE;
             DROP TABLE IF EXISTS {table}__v5;
             {create};
             INSERT INTO {table}__v5 ({columns}) SELECT {columns} FROM {table};
             DROP TABLE {table};
             ALTER TABLE {table}__v5 RENAME TO {table};
             COMMIT;"
        ))
        .map_err(|e| {
            let _ = db.execute_batch("ROLLBACK");
            fail(table, e)
        })?;
    }
    // The indexes named in SCHEMA_SQL went with the dropped tables.
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
         CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);
         CREATE INDEX IF NOT EXISTS idx_raw_edges_file ON raw_edges(project_id, file_path);",
    )
    .map_err(|e| fail("indexes", e))
}

/// Bring a freshly opened database up to the current schema.
///
/// Every sqlite failure here is returned, not panicked on. It used to `expect` on four of them,
/// with a message ("JS rethrows raw sqlite errors") inherited from the JavaScript version this crate
/// replaced -- and the effect was that any storage error at all killed the process instead of
/// producing the structured error every other path in this file produces. Not hypothetical: on
/// 2026-09-03 a macOS CI runner returned `SQLITE_IOERR_FSYNC` for a few seconds and eight tests died
/// on this line, reporting a panic rather than a disk problem.
///
/// The cost of the old shape grew when `hook-refresh` arrived. That hook reaches `ensure_schema`
/// through `open_project_tracked`, and it promises in its own doc comment to be silent and exit 0
/// whatever happens -- a promise a panic here would break on every edit for as long as the disk
/// misbehaved. `classify_sqlite` already knows how to tell a stale schema from a busy store, and
/// `cmd_hook_refresh` already has a quiet path (`db_unavailable`) waiting to catch it.
pub fn ensure_schema(db: &Db) -> Result<(), CortError> {
    db.execute_batch(SCHEMA_SQL)
        .map_err(|e| classify_sqlite(&e))?;
    let existing = get_meta(db, "SCHEMA_VERSION").map_err(|e| classify_sqlite(&e))?;
    let expected = SCHEMA_VERSION.to_string();
    let upgrading = existing
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok())
        .is_some_and(|v| v < SCHEMA_VERSION);
    if existing.is_none() {
        set_meta(db, "SCHEMA_VERSION", &expected).map_err(|e| classify_sqlite(&e))?;
    } else if upgrading {
        migrate_v4(db)?;
        migrate_v5(db)?;
        // v3 added `raw_edges`; v4 adds `call_form` and the call-site line. An older database has
        // chunks whose edges cannot be re-derived with the new columns filled in, so a rebuild would
        // silently wipe the graph. Mark it pending: `status` reports stale and the next incremental
        // index falls back to a full one.
        set_meta(db, "graph_pending", "1").map_err(|e| classify_sqlite(&e))?;
        set_meta(db, "SCHEMA_VERSION", &expected).map_err(|e| classify_sqlite(&e))?;
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
    delete_project_db(&db_path_for(real_path))
}

/// Delete by database path rather than by project path.
///
/// `delete_project` derives the database name by hashing the *canonicalised* project path, which
/// is unavailable for exactly the row most worth deleting: one whose directory no longer exists.
/// `list_projects` already records each row's own `db_path`, so a caller that found the row there
/// can hand it straight back. Observed on 2026-09-02, when two rows left behind by the install
/// smoke test pointed at deleted `/tmp` directories and `cort delete` refused both with
/// `file_not_found` -- the registry could name the garbage but not remove it.
pub fn delete_project_db(db_path: &Path) -> DeleteResult {
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

/// Classify a rusqlite error into a code the caller can act on.
///
/// Every sqlite failure used to come back as `storage_busy`, which names contention: wait, retry,
/// and it clears. A missing table is the opposite claim -- it will never clear, because the
/// database predates the schema that added the table, and the only fix is `cort index`. That
/// mattered most in `cort status`, which opens the index read-only and therefore cannot migrate on
/// the way past: the one command whose job is to audit indexes was the one that failed on an old
/// one, and it blamed contention while doing it. Observed on two projects on 2026-09-02, where
/// `status` reported `storage_busy: no such table: reading_notes` and a single `impact` -- which
/// opens read-write and migrates -- made the same `status` succeed.
///
/// SQLite reports a missing table as a generic `SQLITE_ERROR` with no distinct code, so the
/// message is the only thing left to read.
pub fn classify_sqlite(err: &rusqlite::Error) -> CortError {
    let message = err.to_string();
    if message.contains("no such table") || message.contains("no such column") {
        return CortError::new(
            "schema_outdated",
            json!({
                "message": message,
                "hint": "this index predates the current schema -- run `cort index` to rebuild it",
            }),
        );
    }
    CortError::new("storage_busy", json!({ "message": message }))
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
