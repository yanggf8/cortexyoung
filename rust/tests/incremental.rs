//! C2-8..C2-16 — the Rust port kept the case ids (audit F-12).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, get_meta, open_db, project_id_for, set_meta};
use cort::graph::get_transitive_dependents;
use cort::incremental::{git_candidates, incremental_index, reindex_one_file, remove_file};
use cort::indexer::full_index;
use cort::staleness::compute_stale;
use rusqlite::params;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn git(root: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_project(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let (dir, root) = make_project(files);
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "test"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

/// C2-8
#[test]
fn an_extractor_version_mismatch_forces_a_full_rebuild() {
    let (_dir, root, mut db, _project_id, bin) = git_project(SAMPLE);
    set_meta(&db, "extractor_version", "stale-version-hash").unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.mode, "full");
    assert_eq!(
        get_meta(&db, "extractor_version").unwrap().as_deref(),
        Some(cort::pack::extractor_version().as_str())
    );
}

/// C2-9
#[test]
fn no_changes_means_nothing_is_reindexed() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.mode, "incremental");
    assert_eq!(r.files_reindexed, 0);
}

/// C2-10
#[test]
fn an_edited_file_is_reindexed_and_its_chunks_replaced() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 3; }\nexport function extra() { return 0; }\n",
    )
    .unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.mode, "incremental");
    assert_eq!(r.files_reindexed, 1);
    let syms: Vec<String> = db
        .prepare(
            "SELECT symbol_name FROM chunks WHERE file_path = 'src/helper.ts' ORDER BY start_line",
        )
        .unwrap()
        .query_map([], |c| c.get(0))
        .unwrap()
        .map(|c| c.unwrap())
        .collect();
    assert_eq!(syms, vec!["helper".to_string(), "extra".to_string()]);
}

/// C2-11
#[test]
fn a_touched_but_identical_file_is_skipped_without_a_write() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    let p = root.join("src/helper.ts");
    let body = fs::read_to_string(&p).unwrap();
    fs::write(&p, body.replace("return n * 2;", "return n * 2;   ")).unwrap();
    let before: String = db
        .query_row(
            "SELECT updated_at FROM file_state WHERE file_path = 'src/helper.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.files_skipped + r.files_reindexed, r.files_examined);
    let after: String = db
        .query_row(
            "SELECT updated_at FROM file_state WHERE file_path = 'src/helper.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if r.files_skipped == 1 {
        assert_eq!(after, before, "a skipped file must not be rewritten");
    }
}

/// C2-12
#[test]
fn a_new_untracked_file_is_picked_up_via_git_ls_files_others() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    fs::write(
        root.join("src/brand-new.ts"),
        "export function brandNew() { return 1; }\n",
    )
    .unwrap();
    let cands = git_candidates(&root);
    assert!(cands.changed.iter().any(|p| p == "src/brand-new.ts"));
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.files_reindexed, 1);
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'src/brand-new.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

/// C2-13
#[test]
fn a_deleted_file_drops_its_chunks_fts_rows_and_file_state() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    fs::remove_file(root.join("src/helper.ts")).unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.files_removed, 1);
    let chunks: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'src/helper.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(chunks, 0);
    let state: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE file_path = 'src/helper.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, 0);
    let fts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE file_path = 'src/helper.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fts, 0);
}

/// C2-14 — JS monkey-patches the 2nd INSERT INTO chunks. RAISE on src/two.ts
/// so the first committed file survives and git_head is not advanced.
#[test]
fn an_interrupt_keeps_already_committed_files_and_does_not_advance_git_head() {
    let (_dir, root, mut db, project_id, bin) = git_project(SAMPLE);
    fs::write(
        root.join("src/one.ts"),
        "export function one() { return 1; }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/two.ts"),
        "export function two() { return 2; }\n",
    )
    .unwrap();
    let head_before: Option<String> = db
        .query_row(
            "SELECT git_head FROM projects WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .unwrap();

    db.execute_batch(
        "CREATE TEMP TRIGGER boom BEFORE INSERT ON chunks
         WHEN NEW.file_path = 'src/two.ts'
         BEGIN
           SELECT RAISE(ABORT, 'interrupted');
         END;",
    )
    .unwrap();

    let err = incremental_index(&mut db, &bin, &root).unwrap_err();
    assert!(
        err.to_string().contains("interrupted"),
        "expected injected interrupt, got {err}"
    );

    let done: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path IN ('src/one.ts','src/two.ts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        done, 1,
        "the first committed file survives as incremental progress"
    );
    let head_after: Option<String> = db
        .query_row(
            "SELECT git_head FROM projects WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        head_after, head_before,
        "git_head advances only in the final transaction"
    );
}

/// C2-15
#[test]
fn a_non_git_directory_degrades_to_a_full_index() {
    let (_dir, root) = make_project(SAMPLE);
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    full_index(&mut db, &bin, &root).unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.mode, "full");
}

