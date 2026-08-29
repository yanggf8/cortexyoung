//! D-35..D-41 — the Rust port kept the case ids (audit F-12).
//! Read/recall lean shapes follow proposal §1 and §3 (supersede JS D-41).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::budget::estimate_tokens;
use cort::context::{context_command, ContextOptions, DEFAULT_BUDGET};
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::errors::CortError;
use cort::impact::impact_command;
use cort::indexer::full_index;
use cort::r#struct::{struct_command, StructOptions};
use cort::render::{parse_format, render, render_error, Format};
use serde_json::{json, Value};
use std::fs;
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

const CHAIN: &[(&str, &str)] = &[
    ("src/d.ts", "export function d() { return 1; }\n"),
    (
        "src/c.ts",
        "import { d } from './d';\nexport function c() { return d(); }\n",
    ),
    (
        "src/b.ts",
        "import { c } from './c';\nexport function b() { return c(); }\n",
    ),
    (
        "src/a.ts",
        "import { b } from './b';\nexport function a() { return b(); }\n",
    ),
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

fn indexed(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let (dir, root) = make_project(files);
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

fn chain() -> (tempfile::TempDir, rusqlite::Connection, Value, Value, Value) {
    let (dir, root, db, project_id, bin) = indexed(CHAIN);
    let impact = impact_command(&db, &bin, &root, &project_id, "d", 3).unwrap();
    let context = context_command(
        &db,
        &bin,
        &root,
        &project_id,
        "c",
        ContextOptions {
            budget: DEFAULT_BUDGET,
            include_ambiguous: false,
            full_content: false,
        },
    )
    .unwrap();
    let glob = root.join("src/c.ts").to_string_lossy().into_owned();
    let strukt = struct_command(
        &db,
        &bin,
        &root,
        &project_id,
        "d()",
        "ts",
        StructOptions {
            globs: vec![glob.clone()],
            budget: 1500,
            file_limit: None,
        },
    )
    .unwrap();
    (dir, db, impact, context, strukt)
}

/// D-35
#[test]
fn parse_format_accepts_json_and_lean_case_insensitively_and_rejects_anything_else() {
    assert_eq!(parse_format(None), Some(Format::Json));
    assert_eq!(parse_format(Some("json")), Some(Format::Json));
    assert_eq!(parse_format(Some("lean")), Some(Format::Lean));
    assert_eq!(parse_format(Some("LEAN")), Some(Format::Lean));
    assert_eq!(parse_format(Some("yaml")), None);
}

/// D-36
#[test]
fn lean_impact_output_lists_every_dependent_with_its_hop_and_drops_the_stored_chunk_id() {
    let (_dir, _db, impact, _, _) = chain();
    let out = render(Some("impact"), Format::Lean, &impact);
    assert!(
        out.lines()
            .any(|l| l == "# impact d depth=3 seeds=1 dependents=3 stale=false"),
        "{out}"
    );
    assert!(out.lines().any(|l| l == "h1\tsrc/c.ts\tc\t2"), "{out}");
    assert!(out.lines().any(|l| l == "h3\tsrc/a.ts\ta\t2"), "{out}");
    let chunk_id = impact["dependents"][0]["chunk_id"].as_str().unwrap();
    assert!(!out.contains(chunk_id), "lean must not repeat the chunk_id");
    assert!(
        !out.lines()
            .any(|l| { l.chars().all(|c| c.is_ascii_hexdigit()) && l.len() == 64 }),
        "no 64-char project hash in lean output:\n{out}"
    );
}

/// D-37
#[test]
fn lean_is_smaller_than_json_for_the_same_payload_on_all_three_verbs() {
    let (_dir, _db, impact, context, strukt) = chain();
    for (command, payload) in [
        ("impact", &impact),
        ("context", &context),
        ("struct", &strukt),
    ] {
        let json = render(Some(command), Format::Json, payload);
        let lean = render(Some(command), Format::Lean, payload);
        assert!(
            lean.len() < json.len(),
            "{command}: lean {} should be < json {}",
            lean.len(),
            json.len()
        );
        assert!(lean.ends_with('\n'));
        let _ = estimate_tokens(&json);
    }
}

/// D-38
#[test]
fn lean_context_keeps_neighbours_and_unresolved_refs_one_per_line() {
    let (_dir, _db, _, context, _) = chain();
    let out = render(Some("context"), Format::Lean, &context);
    assert!(
        out.starts_with("# context c resolution=exact_symbol"),
        "{out}"
    );
    assert!(out.lines().any(|l| l == "src/c.ts:2\tc\tfunction"), "{out}");
    for n in context["seeds"][0]["neighbors"].as_array().unwrap() {
        let path = n["file_path"].as_str().unwrap();
        assert!(out.contains(path), "neighbour {} missing", n["symbol_name"]);
    }
}

/// D-39
#[test]
fn lean_struct_emits_one_row_per_match_with_the_enclosing_symbol() {
    let (_dir, _db, _, _, strukt) = chain();
    let out = render(Some("struct"), Format::Lean, &strukt);
    assert!(
        out.lines()
            .any(|l| { l == "# struct d() lang=ts matches=1 shown=1 truncated=false stale=false" }),
        "{out}"
    );
    let rows: Vec<&str> = out.split('\n').skip(1).filter(|s| !s.is_empty()).collect();
    assert_eq!(rows.len(), strukt["matches"].as_array().unwrap().len());
    assert!(
        rows[0].starts_with("src/c.ts:") && rows[0].contains("\tc\t"),
        "{}",
        rows[0]
    );
}

/// D-40
#[test]
fn unknown_commands_and_json_format_fall_through_to_the_json_contract() {
    let payload = json!({ "ok": true });
    assert_eq!(
        render(Some("status"), Format::Lean, &payload),
        format!("{}\n", serde_json::to_string_pretty(&payload).unwrap())
    );
    assert_eq!(
        render(Some("impact"), Format::Json, &payload),
        format!("{}\n", serde_json::to_string_pretty(&payload).unwrap())
    );
}

/// D-41 — proposal §1 receipt/full lean headers (supersedes JS header without content=/hash=).
#[test]
fn lean_reading_output_identifies_cache_provenance_and_keeps_stored_content() {
    let receipt = json!({
        "file_path": "src/main.rs",
        "start_line": 10,
        "end_line": 12,
        "source": "store",
        "read_count": 2,
        "content_mode": "receipt",
        "content_hash_prefix": "82d25b9f72a6",
    });
    let out = render(Some("read"), Format::Lean, &receipt);
    assert_eq!(
        out,
        "# read src/main.rs:10-12 source=store reads=2 content=receipt hash=82d25b9f72a6\n"
    );
    assert!(!out.contains("fn work()"));

    let full = json!({
        "file_path": "src/main.rs",
        "start_line": 10,
        "end_line": 12,
        "source": "store",
        "read_count": 2,
        "content_mode": "full",
        "content_hash_prefix": "82d25b9f72a6",
        "content": "fn work() {\n}",
    });
    let full_out = render(Some("read"), Format::Lean, &full);
    assert!(full_out.starts_with(
        "# read src/main.rs:10-12 source=store reads=2 content=full hash=82d25b9f72a6\n"
    ));
    assert!(full_out.contains("fn work() {"));
    assert!(full_out.ends_with('\n'));

    let recall = render(
        Some("recall"),
        Format::Lean,
        &json!({
            "query": "work",
            "reading_count": 1,
            "truncated_query": false,
            "readings": [{
                "file_path": "src/main.rs",
                "start_line": 10,
                "end_line": 12,
                "content": "fn work() {\n}",
                "content_truncated": false,
                "read_count": 2,
                "last_read_at": 1,
            }],
        }),
    );
    assert!(recall
        .lines()
        .any(|l| l == "# recall work readings=1 truncated_query=false"));
    assert!(recall.lines().any(|l| l == "src/main.rs:10-12\treads=2"));
}

/// Proposal §3 — validation_error lean is one line; null errno/os_code become "-".
#[test]
fn validation_error_lean_is_a_single_line_and_nulls_become_dash() {
    let err = CortError::new(
        "validation_error",
        json!({
            "command": "recall",
            "file_path": "src/main.rs",
            "operation": "read",
            "errno": "EIO",
            "os_code": 5,
            "retryable": true,
            "note_action": "retained",
        }),
    );
    let lean = render_error(Format::Lean, &err);
    assert_eq!(
        lean,
        "! validation_error command=recall file=src/main.rs operation=read errno=EIO os_code=5 retryable=true note=retained\n"
    );
    assert!(!lean.contains('{'));

    let nulls = CortError::new(
        "validation_error",
        json!({
            "command": "recall",
            "file_path": "src/main.rs",
            "operation": "read",
            "errno": null,
            "os_code": null,
            "retryable": false,
            "note_action": "retained",
        }),
    );
    assert_eq!(
        render_error(Format::Lean, &nulls),
        "! validation_error command=recall file=src/main.rs operation=read errno=- os_code=- retryable=false note=retained\n"
    );

    let json_out = render_error(Format::Json, &err);
    let parsed: Value = serde_json::from_str(json_out.trim_end()).unwrap();
    assert_eq!(parsed["error"], "validation_error");
    assert_eq!(parsed["detail"]["errno"], "EIO");
}

/// Error envelope to_json field names (spec §4.1).
#[test]
fn error_envelope_to_json_uses_error_not_code() {
    let err = CortError::new("unknown_format", json!({ "hint": "--format json|lean" }));
    let out = render_error(Format::Json, &err);
    let parsed: Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(parsed, err.to_json());
    assert!(parsed.get("code").is_none());
    assert_eq!(parsed["error"], "unknown_format");
}

/// SAMPLE is referenced so the frozen fixture stays available for field-order checks.
/// serde_json's default map is alphabetical: cort's JSON key order contract IS
/// alphabetical, and every golden in this suite is written that way.
#[test]
fn json_pretty_print_uses_two_space_indent_and_trailing_newline() {
    let (_dir, _root, _db, _id, _bin) = indexed(SAMPLE);
    let payload = json!({ "ok": true, "n": 1 });
    let out = render(None, Format::Json, &payload);
    assert_eq!(out, "{\n  \"n\": 1,\n  \"ok\": true\n}\n");
}
