//! Recall screen: "what did this enumeration miss?" — the question `dependents` cannot answer.

use cort::ast_grep::resolve_ast_grep_bin;
use cort::coverage::{attach, bare_name, cause_of, mentions};
use cort::db::{ensure_schema, open_db, project_id_for};
use cort::impact::impact_command;
use cort::indexer::full_index;
use serde_json::{json, Value};
use std::fs;

fn indexed(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let dir = tempfile::Builder::new()
        .prefix("cort-cov-")
        .tempdir()
        .unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, body).unwrap();
    }
    let root = fs::canonicalize(dir.path()).unwrap();
    let mut db = open_db(":memory:").unwrap();
    ensure_schema(&db).unwrap();
    let project_id = project_id_for(root.to_str().unwrap());
    let bin = resolve_ast_grep_bin().expect("ast-grep");
    full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

fn coverage_of(
    db: &rusqlite::Connection,
    project_id: &str,
    root: &std::path::Path,
    bin: &str,
    symbol: &str,
) -> Value {
    let mut out = impact_command(db, bin, root, project_id, symbol, 3).unwrap();
    attach(db, project_id, root, &mut out)
        .unwrap_or_else(|e| panic!("coverage attaches: {} {}", e.code, e.detail));
    out["coverage"].clone()
}

#[test]
fn a_receiver_call_is_reported_even_though_the_graph_has_no_edge_for_it() {
    // The shipped Rust pack extracts `foo()` and `Type::method()`, not `x.method()`. So the graph
    // legitimately answers `dependents=0` here -- and without this screen that is indistinguishable
    // from "nothing calls it", which is the mistake the product's whole purpose is to prevent.
    let (_dir, root, db, project_id, bin) = indexed(&[
        (
            "src/lib.rs",
            "pub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
        ),
        (
            "src/use.rs",
            "use crate::lib::T;\nfn go(t: &T) -> u32 { t.take() }\n",
        ),
    ]);
    let impact = impact_command(&db, &bin, &root, &project_id, "T::take", 3).unwrap();
    assert_eq!(
        impact["dependent_count"].as_i64(),
        Some(0),
        "this test is pinning the documented extractor limitation; if the pack starts extracting \
         receiver calls, update both this assertion and the screen"
    );
    let cov = coverage_of(&db, &project_id, &root, &bin, "T::take");
    let seed = &cov["seeds"][0];
    assert_eq!(seed["enumeration_may_be_incomplete"], Value::Bool(true));
    let gaps = seed["mentions_without_edge"].as_array().unwrap();
    assert!(
        gaps.iter()
            .any(|g| g["file_path"] == Value::String("src/use.rs".into())
                && g["cause"] == Value::String("receiver".into())),
        "{cov}"
    );
}

#[test]
fn a_call_the_pack_did_extract_is_not_counted_as_a_gap() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        (
            "src/c.ts",
            "import { d } from './d';\nexport function c() { return d(); }\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let seed = &cov["seeds"][0];
    assert!(
        seed["mentions_covered_by_edge"].as_u64().unwrap() >= 1,
        "{cov}"
    );
    assert!(
        seed["mentions_without_edge"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["file_path"] != Value::String("src/c.ts".into())),
        "the real call site was extracted, so it must not be reported as missing: {cov}"
    );
}

#[test]
fn a_name_inside_a_string_is_attributed_to_quoted_and_sorted_last() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/a.ts", "export function a() { return 1; }\n"),
        ("src/b.ts", "export const msg = \"do not call a() manually\";\nexport function b() { return a(); }\n"),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "a");
    let gaps = cov["seeds"][0]["mentions_without_edge"].as_array().unwrap();
    let quoted: Vec<&Value> = gaps
        .iter()
        .filter(|g| g["cause"] == Value::String("quoted".into()))
        .collect();
    assert!(
        !quoted.is_empty(),
        "the string mention should still surface: {cov}"
    );
    // Cause ordering is what keeps a real hole visible on a busy name: rank 0 rows come first.
    let first = &gaps[0];
    assert_eq!(first["cause"], Value::String("quoted".into()));
    assert!(
        cov["blind_files"]["unindexed"].as_u64().unwrap() == 0,
        "{cov}"
    );
}

