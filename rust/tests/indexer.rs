//! C2-1..C2-7 — the Rust port kept the case ids (audit F-12).
//! Plus plan §7 B-gap: canonicalize → project_id_for.

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, get_meta, open_db, project_id_for};
use cort::indexer::{full_index, project_id_for_root, status_of, walk_files};
use rusqlite::params;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

const SAMPLE: &[(&str, &str)] = &[
    (
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    ),
    (
        "src/alpha.ts",
        "import { helper } from './helper';\n\
export function alpha(a: number) { return helper(a) + 1; }\n\
export class Beta {\n\
  go() { return alpha(2); }\n\
}\n",
    ),
    (
        "node_modules/pkg/index.ts",
        "export function shouldBeIgnored() {}\n",
    ),
    ("README.md", "# not a source file\n"),
];

fn make_project(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-proj-")
        .tempdir()
        .unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, body).unwrap();
    }
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn setup(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let (dir, root) = make_project(files);
    let db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    (dir, root, db, project_id, bin)
}

/// C2-1
#[test]
fn walk_files_skips_ignored_dirs_and_non_source_extensions() {
    let (_dir, root, _db, _id, _bin) = setup(SAMPLE);
    assert_eq!(
        walk_files(&root),
        vec!["src/alpha.ts".to_string(), "src/helper.ts".to_string()]
    );
}

