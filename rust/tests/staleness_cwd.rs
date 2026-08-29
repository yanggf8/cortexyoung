//! C2-22, isolated on purpose.
//!
//! This test proves staleness follows `projects.path` rather than the working directory, so it
//! has to change the process working directory. `std::env::set_current_dir` is *process*-global:
//! every libtest thread in the same binary shares it, and `Command::spawn` inherits it — so an
//! ast-grep subprocess started by a sibling test while this one had the cwd moved elsewhere could
//! resolve the scan differently. That made `a_dirty_but_semantically_identical_file_is_not_stale`
//! fail in ~4 of 10 `cargo test --test staleness` runs (0 of 10 with `--test-threads=1`), and the
//! same failure reproduced at HEAD without any of the F-01 changes.
//!
//! Cargo runs distinct test binaries one after another, so living in its own target makes the cwd
//! mutation safe. Do not merge this back into `staleness.rs`.

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::indexer::full_index;
use cort::staleness::compute_stale;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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
    ("README.md", "# not a source file\n"),
];

fn make_project(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-cwd-proj-")
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

fn git(root: &Path, args: &[&str]) {
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

#[test]
fn staleness_is_computed_against_projects_path_not_the_cwd() {
    let (_dir, root) = make_project(SAMPLE);
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

    let (_else_dir, elsewhere) = make_project(&[("src/unrelated.ts", "export function u() {}\n")]);
    let prev = env::current_dir().unwrap();
    env::set_current_dir(&elsewhere).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let db_ref: &cort::db::Db = &db;
        let s = compute_stale(db_ref, &bin, &root, &project_id).unwrap();
        assert!(
            !s.index_is_stale,
            "cwd is not the project: {:?}",
            (s.changed_files, s.deleted_files)
        );
    }));
    env::set_current_dir(prev).unwrap();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