#[test]
fn a_file_that_never_entered_the_index_is_a_blind_spot_and_says_so() {
    let (dir, root, db, project_id, bin) =
        indexed(&[("src/d.ts", "export function d() { return 1; }\n")]);
    // Written after the index: exactly the "new untracked file" hole the stale flag does not cover,
    // because the graph is fresh -- it just cannot see this file.
    fs::write(
        dir.path().join("src/new.ts"),
        "import { d } from './d';\nexport function n() { return d(); }\n",
    )
    .unwrap();
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let blind = &cov["blind_files"];
    assert!(blind["unindexed"].as_u64().unwrap() >= 1, "{cov}");
    assert!(
        blind["unindexed_example"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("src/new.ts")),
        "{blind}"
    );
}

#[test]
fn absence_of_a_signal_is_never_named_like_proof() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        (
            "src/c.ts",
            "import { d } from './d';\nexport function c() { return d(); }\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let text = cov["seeds"][0].as_object().unwrap();
    assert!(
        !text.contains_key("complete")
            && !text.contains_key("is_complete")
            && !text.contains_key("verified"),
        "the field naming has to keep the asymmetry: {text:?}"
    );
    assert_eq!(text["enumeration_may_be_incomplete"], Value::Bool(false));
    assert!(cov["reading"]
        .as_str()
        .unwrap()
        .contains("Never read absence of a signal"));
}

