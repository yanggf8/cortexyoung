//! C2-17..C2-22 — the Rust port kept the case ids (audit F-12).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::indexer::full_index;
use cort::staleness::compute_stale;
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

/// A commit that arrives without dirtying the tree -- `git pull`, `checkout`, `rebase`, `reset`,
/// or simply another agent committing -- moves HEAD while `git diff HEAD` stays empty. The
/// candidate set is built from that diff, so nothing gets hashed and the index looks fresh at a
/// head it was never built from. This is C2-19's edit, made invisible by committing it.
#[test]
fn a_commit_that_moves_head_without_dirtying_the_tree_is_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 99; }\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "pulled"]);
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["diff", "--quiet", "HEAD"])
            .status()
            .unwrap()
            .success(),
        "precondition: the tree must be clean against the new head"
    );

    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.index_is_stale,
        "the index was built at the previous head; a clean tree at a different head is not fresh"
    );
    assert!(
        s.changed_files.iter().any(|f| f == "src/helper.ts"),
        "a moved head must name the files it moved past, not just flip the flag: {:?}",
        s.changed_files
    );
}

/// The stored head can be unreachable: a force-push, a shallow clone, a rebased-away commit. The
/// diff against it fails, and a failed diff is not evidence that nothing changed -- fall back to
/// hashing every file rather than trusting an answer git refused to give.
#[test]
fn an_unreachable_stored_head_falls_back_to_hashing_every_file() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    db.execute(
        "UPDATE projects SET git_head = ?1 WHERE project_id = ?2",
        rusqlite::params!["0000000000000000000000000000000000000000", &project_id],
    )
    .unwrap();
    fs::write(
        root.join("src/helper.ts"),
        "export function helper(n: number) { return n * 99; }\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "pulled"]);

    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.index_is_stale,
        "a head git cannot resolve must widen the search, never narrow it"
    );
    assert!(s.changed_files.iter().any(|f| f == "src/helper.ts"));
}

/// `compute_stale` narrows through the same git call, so it goes blind the same way: a gitignored
/// file that a full index picked up, then edited, leaves `index_is_stale` reading `false` over a
/// row that is already wrong. The flag is what the skill tells agents to check, so this one is not
/// merely incomplete -- it is confidently incomplete.
#[test]
fn an_indexed_file_git_will_not_speak_for_is_stale_when_it_changes() {
    // Not `.gitignore`d -- `walk_files` honours ignore files as of 2026-09-03, so such a path is
    // never indexed at all and surfaces as a coverage `unindexed` row. And not merely untracked
    // either: `ls-files --others` names those, so they are vouched for and this predicate is never
    // reached. What lands in the index with git silent is a path the *global* ignore file excludes
    // -- `walk_files` sets `git_global(false)` on purpose, git's `--exclude-standard` does not.
    let (_dir, root, mut db, project_id, bin) = setup(&[(
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    )]);
    fs::create_dir_all(root.join("generated")).unwrap();
    fs::write(
        root.join("generated/gen.ts"),
        "export function generatedAlpha() { return 1; }\n",
    )
    .unwrap();
    let global_ignore = root.join("global-ignore");
    fs::write(&global_ignore, "generated/\n").unwrap();
    git(
        &root,
        &[
            "config",
            "core.excludesFile",
            global_ignore.to_str().unwrap(),
        ],
    );
    cort::indexer::full_index(&mut db, &bin, &root).unwrap();
    assert!(
        !compute_stale(&db, &bin, &root, &project_id)
            .unwrap()
            .index_is_stale,
        "precondition: freshly indexed, nothing changed yet"
    );

    fs::write(
        root.join("generated/gen.ts"),
        "export function generatedBeta() { return 2; }\n",
    )
    .unwrap();
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.index_is_stale,
        "the screen must not vouch for a row it has no way to keep true"
    );
    assert!(
        s.changed_files.iter().any(|f| f == "generated/gen.ts"),
        "and it must name the file: {:?}",
        s.changed_files
    );
}

/// The 2026-09-05 shape: the tree is clean, the git head has not moved, and every file hash
/// matches -- but the index was built by an extractor this binary no longer uses. Until now that
/// read as fresh, which is what let seven projects answer `impact` with `index_is_stale: false`
/// while their rows were computed by superseded semantics.
#[test]
fn an_index_built_by_another_extractor_is_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();

    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.index_is_stale,
        "a superseded extractor is staleness: {s:?}"
    );
    assert!(
        s.rebuild_required.iter().any(|r| r == "extractor_changed"),
        "the reason is named, not merely implied: {s:?}"
    );
    assert!(
        s.changed_files.is_empty() && s.deleted_files.is_empty(),
        "nothing in the tree moved -- this is the case the old check called fresh: {s:?}"
    );
}

/// The schema axis is independent: an index at a superseded schema is stale even when its extractor
/// is current. A `rebuild_required` computed from the extractor alone passes the test above and
/// fails this one.
#[test]
fn an_index_at_an_older_schema_is_stale_independently() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    cort::db::set_meta(&db, "SCHEMA_VERSION", "3").unwrap();

    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(s.index_is_stale, "{s:?}");
    assert!(
        s.rebuild_required.iter().any(|r| r == "schema_changed"),
        "{s:?}"
    );
    assert!(
        !s.rebuild_required.iter().any(|r| r == "extractor_changed"),
        "the extractor was untouched, so only one axis may fire: {s:?}"
    );
}

/// And the healthy answer. Plan 1 shipped a verdict word that no test required the implementation
/// to be able to produce, and an implementation that could never produce it passed everything; that
/// mistake is not repeated here.
#[test]
fn a_freshly_indexed_tree_owes_no_rebuild() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.rebuild_required.is_empty(),
        "an index this binary just built owes nothing: {s:?}"
    );
}
