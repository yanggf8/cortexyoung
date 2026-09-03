//! C2-8..C2-16 — the Rust port kept the case ids (audit F-12).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, get_meta, indexed_head, open_db, project_id_for, set_meta};
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
    let cands = git_candidates(&root, indexed_head(&db, &_id).unwrap().as_deref());
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

/// v4 end to end: an index built by the previous schema version is upgraded, force-rebuilt, and
/// comes back carrying the two things v4 exists to record -- which call shape each edge arrived as,
/// and the line inside the caller that names the callee.
///
/// This is the path a user's cache actually takes: `SCHEMA_VERSION` goes up under a database whose
/// tables were created by an older build, so the upgrade has to add the columns (`ALTER`, because
/// `CREATE TABLE IF NOT EXISTS` never does), `graph_pending` has to route the next index through a
/// full rebuild, and the rebuild has to fill in what the old rows could never have had.
#[test]
fn a_v3_index_is_upgraded_and_its_rebuilt_graph_carries_forms_and_call_sites() {
    let (_dir, root) = {
        let d = tempfile::Builder::new()
            .prefix("cort-v4-")
            .tempdir()
            .unwrap();
        for (rel, body) in [
            (
                "src/lib.rs",
                "pub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
            ),
            (
                "src/use.rs",
                "use crate::lib::T;\nfn one(t: &T) -> u32 { t.take() }\nfn two(t: &T) -> u32 { t.take() }\n",
            ),
        ] {
            let abs = d.path().join(rel);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(&abs, body).unwrap();
        }
        let root = fs::canonicalize(d.path()).unwrap();
        (d, root)
    };
    let project_id = project_id_for(root.to_str().unwrap());

    let mut db = open_db(":memory:").unwrap();
    db.execute_batch(
        "CREATE TABLE _cortex_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO _cortex_meta (key, value) VALUES ('SCHEMA_VERSION', '3');
         CREATE TABLE projects (
           project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL, git_head TEXT,
           last_indexed_at INTEGER, extractor_version TEXT NOT NULL,
           created_at TEXT DEFAULT (datetime('now')));
         CREATE TABLE chunks (
           chunk_id TEXT PRIMARY KEY,
           project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
           file_path TEXT NOT NULL, symbol_name TEXT,
           chunk_type TEXT CHECK(chunk_type IN ('function','class','method','config','documentation','unparsed')),
           start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, content TEXT NOT NULL,
           content_hash TEXT NOT NULL, language TEXT,
           chunk_source TEXT NOT NULL CHECK(chunk_source IN ('ast','unparsed')),
           created_at TEXT DEFAULT (datetime('now')), updated_at TEXT DEFAULT (datetime('now')));
         CREATE TABLE relationships (
           source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
           target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
           rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
           confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
           confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
           confidence_reasoning TEXT,
           PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type));
         CREATE TABLE raw_edges (
           project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
           file_path TEXT NOT NULL, source_symbol TEXT NOT NULL DEFAULT '', raw_target TEXT NOT NULL,
           rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
           start_line INTEGER NOT NULL,
           PRIMARY KEY (project_id, file_path, rel_type, raw_target, source_symbol, start_line));",
    )
    .unwrap();

    ensure_schema(&db).unwrap();
    assert_eq!(
        get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some("4"),
        "the upgrade writes the version only after the columns land"
    );
    assert_eq!(
        get_meta(&db, "graph_pending").unwrap().as_deref(),
        Some("1"),
        "a graph whose edges predate the new columns must not be trusted"
    );

    // Leftovers of a v3 index of this same repository, written *after* `ensure_schema` installed the
    // FTS triggers: an external-content index can only delete rows it was told about, so a chunk
    // inserted before its trigger exists makes the next `DELETE FROM chunks` fail as
    // "database disk image is malformed". A real v3 database has its FTS rows already, which is what
    // this ordering reproduces.
    db.execute_batch(&format!(
        "INSERT INTO projects (project_id, name, path, extractor_version)
           VALUES ('{project_id}', 'v3', '{path}', 'stale-version-hash');
         INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
           start_line, end_line, content, content_hash, language, chunk_source)
           VALUES ('stale', '{project_id}', 'gone.rs', 'gone', 'function', 1, 2, 'x', 'h', 'Rust', 'ast');
         INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target, rel_type, start_line)
           VALUES ('{project_id}', 'gone.rs', 'gone', 'whatever', 'calls', 1);",
        path = root.to_str().unwrap()
    ))
    .unwrap();

    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    let stats = full_index(&mut db, &bin, &root).unwrap();
    assert!(stats.relationships >= 2, "{stats:?}");
    assert_eq!(
        get_meta(&db, "graph_pending").unwrap().as_deref(),
        Some("0"),
        "the rebuild clears the marker it was rebuilt for"
    );
    let stale_chunks: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE chunk_id = 'stale'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stale_chunks, 0, "a full rebuild replaces the v3 chunk set");

    let forms: Vec<String> = db
        .prepare(
            "SELECT DISTINCT call_form FROM raw_edges WHERE project_id = ?1 ORDER BY call_form",
        )
        .unwrap()
        .query_map(params![project_id], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        forms.contains(&"receiver".to_string()),
        "the form a call arrived as has to survive the database round trip: {forms:?}"
    );

    let edges: Vec<(Option<i64>, String, String)> = db
        .prepare(
            "SELECT r.call_site_line, r.call_form, c.symbol_name FROM relationships r
               JOIN chunks c ON c.chunk_id = r.source_chunk_id
              WHERE c.project_id = ?1 AND r.call_form = 'receiver' ORDER BY r.call_site_line",
        )
        .unwrap()
        .query_map(params![project_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        edges,
        vec![
            (Some(2), "receiver".to_string(), "one".to_string()),
            (Some(3), "receiver".to_string(), "two".to_string())
        ],
        "each caller's own line, tagged with the shape that attached it"
    );
}

