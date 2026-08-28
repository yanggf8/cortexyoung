//! B-22..B-29 — fts.test.js
//!
//! keywordSearch JS tests go through fullIndex (Job C2). Job B seeds chunks
//! directly so we do not call indexer.

use cort::db::{ensure_schema, open_db, Db};
use cort::fts::{keyword_search, sanitize_fts_query, MAX_OR_TERMS};
use rusqlite::params;

fn sanitize(raw: &str) -> cort::fts::SanitizedFtsQuery {
    sanitize_fts_query(raw).unwrap()
}

/// B-22
#[test]
fn each_term_is_quoted_so_fts_operators_cannot_leak_through() {
    assert_eq!(sanitize("helper").query, r#""helper""#);
    assert_eq!(sanitize("foo(bar)").query, r#""foo(bar)""#);
    assert_eq!(sanitize("a - b").query, r#""a" OR "-" OR "b""#);
    assert_eq!(sanitize("src/alpha.ts").query, r#""src/alpha.ts""#);
    assert_eq!(sanitize(r#"say "hi""#).query, r#""say" OR """hi""""#);
}

/// B-23
#[test]
fn more_than_max_or_terms_terms_truncates_and_reports_it() {
    let many: String = (0..(MAX_OR_TERMS + 5))
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let s = sanitize(&many);
    assert!(s.truncated_query);
    assert_eq!(s.query.split(" OR ").count(), MAX_OR_TERMS);
    assert!(!sanitize("one two").truncated_query);
}

/// B-24
#[test]
fn an_empty_query_is_rejected_loudly() {
    let err = sanitize_fts_query("   ").unwrap_err();
    assert_eq!(err.code, "empty_query");
}

fn seed_project(db: &Db, project_id: &str) {
    db.execute(
        "INSERT INTO projects (project_id, name, path, extractor_version)
         VALUES (?1, ?2, ?3, 'v')",
        params![project_id, "n", "/n"],
    )
    .unwrap();
}

fn seed_chunk(
    db: &Db,
    project_id: &str,
    chunk_id: &str,
    file_path: &str,
    symbol_name: &str,
    content: &str,
) {
    db.execute(
        "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
            start_line, end_line, content, content_hash, language, chunk_source)
         VALUES (?1, ?2, ?3, ?4, 'function', 1, 3, ?5, 'h', 'TypeScript', 'ast')",
        params![chunk_id, project_id, file_path, symbol_name, content],
    )
    .unwrap();
}

/// SAMPLE-shaped corpus (without calling indexer).
fn indexed_sample() -> (Db, String) {
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = "proj-sample";
    seed_project(&db, project_id);
    seed_chunk(
        &db,
        project_id,
        "p:src/helper.ts:1",
        "src/helper.ts",
        "helper",
        "export function helper(n: number) { return n * 2; }\n",
    );
    seed_chunk(
        &db,
        project_id,
        "p:src/alpha.ts:1",
        "src/alpha.ts",
        "alpha",
        "export function alpha(a: number) { return helper(a) + 1; }\n",
    );
    seed_chunk(
        &db,
        project_id,
        "p:src/alpha.ts:go",
        "src/alpha.ts",
        "go",
        "go() { return alpha(2); }",
    );
    (db, project_id.to_string())
}

/// B-25
#[test]
fn keyword_search_finds_a_symbol_by_name() {
    let (db, project_id) = indexed_sample();
    let result = keyword_search(&db, &project_id, "helper", 10).unwrap();
    assert!(!result.rows.is_empty());
    assert!(result
        .rows
        .iter()
        .any(|r| r.symbol_name.as_deref() == Some("helper")));
    assert!(result.rows.iter().all(|r| !r.chunk_id.is_empty()));
}

/// B-26
#[test]
fn keyword_search_survives_punctuation_that_would_otherwise_be_fts_syntax() {
    let (db, project_id) = indexed_sample();
    keyword_search(&db, &project_id, "helper(a) - alpha", 10)
        .expect("punctuation must not blow up MATCH");
}

/// B-27
#[test]
fn unicode61_tokenizing_lets_cjk_identifiers_through() {
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = "proj-cjk";
    seed_project(&db, project_id);
    seed_chunk(
        &db,
        project_id,
        "p:src/cjk.ts:1",
        "src/cjk.ts",
        "查詢使用者",
        "export function 查詢使用者() { return 1; }\n",
    );
    let result = keyword_search(&db, project_id, "查詢使用者", 10).unwrap();
    assert!(!result.rows.is_empty());
}

/// B-28
#[test]
fn results_are_scoped_to_the_project() {
    let (db, _project_id) = indexed_sample();
    let result = keyword_search(&db, "some-other-project-id", "helper", 10).unwrap();
    assert!(result.rows.is_empty());
}

/// B-29
#[test]
fn the_limit_is_honoured() {
    let (db, project_id) = indexed_sample();
    let result = keyword_search(&db, &project_id, "return", 1).unwrap();
    assert!(result.rows.len() <= 1);
}