/// C2-2
#[test]
fn walk_files_includes_rust_sources_and_full_index_stores_function_fragments() {
    let (_dir, root, mut db, _id, bin) = setup(&[
        (
            "src/main.rs",
            "fn small() -> i32 {\n    1\n}\n\nfn other() -> i32 {\n    2\n}\n",
        ),
        ("README.md", "not source"),
    ]);
    assert_eq!(walk_files(&root), vec!["src/main.rs".to_string()]);
    let stats = full_index(&mut db, &bin, &root).unwrap();
    assert_eq!(stats.files, 1);
    assert_eq!(stats.unparsed, 0);
    let chunks: Vec<String> = db
        .prepare(
            "SELECT symbol_name FROM chunks WHERE file_path = 'src/main.rs' ORDER BY start_line",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(chunks, vec!["small".to_string(), "other".to_string()]);
    let content: String = db
        .query_row(
            "SELECT content FROM chunks WHERE file_path = 'src/main.rs' ORDER BY start_line LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(content, "fn small() -> i32 {\n    1\n}");
}

/// C2-3
#[test]
fn a_full_index_writes_chunks_fts_rows_file_state_and_meta() {
    let (_dir, root, mut db, project_id, bin) = setup(SAMPLE);
    let stats = full_index(&mut db, &bin, &root).unwrap();
    assert_eq!(stats.files, 2);
    assert!(stats.chunks >= 4);
    assert_eq!(stats.unparsed, 0);

    let mut stmt = db
        .prepare("SELECT symbol_name, chunk_type, chunk_source FROM chunks WHERE project_id = ?1 ORDER BY file_path, start_line")
        .unwrap();
    let rows: Vec<(Option<String>, String, String)> = stmt
        .query_map(params![project_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(rows
        .iter()
        .any(|(s, t, _)| s.as_deref() == Some("alpha") && t == "function"));
    assert!(rows
        .iter()
        .any(|(s, t, _)| s.as_deref() == Some("Beta") && t == "class"));
    assert!(rows
        .iter()
        .any(|(s, t, _)| s.as_deref() == Some("go") && t == "method"));
    assert!(rows.iter().all(|(_, _, src)| src == "ast"));

    let fts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'helper'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(fts > 0);

    let states: Vec<String> = db
        .prepare("SELECT file_content_hash FROM file_state WHERE project_id = ?1")
        .unwrap()
        .query_map(params![project_id], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(states.len(), 2);
    assert!(states.iter().all(|h| h.len() == 64
        && h.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())));

    let version = cort::pack::extractor_version();
    assert_eq!(
        get_meta(&db, "extractor_version").unwrap().as_deref(),
        Some(version.as_str())
    );
    let (path, extractor_version, last_indexed_at): (String, String, i64) = db
        .query_row(
            "SELECT path, extractor_version, last_indexed_at FROM projects WHERE project_id = ?1",
            params![project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(path, root.to_str().unwrap());
    assert_eq!(extractor_version, version);
    assert!(last_indexed_at > 0);
}

/// C2-4
#[test]
fn re_indexing_is_idempotent_no_duplicate_chunks_no_orphan_fts_rows() {
    let (_dir, root, mut db, _id, bin) = setup(SAMPLE);
    full_index(&mut db, &bin, &root).unwrap();
    let first: i64 = db
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .unwrap();
    let fts_first: i64 = db
        .query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))
        .unwrap();
    full_index(&mut db, &bin, &root).unwrap();
    let second: i64 = db
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .unwrap();
    let fts_second: i64 = db
        .query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(second, first);
    assert_eq!(fts_second, fts_first);
}

/// C2-5
#[test]
fn an_unparsable_file_is_indexed_as_unparsed_without_failing_the_run() {
    let (_dir, root, mut db, _id, bin) = setup(&[
        ("src/ok.ts", "export function ok() { return 1; }\n"),
        ("src/bad.ts", "function (((\n"),
    ]);
    let stats = full_index(&mut db, &bin, &root).unwrap();
    assert_eq!(stats.files, 2);
    assert_eq!(stats.unparsed, 1);
    let bad_n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'src/bad.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bad_n, 1);
    let src: String = db
        .query_row(
            "SELECT chunk_source FROM chunks WHERE file_path = 'src/bad.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(src, "unparsed");
    let ok_n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'src/ok.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ok_n, 1);
}

/// C2-6 — JS monkey-patches db.prepare; we RAISE(ABORT) on the 2nd chunk insert
/// so the whole fullIndex transaction rolls back (spec §7.6 / §7.9).
#[test]
fn the_whole_index_is_one_transaction_a_mid_run_failure_leaves_the_db_untouched() {
    let (_dir, root, mut db, _id, bin) = setup(SAMPLE);
    full_index(&mut db, &bin, &root).unwrap();
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .unwrap();
    fs::write(root.join("src/new.ts"), "export function added() {}\n").unwrap();

    db.execute_batch(
        "CREATE TEMP TRIGGER boom BEFORE INSERT ON chunks
         BEGIN
           SELECT RAISE(ABORT, 'boom')
           WHERE (SELECT COUNT(*) FROM chunks) >= 1;
         END;",
    )
    .unwrap();

    let err = full_index(&mut db, &bin, &root).unwrap_err();
    assert!(
        err.to_string().contains("boom"),
        "expected injected boom, got {err}"
    );

    let after: i64 = db
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        after, before,
        "a failed full index must roll back entirely, leaving the previous index readable"
    );
    let new_n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'src/new.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new_n, 0);
}

/// C2-7
#[test]
fn status_of_reports_the_indexed_project_without_touching_ast_grep() {
    let (_dir, root, mut db, project_id, bin) = setup(SAMPLE);
    full_index(&mut db, &bin, &root).unwrap();
    let s = status_of(&db, &root).unwrap();
    assert_eq!(s.project_id, project_id);
    assert_eq!(s.path, root.to_str().unwrap());
    assert_eq!(s.files, 2);
    assert_eq!(s.readings, 0);
    assert_eq!(s.extractor_version, cort::pack::extractor_version());
    assert_eq!(s.git_head, None, "the fixture is not a git repo");
}

/// Plan §7 B-gap: index/status entry canonicalizes before project_id_for.
/// Symlink path in, real path hashed — equals the JS projectId (sha256 of realpath).
#[test]
fn canonicalize_then_hash_equals_js_project_id_for_the_same_directory() {
    let tmp = tempfile::Builder::new()
        .prefix("cort-proj-")
        .tempdir()
        .unwrap();
    let real = tmp.path().join("real_root");
    fs::create_dir(&real).unwrap();
    let link = tmp.path().join("link_root");
    symlink(&real, &link).unwrap();

    let via_link = project_id_for_root(&link).unwrap();
    let real_canon = fs::canonicalize(&real).unwrap();
    let js_id = project_id_for(real_canon.to_str().unwrap());
    assert_eq!(via_link, js_id);

    let hashed_symlink_string = project_id_for(link.to_str().unwrap());
    assert_ne!(
        via_link, hashed_symlink_string,
        "must hash the real path, not the symlink path string"
    );
}

/// Build output is not source, and `.gitignore` is where a project already says so.
///
/// The filter used to be extension plus a hard-coded `IGNORE_DIRS` list, which let generated files
/// in: on 2026-09-03 the local indexes held `out/_next/static/<hash>/_buildManifest.js` and
/// `backend/.wrangler/tmp/bundle-<hash>/middleware-loader.entry.ts`, 65 such files across three
/// projects. That is not a cost problem. `impact --symbol` counted a bundled copy of a function as
/// a dependent of the original, and printed a path that dies at the next build because the hash
/// directory is regenerated -- an edge nobody can go and check, which is the one thing this product
/// may not produce.
#[test]
fn walk_files_honours_gitignore_so_build_output_is_not_indexed() {
    let (dir, root, _db, _id, _bin) = setup(&[
        ("src/real.ts", "export function real() { return 1; }\n"),
        (
            "out/_next/static/abc123/_buildManifest.js",
            "export function built() { return 2; }\n",
        ),
        (
            "backend/.wrangler/tmp/bundle-XY/loader.ts",
            "export function bundled() { return 3; }\n",
        ),
        (".gitignore", "out/\nbackend/.wrangler/\n"),
    ]);
    let _ = dir;
    assert_eq!(
        walk_files(&root),
        vec!["src/real.ts".to_string()],
        "generated files reached the index"
    );

    // Without a .gitignore the hard-coded list is still the whole answer: this narrows the walk
    // where a project has said what it generates, and narrows nothing where it has not.
    std::fs::remove_file(root.join(".gitignore")).unwrap();
    let all = walk_files(&root);
    assert!(
        all.len() == 3,
        "removing .gitignore must restore the unfiltered walk: {all:?}"
    );
}

/// A dot-directory is not automatically ignored: `.github/workflows` is source people edit, and the
/// `ignore` crate hides dotfiles by default.
#[test]
fn walk_files_keeps_dot_directories_that_no_ignore_file_excludes() {
    let (_dir, root, _db, _id, _bin) = setup(&[
        ("src/real.ts", "export function real() { return 1; }\n"),
        (".config/tool.ts", "export function cfg() { return 2; }\n"),
    ]);
    assert_eq!(
        walk_files(&root),
        vec![".config/tool.ts".to_string(), "src/real.ts".to_string()]
    );
}