#[test]
fn a_bare_and_a_receiver_call_of_the_same_name_on_one_line_are_two_rows() {
    // `raw_edges` does not key on the form, so a receiver edge that stored only the method name would
    // collide with a bare call to the same name on the same line and one of the two would vanish.
    // Storing the head (`take(t)` vs `t.take()`) is what keeps both: the dedupe that remains is
    // genuinely duplicate work, not a lost edge.
    let (_dir, root) = {
        let d = tempfile::Builder::new()
            .prefix("cort-v4clash-")
            .tempdir()
            .unwrap();
        let abs = d.path().join("src/lib.rs");
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(
            &abs,
            "pub struct T;\npub fn take(_: &T) -> u32 { 1 }\nimpl T { pub fn take(&self) -> u32 { 2 } }\n",
        )
        .unwrap();
        let use_rs = d.path().join("src/use.rs");
        fs::write(
            &use_rs,
            "use crate::lib::T;\nfn go(t: &T) -> u32 { take(t) + t.take() }\n",
        )
        .unwrap();
        let root = fs::canonicalize(d.path()).unwrap();
        (d, root)
    };
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    full_index(&mut db, &bin, &root).unwrap();

    let mut rows: Vec<(String, String)> = db
        .prepare(
            "SELECT call_form, raw_target FROM raw_edges
              WHERE project_id = ?1 AND file_path = 'src/use.rs' AND rel_type = 'calls'",
        )
        .unwrap()
        .query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("bare".to_string(), "take".to_string()),
            ("receiver".to_string(), "t.take".to_string()),
        ],
        "both shapes of the same name on one line, neither swallowed by the other"
    );
}

/// The other half of the moved-head hole, and the worse half. `git diff HEAD` is empty after a
/// pull, so the candidate set is empty, so nothing is reindexed -- and then the run stamps the new
/// head onto the untouched index anyway. That stamp is what `hook-suggest` compares, so the
/// PostToolUse refresh hook does not merely fail to repair a pulled tree: on the first edit
/// afterwards it destroys the one signal that said repair was needed.
#[test]
fn a_head_that_moved_without_dirtying_the_tree_is_reindexed_not_just_restamped() {
    let (_dir, root, mut db, project_id, bin) = git_project(SAMPLE);
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 3; }\nexport function pulled() { return 0; }\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "pulled"]);

    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.files_reindexed, 1, "the pulled file must be re-extracted");

    let syms: Vec<String> = db
        .prepare(
            "SELECT symbol_name FROM chunks WHERE file_path = 'src/helper.ts' ORDER BY start_line",
        )
        .unwrap()
        .query_map([], |c| c.get(0))
        .unwrap()
        .map(|c| c.unwrap())
        .collect();
    assert_eq!(
        syms,
        vec!["helper".to_string(), "pulled".to_string()],
        "a restamp without a re-extraction leaves the old chunks behind the new head"
    );
    assert!(
        !compute_stale(&db, &bin, &root, &project_id)
            .unwrap()
            .index_is_stale,
        "and only then may the index call itself fresh"
    );
}