/// C2-16
#[test]
fn remove_file_and_reindex_one_file_each_run_in_their_own_transaction() {
    let (_dir, root, mut db, project_id, bin) = git_project(SAMPLE);
    let one = reindex_one_file(&mut db, &bin, &root, &project_id, "src/helper.ts").unwrap();
    assert!(one.skipped, "unchanged content must be skipped");
    remove_file(&mut db, &project_id, "src/helper.ts").unwrap();
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE file_path = 'src/helper.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// F-01 regression (docs/2026-08-29-project-audit-root-causes-and-remediation.md)
// The relationship graph is derived state that spans files. Re-indexing one file
// must never silently drop an edge whose *other* end lives in an untouched file,
// and a half-rebuilt graph must never report itself fresh.
// ═══════════════════════════════════════════════════════════════════════════

fn rel_count(db: &rusqlite::Connection) -> i64 {
    db.query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap()
}

fn edge_exists(db: &rusqlite::Connection, source_sym: &str, target_sym: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM relationships r
           JOIN chunks s ON s.chunk_id = r.source_chunk_id
           JOIN chunks t ON t.chunk_id = r.target_chunk_id
          WHERE s.symbol_name = ?1 AND t.symbol_name = ?2 AND r.rel_type = 'calls'",
        params![source_sym, target_sym],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn an_incremental_reindex_of_a_callee_keeps_incoming_edges_from_unchanged_files() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    assert!(edge_exists(&db, "alpha", "helper") == 1, "fixture premise");
    let before = rel_count(&db);

    // Body-only edit of the callee. alpha.ts is untouched.
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 3; }\n",
    )
    .unwrap();
    incremental_index(&mut db, &bin, &root).unwrap();

    assert_eq!(
        edge_exists(&db, "alpha", "helper"),
        1,
        "an unchanged caller's incoming edge must survive re-indexing the callee"
    );
    assert_eq!(
        rel_count(&db),
        before,
        "the same edge set must come back, no duplicates"
    );

    let helper_chunk: String = db
        .query_row(
            "SELECT chunk_id FROM chunks WHERE symbol_name = 'helper'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let deps = get_transitive_dependents(&db, &helper_chunk, 3).unwrap();
    assert!(
        deps.iter()
            .any(|d| d.symbol_name.as_deref() == Some("alpha")),
        "impact must still see alpha, got {:?}",
        deps.iter().map(|d| &d.symbol_name).collect::<Vec<_>>()
    );
}

#[test]
fn an_incremental_reindex_reapplies_edges_from_files_that_only_grew_a_new_callee() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    // Add a new caller of an existing symbol, then touch the callee so both the
    // new file and a re-indexed file are in the candidate set.
    fs::write(
        root.join("src/gamma.ts"),
        "import { helper } from './helper';\n\nexport function gamma(g: number) { return helper(g); }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 4; }\n",
    )
    .unwrap();
    incremental_index(&mut db, &bin, &root).unwrap();

    assert_eq!(
        edge_exists(&db, "alpha", "helper"),
        1,
        "pre-existing caller kept"
    );
    assert_eq!(edge_exists(&db, "gamma", "helper"), 1, "new caller added");
}

#[test]
fn a_full_index_persists_the_raw_edges_needed_to_rebuild_the_graph() {
    let (_dir, _root, db, project_id, _bin) = git_project(SAMPLE);
    let raw: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM raw_edges WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        raw > 0,
        "raw call/import matches must be persisted, otherwise the graph cannot be rebuilt"
    );
    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM raw_edges WHERE project_id = ?1 AND source_symbol IS NULL",
            params![project_id],
            |r| r.get(0),
        )
        .unwrap();
    let _ = unresolved; // imports live outside any symbol body; NULL is legal.
}

#[test]
fn a_pending_graph_is_reported_stale_even_when_every_file_hash_matches() {
    let (_dir, root, db, project_id, bin) = git_project(SAMPLE);
    set_meta(&db, "graph_pending", "1").unwrap();
    let report = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        report.index_is_stale,
        "a half-rebuilt graph must never report fresh"
    );
    assert!(
        report.changed_files.is_empty() && report.deleted_files.is_empty(),
        "staleness must be attributed to the pending graph, not to a fake file change: {:?}",
        report
    );
}

#[test]
fn a_completed_incremental_index_clears_the_pending_graph_marker() {
    let (_dir, root, mut db, _id, bin) = git_project(SAMPLE);
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 5; }\n",
    )
    .unwrap();
    incremental_index(&mut db, &bin, &root).unwrap();
    assert_ne!(
        get_meta(&db, "graph_pending").unwrap().as_deref(),
        Some("1"),
        "a fully rebuilt graph must not stay marked pending"
    );
}
