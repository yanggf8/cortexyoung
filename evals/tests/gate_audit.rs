//! Item B's acceptance tests: the classes are the gate's own ordered refusal reasons, they are
//! disjoint and sum to the refused count, the shape-refused subset is reported beside its class
//! and never merged into it, and every example carries the `file:line` a reader needs. The
//! decision under test is `cort::graph`'s — these tests feed it targets and check the census
//! folds what the gate says, never what a copy of it would say.

use cort::db;
use cort::graph::ReceiverIndex;
use cort_evals::gate_audit::{
    census, classify, markdown, report_from, write_markdown, EdgeRow, Verdict,
};
use rusqlite::params;
use std::sync::Mutex;

/// An in-memory index whose chunks declare exactly `symbols` — `T::method` for owned methods,
/// a bare name for a free function — which is all `ReceiverIndex` keys on.
fn index_with(symbols: &[&str]) -> ReceiverIndex {
    let db = db::open_db(":memory:").unwrap();
    db::ensure_schema(&db).unwrap();
    db.execute(
        "INSERT INTO projects (project_id, name, path, extractor_version, git_head)
         VALUES ('p', 'test', '/test', 'test-v1', 'abc1234')",
        [],
    )
    .unwrap();
    for (i, symbol) in symbols.iter().enumerate() {
        db.execute(
            "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
             start_line, end_line, content, content_hash, chunk_source)
             VALUES (?1, 'p', ?2, ?3, 'method', 1, 2, 'fn x() {}', 'hash', 'ast')",
            params![format!("c{i}"), format!("src/f{i}.rs"), symbol],
        )
        .unwrap();
    }
    ReceiverIndex::build(&db, "p").unwrap()
}

fn edge(file: &str, line: i64, target: &str, source: &str) -> EdgeRow {
    EdgeRow {
        file_path: file.to_string(),
        source_symbol: source.to_string(),
        raw_target: target.to_string(),
        start_line: line,
    }
}

#[test]
fn the_classes_are_the_gate_s_own_ordered_refusal_reasons() {
    let index = index_with(&["Store::add", "Tally::add", "log", "Store::refresh"]);
    assert_eq!(
        classify(&index, "x.missing", Some("Store::add")),
        Verdict::ZeroCandidates
    );
    assert_eq!(
        classify(&index, "t.add", Some("Store::add")),
        Verdict::MultipleCandidates
    );
    // A free function is a candidate with no owner: with receiver shape present, the gate's
    // next refusal reason is ownership, and the census must not call it anything else.
    assert_eq!(
        classify(&index, "p.log", Some("Store::add")),
        Verdict::OneOwnerless
    );
    // The receiver that *is* the owner's name binds — the normal attach path.
    assert_eq!(
        classify(&index, "store.refresh", Some("Store::add")),
        Verdict::Attached
    );
    assert_eq!(
        classify(&index, "self.refresh", Some("Store::refresh")),
        Verdict::Attached
    );
    // A one-letter receiver cannot be the owner's name under the gate's length rule.
    assert_eq!(
        classify(&index, "w.refresh", Some("Store::add")),
        Verdict::BindingRefused {
            no_receiver_shape: false
        }
    );
}

#[test]
fn shape_is_attributed_before_ownership_just_as_the_gate_applies_it() {
    // Dotless targets exercise the ordering: the gate refuses on shape before it ever asks who
    // owns the candidate, so a dotless *ownerless* candidate is a shape refusal, not an
    // ownership one — the adversarial case the first draft of this module got wrong.
    let index = index_with(&["Store::frobnicate", "log"]);
    assert_eq!(
        classify(&index, "frobnicate", Some("Store::frobnicate")),
        Verdict::BindingRefused {
            no_receiver_shape: true
        }
    );
    assert_eq!(
        classify(&index, "log", Some("Store::frobnicate")),
        Verdict::BindingRefused {
            no_receiver_shape: true
        }
    );
    // With shape present, the same free function is refused for ownership instead.
    assert_eq!(
        classify(&index, "p.log", Some("Store::frobnicate")),
        Verdict::OneOwnerless
    );
}

