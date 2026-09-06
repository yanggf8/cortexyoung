//! B-10..B-21 — the Rust port kept the case ids (audit F-12).

use cort::db::{
    cache_dir, db_path_for, delete_project, ensure_schema, get_meta, list_projects, open_db,
    project_id_for, project_root_for_path, set_meta, with_busy_retry, DeleteResult, ProjectEntry,
    SqliteErrorCode, WithBusyRetryError, SCHEMA_VERSION,
};
use rusqlite::params;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_var(key: &str, val: Option<&str>, f: impl FnOnce()) {
    let prev = std::env::var(key).ok();
    // SAFETY: tests in this file take ENV_LOCK so no other thread mutates env.
    unsafe {
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    // SAFETY: restoring the value we read above, still under ENV_LOCK.
    unsafe {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME"))
}

fn fresh() -> rusqlite::Connection {
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    db
}

/// B-10
#[test]
fn project_id_is_a_stable_sha256_of_the_real_path() {
    let a = project_id_for("/tmp/some/project");
    assert!(
        a.len() == 64
            && a.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "expected 64 lowercase hex, got {a}"
    );
    assert_eq!(a, project_id_for("/tmp/some/project"));
    assert_ne!(a, project_id_for("/tmp/other/project"));
}

/// B-11
#[test]
fn db_path_lands_under_the_cortex_ng_cache_keyed_by_project_id() {
    let _g = env_guard();
    with_var("CORT_CACHE_DIR", None, || {
        let p = db_path_for("/tmp/some/project");
        let expected = home_dir()
            .join(".cache")
            .join("cortex-ng")
            .join(format!("{}.db", project_id_for("/tmp/some/project")));
        assert_eq!(p, expected);
        assert_eq!(cache_dir(), home_dir().join(".cache").join("cortex-ng"));
    });
}

/// B-12
#[test]
fn ensure_schema_is_idempotent_and_records_the_schema_version() {
    let db = fresh();
    let expected = SCHEMA_VERSION.to_string();
    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some(expected.as_str())
    );
    ensure_schema(&db).unwrap();
    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some(SCHEMA_VERSION.to_string().as_str())
    );
}

/// B-13
#[test]
fn ensure_schema_upgrades_a_v1_database_with_the_reading_notes_fts_layer() {
    let db = open_db(":memory:").unwrap();
    db.execute_batch(
        "CREATE TABLE _cortex_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO _cortex_meta (key, value) VALUES ('SCHEMA_VERSION', '1');",
    )
    .unwrap();
    ensure_schema(&db).unwrap();
    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some(SCHEMA_VERSION.to_string().as_str())
    );
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'view')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(tables.iter().any(|n| n == "reading_notes"));
    assert!(tables.iter().any(|n| n == "reading_notes_fts"));
}

/// B-14
#[test]
fn schema_uses_the_v6_column_names_required_by_the_spec() {
    let db = fresh();
    let cols = |t: &str| -> Vec<String> {
        db.prepare(&format!("PRAGMA table_info({t})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert!(cols("projects").iter().any(|c| c == "project_id"));
    assert!(cols("chunks").iter().any(|c| c == "chunk_id"));
    assert!(cols("chunks").iter().any(|c| c == "chunk_source"));
    assert!(!cols("chunks").iter().any(|c| c == "embedding"));
    assert!(cols("relationships").iter().any(|c| c == "rel_type"));
    assert!(cols("file_state").iter().any(|c| c == "file_content_hash"));
    assert!(cols("reading_notes").iter().any(|c| c == "source_hash"));
    assert!(cols("reading_notes").iter().any(|c| c == "ends_at_eof"));
    assert!(cols("reading_notes").iter().any(|c| c == "read_count"));
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!tables.iter().any(|n| n == "unresolved_refs"));
}

/// B-15
#[test]
fn relationships_primary_key_is_the_composite_triple() {
    let db = fresh();
    let mut rows: Vec<(i64, String)> = db
        .prepare("PRAGMA table_info(relationships)")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(5)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .filter(|(pk, _)| *pk > 0)
        .collect();
    rows.sort_by_key(|(pk, _)| *pk);
    let pk: Vec<String> = rows.into_iter().map(|(_, n)| n).collect();
    assert_eq!(
        pk,
        vec![
            "source_chunk_id".to_string(),
            "target_chunk_id".to_string(),
            "rel_type".to_string()
        ]
    );
}

/// B-16
#[test]
fn fts_triggers_mirror_chunk_writes() {
    let db = fresh();
    db.execute(
        "INSERT INTO projects (project_id, name, path, extractor_version) VALUES ('p','n','/n','v')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
            start_line, end_line, content, content_hash, language, chunk_source)
            VALUES ('p:a.ts:1','p','a.ts','alpha','function',1,3,'function alpha() {}','h','TypeScript','ast')",
        [],
    )
    .unwrap();
    let hit: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'alpha'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hit, 1);
    db.execute("DELETE FROM chunks WHERE chunk_id = 'p:a.ts:1'", [])
        .unwrap();
    let after: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'alpha'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, 0);
}

