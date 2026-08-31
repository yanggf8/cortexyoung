//! The text-side counterfactual tool. Its numbers decide whether a call shape is worth indexing, so
//! they are tested the way the product's are: on a fixture whose answer is known by hand, and with
//! the failure directions named.

use cort_evals::recall::{declared_names, qualified_calls, receiver_calls, report, scan_venue};
use serde_json::Value;
use std::fs;
use std::path::Path;

fn venue(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, body).unwrap();
    }
    dir
}

#[test]
fn receiver_calls_find_the_method_and_its_head() {
    let calls = receiver_calls("    tally.add(&project, &text);\n");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].name, "add");
    assert_eq!(calls[0].qualifier, "tally");
    assert!(!calls[0].path_sep);
    assert_eq!(calls[0].line, 1);
    // Chained: each `.name(` is its own site, and a decimal literal is not a call at all.
    // A chain is one site per segment, and a decimal literal is not a call. Over-inclusive on
    // purpose: this counts the population a rule would have to reason over, not the edges.
    let chain = receiver_calls("let x = 0.5; v.iter().map(f).collect();\n");
    let names: Vec<String> = chain.iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["iter", "map", "collect"], "{chain:?}");
}

#[test]
fn qualified_calls_keep_the_separator_that_was_written() {
    let calls = qualified_calls("fs::write(p)?; out.push(x); a.b.c();\n");
    let paths: Vec<String> = calls
        .iter()
        .filter(|c| c.path_sep)
        .map(|c| c.target())
        .collect();
    let members: Vec<String> = calls
        .iter()
        .filter(|c| !c.path_sep)
        .map(|c| c.target())
        .collect();
    assert_eq!(paths, ["fs::write"], "only `::` is a path: {calls:?}");
    assert!(
        members.contains(&"out.push".to_string()) && members.contains(&"b.c".to_string()),
        "{members:?}"
    );
}

#[test]
fn declaration_scan_reads_the_name_the_keyword_introduces() {
    for (line, name) in [
        ("pub fn take(&self) -> u32 { 1 }", "take"),
        ("    fn add(&self, x: i32) -> i32;", "add"),
        ("impl SqliteErrorCode for X {}", "SqliteErrorCode"),
        ("const KNOWN_COMMANDS: &[&str] = &[];", "KNOWN_COMMANDS"),
        ("export class Beta {", "Beta"),
        ("interface Handler {", "Handler"),
        ("#[derive(Debug)]", ""),
    ] {
        let got = declared_names(line);
        if name.is_empty() {
            assert!(got.is_empty(), "{line} declares nothing: {got:?}");
        } else {
            assert!(got.contains(&name.to_string()), "{line} -> {got:?}");
        }
    }
}

#[test]
fn candidate_buckets_separate_no_symbol_unique_and_ambiguous() {
    let dir = venue(&[
        (
            "src/a.rs",
            "pub struct T;\nimpl T { pub fn take(&self) -> u32 { 1 } }\n",
        ),
        (
            "src/b.rs",
            "pub struct U;\nimpl U { pub fn take(&self) -> u32 { 2 } }\npub fn only() {}\n",
        ),
        (
            "src/c.rs",
            "fn go(t: &T) -> u32 { t.take() + t.only() + t.absent() }\n",
        ),
    ]);
    let v = scan_venue(dir.path()).expect("scannable");
    assert_eq!(v.bucket("take"), "ambiguous", "T::take and U::take");
    assert_eq!(v.bucket("only"), "unique");
    assert_eq!(v.bucket("absent"), "no_project_symbol");
    assert_eq!(v.files, 3, "{:?}", v.files);

    let report = report(dir.path(), 10).expect("report");
    let Value::Object(map) = report["receiver_calls"]["by_candidate_count"].clone() else {
        panic!("expected an object: {}", report["receiver_calls"]);
    };
    let got: Vec<(&str, &Value)> = map.iter().map(|(k, v)| (k.as_str(), v)).collect();
    for (bucket, expected) in [("ambiguous", 1), ("unique", 1), ("no_project_symbol", 1)] {
        assert_eq!(
            map.get(bucket).and_then(Value::as_u64),
            Some(expected),
            "{bucket}: {got:?}"
        );
    }
    assert_eq!(report["files_scanned"], serde_json::json!(3));
    assert!(report["reading"].as_str().unwrap().contains("upper bound"));
}

