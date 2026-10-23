//! D-35..D-41 — the Rust port kept the case ids (audit F-12).
//! Read/recall lean shapes follow proposal §1 and §3 (supersede JS D-41).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::errors::CortError;
use cort::impact::impact_command;
use cort::indexer::full_index;
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

fn chain() -> (tempfile::TempDir, rusqlite::Connection, Value) {
    let (dir, root, db, project_id, bin) = indexed(CHAIN);
    let impact = impact_command(&db, &bin, &root, &project_id, "d", 3).unwrap();
    (dir, db, impact)
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
    let (_dir, _db, impact) = chain();
    let out = render(Some("impact"), Format::Lean, &impact);
    assert!(
        out.lines()
            .any(|l| l == "# impact d depth=3 seeds=1 dependents=3 stale=false repair=none"),
        "{out}"
    );
    // Six columns, the last two new in schema v4: the line *inside* the dependent that names the
    // callee, and the call shape the extractor saw. In this fixture every function is one line, so
    // the definition line and the call site are the same number -- which is itself the reason the
    // two are printed separately.
    assert!(
        out.lines().any(|l| l == "h1\tsrc/c.ts\tc\t2\t@2\tbare"),
        "{out}"
    );
    assert!(
        out.lines().any(|l| l == "h3\tsrc/a.ts\ta\t2\t@2\tbare"),
        "{out}"
    );
    for line in out.lines().skip(1).filter(|l| l.starts_with('h')) {
        assert_eq!(line.split('\t').count(), 6, "fixed shape: {line}");
    }
    // The blind row names all three counts, so `unparsed` can never be read as "unread".
    let with_blind = serde_json::json!({
        "symbol": "d", "depth": 1, "seed_count": 0, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2",
            "seeds": [{"symbol": "d", "mentions_on_disk": 0, "mentions_without_edge": [],
                        "extracted_but_unresolved": [], "enumeration_may_be_incomplete": false}],
            "blind_files": {"unparsed": 2, "unindexed": 0, "scan_skipped": 0,
                             "unparsed_example": ["rust/src/lib.rs", "src/notes.ts"]},
        },
    });
    let blind_out = render(Some("impact"), Format::Lean, &with_blind);
    assert!(
        blind_out.contains("blind\tunparsed=2\tunindexed=0\tunread=0"),
        "{blind_out}"
    );
    assert!(
        blind_out.lines().any(|l| l.contains("advisory") && l.contains("does not flip")),
        "the lean reader has to be told how to read a false next to a non-zero unparsed: {blind_out}"
    );
    assert!(
        blind_out.contains("blind\tunparsed\trust/src/lib.rs"),
        "paths, not just the count: {blind_out}"
    );
    // The no-seed shape never reads the tree, so `scan_skipped` was never computed. A blank field
    // reads as zero, and zero reads as "nothing was skipped" -- the false-safe claim itself.
    let no_scan = serde_json::json!({
        "symbol": "z", "depth": 1, "seed_count": 0, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2", "no_seed_resolved": true, "seeds": [],
            "blind_files": {"unparsed": 1, "unindexed": 0},
        },
    });
    let no_scan_out = render(Some("impact"), Format::Lean, &no_scan);
    assert!(
        no_scan_out.contains("blind\tunparsed=1\tunindexed=0\tunread=-"),
        "unknown must print as a dash: {no_scan_out}"
    );
    // A dependency that came in through an import has no call site. `-`, never line 0 and never a
    // shorter row: a missing value that changes the row's shape gets misread as a different column.
    let no_site = serde_json::json!({
        "symbol": "x", "depth": 1, "seed_count": 1, "seeds": [], "dependent_count": 1,
        "index_is_stale": false,
        "dependents": [{
            "chunk_id": "p:a.ts:9", "symbol_name": "uses", "file_path": "a.ts",
            "start_line": 9, "end_line": 12, "hop": 1,
        }],
    });
    assert!(
        render(Some("impact"), Format::Lean, &no_site)
            .lines()
            .any(|l| l == "h1\ta.ts\tuses\t9\t-\t-"),
        "{no_site}"
    );
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
fn lean_is_smaller_than_json_for_the_same_impact_payload() {
    let (_dir, _db, impact) = chain();
    let json = render(Some("impact"), Format::Json, &impact);
    let lean = render(Some("impact"), Format::Lean, &impact);
    assert!(
        lean.len() < json.len(),
        "impact: lean {} should be < json {}",
        lean.len(),
        json.len()
    );
    assert!(lean.ends_with('\n'));
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

/// Error envelope to_json field names (spec §4.1).
#[test]
fn error_envelope_to_json_uses_error_not_code() {
    let err = CortError::new("unknown_format", json!({ "hint": "--format json|lean" }));
    let out = render_error(Format::Json, &err);
    let parsed: Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(parsed, err.to_json());
    assert!(parsed.get("code").is_none());
    assert_eq!(parsed["error"], "unknown_format");
    // Errors have no lean shape: `-f lean` gets the same JSON envelope.
    assert_eq!(render_error(Format::Lean, &err), out);
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

/// The gap list is capped at `MAX_GAP_ROWS`, and lean is the format the routing skill tells agents
/// to always pass. Printing the *printed* row count as `no_edge` told that reader it had seen every
/// gap: `Tally::add` on this repo produces 51 rows, prints 20, and said `no_edge=20`. "Read the
/// rows, not the boolean" is only true if the reader is told when the rows were cut.
#[test]
fn a_truncated_gap_list_says_how_many_rows_it_dropped() {
    let rows: Vec<Value> = (0..20)
        .map(|i| {
            json!({
                "file_path": "src/a.rs", "line": i + 1, "cause": "mention",
                "occurrences": 1, "text": "add(x);",
            })
        })
        .collect();
    let truncated = json!({
        "symbol": "add", "depth": 1, "seed_count": 1, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2",
            "seeds": [{
                "symbol": "add", "mentions_on_disk": 69, "gap_count": 51,
                "mentions_truncated": true, "mentions_without_edge": rows,
                "extracted_but_unresolved": [], "enumeration_may_be_incomplete": true,
            }],
        },
    });
    let out = render(Some("impact"), Format::Lean, &truncated);
    assert!(
        out.contains("no_edge=51"),
        "the seed line must carry every gap found, not the number that fitted: {out}"
    );
    assert!(
        !out.contains("no_edge=20"),
        "20 is the printed count and reads as the total: {out}"
    );
    assert!(
        out.lines().any(|l| l.starts_with("miss\ttruncated\t")
            && l.contains("shown=20")
            && l.contains("of=51")),
        "a row has to name the cut, beside the rows it cut: {out}"
    );

    // Not truncated: no row claiming a cut that did not happen.
    let whole = json!({
        "symbol": "add", "depth": 1, "seed_count": 1, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2",
            "seeds": [{
                "symbol": "add", "mentions_on_disk": 4, "gap_count": 1,
                "mentions_truncated": false,
                "mentions_without_edge": [{"file_path": "src/a.rs", "line": 3,
                                            "cause": "mention", "occurrences": 1, "text": "add(x);"}],
                "extracted_but_unresolved": [], "enumeration_may_be_incomplete": true,
            }],
        },
    });
    let whole_out = render(Some("impact"), Format::Lean, &whole);
    assert!(whole_out.contains("no_edge=1"), "{whole_out}");
    assert!(
        !whole_out.contains("truncated"),
        "an uncut list must not announce a cut: {whole_out}"
    );

    // A report written without `gap_count` still prints a number, never a dash: a blank there reads
    // as "no gaps", which is the false-safe claim this screen exists to prevent.
    let legacy = json!({
        "symbol": "add", "depth": 1, "seed_count": 1, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2",
            "seeds": [{
                "symbol": "add", "mentions_on_disk": 4,
                "mentions_without_edge": [{"file_path": "src/a.rs", "line": 3,
                                            "cause": "mention", "occurrences": 1, "text": "add(x);"}],
                "extracted_but_unresolved": [], "enumeration_may_be_incomplete": true,
            }],
        },
    });
    assert!(
        render(Some("impact"), Format::Lean, &legacy).contains("no_edge=1"),
        "missing gap_count falls back to the rows in hand"
    );
}

