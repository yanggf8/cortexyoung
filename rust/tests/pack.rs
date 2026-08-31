//! C1-1..C1-5 — the Rust port kept the case ids (audit F-12).

use cort::ast_grep::{exec_ast_grep, resolve_ast_grep_bin, ExecOpts};
use cort::pack::{extractor_version, pack_dir, pack_files, sgconfig};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static PACK_LOCK: Mutex<()> = Mutex::new(());

fn pack_guard() -> MutexGuard<'static, ()> {
    PACK_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// C1-1
#[test]
fn pack_files_are_enumerated_in_sorted_order_and_hash_deterministically() {
    let _g = pack_guard();
    let files = pack_files();
    assert!(files.len() >= 5);
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(files, sorted);
    assert!(files.iter().all(|f| f.is_absolute()));
    let v = extractor_version();
    assert!(
        v.len() == 64
            && v.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "expected 64 lowercase hex, got {v}"
    );
    assert_eq!(v, extractor_version());
}

/// C1-2
#[test]
fn extractor_version_changes_when_any_pack_file_changes() {
    let _g = pack_guard();
    let target = pack_files()
        .into_iter()
        .find(|f| f.ends_with("typescript.yml"))
        .expect("typescript.yml in pack");
    let before = fs::read(&target).expect("read typescript.yml");
    let v1 = extractor_version();
    struct Restore {
        path: PathBuf,
        original: Vec<u8>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = fs::write(&self.path, &self.original);
        }
    }
    let restore = Restore {
        path: target.clone(),
        original: before.clone(),
    };
    let mut probe = before.clone();
    probe.extend_from_slice(b"\n# probe\n");
    fs::write(&target, &probe).expect("write probe");
    assert_ne!(extractor_version(), v1);
    drop(restore);
    assert_eq!(extractor_version(), v1);
}

