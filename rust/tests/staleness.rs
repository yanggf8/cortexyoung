//! C2-17..C2-22 — tests/staleness.test.js (frozen JS reference)

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::indexer::full_index;
use cort::staleness::compute_stale;
use std::env;
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
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "t@e.com"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

/// C2-17
#[test]
fn a_freshly_indexed_clean_tree_is_not_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(!s.index_is_stale);
    assert!(s.deleted_files.is_empty());
}

/// C2-18
#[test]
fn a_dirty_but_semantically_identical_file_is_not_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    let p = root.join("src/alpha.ts");
    let body = fs::read_to_string(&p).unwrap();
    fs::write(&p, format!("{body}\n// trailing comment\n")).unwrap();
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        !s.index_is_stale,
        "git dirty alone must not mark the index stale — extraction output is unchanged"
    );
}

/// C2-19
#[test]
fn a_changed_chunk_body_makes_the_index_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 99; }\n",
    )
    .unwrap();
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(s.index_is_stale);
    assert!(s.changed_files.iter().any(|f| f == "src/helper.ts"));
}

/// C2-20
#[test]
fn an_edge_only_change_makes_the_index_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    fs::write(
        root.join("src/alpha.ts"),
        "import { helper } from './helper';\n\
export function alpha(a: number) { return helper(a) + 1; }\n\
export class Beta {\n\
  go() { return helper(2); }\n\
}\n",
    )
    .unwrap();
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.index_is_stale,
        "file_content_hash covers edges, not just chunk contents"
    );
}

/// C2-21
#[test]
fn a_deleted_file_makes_the_index_stale_and_is_reported() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    fs::remove_file(root.join("src/helper.ts")).unwrap();
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(s.index_is_stale);
    assert_eq!(s.deleted_files, vec!["src/helper.ts".to_string()]);
}

/// C2-22
#[test]
fn staleness_is_computed_against_projects_path_not_the_cwd() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    let (_else_dir, elsewhere) = make_project(&[("src/unrelated.ts", "export function u() {}\n")]);
    let prev = env::current_dir().unwrap();
    env::set_current_dir(&elsewhere).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
        assert!(!s.index_is_stale);
    }));
    env::set_current_dir(prev).unwrap();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