/// An import drop has no source symbol to name, and a brace import carries its newlines into the
/// stored raw target. A blank field reads as an empty column and an embedded newline breaks
/// one-row-per-line, so the lean contract collapses both.
#[test]
fn an_import_drop_row_prints_a_dash_for_its_missing_symbol_and_one_line_per_row() {
    let payload = json!({
        "symbol": "beta_fn", "depth": 1, "seed_count": 1, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2",
            "seeds": [{
                "symbol": "beta_fn", "mentions_on_disk": 2, "gap_count": 0,
                "mentions_without_edge": [],
                "extracted_but_unresolved": [{
                    "file_path": "src/main.rs", "line": 3, "from_symbol": "",
                    "raw_target": "cort_evals::demand::{\n    classify, beta_fn,\n}",
                    "via": "import",
                }],
                "enumeration_may_be_incomplete": true,
            }],
        },
    });
    let out = render(Some("impact"), Format::Lean, &payload);
    let drop = out
        .lines()
        .find(|l| l.starts_with("drop\t"))
        .expect("the drop row prints");
    assert_eq!(
        drop, "drop\tsrc/main.rs:3\t- -> cort_evals::demand::{ classify, beta_fn, }",
        "dash for the missing symbol, newlines collapsed: {out}"
    );
}