#[test]
fn classes_are_disjoint_and_cover_the_input_with_examples_capped() {
    let index = index_with(&[
        "Store::add",
        "Tally::add",
        "log",
        "Store::refresh",
        "Store::frobnicate",
    ]);
    let edges = vec![
        edge("src/a.rs", 1, "store.refresh", "Store::refresh"),
        edge("src/a.rs", 2, "t.add", "Other::x"),
        edge("src/b.rs", 3, "p.log", "Other::x"),
        edge("src/b.rs", 4, "x.missing", "Other::x"),
        edge("src/c.rs", 5, "w.refresh", "Other::x"),
        edge("src/c.rs", 6, "frobnicate", "Other::x"),
        edge("src/c.rs", 7, "log", "Other::x"),
    ];
    let c = census(&index, &edges, 2);
    assert_eq!(c.population, 7);
    assert_eq!(c.attached, 1);
    assert_eq!(c.zero_candidates, 1);
    assert_eq!(c.multiple_candidates, 1);
    assert_eq!(c.one_ownerless, 1);
    assert_eq!(c.binding_refused, 3);
    // `population` comes from the input row count, so this sum is a coverage claim: a bucket
    // that silently dropped rows would fail it.
    assert_eq!(
        c.attached
            + c.zero_candidates
            + c.multiple_candidates
            + c.one_ownerless
            + c.binding_refused,
        c.population
    );
    // Both shape-refused rows sit inside the class, counted beside it — inflating it silently
    // would corrupt the one number the whole module exists to read.
    assert_eq!(c.binding_refused_no_shape, 2);
    assert_eq!(c.binding_refused_examples.len(), 2);
    assert_eq!(c.binding_refused_examples[0].file, "src/c.rs");
    assert_eq!(c.binding_refused_examples[0].line, 5);
    assert_eq!(c.binding_refused_examples[0].target, "w.refresh");
}

#[test]
fn the_report_partitions_the_population_and_stamps_both_heads() {
    let db = db::open_db(":memory:").unwrap();
    db::ensure_schema(&db).unwrap();
    db.execute(
        "INSERT INTO projects (project_id, name, path, extractor_version, git_head)
         VALUES ('p', 't', '/t', 'test-v1', '83b66a9fa971474b0f1b0f3ef1f441e2782160ed')",
        [],
    )
    .unwrap();
    for (i, symbol) in ["Store::add", "Tally::add", "Store::refresh"]
        .iter()
        .enumerate()
    {
        db.execute(
            "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
             start_line, end_line, content, content_hash, chunk_source)
             VALUES (?1, 'p', ?2, ?3, 'method', 1, 2, 'fn x() {}', 'hash', 'ast')",
            params![format!("c{i}"), format!("src/f{i}.rs"), symbol],
        )
        .unwrap();
    }
    let edge_row = |file: &str, line: i64, target: &str, form: &str| {
        db.execute(
            "INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target, rel_type,
             call_form, start_line) VALUES ('p', ?1, 's', ?2, 'calls', ?3, ?4)",
            params![file, target, form, line],
        )
        .unwrap();
    };
    edge_row("src/a.rs", 1, "store.refresh", "receiver");
    edge_row("src/a.rs", 2, "t.add", "receiver");
    edge_row("src/a.rs", 3, "x.missing", "receiver");
    // A bare call with the same name must not enter a receiver census: the form is the filter.
    edge_row("src/a.rs", 4, "add", "bare");

    // The index stores the full 40-char head; the venue side carries the short form, so
    // agreement is a prefix match — the first end-to-end run caught string equality reporting
    // a fresh index as disagreed.
    let v = report_from(&db, "p", "/venue", "83b66a9f", 5).unwrap();
    assert_eq!(v["venue"].as_str(), Some("/venue"));
    assert_eq!(v["population"].as_u64(), Some(3));
    assert_eq!(v["attached"].as_u64(), Some(1));
    assert_eq!(v["refused"].as_u64(), Some(2));
    assert_eq!(
        v["index_head"].as_str(),
        Some("83b66a9fa971474b0f1b0f3ef1f441e2782160ed")
    );
    assert_eq!(v["heads_agree"].as_bool(), Some(true));
    let classes = &v["refused_classes"];
    assert_eq!(classes["zero_candidates"]["count"].as_u64(), Some(1));
    assert_eq!(classes["multiple_candidates"]["count"].as_u64(), Some(1));
    assert_eq!(classes["one_ownerless"]["count"].as_u64(), Some(0));
    assert_eq!(classes["binding_refused"]["count"].as_u64(), Some(0));
    assert_eq!(
        classes["binding_refused"]["no_receiver_shape"].as_u64(),
        Some(0)
    );
    let example = &classes["multiple_candidates"]["examples"][0];
    assert_eq!(example["file"].as_str(), Some("src/a.rs"));
    assert_eq!(example["line"].as_i64(), Some(2));
    assert_eq!(example["target"].as_str(), Some("t.add"));
    // The reading field is part of the contract: without it, a 96%-zero report reads as a
    // recall catastrophe instead of the std frontier it is.
    assert!(v["reading"]
        .as_str()
        .is_some_and(|r| r.contains("static-analysis frontier")));

    // A venue head that is not the index head is never quietly reconciled: the whole point of
    // carrying both is that old counts never wear the current commit's name.
    let stale = report_from(&db, "p", "/venue", "feedc0de", 5).unwrap();
    assert_eq!(stale["heads_agree"].as_bool(), Some(false));
}

