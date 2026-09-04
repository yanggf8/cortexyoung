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

/// The case that started the recall line: one `t.take()`, and `take` belongs to exactly one symbol
/// in the project. v4 attaches it and records the line to read, so the screen has no hole left here
/// to report -- a gap row that disappears because the edge exists is the only good outcome.
#[test]
fn a_unique_receiver_call_becomes_an_edge_with_a_line_to_check() {
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
    assert_eq!(impact["dependent_count"].as_i64(), Some(1), "{impact}");
    let dep = &impact["dependents"][0];
    assert_eq!(dep["symbol_name"], Value::String("go".into()));
    assert_eq!(dep["start_line"].as_i64(), Some(2), "where `go` is defined");
    assert_eq!(
        dep["call_site_line"].as_i64(),
        Some(2),
        "where `go` calls `take` -- the same line here, because the fixture is one line long"
    );
    assert_eq!(dep["call_form"], Value::String("receiver".into()));

    let cov = coverage_of(&db, &project_id, &root, &bin, "T::take");
    let seed = &cov["seeds"][0];
    assert_eq!(seed["mentions_without_edge"], json!([]), "{cov}");
    assert_eq!(seed["extracted_but_unresolved"], json!([]), "{cov}");
    assert_eq!(seed["enumeration_may_be_incomplete"], Value::Bool(false));
}

/// The other half of the gate, and the half that keeps the first one honest: `take` belongs to two
/// symbols, so no edge is invented -- and the call site is still reported, one layer down, as
/// "extracted but unresolved" instead of vanishing from both answers.
#[test]
fn an_ambiguous_receiver_call_attaches_nothing_and_still_shows_up_as_a_gap() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        (
            "src/lib.rs",
            concat!(
                "pub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
                "pub struct U;\nimpl U { pub fn take(&self) -> u32 { 2 } }\n",
            ),
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
        "two candidates named `take` is a guess: {impact}"
    );
    let cov = coverage_of(&db, &project_id, &root, &bin, "T::take");
    let seed = &cov["seeds"][0];
    assert_eq!(seed["enumeration_may_be_incomplete"], Value::Bool(true));
    assert!(
        seed["why"]
            .as_array()
            .unwrap()
            .contains(&Value::String("extracted_but_unresolved".into())),
        "{cov}"
    );
    let dropped = seed["extracted_but_unresolved"].as_array().unwrap();
    assert!(
        dropped
            .iter()
            .any(|d| d["file_path"] == Value::String("src/use.rs".into())
                && d["raw_target"] == Value::String("t.take".into())
                && d["line"].as_i64() == Some(2)),
        "the refused edge has to be discoverable from the row, not just from the count: {cov}"
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
    let reading = cov["reading"].as_str().unwrap();
    assert!(
        reading.contains("Read the rows, not the boolean")
            && reading.contains("neither is a proof of anything"),
        "the report has to say how to read the flag, in both directions: {reading}"
    );
    assert!(
        reading.contains(".sh, .txt, config")
            && reading.contains("2 MB")
            && reading.contains("barrel"),
        "and the boundaries K3 listed as missing must be in the same sentence: {reading}"
    );
    // The one hole no text screen can close (34a2ca10): a caller that exists only after macro
    // expansion. Nothing pins it anywhere in the suite -- it lives only in prose -- so the sentence
    // the report must keep carrying gets its own assertion here. If the wording drifts without the
    // disclosure moving, this fails.
    assert!(
        reading.contains("macro expansion") && reading.contains("cargo check"),
        "the disclosure of the macro-expanded caller must name the tool that can answer it: {reading}"
    );
    // The version-discipline rule behind `COVERAGE_METHOD`: the method string names the version
    // majorly enough that a semantic change without a version bump is visible in the report
    // itself. The constant's *value* is pinned by the render contract tests; what is pinned here is
    // that a reader can tell v2 from v1 by reading the method field.
    let method = cov["method"].as_str().unwrap();
    assert!(
        method.contains("coverage-v2"),
        "a reader must be able to tell which screen generation produced this report: {method}"
    );
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
    // this screen never read holds callers nothing else can say anything about, so the per-seed gap
    // list was empty and the flag answered `false` -- "nothing missed" while the tree was knowingly
    // blind. Absence of signal in an unread tree is not absence of gaps: the flag fails toward "may
    // be incomplete". This is the *unread* case (a file on disk that never entered the index); the
    // read-but-chunk-less case is the next test and must not behave this way.
    let (dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        (
            "src/c.ts",
            "import { d } from './d';\nexport function c() { return d(); }\n",
        ),
    ]);
    fs::write(
        dir.path().join("src/extra.ts"),
        "// added after the index: nobody read it at all\n",
    )
    .unwrap();
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let blind = &cov["blind_files"];
    assert!(
        blind["unindexed"].as_u64().unwrap_or(0) >= 1,
        "fixture should produce an unread file: {cov}"
    );
    assert!(
        cov["seeds"].as_array().unwrap().iter().any(|s| {
            s["enumeration_may_be_incomplete"] == Value::Bool(true)
                && s["why"]
                    .as_array()
                    .map(|w| w.contains(&Value::String("blind_files".into())))
                    == Some(true)
        }),
        "the unread file must reach the per-seed verdict, not just a side table: {cov}"
    );
    assert!(
        blind["unindexed_example"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or_default().contains("extra.ts")),
        "a count without the path is not actionable: {blind}"
    );
}