/// B-17
#[test]
fn zero_target_relationships_are_impossible_target_chunk_id_is_not_null() {
    let db = fresh();
    let notnull: i64 = db
        .prepare("PRAGMA table_info(relationships)")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .find(|(name, _)| name == "target_chunk_id")
        .unwrap()
        .1;
    assert_eq!(notnull, 1);
}

/// B-18
#[test]
fn list_projects_enumerates_every_indexed_project_in_the_cache_dir() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        assert!(list_projects().is_empty());
        let root = tempfile::tempdir().unwrap();
        let root_s = root.path().to_str().unwrap();
        let db = open_db(db_path_for(root_s)).unwrap();
        ensure_schema(&db).unwrap();
        let pid = project_id_for(root_s);
        let name = root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        db.execute(
            "INSERT INTO projects (project_id, name, path, extractor_version)
             VALUES (?1, ?2, ?3, 'v')",
            params![pid, name, root_s],
        )
        .unwrap();
        drop(db);
        let rows = list_projects();
        assert_eq!(rows.len(), 1);
        let ProjectEntry::Indexed(r) = &rows[0] else {
            panic!("expected an indexed project, got {:?}", rows[0]);
        };
        assert_eq!(r.path, root_s);
        assert!(r
            .db_path
            .ends_with(&format!("{}.db", project_id_for(root_s))));
    });
}

/// B-19
#[test]
fn delete_project_removes_only_that_project_db_and_reports_what_it_did() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        let root = tempfile::tempdir().unwrap();
        let root_s = root.path().to_str().unwrap();
        let db_path = db_path_for(root_s);
        open_db(&db_path).unwrap();
        assert!(db_path.exists());
        assert_eq!(
            delete_project(root_s),
            DeleteResult {
                deleted: true,
                db_path: db_path.to_string_lossy().into_owned(),
            }
        );
        assert!(!db_path.exists());
        assert_eq!(
            delete_project(root_s),
            DeleteResult {
                deleted: false,
                db_path: db_path.to_string_lossy().into_owned(),
            }
        );
    });
}

struct CodeErr {
    code: &'static str,
    msg: &'static str,
}

impl std::fmt::Display for CodeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error: {}", self.msg)
    }
}

impl SqliteErrorCode for CodeErr {
    fn sqlite_code(&self) -> Option<&str> {
        Some(self.code)
    }
}

/// B-20
#[test]
fn with_busy_retry_retries_sqlite_busy_and_gives_up_after_three_retries() {
    let mut calls = 0;
    let value = with_busy_retry(|| {
        calls += 1;
        if calls < 3 {
            return Err(CodeErr {
                code: "SQLITE_BUSY",
                msg: "busy",
            });
        }
        Ok("ok")
    })
    .unwrap_or_else(|_| panic!("expected ok"));
    assert_eq!(value, "ok");
    assert_eq!(calls, 3);

    let mut always = 0;
    let err = with_busy_retry(|| -> Result<(), _> {
        always += 1;
        Err(CodeErr {
            code: "SQLITE_BUSY",
            msg: "busy",
        })
    })
    .unwrap_err();
    match err {
        WithBusyRetryError::Cort(e) => assert_eq!(e.code, "storage_busy"),
        WithBusyRetryError::Other(_) => panic!("expected CortError storage_busy"),
    }
    assert_eq!(always, 4, "one attempt plus three retries");
}