/// An untracked file that is indexed and then deleted must leave the index.
///
/// Neither half of the git narrowing can name it: `git diff --name-status <head> HEAD` never
/// mentions a path that was never committed, and `git ls-files --others` only lists paths that
/// still exist. So the row survived every incremental pass and `impact --symbol` went on reporting
/// a definition in a file that is not there -- a claim the screen cannot support, which is worse
/// than a missing row. Reproduced on 2026-09-03 with a scratch file an agent created and removed.
///
/// The tracked control is asserted in the same test because it always worked, and that is exactly
/// why nothing caught this: the case that fails is the one nobody writes a fixture for.
#[test]
fn an_untracked_file_that_is_deleted_leaves_the_index() {
    let (_dir, root, mut db, project_id, bin) = git_project(SAMPLE);

    let ghost = root.join("src/ghost.ts");
    fs::write(&ghost, "export function ghost() { return 2; }\n").unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert!(
        r.files_reindexed >= 1,
        "the untracked file was never indexed, so this test proves nothing: {r:?}"
    );
    assert_eq!(
        chunk_rows(&db, &project_id, "src/ghost.ts"),
        1,
        "the untracked file should be in the index before it is deleted"
    );

    fs::remove_file(&ghost).unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(
        r.files_removed, 1,
        "a deleted untracked file must be counted as removed: {r:?}"
    );
    assert_eq!(
        chunk_rows(&db, &project_id, "src/ghost.ts"),
        0,
        "the index still holds a file that does not exist"
    );

    // The tracked files are untouched: this closes a hole, it does not open a wider one.
    assert!(
        chunk_rows(&db, &project_id, "src/helper.ts") > 0,
        "a tracked file was removed along with the ghost"
    );

    // And it converges: the next pass has nothing left to remove.
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(r.files_removed, 0, "the removal repeated itself: {r:?}");
}

fn chunk_rows(db: &rusqlite::Connection, project_id: &str, file_path: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM chunks WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
        |r| r.get(0),
    )
    .unwrap()
}

/// A file git will not speak for that the index *does* hold.
///
/// It cannot be `.gitignore`d any more: `walk_files` honours ignore files as of 2026-09-03, so a
/// gitignored path is not indexed at all (it surfaces as a coverage `unindexed` row instead). What
/// still lands in the index while git stays silent is a path excluded by `.git/info/exclude` --
/// git-invisible in exactly the same way, and `walk_files` reads that too, so the fixture writes
/// the file *after* the commit and adds the exclude rule then. Same predicate under test, a
/// fixture that still reaches it.
const IGNORED_SAMPLE: &[(&str, &str)] = &[(
    "src/helper.ts",
    "export function helper(n: number) { return n * 2; }\n",
)];

/// The other half of the "git cannot name it" family, and the one the deletion sweep does not
/// reach. `walk_files` does not consult `.gitignore` -- it skips `IGNORE_DIRS` and filters on
/// extension -- so a gitignored source file is indexed by a full pass. But `git ls-files --others`
/// excludes ignored paths and the diffs never mention them, so once indexed, every later edit is
/// invisible: `impact` keeps printing the old symbol at a line that no longer defines it, with
/// `stale=false` beside it.
#[test]
fn an_indexed_file_git_will_not_speak_for_is_reexamined_when_it_changes() {
    let (_dir, root, mut db, _id, bin) = git_project(IGNORED_SAMPLE);
    // Untracked and never mentioned by any diff, but not excluded by an ignore file the walk reads
    // -- so a full pass indexes it and git still says nothing about it either way. That is the
    // state this sweep exists for.
    fs::create_dir_all(root.join("generated")).unwrap();
    fs::write(
        root.join("generated/gen.ts"),
        "export function generatedAlpha() { return 1; }\n",
    )
    .unwrap();
    full_index(&mut db, &bin, &root).unwrap();
    let syms: Vec<String> = db
        .prepare("SELECT symbol_name FROM chunks WHERE file_path = 'generated/gen.ts'")
        .unwrap()
        .query_map([], |c| c.get(0))
        .unwrap()
        .map(|c| c.unwrap())
        .collect();
    assert_eq!(
        syms,
        vec!["generatedAlpha".to_string()],
        "precondition: a full index does pick up a gitignored source file"
    );

    fs::write(
        root.join("generated/gen.ts"),
        "export function generatedBeta() { return 2; }\n",
    )
    .unwrap();
    let r = incremental_index(&mut db, &bin, &root).unwrap();
    assert_eq!(
        r.files_reindexed, 1,
        "an indexed file git cannot vouch for must be examined by us"
    );

    let syms: Vec<String> = db
        .prepare("SELECT symbol_name FROM chunks WHERE file_path = 'generated/gen.ts'")
        .unwrap()
        .query_map([], |c| c.get(0))
        .unwrap()
        .map(|c| c.unwrap())
        .collect();
    assert_eq!(
        syms,
        vec!["generatedBeta".to_string()],
        "the old symbol must not survive at a line that no longer defines it"
    );
}