/// C1-3
#[test]
fn the_pack_extracts_chunks_and_edges_from_typescript_with_the_expected_tags() {
    let _g = pack_guard();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("k.ts");
    fs::write(
        &file,
        [
            "import { helper } from './helper';",
            "export function alpha(a: number) { return helper(a) + 1; }",
            "export class Beta {",
            "  go() { return alpha(2); }",
            "}",
        ]
        .join("\n"),
    )
    .unwrap();
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    let sg = sgconfig();
    let r = exec_ast_grep(
        &bin,
        &[
            "scan",
            "--json=stream",
            "--config",
            sg.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
        ExecOpts::default(),
    )
    .unwrap();
    assert_eq!(r.code, 0);
    let recs: Vec<serde_json::Value> = r
        .stdout
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut tags: Vec<String> = recs
        .iter()
        .map(|x| x["message"].as_str().unwrap().to_string())
        .collect();
    tags.sort();
    assert_eq!(
        tags,
        [
            "chunk:class",
            "chunk:function",
            "chunk:method",
            "edge:calls",
            "edge:calls",
            "edge:imports"
        ]
    );
    let fn_rec = recs
        .iter()
        .find(|x| x["message"] == "chunk:function")
        .unwrap();
    assert_eq!(fn_rec["metaVariables"]["single"]["NAME"]["text"], "alpha");
    let imp = recs
        .iter()
        .find(|x| x["message"] == "edge:imports")
        .unwrap();
    // unquote lives in chunker, not pack — $SRC still carries quotes.
    assert_eq!(imp["metaVariables"]["single"]["SRC"]["text"], "'./helper'");
}

/// C1-4
#[test]
fn the_pack_extracts_chunks_and_edges_from_python() {
    let _g = pack_guard();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("k.py");
    fs::write(
        &file,
        [
            "import os",
            "from helper import assist",
            "def alpha(a):",
            "    return assist(a) + 1",
            "class Beta:",
            "    def go(self):",
            "        return alpha(2)",
        ]
        .join("\n"),
    )
    .unwrap();
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    let sg = sgconfig();
    let r = exec_ast_grep(
        &bin,
        &[
            "scan",
            "--json=stream",
            "--config",
            sg.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
        ExecOpts::default(),
    )
    .unwrap();
    let recs: Vec<serde_json::Value> = r
        .stdout
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut imports: Vec<String> = recs
        .iter()
        .filter(|x| x["message"] == "edge:imports")
        .map(|x| {
            x["metaVariables"]["single"]["SRC"]["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    imports.sort();
    assert_eq!(imports, ["helper", "os"]);
    assert_eq!(
        recs.iter()
            .filter(|x| x["message"] == "chunk:class")
            .count(),
        1
    );
    assert_eq!(
        recs.iter()
            .filter(|x| x["message"] == "chunk:function")
            .count(),
        2
    );
}

/// Three shapes, three forms, and one rule that has not changed: a *qualified* call may never be
/// emitted with its qualification stripped, because a project-wide bare-name match would fabricate
/// an INFERRED caller for any same-named free function.
///
/// v4 added the receiver shape (`w.run()`) with the method name as its target -- which is exactly
/// the stripped shape this test used to say must never appear. It is allowed only because the
/// resolution side refuses it unless the name is unique project-wide
/// (`graph::resolve_edge_targets`), and because the form is stored so the refusal is auditable. A
/// `bare` or `scoped` row carrying `run` for `Worker::run(w)` is still fabrication.
#[test]
fn the_pack_extracts_rust_call_edges_with_exact_targets_only() {
    let _g = pack_guard();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("k.rs");
    fs::write(
        &file,
        [
            "pub fn run(x: u8) -> u8 { x }",
            "pub struct Worker;",
            "impl Worker {",
            "    pub fn run(&self) -> u8 { 1 }",
            "}",
            "pub fn caller(w: &Worker) -> u8 {",
            "    run(1) + Worker::run(w) + w.run()",
            "}",
        ]
        .join("\n"),
    )
    .unwrap();
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    let sg = sgconfig();
    let r = exec_ast_grep(
        &bin,
        &[
            "scan",
            "--json=stream",
            "--config",
            sg.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
        ExecOpts::default(),
    )
    .unwrap();
    assert_eq!(r.code, 0);
    let mut tagged: Vec<String> = r
        .stdout
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|x| {
            x["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("edge:calls"))
        })
        .map(|x| {
            format!(
                "{} {}",
                x["message"].as_str().unwrap(),
                x["metaVariables"]["single"]["CALLEE"]["text"]
                    .as_str()
                    .unwrap()
            )
        })
        .collect();
    tagged.sort();
    assert_eq!(
        tagged,
        [
            "edge:calls:bare run",
            // The receiver edge keeps its receiver: `w.run()`, not `run`. Resolution needs it
            // (`graph::receiver_binds` asks whether `w` can be a `Worker`), and a reader who is
            // checking the edge needs it more.
            "edge:calls:receiver w.run",
            "edge:calls:scoped Worker::run",
        ],
        "one edge per call shape, each carrying the form that decides how it resolves"
    );
    let stripped: Vec<&String> = tagged
        .iter()
        .filter(|row| {
            row.starts_with("edge:calls:bare Worker") || row.starts_with("edge:calls:scoped run")
        })
        .collect();
    assert!(
        stripped.is_empty(),
        "a qualified call must never surface as a loose name: {tagged:?}"
    );
}

/// C1-5
#[test]
fn pack_dir_points_at_a_real_directory_containing_sgconfig_yml() {
    let _g = pack_guard();
    let dir = pack_dir();
    assert!(dir.is_dir());
    assert!(sgconfig().is_file());
}

// Cutover: the installed binary cannot rely on CARGO_MANIFEST_DIR (a compile-time
// path valid only on the build machine). It must honour CORT_PACK_DIR and, failing
// that, refuse to run rather than silently hash nothing.
#[test]
fn cort_pack_dir_override_points_somewhere_else() {
    let _g = pack_guard();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pack");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sgconfig.yml"), "languageGlobs: {}\n").unwrap();
    std::fs::write(dir.join("rules.yml"), "id: x\nlanguage: Tsx\n").unwrap();
    set_pack_env(Some(dir.to_str().unwrap()));
    assert_eq!(pack_dir(), dir);
    assert!(sgconfig().ends_with("pack/sgconfig.yml"));
    let files = pack_files();
    assert_eq!(files.len(), 2);
    // the override pack hashes differently from the repo pack
    let overridden = extractor_version();
    set_pack_env(None);
    assert_ne!(overridden, extractor_version());
}

/// Env mutation that survives a panicking assertion inside the scope: the var is
/// removed on the unwind path too, or it poisons every later test in this binary.
fn set_pack_env(value: Option<&str>) {
    match value {
        Some(v) => std::env::set_var("CORT_PACK_DIR", v),
        None => std::env::remove_var("CORT_PACK_DIR"),
    }
}

#[test]
fn an_override_pack_dir_without_sgconfig_fails_closed() {
    let _g = pack_guard();
    let tmp = tempfile::tempdir().unwrap();
    set_pack_env(Some(tmp.path().to_str().unwrap()));
    let result = std::panic::catch_unwind(cort::pack::sgconfig);
    set_pack_env(None);
    // sgconfig must fail closed: the override dir has no sgconfig.yml
    assert!(
        result.is_err(),
        "sgconfig() must refuse an override pack without sgconfig.yml"
    );
}

#[test]
fn without_the_override_the_default_is_the_repo_pack() {
    let _g = pack_guard();
    set_pack_env(None);
    assert!(pack_dir().join("sgconfig.yml").exists());
}