/// B-21
#[test]
fn with_busy_retry_converts_a_full_or_corrupt_db_into_storage_full() {
    let err = with_busy_retry(|| -> Result<(), _> {
        Err(CodeErr {
            code: "SQLITE_FULL",
            msg: "disk full",
        })
    })
    .unwrap_err();
    match err {
        WithBusyRetryError::Cort(e) => assert_eq!(e.code, "storage_full"),
        WithBusyRetryError::Other(_) => panic!("expected CortError storage_full"),
    }
}

// ---------------------------------------------------------------------------
// schema v4: `raw_edges.call_form`, `relationships.call_site_line` + `call_form`.
//
// The reason this is a migration and not a rebuild: `CREATE TABLE IF NOT EXISTS` never adds a
// column to a table that already exists, so an older database would come back with a schema
// version of 4 and columns that do not exist -- and fail at the first insert, on someone's
// repository, hours later.
// ---------------------------------------------------------------------------

/// The two tables exactly as schema v3 defined them, plus one row in each.
const V3_SHAPED_DB: &str = "
CREATE TABLE _cortex_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO _cortex_meta (key, value) VALUES ('SCHEMA_VERSION', '3');
CREATE TABLE projects (
  project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL, git_head TEXT,
  last_indexed_at INTEGER, extractor_version TEXT NOT NULL, created_at TEXT DEFAULT (datetime('now')));
INSERT INTO projects (project_id, name, path, extractor_version) VALUES ('p', 'p', '/p', 'v3');
CREATE TABLE chunks (
  chunk_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL, symbol_name TEXT,
  chunk_type TEXT CHECK(chunk_type IN ('function','class','method','config','documentation','unparsed')),
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, content TEXT NOT NULL,
  content_hash TEXT NOT NULL, language TEXT,
  chunk_source TEXT NOT NULL CHECK(chunk_source IN ('ast','unparsed')),
  created_at TEXT DEFAULT (datetime('now')), updated_at TEXT DEFAULT (datetime('now')));
INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type, start_line, end_line,
  content, content_hash, language, chunk_source)
  VALUES ('p:a.rs:1','p','a.rs','T::take','method',1,3,'fn take(){}','h','Rust','ast');
INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type, start_line, end_line,
  content, content_hash, language, chunk_source)
  VALUES ('p:b.rs:1','p','b.rs','go','function',1,3,'fn go(){}','h2','Rust','ast');
CREATE TABLE relationships (
  source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
  confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
  confidence_reasoning TEXT,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type));
INSERT INTO relationships (source_chunk_id, target_chunk_id, rel_type, confidence,
  confidence_score, confidence_reasoning)
  VALUES ('p:b.rs:1','p:a.rs:1','calls','INFERRED',0.7,'resolved: take');
CREATE TABLE raw_edges (
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL, source_symbol TEXT NOT NULL DEFAULT '', raw_target TEXT NOT NULL,
  rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
  start_line INTEGER NOT NULL,
  PRIMARY KEY (project_id, file_path, rel_type, raw_target, source_symbol, start_line));
INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target, rel_type, start_line)
  VALUES ('p','b.rs','go','take','calls',2);
";

