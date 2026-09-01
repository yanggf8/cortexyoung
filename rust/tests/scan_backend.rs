//! Backend acceptance: the crate scan and the CLI scan must produce the same index.
//!
//! `src/scan.rs` replaced the `ast-grep scan` subprocess on the default path after the parity probe
//! proved record-level identity on real files. This test holds that proof at the level the product
//! actually consumes: a full index of one fixture tree built through each backend, compared row by
//! row over chunks and relationships. If a grammar bump ever breaks record parity in a way the
//! probe's sampled files miss, a difference here is what fires.
//!
//! The CLI leg needs the real binary; where it is absent the crate leg still runs and the test
//! says SKIP rather than failing, per repo convention.

use cort::db::{ensure_schema, open_db, project_id_for};
use cort::graph::rebuild_relationships;
use cort::indexer::full_index;
use std::fs;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// One small tree that exercises every language leg and every call form the pack extracts: bare,
/// receiver, scoped, module-path, brace imports, plus a Python and a plain-JS file.
const TREE: &[(&str, &str)] = &[
    (
        "src/util.rs",
        "pub fn helper(x: u8) -> u8 { x }\npub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
    ),
    (
        "src/app.rs",
        "mod util;\nuse crate::util::{helper, T};\n\npub fn run() -> u8 {\n    let t = T;\n    helper(t.take() as u8)\n}\n",
    ),
    (
        "src/helper.ts",
        "export function helper(n: number) { return n * 2; }\n",
    ),
    (
        "src/alpha.ts",
        "import { helper } from './helper';\nexport function alpha(a: number) { return helper(a) + 1; }\n",
    ),
    (
        "pkg/mod.py",
        "def helper(n):\n    return n * 2\n\ndef caller(n):\n    return helper(n) + 1\n",
    ),
];

fn set_backend(v: Option<&str>) {
    // SAFETY: tests take ENV_LOCK; no other thread mutates the environment while held.
    unsafe {
        match v {
            Some(s) => std::env::set_var("CORT_SCAN_BACKEND", s),
            None => std::env::remove_var("CORT_SCAN_BACKEND"),
        }
    }
}

/// Index the tree under the current backend and dump the row-level facts the product serves.
fn index_tree() -> (tempfile::TempDir, Vec<String>, Vec<String>) {
    let dir = tempfile::Builder::new()
        .prefix("cort-backend-")
        .tempdir()
        .unwrap();
    for (rel, body) in TREE {
        let abs = dir.path().join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, body).unwrap();
    }
    let root = fs::canonicalize(dir.path()).unwrap();
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = match cort::ast_grep::resolve_ast_grep_bin() {
        Ok(b) => b,
        Err(e) => panic!("the CLI leg needs the real binary: {e:?}"),
    };
    full_index(&mut db, &bin, &root).unwrap();
    rebuild_relationships(&db, &project_id).unwrap();

    let mut chunks: Vec<String> = db
        .prepare("SELECT file_path, symbol_name, chunk_type, start_line, end_line, language
                  FROM chunks WHERE project_id = ?1 AND chunk_source != 'unparsed' ORDER BY file_path, start_line")
        .unwrap()
        .query_map([&project_id], |r| {
            Ok(format!(
                "{}|{:?}|{}|{}|{}|{:?}",
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?
            ))
        })
        .unwrap()
        .flatten()
        .collect();
    chunks.sort();
    let mut rels: Vec<String> = db
        .prepare(
            "SELECT sc.symbol_name, t.symbol_name, r.rel_type, r.call_form, r.call_site_line
                  FROM relationships r
                  JOIN chunks sc ON sc.chunk_id = r.source_chunk_id
                  JOIN chunks t ON t.chunk_id = r.target_chunk_id
                  WHERE sc.project_id = ?1 ORDER BY sc.symbol_name, t.symbol_name, r.rel_type",
        )
        .unwrap()
        .query_map([&project_id], |r| {
            Ok(format!(
                "{}|{:?}|{}|{}|{:?}",
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?
            ))
        })
        .unwrap()
        .flatten()
        .collect();
    rels.sort();
    (dir, chunks, rels)
}

#[test]
fn the_crate_backend_indexes_the_same_tree_as_the_cli_backend() {
    let _g = ENV_LOCK.lock().unwrap();

    let cli_available = cort::ast_grep::resolve_ast_grep_bin().is_ok();
    if !cli_available {
        eprintln!("SKIP: ast-grep CLI not present; backend equality not exercised");
        return;
    }

    set_backend(Some("cli"));
    let (_d1, cli_chunks, cli_rels) = index_tree();
    set_backend(Some("crate"));
    let (_d2, crate_chunks, crate_rels) = index_tree();
    set_backend(None);

    assert_eq!(
        cli_chunks, crate_chunks,
        "chunks must be row-identical across backends"
    );
    assert_eq!(
        cli_rels, crate_rels,
        "relationships must be row-identical across backends"
    );
    // Guard against the test passing because both legs indexed nothing.
    assert!(
        cli_chunks.len() >= 6,
        "the fixture must produce chunks across languages: {cli_chunks:?}"
    );
    assert!(
        cli_rels.len() >= 3,
        "the fixture must produce relationships across forms: {cli_rels:?}"
    );
}