/// `gap_count` is the number the boolean reads -- mention rows plus dropped resolutions -- so it
/// must not drive the truncation claim: a seed with three printed mention rows and two dropped
/// resolutions would announce "truncated shown=3 of=5" over a mention list nothing was cut from.
/// `mention_gap_count` is the uncapped mention-layer figure that claim is derived from.
#[test]
fn dropped_resolutions_do_not_make_the_mention_list_look_truncated() {
    let rows: Vec<Value> = (0..3)
        .map(|i| {
            json!({
                "file_path": "src/a.rs", "line": i + 1, "cause": "mention",
                "occurrences": 1, "text": "beta_fn(x);",
            })
        })
        .collect();
    let payload = json!({
        "symbol": "x", "depth": 1, "seed_count": 1, "seeds": [], "dependent_count": 0,
        "unresolved": [], "index_is_stale": false,
        "coverage": {
            "method": "coverage-v2",
            "seeds": [{
                "symbol": "x", "mentions_on_disk": 9, "gap_count": 5, "mention_gap_count": 3,
                "mentions_truncated": false, "mentions_without_edge": rows,
                "extracted_but_unresolved": [
                    {"file_path": "src/b.rs", "line": 8, "from_symbol": "caller",
                     "raw_target": "x", "via": "import"},
                    {"file_path": "src/c.rs", "line": 12, "from_symbol": "caller2",
                     "raw_target": "x"},
                ],
                "enumeration_may_be_incomplete": true,
            }],
        },
    });
    let out = render(Some("impact"), Format::Lean, &payload);
    assert!(out.contains("no_edge=3"), "{out}");
    assert!(out.contains("dropped=2"), "{out}");
    assert!(
        !out.contains("truncated"),
        "nothing was cut from the mention list; counting the drops in would fake a cut: {out}"
    );
}

/// The lean header is the line agents read, so the repair classification rides on it verbatim.
/// The values are the hook's own outcome vocabulary: `none` (nothing to repair), `refreshable`
/// (an incremental is accepted; the PostToolUse hook heals it on the next edit),
/// `rebuild_required` (the hook refuses; only a foreground `cort index` fixes it). Impact's
/// payload derives the token; render only carries it.
#[test]
fn the_lean_header_carries_the_repair_classification() {
    let payload = serde_json::json!({
        "symbol": "d",
        "depth": 3,
        "seeds": [],
        "dependent_count": 0,
        "index_is_stale": true,
        "repair": "rebuild_required",
    });
    let out = render(Some("impact"), Format::Lean, &payload);
    assert!(
        out.lines().any(|l| {
            l == "# impact d depth=3 seeds=0 dependents=0 stale=true repair=rebuild_required"
        }),
        "{out}"
    );
}