fn columns(db: &rusqlite::Connection, table: &str) -> Vec<String> {
    db.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn a_v3_database_is_upgraded_in_place_and_its_rows_survive_the_column_addition() {
    let db = open_db(":memory:").unwrap();
    db.execute_batch(V3_SHAPED_DB).unwrap();
    ensure_schema(&db).unwrap();

    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some(SCHEMA_VERSION.to_string().as_str())
    );
    // The columns, on the tables that already existed: an ALTER that silently no-opped is the
    // failure this test exists to catch.
    for table in ["raw_edges", "relationships"] {
        let cols = columns(&db, table);
        assert!(cols.iter().any(|c| c == "call_form"), "{table}: {cols:?}");
    }
    assert!(columns(&db, "relationships")
        .iter()
        .any(|c| c == "call_site_line"));
    assert_eq!(
        get_meta(&db, "graph_pending").unwrap().as_deref(),
        Some("1"),
        "a graph whose rows predate the new columns is not trusted until a full re-index"
    );

    let (target, form, line): (String, String, Option<i64>) = db
        .query_row(
            "SELECT target_chunk_id, call_form, call_site_line FROM relationships",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((target.as_str(), form.as_str()), ("p:a.rs:1", "bare"));
    assert_eq!(line, None, "a v3 row cannot claim a line it never recorded");
    let raw_form: String = db
        .query_row("SELECT call_form FROM raw_edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(raw_form, "bare", "old edges mean what they always meant");
}

#[test]
fn the_call_form_column_is_checked_on_upgraded_and_fresh_databases() {
    // An `ALTER TABLE ADD COLUMN` that accepted the CHECK text but did not enforce it would let a
    // bad form into the column the receiver gate reads its policy from.
    let upgraded = open_db(":memory:").unwrap();
    upgraded.execute_batch(V3_SHAPED_DB).unwrap();
    ensure_schema(&upgraded).unwrap();
    let fresh = fresh();

    for (label, db) in [("upgraded", &upgraded), ("fresh", &fresh)] {
        let err = db
            .execute(
                "INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target,
                   rel_type, call_form, start_line)
                 VALUES ('p','a.rs','go','take','calls','reciever',2)",
                [],
            )
            .expect_err("a misspelled form must not be storable");
        assert!(
            err.to_string().contains("CHECK"),
            "{label}: expected a CHECK violation, got {err}"
        );
        let rel_err = db
            .execute(
                "INSERT INTO relationships (source_chunk_id, target_chunk_id, rel_type,
                   confidence, confidence_score, call_form)
                 VALUES ('p:b.rs:1','p:a.rs:1','calls','INFERRED',0.7,'maybe')",
                [],
            )
            .expect_err("a made-up form must not be storable");
        assert!(rel_err.to_string().contains("CHECK"), "{label}: {rel_err}");
    }
}

#[test]
fn re_running_the_v4_upgrade_is_a_no_op() {
    let db = open_db(":memory:").unwrap();
    db.execute_batch(V3_SHAPED_DB).unwrap();
    ensure_schema(&db).unwrap();
    ensure_schema(&db).unwrap();
    ensure_schema(&fresh()).unwrap();
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM raw_edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "no duplicated edges from a repeated migration");
}

/// Every sqlite failure used to be reported as `storage_busy`. Contention clears on retry; a table
/// that is not there does not, and the caller needs to be told to rebuild instead of to wait.
#[test]
fn a_missing_table_is_classified_as_an_outdated_schema_not_contention() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let err = db
        .query_row("SELECT COUNT(*) FROM reading_notes", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_err();
    let mapped = cort::db::classify_sqlite(&err);
    assert_eq!(mapped.code, "schema_outdated");
    assert!(
        mapped.detail["hint"]
            .as_str()
            .unwrap_or("")
            .contains("cort index"),
        "{:?}",
        mapped.detail
    );
}

#[test]
fn an_ordinary_sqlite_failure_is_still_storage_busy() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE t (a INTEGER);").unwrap();
    let err = db.execute_batch("this is not sql").unwrap_err();
    assert_eq!(cort::db::classify_sqlite(&err).code, "storage_busy");
}

/// The alignment property, on a REAL v3 file rather than a fresh one with its metadata rewritten.
///
/// `migrate_v4` adds columns with `ALTER TABLE ADD COLUMN`, which APPENDS, so a v3→v4 database has
/// `[... rel_type, confidence, confidence_score, confidence_reasoning, call_site_line, call_form]`
/// while a fresh one from `schema.sql` has
/// `[... rel_type, call_site_line, call_form, confidence, confidence_score, confidence_reasoning]`.
/// Five of eight columns differ. A `SELECT *` rebuild would silently misalign them — `confidence`
/// would land in `call_site_line` — and a test that starts from a fresh schema can never see it.
#[test]
fn migrating_a_real_v3_database_to_v5_preserves_and_aligns_every_row() {
    let db = open_db(":memory:").unwrap();
    db.execute_batch(V3_SHAPED_DB).unwrap();
    ensure_schema(&db).unwrap();

    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some("5")
    );
    assert_eq!(
        get_meta(&db, "graph_pending").unwrap().as_deref(),
        Some("1"),
        "a widened graph is not trusted until a full re-index"
    );

    // Read every field back BY NAME. This is what a `SELECT *` rebuild breaks.
    let (rel, conf, score, reasoning): (String, String, f64, Option<String>) = db
        .query_row(
            "SELECT rel_type, confidence, confidence_score, confidence_reasoning
               FROM relationships WHERE source_chunk_id = 'p:b.rs:1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(rel, "calls");
    assert_eq!(conf, "INFERRED");
    assert_eq!(score, 0.7);
    assert_eq!(reasoning.as_deref(), Some("resolved: take"));

    let (target, line, start): (String, String, i64) = db
        .query_row(
            "SELECT raw_target, rel_type, start_line FROM raw_edges WHERE source_symbol = 'go'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (target.as_str(), line.as_str(), start),
        ("take", "calls", 2)
    );

    // The anti-drift assertion, and the one that pins what this migration exists for.
    // `V5_RELATIONSHIPS`/`V5_RAW_EDGES` are a THIRD copy of these table bodies — `schema.sql` plus
    // two consts — and nothing else checks that they agree. The v4 misalignment this migration fixes
    // WAS that kind of drift. A migrated database must end up with the same column ORDER as a fresh
    // one, not merely the same column set, or the next `SELECT *` anywhere silently lies.
    let fresh = fresh();
    for table in ["relationships", "raw_edges"] {
        assert_eq!(
            columns(&db, table),
            columns(&fresh, table),
            "{table}: a migrated database must match a fresh one column for column, in order"
        );
    }

    // And the point of the whole migration.
    db.execute(
        "INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target, rel_type,
           call_form, start_line)
         VALUES ('p','a.rs','caller','settings::SettingsError','references','type',9)",
        [],
    )
    .expect("v5 accepts a qualified type reference");
}