/// A file with no chunks is blind to the *graph*, not to this screen: its text is scanned, so a
/// caller in it shows up as a row. Counting it as a gap anyway made 2 files in this repo (4 in cct)
/// answer `true` for **every** seed -- a boolean that is always true gets read as noise, and the
/// noise drags the real warning down with it. So: still listed with its paths, still named in `why`,
/// never a verdict.
#[test]
fn a_file_with_no_chunks_is_advisory_and_does_not_flip_every_seed() {
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
    assert!(
        blind["unparsed"].as_u64().unwrap_or(0) >= 1,
        "the fixture must really produce a chunk-less file: {blind}"
    );
    assert!(
        blind["unparsed_example"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or_default().contains("notes.ts")),
        "advisory does not mean hidden: the path still has to be there: {blind}"
    );
    assert_eq!(
        blind["unindexed"].as_u64().unwrap_or(0),
        0,
        "nothing here is unread: {blind}"
    );
    let seed = &cov["seeds"][0];
    assert_eq!(
        seed["enumeration_may_be_incomplete"],
        Value::Bool(false),
        "an unread-free tree with no gap rows is not a gap, chunk-less file or not: {cov}"
    );
    assert!(
        seed["why"]
            .as_array()
            .unwrap()
            .contains(&Value::String("unparsed_files".into())),
        "and the advisory still has to be named where the reasons live: {seed}"
    );
    assert!(
        !seed["why"]
            .as_array()
            .unwrap()
            .contains(&Value::String("blind_files".into())),
        "`blind_files` is reserved for trees this screen never read: {seed}"
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
fn a_file_too_big_to_read_is_reported_as_skipped_instead_of_clean() {
    // K3's worst false negative: the mention scan has a size cap, and a file over it used to vanish
    // from every field -- an indexed, parsed, healthy file whose callers were never looked at still
    // answered `incomplete=false`. The cap is fine; the silence was not.
    let big = format!(
        "export const blob = \"{}\";\nexport function use_it() {{ return d(); }}\n",
        "x".repeat(2_100_000)
    );
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        ("src/huge.ts", big.as_str()),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let blind = &cov["blind_files"];
    assert!(blind["scan_skipped"].as_u64().unwrap_or(0) >= 1, "{blind}");
    assert!(
        blind["scan_skipped_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or_default() == "src/huge.ts"),
        "{blind}"
    );
    assert!(
        cov["seeds"].as_array().unwrap().iter().all(|s| {
            s["enumeration_may_be_incomplete"] == Value::Bool(true)
                && s["why"]
                    .as_array()
                    .map(|w| w.contains(&Value::String("scan_skipped".into())))
                    == Some(true)
        }),
        "a skipped file must reach every seed's verdict: {cov}"
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

#[test]
fn a_declaration_line_is_never_a_call_even_when_the_declaration_is_not_a_chunk() {
    // The shape that motivated it: a trait method signature. `trait_item` bodies declare `fn add(...)`
    // without a body, the pack has no rule for that node, so no chunk exists at that line and the old
    // label was `call`.
    assert_eq!(
        cause_of("    fn add(&self, x: i32) -> i32;", 8, 3),
        "definition"
    );
    assert_eq!(cause_of("pub fn add(&self) {}", 8, 3), "definition");
    assert_eq!(cause_of("pub(crate) fn add() {}", 15, 3), "definition");
    assert_eq!(cause_of("async fn add() {}", 10, 3), "definition");
    assert_eq!(cause_of("trait Reader { fn read(); }", 19, 4), "definition");
    assert_eq!(cause_of("mod fs;", 5, 2), "definition");
    // A call *inside* a declaration line keeps its own label: only the name directly after the
    // keyword is the thing being declared.
    assert_eq!(
        cause_of("pub fn add(x: i32) -> i32 { helper(x) }", 29, 6),
        "call"
    );
    assert_eq!(cause_of("let add = compute(1);", 5, 3), "mention");
    assert_eq!(cause_of("tally.add(&p);", 7, 3), "receiver");
}

/// What the row order is for: the most severe finding must not lose to unrelated prose.
///
/// `file_covered` used to be the *primary* sort key, so a genuine hole in a file that also contained
/// one extracted call was ranked below comment noise from an unrelated file -- and `MAX_GAP_ROWS`
/// truncates the printed list, which is how "read the rows, not the boolean" quietly stops being
/// true. The boolean still said `true`; the row an agent needed was off the page.
#[test]
fn a_real_gap_in_a_partially_covered_file_outranks_comment_noise_from_an_uncovered_one() {
    let c_ts = format!(
        "import {{ d }} from './d';\nexport function c() {{ return d(); }}\n{}export function c2(x: number) {{ return x + d; }}\n",
        "// padding keeps the two mentions outside the line tolerance\n".repeat(18)
    );
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/d.ts", "export function d() { return 1; }\n"),
        ("src/c.ts", &c_ts),
        (
            "src/u.ts",
            "// d is probably unused around here\nexport function u() { return 2; }\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let gaps = cov["seeds"][0]["mentions_without_edge"].as_array().unwrap();
    let causes: Vec<&str> = gaps.iter().map(|g| g["cause"].as_str().unwrap()).collect();
    assert!(
        causes.contains(&"comment"),
        "the unrelated file's prose should still be listed: {cov}"
    );
    assert_eq!(
        (gaps[0]["cause"].as_str().unwrap(), gaps[0]["file_path"].as_str().unwrap()),
        ("mention", "src/c.ts"),
        "the unmapped use of the name in a partially-covered file is the row that must be on top, got {cov}"
    );
}

// ── L2 must see what the extractor extracted and resolution dropped -- imports included ──
//
// Rust `edge:imports` raw edges never become relationships: a top-level `use` belongs to no
// function chunk, so `chunk_by_symbol.get(source_symbol)` misses and the edge is dropped before
// resolution even runs. Until 2026-09-01 L2 queried `rel_type = 'calls'` only, so that drop was
// invisible to the one screen whose job is to name dropped resolutions -- the mention layer saw the
// text, but nothing distinguished "the pack never saw this line" from "the pack saw it and
// resolution discarded it".

/// A file that imports a symbol and never calls it in an extracted shape has its dependency on that
/// symbol wholly absent from the graph: no edge, and no other file carries it. The use line is the
/// only trace, and L2 is the layer that must name it -- pack-attested, not a text coincidence.
#[test]
fn an_import_the_extractor_saw_but_could_not_resolve_is_a_pack_attested_drop() {
    let (_d, root, db, project_id, bin) = indexed(&[
        ("src/foo.rs", "pub fn target_fn(x: u8) -> u8 { x }\n"),
        (
            "src/main.rs",
            "mod foo;\nuse crate::foo::target_fn;\n\npub fn unrelated() -> u8 { 1 }\n",
        ),
    ]);
    let c = coverage_of(&db, &project_id, &root, &bin, "target_fn");
    let seed = &c["seeds"][0];
    let dropped = seed["extracted_but_unresolved"].as_array().unwrap();
    assert!(
        dropped
            .iter()
            .any(|g| g["file_path"] == "src/main.rs" && g["raw_target"] == "crate::foo::target_fn"),
        "the use line must appear as a dropped resolution: {dropped:?}"
    );
    assert!(
        seed["why"]
            .as_array()
            .unwrap()
            .contains(&json!("extracted_but_unresolved")),
        "the drop must name its reason; mentions_without_edge is also expected -- the use line is text too: {}",
        seed["why"]
    );
    assert_eq!(seed["enumeration_may_be_incomplete"], json!(true));
}

/// The suppression is file-level, not line-level, and that is the semantics: a use whose symbol the
/// same file already reaches through a resolved call contributes a dependency the graph already
/// carries. Reporting it would bury the drops that matter under duplicates.
#[test]
fn an_import_whose_file_already_reaches_the_seed_is_suppressed() {
    let (_d, root, db, project_id, bin) = indexed(&[
        ("src/foo.rs", "pub fn target_fn(x: u8) -> u8 { x }\n"),
        (
            "src/main.rs",
            "mod foo;\nuse crate::foo::target_fn;\n\npub fn caller() -> u8 { target_fn(1) }\n",
        ),
    ]);
    let c = coverage_of(&db, &project_id, &root, &bin, "target_fn");
    let seed = &c["seeds"][0];
    let dropped = seed["extracted_but_unresolved"].as_array().unwrap();
    assert!(
        dropped.is_empty(),
        "the file reaches the seed through a resolved call; the use line is not a loss: {dropped:?}"
    );
}

/// `use crate::foo::{alpha_fn, beta_fn};` is one raw edge carrying two names. The leaf match has to
/// open the braces: this exact shape is how `recall-exp`'s shadow metric once reported a safe 0
/// (2461d2c8), and coverage must not repeat that blind spot.
#[test]
fn a_brace_import_reports_each_name_the_extractor_could_not_resolve() {
    let (_d, root, db, project_id, bin) = indexed(&[
        (
            "src/foo.rs",
            "pub fn alpha_fn(x: u8) -> u8 { x }\npub fn beta_fn(x: u8) -> u8 { x }\n",
        ),
        (
            "src/main.rs",
            "mod foo;\nuse crate::foo::{alpha_fn, beta_fn};\n\npub fn caller() -> u8 { alpha_fn(1) }\n",
        ),
    ]);
    let c = coverage_of(&db, &project_id, &root, &bin, "beta_fn");
    let dropped = c["seeds"][0]["extracted_but_unresolved"]
        .as_array()
        .unwrap();
    assert!(
        dropped.iter().any(|g| g["file_path"] == "src/main.rs"
            && g["raw_target"].as_str().unwrap().contains("beta_fn")),
        "the brace import must be opened and beta_fn named: {dropped:?}"
    );
    // alpha_fn resolves through the call, so the same use line must not be reported for it.
    let c_alpha = coverage_of(&db, &project_id, &root, &bin, "alpha_fn");
    let dropped_alpha = c_alpha["seeds"][0]["extracted_but_unresolved"]
        .as_array()
        .unwrap();
    assert!(
        dropped_alpha.is_empty(),
        "alpha_fn is reached through a resolved call in the same file: {dropped_alpha:?}"
    );
}

/// `gap_count` must be the number `enumeration_may_be_incomplete` reads. The boolean's row count is
/// `mention rows + unresolved rows`; until 2026-09-01 `gap_count` published only the mention half, so
/// every seed carrying a dropped resolution under-reported by exactly that drop's size -- and a seed
/// whose drops carried the whole signal would publish `gap_count: 0` under `incomplete: true`, a
/// count saying "nothing" under a flag saying "something". This fixture carries one of each: the
/// `t.take` receiver call is refused by the gate (the drop), and `take`'s own declarations are
/// definition-cause mention rows.
#[test]
fn gap_count_is_the_number_the_boolean_reads_not_the_mention_layers_alone() {
    let (_d, root, db, project_id, bin) = indexed(&[
        (
            "src/lib.rs",
            concat!(
                "pub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
                "pub struct U;\nimpl U { pub fn take(&self) -> u32 { 2 } }\n",
            ),
        ),
        (
            "src/use.rs",
            "use crate::lib::T;\nfn go(t: &T) -> u32 { t.take() }\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "T::take");
    let seed = &cov["seeds"][0];
    let dropped = seed["extracted_but_unresolved"].as_array().unwrap().len();
    assert_eq!(
        dropped, 1,
        "the refused receiver call, and nothing else: {cov}"
    );
    let cause_total: u64 = seed["gap_cause_totals"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert_eq!(
        seed["mention_gap_count"],
        json!(cause_total),
        "the mention layer's own count is the sum of its cause totals: {cov}"
    );
    assert_eq!(
        seed["gap_count"],
        json!(cause_total + dropped as u64),
        "gap_count is the number the boolean reads -- both layers, never the mention half alone: {cov}"
    );
    assert_eq!(seed["enumeration_may_be_incomplete"], json!(true));
}

/// A file the index skipped for being generated is still a file this screen never read.
///
/// `walk_files` honours `.gitignore` as of 2026-09-03, which keeps build output out of the graph --
/// bundled copies of a function were being counted as dependents, at paths that die at the next
/// build. But narrowing what gets indexed must never narrow what gets *disclosed*:
/// `enumeration_may_be_incomplete` has two causes and one of them is "a file this screen never
/// read". If the coverage walk had inherited the same filter, the skipped directory would have
/// vanished from both sides of the subtraction and left `incomplete: false` on an enumeration that
/// declined to enter it -- the exact false clearance this layer exists to prevent.
#[test]
fn a_gitignored_file_is_disclosed_as_unindexed_rather_than_silently_dropped() {
    let (dir, root, db, project_id, bin) =
        indexed(&[("src/d.ts", "export function d() { return 1; }\n")]);
    // `walk_files` reads `.gitignore` through the `ignore` crate, whose `git_ignore` switch only
    // applies inside a git repository -- outside one the file is not consulted at all. Production
    // projects always are one (the staleness checks key on git heads), so a fixture that omits
    // `git init` is asserting against a configuration the product never runs in. Only this test
    // needs it: the shared `indexed` helper stays git-free so the other twenty tests keep exercising
    // the unfiltered walk.
    let init = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["init", "-q"])
        .output()
        .expect("git is on PATH");
    assert!(init.status.success(), "git init: {init:?}");
    fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
    fs::create_dir_all(dir.path().join("generated")).unwrap();
    fs::write(
        dir.path().join("generated/bundle.ts"),
        "import { d } from '../src/d';\nexport function bundled() { return d(); }\n",
    )
    .unwrap();

    // It is not in the graph: that is the point of honouring the ignore file.
    let files: Vec<String> = cort::indexer::walk_files(&root);
    assert!(
        !files.iter().any(|f| f.starts_with("generated/")),
        "the ignored directory reached the index: {files:?}"
    );

    // But the screen says out loud that it never read it.
    let cov = coverage_of(&db, &project_id, &root, &bin, "d");
    let blind = &cov["blind_files"];
    assert!(
        blind["unindexed"].as_u64().unwrap_or(0) >= 1,
        "an ignored source file must still count as unread: {cov}"
    );
    assert!(
        blind["unindexed_example"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("generated/bundle.ts")),
        "the row has to name the file, because the instruction is read the rows: {blind}"
    );
}

/// The suppression half. A reference edge that WAS built must not also be reported as a missing one,
/// or a type seed's screen is pure noise and the boolean is permanently true. This is what
/// `extracted_edges` covering `references` buys: left filtering `calls` only, every resolved type
/// reference would appear in both columns at once.
#[test]
fn a_resolved_type_reference_is_not_also_reported_as_a_mention_gap() {
    let (_dir, root, db, project_id, bin) = indexed(&[(
        "src/lib.rs",
        "pub struct FeedSpec { pub url: String }\npub fn take(s: FeedSpec) -> u8 { 1 }\n",
    )]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "FeedSpec");
    let seed = &cov["seeds"][0];
    let mentions = seed["mentions_without_edge"].as_array().unwrap();
    assert!(
        !mentions.iter().any(|m| m["line"].as_i64() == Some(2)),
        "line 2 has an edge; it is not a gap: {cov}"
    );
    assert_eq!(
        seed["enumeration_may_be_incomplete"].as_bool(),
        Some(false),
        "a clean, fully-resolved type answer says so: {cov}"
    );
}

/// The disclosure half. A reference the extractor SAW but resolution could not place must be named.
/// The fixture uses a qualified path to a module the project does not contain: the suffix resolver
/// finds no chunk whose module path ends in `nowhere`, so the edge is dropped -- unlike a bare
/// multi-candidate name, which attaches as AMBIGUOUS rather than being dropped (`graph.rs`, the
/// comment above `resolve_edge_targets`). Getting that distinction wrong is how the first draft of
/// this test asserted a path it could never reach.
#[test]
fn a_dropped_type_reference_is_reported_as_an_unresolved_extraction() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/real.rs", "pub struct Widget { pub x: u8 }\n"),
        (
            "src/user.rs",
            "pub fn take(w: nowhere::Widget) -> u8 { 1 }\n",
        ),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "Widget");
    let seed = &cov["seeds"][0];
    let unresolved = seed["extracted_but_unresolved"].as_array().unwrap();
    assert!(
        unresolved
            .iter()
            .any(|r| r["raw_target"].as_str() == Some("nowhere::Widget")
                && r["file_path"].as_str() == Some("src/user.rs")),
        "the dropped reference is named, not silently absent: {cov}"
    );
    assert_eq!(
        seed["enumeration_may_be_incomplete"].as_bool(),
        Some(true),
        "a named gap flips the boolean: {cov}"
    );
}
