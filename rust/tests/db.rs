//! B-10..B-21 — the Rust port kept the case ids (audit F-12).

use cort::db::{
    cache_dir, db_path_for, delete_project, ensure_schema, get_meta, list_projects, open_db,
    project_id_for, with_busy_retry, DeleteResult, SqliteErrorCode, WithBusyRetryError,
    SCHEMA_VERSION,
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
        assert_eq!(rows[0].path, root_s);
        assert!(rows[0]
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