/// A rebuild that died half way must leave the database retryable, not wedged behind a stray table.
#[test]
fn a_stale_v5_temporary_table_does_not_wedge_the_next_upgrade() {
    let db = open_db(":memory:").unwrap();
    db.execute_batch(V3_SHAPED_DB).unwrap();
    db.execute_batch("CREATE TABLE relationships__v5 (bogus TEXT);")
        .unwrap();

    ensure_schema(&db).expect("a stale temporary table is cleared, not fatal");

    let stale: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%__v5'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stale, 0, "the rebuild renames its temporary table away");
    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some("5")
    );
}

/// A database file with a schema and no project row -- the state any command that opens a project
/// leaves behind, and the one `project_root_for_path` must refuse.
fn mark_schema_only(root: &std::path::Path) {
    let db = open_db(db_path_for(root.to_str().unwrap())).unwrap();
    ensure_schema(&db).unwrap();
}

/// A real index: schema plus the `projects` row with `last_indexed_at` set, which is what
/// `full_index` writes and what `status_of` calls `indexed: true`.
fn mark_indexed(root: &std::path::Path) {
    let db = open_db(db_path_for(root.to_str().unwrap())).unwrap();
    ensure_schema(&db).unwrap();
    db.execute(
        "INSERT INTO projects (project_id, name, path, last_indexed_at, extractor_version)
         VALUES (?1, 'p', ?2, 1, 'v')
         ON CONFLICT(project_id) DO UPDATE SET last_indexed_at = 1",
        params![
            project_id_for(root.to_str().unwrap()),
            root.to_str().unwrap()
        ],
    )
    .unwrap();
}

/// The nearest indexed ancestor, not the git root: one repository can hold several indexed projects
/// (this machine has `b/finance-engineering` and `b/finance-engineering/tools/finance-cli`), and a
/// file under the inner one belongs to the inner one.
///
/// A schema-only database is walked past, and that is the property that keeps this hook honest: a
/// resolver that stopped there would hand the root to `incremental_index`, which falls through to
/// `full_index` on a version or candidate mismatch, and `full_index` INSERTS the project row -- the
/// repair hook creating an index in a directory nobody asked about.
#[test]
fn a_path_resolves_to_its_nearest_indexed_ancestor() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let outer = std::fs::canonicalize(tree.path()).unwrap();
    std::fs::create_dir_all(outer.join("tools/inner/src")).unwrap();
    let inner = std::fs::canonicalize(outer.join("tools/inner")).unwrap();
    std::fs::write(inner.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    let file = inner.join("src/lib.rs");

    with_var(
        "CORT_CACHE_DIR",
        Some(cache.path().to_str().unwrap()),
        || {
            assert_eq!(
                project_root_for_path(&file),
                Ok(None),
                "nothing indexed yet"
            );

            mark_schema_only(&outer);
            assert_eq!(
                project_root_for_path(&file),
                Ok(None),
                "a db file with no projects row is not an index"
            );

            mark_indexed(&outer);
            assert_eq!(project_root_for_path(&file), Ok(Some(outer.clone())));

            mark_indexed(&inner);
            assert_eq!(
                project_root_for_path(&file),
                Ok(Some(inner.clone())),
                "the nearer answer wins"
            );
        },
    );
}