#[test]
fn a_generated_bundle_is_not_allowed_to_look_like_a_missed_caller() {
    // cct's index really does hold seven .wrangler/tmp/deploy-*/index.js copies. They mention every
    // symbol in the app and can never be callers, so they must not headline the orphan list.
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        (
            "src/c.ts",
            "import { d } from './d';\nexport function c() { return d(); }\n",
        ),
        (
            ".wrangler/tmp/deploy-x/index.js",
            "var q=Object.create;function d(){return 1} export { d };\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    // Two chunks answer to `d` here (the source file and the bundle both define it), so assert over
    // every seed instead of assuming which one the resolver emits first.
    let seeds = cov["seeds"].as_array().unwrap();
    assert!(
        seeds
            .iter()
            .any(|s| s["generated_files_with_gaps"].as_u64() == Some(1)),
        "no seed flagged the generated copy: {cov}"
    );
    for seed in seeds {
        assert!(
            seed["files_with_no_edge_at_all"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| !v.as_str().unwrap_or_default().contains(".wrangler")),
            "a generated copy is never a missed caller: {cov}"
        );
    }
    // The copy is still listed -- hiding a mention would be the actual sin -- but labelled for what
    // it is, and ranked behind every kind of real hole.
    let rows = seeds
        .iter()
        .map(|s| s["mentions_without_edge"].as_array().unwrap().clone())
        .find(|r| {
            r.iter()
                .any(|g| g["cause"] == Value::String("artifact".into()))
        })
        .expect("the bundle mention should surface, labelled: none of {cov}");
    assert_eq!(
        rows.last().unwrap()["cause"],
        Value::String("artifact".into()),
        "a generated copy must never outrank a real hole"
    );
}

#[test]
fn a_blind_file_is_never_a_clean_bill_of_health() {
    // The independent review (agy + gemini-3.1-pro, 2026-08-31) found the dangerous reading: a file
    // the chunker could not read holds callers the mention scan never sees, so every per-seed gap
    // list was empty and the flag answered `false` -- "nothing missed" while the graph was knowingly
    // blind. Absence of signal in a blind tree must fail toward "may be incomplete".
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        (
            "src/c.ts",
            "import { d } from './d';\nexport function c() { return d(); }\n",
        ),
        ("src/notes.ts", "// a file with no symbols at all\n"),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let blind = &cov["blind_files"];
    let blind_seen =
        blind["unparsed"].as_u64().unwrap_or(0) + blind["unindexed"].as_u64().unwrap_or(0);
    assert!(
        blind_seen >= 1,
        "fixture should produce a blind file: {cov}"
    );
    assert!(
        cov["seeds"].as_array().unwrap().iter().any(|s| {
            s["enumeration_may_be_incomplete"] == Value::Bool(true)
                && s["why"]
                    .as_array()
                    .map(|w| w.contains(&Value::String("blind_files".into())))
                    == Some(true)
        }),
        "the blind file must reach the per-seed verdict, not just a side table: {cov}"
    );
    assert!(
        blind["unparsed_example"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or_default().contains("notes.ts")),
        "a count without the path is not actionable: {blind}"
    );
}

#[test]
fn a_single_quoted_string_is_not_reported_as_a_bare_mention() {
    // K3's review: the quote arithmetic only counted `\u{201c}`, so `\'./d\'` came back as a plain
    // `mention` and competed with real holes for the top of the list.
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        (
            "src/c.ts",
            "import { d } from './d';\nexport function c() { return d(); }\n",
        ),
        (
            "src/p.ts",
            "export const p = 1;\nconst where = './d';\nexport const q = p;\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let quoted_p = cov["seeds"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["mentions_without_edge"].as_array().unwrap().iter())
        .filter(|g| g["file_path"] == Value::String("src/p.ts".into()))
        .collect::<Vec<_>>();
    assert!(
        !quoted_p.is_empty(),
        "the string mention should surface: {cov}"
    );
    assert_eq!(
        quoted_p[0]["cause"],
        Value::String("quoted".into()),
        "{quoted_p:?}"
    );
}

#[test]
fn two_mentions_on_one_line_collapse_into_one_row_that_says_so() {
    // Before this, `d + d` on one line printed the same row twice and made a one-line file look
    // like a two-line hole.
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        // The duplicate sits 9 lines below the real call so the line tolerance cannot absorb it:
        // that absorption is a separate deliberate behaviour, not something this test may hide.
        (
            "src/u.ts",
            "import { d } from './d';\nexport function u() { return d(); }\n\n\n\n\n\n\n\nexport const sum = d + d;\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let rows = cov["seeds"][0]["mentions_without_edge"].as_array().unwrap();
    let dup = rows
        .iter()
        .filter(|g| g["file_path"] == Value::String("src/u.ts".into()) && g["line"] == json!(10))
        .collect::<Vec<_>>();
    assert_eq!(
        dup.len(),
        1,
        "one row for line 10, not one per mention: {rows:?}"
    );
    assert_eq!(dup[0]["occurrences"], json!(2), "{rows:?}");
    assert_eq!(dup[0]["cause"], Value::String("mention".into()), "{rows:?}");
}

#[test]
fn a_symbol_that_is_not_indexed_reports_itself_instead_of_failing() {
    // An unanswerable question is a finding, not a crash: exit 1 with `nothing_indexed` let a
    // caller reading "command failed" treat the whole check as not-applicable.
    let (_dir, root, db, project_id, bin) =
        indexed(&[("src/d.ts", "export function d() { return 1; }\n")]);
    let mut out = impact_command(&db, &bin, &root, &project_id, "never_heard_of_it", 3).unwrap();
    assert_eq!(out["seed_count"], json!(0));
    attach(&db, &project_id, &root, &mut out).expect("attach must succeed with no seed");
    let cov = &out["coverage"];
    assert_eq!(cov["no_seed_resolved"], Value::Bool(true), "{cov}");
    assert_eq!(
        cov["enumeration_may_be_incomplete"],
        Value::Bool(true),
        "{cov}"
    );
    assert_eq!(
        cov["why"][0],
        Value::String("no_seed_resolved".into()),
        "{cov}"
    );
}

#[test]
fn qualification_and_word_boundaries_are_parsed_the_way_the_caller_reads_them() {
    assert_eq!(bare_name("Tally::add"), "add");
    assert_eq!(bare_name("formatter.formatToParts"), "formatToParts");
    assert_eq!(bare_name("parseInt"), "parseInt");
    assert_eq!(bare_name(""), "");

    let hits = mentions("tally.add(x); addx(1); add(2);", "add");
    // 1-based (line, column): the receiver call and the bare call, not `addx`.
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert_eq!(hits[0].0, 1);
    assert_eq!(
        cause_of("tally.add(x); addx(1); add(2);", hits[0].1, 3),
        "receiver"
    );
    assert_eq!(
        cause_of("tally.add(x); addx(1); add(2);", hits[1].1, 3),
        "call"
    );
    assert_eq!(cause_of("const s = \"call add now\";", 16, 3), "quoted");
    // Prose that quotes a call is not a hole in the enumeration, and must not outrank one.
    assert_eq!(cause_of("/// t.add() appears twice", 7, 3), "comment");
    assert_eq!(cause_of(" * t.add() as documentation", 7, 3), "comment");
}