#[test]
fn an_unindexed_venue_is_refused_not_indexed() {
    // `cache_dir()` is read per call, so the redirect is both isolated from the developer's real
    // cache and provable: the database this audit refuses to create must still be absent after.
    static CACHE_LOCK: Mutex<()> = Mutex::new(());
    let _guard = CACHE_LOCK.lock().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::env::set_var("CORT_CACHE_DIR", cache.path());
    let venue = tempfile::tempdir().unwrap();
    let root = venue.path().canonicalize().unwrap();
    let db_path = db::db_path_for(root.to_str().unwrap());
    assert!(!db_path.exists());
    let err = cort_evals::gate_audit::report(venue.path().to_str().unwrap(), 5).unwrap_err();
    assert!(err.contains("no index for this directory"), "{err}");
    assert!(
        !db_path.exists(),
        "gate-audit created the database it refused to audit: {db_path:?}"
    );
    std::env::remove_var("CORT_CACHE_DIR");
}

/// Item A's golden input: the same fixture shape `report_from` produces, with the machine stamp
/// the CLI applies before rendering. One measured value feeds both the JSON stdout and the
/// markdown file; this is that value.
fn stamped_report() -> serde_json::Value {
    let db = db::open_db(":memory:").unwrap();
    db::ensure_schema(&db).unwrap();
    db.execute(
        "INSERT INTO projects (project_id, name, path, extractor_version, git_head)
         VALUES ('p', 't', '/t', 'test-v1', '83b66a9fa971474b0f1b0f3ef1f441e2782160ed')",
        [],
    )
    .unwrap();
    for (i, symbol) in ["Store::add", "Tally::add", "Store::refresh"]
        .iter()
        .enumerate()
    {
        db.execute(
            "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
             start_line, end_line, content, content_hash, chunk_source)
             VALUES (?1, 'p', ?2, ?3, 'method', 1, 2, 'fn x() {}', 'hash', 'ast')",
            params![format!("c{i}"), format!("src/f{i}.rs"), symbol],
        )
        .unwrap();
    }
    let edge_row = |file: &str, line: i64, target: &str| {
        db.execute(
            "INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target, rel_type,
             call_form, start_line) VALUES ('p', ?1, 's', ?2, 'calls', 'receiver', ?3)",
            params![file, target, line],
        )
        .unwrap();
    };
    edge_row("src/a.rs", 1, "store.refresh");
    edge_row("src/a.rs", 2, "t.add");
    edge_row("src/a.rs", 3, "x.missing");
    let mut v = report_from(&db, "p", "/venue", "83b66a9f", 5).unwrap();
    v["machine"] = serde_json::json!({"id": "mach-1", "source": "etc-machine-id"});
    v
}

#[test]
fn the_markdown_table_is_a_golden_snapshot_of_the_report() {
    // The rendering is part of the contract: a committed artifact is diffed and quoted, so a
    // shape change must be a visible event, not a silent one. Rendered from the measured value
    // the JSON prints -- never a second computation.
    let golden = "\
# receiver-gate census

method: gate-audit-v1 (index-side census of raw_edges receiver calls, classified by cort::graph's own gate primitives)

reading: zero_candidates counts calls whose method name the project never declares — std, dependencies, iterator adapters; it is the static-analysis frontier, not a recall leak. binding_refused holds the type-directed-dispatch candidates; read its examples.

Refusal examples per class are in the JSON report under `refused_classes`; each carries file:line and is checkable by hand. A number in this table is quotable only with its row's commit and machine.

| venue | venue_head | index_head | heads_agree | population | attached | refused | zero_candidates | multiple_candidates | one_ownerless | binding_refused | no_receiver_shape | machine |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| /venue | 83b66a9f | 83b66a9fa971474b0f1b0f3ef1f441e2782160ed | yes | 3 | 1 | 2 | 1 | 1 | 0 | 0 | 0 | mach-1/etc-machine-id |
";
    assert_eq!(markdown(&stamped_report()), golden);
}

#[test]
fn a_pipe_in_the_venue_stays_inside_its_cell() {
    let mut v = stamped_report();
    v["venue"] = serde_json::json!("/tmp/a|b");
    let out = markdown(&v);
    assert!(
        out.contains("| /tmp/a\\|b |"),
        "the pipe must be escaped, the row unbroken:\n{out}"
    );
}

#[test]
fn the_markdown_file_write_returns_its_failure_and_ends_with_a_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("out.md");
    write_markdown(&stamped_report(), path.to_str().unwrap()).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.ends_with('\n'),
        "a committed artifact ends with a newline"
    );
    assert_eq!(raw, format!("{}\n", markdown(&stamped_report())));

    // Storage failures are returned, never panicked: a report a person can hold must not cost
    // the process a backtrace when the disk says no.
    let err = write_markdown(&stamped_report(), "/nonexistent-dir-for-tests/out.md").unwrap_err();
    assert!(err.contains("/nonexistent-dir-for-tests/out.md"), "{err}");
}