/// A path that does not exist still resolves through its existing ancestors -- an edit hook is
/// handed the file a tool just deleted.
#[test]
fn a_deleted_path_still_resolves_through_its_parents() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tree.path()).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();

    with_var(
        "CORT_CACHE_DIR",
        Some(cache.path().to_str().unwrap()),
        || {
            mark_indexed(&root);
            assert_eq!(
                project_root_for_path(&root.join("src/gone.rs")),
                Ok(Some(root.clone())),
                "the file is gone; its directory is not"
            );
        },
    );
}

/// A database that will not answer stops the walk instead of being read as "no row here".
///
/// Collapsing the two is how a momentarily locked inner project gets skipped and the repair lands on
/// the outer one: a project nobody edited is refreshed, the usage row is filed under the wrong
/// project, and the edited project stays broken behind a row claiming otherwise.
#[test]
fn an_unreadable_database_stops_the_walk_rather_than_diverting_it() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let outer = std::fs::canonicalize(tree.path()).unwrap();
    std::fs::create_dir_all(outer.join("inner/src")).unwrap();
    let inner = std::fs::canonicalize(outer.join("inner")).unwrap();
    let file = inner.join("src/lib.rs");
    std::fs::write(&file, "pub fn f() {}\n").unwrap();

    with_var(
        "CORT_CACHE_DIR",
        Some(cache.path().to_str().unwrap()),
        || {
            mark_indexed(&outer);
            // The inner project's database is present and is not a database.
            std::fs::write(db_path_for(inner.to_str().unwrap()), b"not sqlite at all").unwrap();
            assert_eq!(
            project_root_for_path(&file),
            Err(cort::db::RootUnreadable),
            "an unreadable inner database must not silently route the repair to the outer project"
        );
        },
    );
}

/// The two facts that decide whether an index is usable live in `_cortex_meta`, and until 2026-09-06
/// nothing that enumerates projects read either. That is why 7 of 10 projects sat on a superseded
/// extractor while `--check` reported "all current".
///
/// The stored schema is asserted at an **old** value on purpose: a fixture that stores the current
/// one cannot tell a real read from `SCHEMA_VERSION.to_string()`.
#[test]
fn list_projects_reports_the_schema_and_extractor_each_index_was_built_with() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        let root = tempfile::tempdir().unwrap();
        let root_s = root.path().to_str().unwrap();
        let db = open_db(db_path_for(root_s)).unwrap();
        ensure_schema(&db).unwrap();
        let pid = project_id_for(root_s);
        let name = root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        db.execute(
            "INSERT INTO projects (project_id, name, path, extractor_version)
             VALUES (?1, ?2, ?3, 'stale-extractor')",
            params![pid, name, root_s],
        )
        .unwrap();
        set_meta(&db, "extractor_version", "stale-extractor").unwrap();
        set_meta(&db, "SCHEMA_VERSION", "3").unwrap();
        drop(db);

        let rows = list_projects();
        assert_eq!(rows.len(), 1);
        let ProjectEntry::Indexed(r) = &rows[0] else {
            panic!("expected an indexed project, got {:?}", rows[0]);
        };
        assert_eq!(r.extractor_version.as_deref(), Some("stale-extractor"));
        assert_eq!(
            r.schema_version.as_deref(),
            Some("3"),
            "the stored schema must be read, not assumed: {:?}",
            r.schema_version
        );
    });
}