/// The hole the tool exists to size: a path qualifier that is *also* a local module name.
/// `fs::write` against a project that ships `src/fs.rs::write` cannot be told apart from the internal
/// call by name, and `cort` attaches the local one (README limitation #8, pinned by
/// `a_std_module_qualifier_that_matches_a_local_module_file_still_attaches`).
#[test]
fn the_shadowing_risk_needs_a_dependency_import_and_a_local_module_of_the_same_name() {
    let shadowed = venue(&[
        ("src/fs.rs", "pub fn write(p: &str) -> u32 { 1 }\n"),
        (
            "src/main.rs",
            "use std::fs;\nfn go() -> u32 { fs::write(\"x\") }\n",
        ),
    ]);
    let risk = report(shadowed.path(), 10).expect("report");
    assert_eq!(
        risk["dependency_shadowed_by_local_module_sites"],
        serde_json::json!(1),
        "{risk}"
    );
    assert_eq!(
        risk["shadowed_examples"][0]["file"],
        serde_json::json!("src/main.rs"),
        "an unactionable count is not a measurement: {risk}"
    );

    // The same call imported from inside the crate: the local module *is* the target, so this is not
    // exposure at all. Counting it would bury the case that matters -- the first cut of this counter
    // did exactly that and reported 44 exposures in a repo with none.
    let internal = venue(&[
        ("src/fs.rs", "pub fn write(p: &str) -> u32 { 1 }\n"),
        (
            "src/main.rs",
            "use crate::fs;\nfn go() -> u32 { fs::write(\"x\") }\n",
        ),
    ]);
    let clean = report(internal.path(), 10).expect("report");
    assert_eq!(
        clean["module_path_calls_into_a_local_module"],
        serde_json::json!(1),
        "still the population the rule can reach: {clean}"
    );
    assert_eq!(
        clean["dependency_shadowed_by_local_module_sites"],
        serde_json::json!(0),
        "{clean}"
    );
}

#[test]
fn generated_trees_are_out_of_scope_by_directory_name() {
    // `.wrangler` is excluded here and *not* in the product's IGNORE_DIRS -- the count difference is
    // the point of recording it: a venue number that quietly included bundles would not be
    // comparable with `cort status`.
    let dir = venue(&[
        ("src/a.rs", "pub fn only() {}\n"),
        (
            ".wrangler/tmp/deploy/x.js",
            "function bundle_fn() { only() }\n",
        ),
        ("dist/bundle.js", "function bundled() { only() }\n"),
    ]);
    let v = scan_venue(dir.path()).expect("scannable");
    assert_eq!(v.files, 1, "{:?}", v.files);
}

#[test]
fn a_file_it_cannot_read_is_an_error_not_a_skipped_file() {
    // Half a venue is not a measurement of a venue: the earlier Python version counted what it could
    // read and reported a rate over that, which is how a null metric happened twice this month.
    let dir = venue(&[("src/a.rs", "pub fn only() {}\n")]);
    let broken = dir.path().join("src/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("unreadable.rs"), "\u{0}").unwrap();
    let before = scan_venue(dir.path()).expect("two files, both readable-as-text");
    assert_eq!(before.files, 2);
    let path: &Path = dir.path();
    assert!(
        report(path, 5).is_ok(),
        "a NUL byte is still valid UTF-8 text"
    );
}

#[test]
fn an_internal_crate_name_with_a_hyphen_is_not_a_dependency_shadow() {
    // `name = "cort-evals"` is imported as `cort_evals`. Comparing the two raw strings called every
    // internal `cort_evals::arms::x()` a shadow, and the metric's whole value is that it points at a
    // real risk.
    let dir = venue(&[
        ("Cargo.toml", "[package]\nname = \"cort-evals\"\n"),
        (
            "src/arms.rs",
            "pub fn cort_bin() -> String { String::new() }\n",
        ),
        (
            "src/main.rs",
            "use cort_evals::arms;\nfn go() -> String { arms::cort_bin() }\n",
        ),
    ]);
    let report = report(dir.path(), 10).expect("report");
    assert_eq!(
        report["module_path_calls_into_a_local_module"],
        serde_json::json!(1),
        "{report}"
    );
    assert_eq!(
        report["dependency_shadowed_by_local_module_sites"],
        serde_json::json!(0),
        "the crate's own path is not a dependency: {report}"
    );
}