/// A metadata read that *fails* is not a metadata key that is *absent*. Both were flattened to
/// `None` until 2026-09-06, and `None` is counted as drift -- so a database whose `_cortex_meta`
/// could not be read was reported as "built by a superseded extractor", a positive claim about a
/// version this binary never saw. The scan connection takes no busy timeout and the refresh hook
/// writes on every edit, so a transient `SQLITE_BUSY` is enough to reach it.
#[test]
fn a_metadata_read_that_fails_is_unreadable_rather_than_drifted() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        let root = tempfile::tempdir().unwrap();
        let root_s = root.path().to_str().unwrap();
        let db = open_db(db_path_for(root_s)).unwrap();
        ensure_schema(&db).unwrap();
        let pid = project_id_for(root_s);
        let name = root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        db.execute(
            "INSERT INTO projects (project_id, name, path, extractor_version)
             VALUES (?1, ?2, ?3, 'whatever')",
            params![pid, name, root_s],
        )
        .unwrap();
        // The projects row stays readable; only the metadata becomes unreadable.
        db.execute_batch("DROP TABLE _cortex_meta").unwrap();
        drop(db);

        let entries = list_projects();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert!(
            matches!(entries[0], ProjectEntry::Unreadable { .. }),
            "a database whose metadata will not answer must not be reported as a known version \
             mismatch: {entries:?}"
        );
    });
}

/// A cache directory that will not enumerate is not a machine with no indexes. Returning an empty
/// population made `--verdict` answer `compatible 0 0` about a directory it never read.
#[test]
fn a_cache_directory_that_will_not_enumerate_is_not_an_empty_one() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        // Listing a directory needs its read bit, so 0o600 is not enough -- that still enumerates.
        // 0o000 is what makes `read_dir` fail while the directory itself still stats.
        let mut perms = std::fs::metadata(cache.path()).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        std::fs::set_permissions(cache.path(), perms.clone()).unwrap();

        let readable = std::fs::read_dir(cache.path()).is_ok();
        let entries = list_projects();

        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o700);
        std::fs::set_permissions(cache.path(), perms).unwrap();

        if readable {
            // root, or a filesystem that does not enforce this. The property is untestable here
            // rather than false, and saying so is better than asserting something vacuous.
            eprintln!("SKIP: this user can read a 0o000 directory");
            return;
        }
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert!(
            matches!(entries[0], ProjectEntry::Unreadable { .. }),
            "an unreadable cache directory is reported, not reduced to an empty population: \
             {entries:?}"
        );
    });
}

/// `usage.db` lives in the same cache directory and ends in `.db`, but it is the recorder, not an
/// index: `_usage_meta`, no `projects` table (`usage.rs:100-109`). It must never appear in the
/// project population. Three recorder-isolation tests (`rust/tests/usage.rs`) deliberately make it
/// busy or read-only and assert that `cort projects` stdout is byte-identical, so a version of this
/// scan that notices `usage.db` at all breaks them.
#[test]
fn the_usage_recorder_is_not_a_project() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        std::fs::write(cache.path().join("usage.db"), b"not a project index").unwrap();
        assert!(
            list_projects().is_empty(),
            "the recorder is not part of the project population: {:?}",
            list_projects()
        );
    });
}

/// A database that exists and will not answer is not an absent project -- the same conflation
/// `RootProbe::Unreadable` exists to prevent one level down (`db.rs:544`, `db.rs:556`).
///
/// Both failure arms are exercised, because they are different code: a **directory** named `*.db`
/// fails at `Connection::open_with_flags`, while a **file of junk** usually opens fine and fails on
/// the first query. A fixture with only the second cannot detect a regression in the first.
///
/// A database with no `projects` row must stay skipped: `ensure_schema` creates that shape before
/// anything is indexed, so it is correctly not a project.
#[test]
fn an_index_that_will_not_answer_is_reported_rather_than_skipped() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        std::fs::create_dir(cache.path().join("adirectory.db")).unwrap();
        std::fs::write(cache.path().join("junk.db"), b"this is not a sqlite file").unwrap();

        let empty_root = tempfile::tempdir().unwrap();
        let db = open_db(db_path_for(empty_root.path().to_str().unwrap())).unwrap();
        ensure_schema(&db).unwrap();
        drop(db);

        let entries = list_projects();
        let mut unreadable: Vec<String> = entries
            .iter()
            .filter_map(|e| match e {
                ProjectEntry::Unreadable { db_path, .. } => Some(db_path.clone()),
                ProjectEntry::Indexed(_) => None,
            })
            .collect();
        unreadable.sort();
        assert_eq!(unreadable.len(), 2, "entries: {entries:?}");
        assert!(unreadable[0].ends_with("adirectory.db"), "{unreadable:?}");
        assert!(unreadable[1].ends_with("junk.db"), "{unreadable:?}");

        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e, ProjectEntry::Indexed(_)))
                .count(),
            0,
            "a schema-only database is not a project: {entries:?}"
        );
    });
}
