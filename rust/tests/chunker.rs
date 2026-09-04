//! C1-6..C1-19 — the Rust port kept the case ids (audit F-12).

use cort::ast_grep::resolve_ast_grep_bin;
use cort::chunker::{
    chunk_id_for, edge_string, extract_file, file_content_hash, parse_edge_tag, parse_scan_stream,
    CallForm, Chunk, Edge, ExtractFileArgs, EDGE_REL_TYPES,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake_ag() -> PathBuf {
    // Built by cargo as part of this package (see Cargo.toml [[bin]] fake_ast_grep).
    PathBuf::from(env!("CARGO_BIN_EXE_fake_ast_grep"))
}

fn with_vars(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
    // No env_guard here, exactly like `with_var` above: the tests that call this already hold
    // ENV_LOCK for their whole body, and std Mutex is not reentrant -- taking it here is a
    // guaranteed self-deadlock.
    let prev: Vec<(String, Option<String>)> = pairs
        .iter()
        .map(|(k, _)| ((*k).to_string(), std::env::var(k).ok()))
        .collect();
    unsafe {
        for (k, val) in pairs {
            match val {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        for (k, prev_v) in &prev {
            match prev_v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

fn with_var(key: &str, val: Option<&str>, f: impl FnOnce()) {
    let prev = std::env::var(key).ok();
    // SAFETY: tests in this file take ENV_LOCK so no other thread mutates env.
    unsafe {
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    unsafe {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

fn tmp_file(name: &str, body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&p, body).unwrap();
    (dir, p)
}

const TS: &str = concat!(
    "import { helper } from './helper';\n",
    "export function alpha(a: number) { return helper(a) + 1; }\n",
    "export class Beta {\n",
    "  go() { return alpha(2); }\n",
    "}",
);

fn extract_real(abs: &Path, file_path: &str, source: &str) -> cort::chunker::ExtractResult {
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    extract_file(ExtractFileArgs {
        bin: &bin,
        project_id: "p",
        file_path,
        abs_path: abs.to_str().unwrap(),
        source,
        timeout_ms: None,
    })
    .expect("extract")
}

fn chunk(start_line: i64, content: &str) -> Chunk {
    Chunk {
        chunk_id: String::new(),
        project_id: String::new(),
        file_path: String::new(),
        symbol_name: None,
        chunk_type: String::new(),
        start_line,
        end_line: start_line,
        content: content.to_string(),
        content_hash: String::new(),
        language: None,
        chunk_source: String::new(),
    }
}

fn edge(rel_type: &str, source_symbol: Option<&str>, raw_target: &str) -> Edge {
    Edge {
        rel_type: rel_type.to_string(),
        call_form: CallForm::Bare,
        source_symbol: source_symbol.map(|s| s.to_string()),
        raw_target: raw_target.to_string(),
        start_line: 0,
    }
}

/// C1-6
#[test]
fn malformed_lines_are_skipped_and_counted_valid_ones_survive() {
    let r = parse_scan_stream("{\"a\":1}\nnot json\n{\"b\":2}\n");
    assert_eq!(r.total, 3);
    assert_eq!(r.malformed, 1);
    assert_eq!(
        r.records,
        vec![serde_json::json!({"a": 1}), serde_json::json!({"b": 2})]
    );
}

/// C1-7
#[test]
fn edge_strings_use_the_tab_separated_pre_resolution_form() {
    assert_eq!(
        edge_string(&edge("calls", Some("go"), "alpha")),
        "calls\tgo\talpha"
    );
    assert_eq!(
        edge_string(&edge("imports", None, "./helper")),
        "imports\t\t./helper"
    );
}

/// C1-8
#[test]
fn file_content_hash_covers_both_chunk_contents_and_edge_strings() {
    let chunks = vec![chunk(5, "B"), chunk(1, "A")];
    let edges = vec![edge("calls", Some("x"), "z")];
    let base = file_content_hash(&chunks, &edges);
    let reversed = vec![chunk(1, "A"), chunk(5, "B")];
    assert_eq!(
        base,
        file_content_hash(&reversed, &edges),
        "chunk order must not matter"
    );
    let edge_changed = file_content_hash(&chunks, &[edge("calls", Some("x"), "w")]);
    assert_ne!(base, edge_changed, "an edge-only change must move the hash");
    let chunk_changed = file_content_hash(&[chunk(1, "A2"), chunk(5, "B")], &edges);
    assert_ne!(
        base, chunk_changed,
        "a chunk-only change must move the hash"
    );
}

/// C1-9
#[test]
fn extract_file_produces_1_indexed_lines_and_v6_shaped_chunk_ids() {
    let (_dir, abs) = tmp_file("k.ts", TS);
    let out = extract_real(&abs, "k.ts", TS);
    assert!(!out.unparsed);
    let alpha = out
        .chunks
        .iter()
        .find(|c| c.symbol_name.as_deref() == Some("alpha"))
        .expect("alpha");
    assert_eq!(
        alpha.start_line, 2,
        "ast-grep reports line 1 (0-indexed); we store 2"
    );
    assert_eq!(alpha.chunk_id, chunk_id_for("p", "k.ts", 2));
    assert_eq!(alpha.chunk_id, "p:k.ts:2");
    assert_eq!(alpha.chunk_type, "function");
    assert_eq!(alpha.chunk_source, "ast");
    assert_eq!(alpha.language.as_deref(), Some("TypeScript"));
    assert!(out.chunks.iter().all(|c| c.start_line >= 1));
}

/// C1-10
#[test]
fn rust_functions_and_impl_methods_are_symbol_scoped_ast_chunks() {
    let body = [
        "fn alpha(x: i32) -> i32 {",
        "    x + 1",
        "}",
        "",
        "struct Worker;",
        "impl Worker {",
        "    pub async fn work(&self) -> i32 {",
        "        alpha(1)",
        "    }",
        "}",
        "",
    ]
    .join("\n");
    let (_dir, abs) = tmp_file("main.rs", &body);
    let out = extract_real(&abs, "main.rs", &body);
    assert!(!out.unparsed);
    let alpha = out
        .chunks
        .iter()
        .find(|c| c.symbol_name.as_deref() == Some("alpha"))
        .expect("alpha");
    let work = out
        .chunks
        .iter()
        .find(|c| c.symbol_name.as_deref() == Some("Worker::work"))
        .expect("Worker::work");
    assert_eq!(alpha.start_line, 1);
    assert_eq!(alpha.end_line, 3);
    assert_eq!(alpha.language.as_deref(), Some("Rust"));
    assert_eq!(alpha.content, "fn alpha(x: i32) -> i32 {\n    x + 1\n}");
    assert_eq!(work.start_line, 7);
    assert_eq!(work.end_line, 9);
    assert!(work.content.contains("async fn work"));
    assert!(out.chunks.iter().all(|c| c.chunk_source == "ast"));
}

/// C1-11
#[test]
fn edges_are_attributed_to_the_innermost_containing_chunk() {
    let (_dir, abs) = tmp_file("k.ts", TS);
    let out = extract_real(&abs, "k.ts", TS);
    let imp = out
        .edges
        .iter()
        .find(|e| e.rel_type == "imports")
        .expect("import");
    assert_eq!(imp.source_symbol, None);
    assert_eq!(
        imp.raw_target, "./helper",
        "quotes are stripped from the module specifier"
    );
    let call_in_go = out
        .edges
        .iter()
        .find(|e| e.rel_type == "calls" && e.raw_target == "alpha")
        .expect("alpha call");
    assert_eq!(call_in_go.source_symbol.as_deref(), Some("go"));
    let call_in_alpha = out
        .edges
        .iter()
        .find(|e| e.rel_type == "calls" && e.raw_target == "helper")
        .expect("helper call");
    assert_eq!(call_in_alpha.source_symbol.as_deref(), Some("alpha"));
}

/// C1-12
#[test]
fn a_file_ast_grep_cannot_parse_becomes_a_single_unparsed_fts_only_chunk() {
    let body = "function (((\n";
    let (_dir, abs) = tmp_file("broken.ts", body);
    let out = extract_real(&abs, "broken.ts", body);
    assert!(out.unparsed);
    assert_eq!(out.chunks.len(), 1);
    assert_eq!(out.chunks[0].chunk_source, "unparsed");
    assert_eq!(out.chunks[0].chunk_type, "unparsed");
    assert_eq!(out.chunks[0].symbol_name, None);
    assert_eq!(out.chunks[0].start_line, 1);
    assert_eq!(out.chunks[0].content, body);
    assert!(out.edges.is_empty());
    assert_eq!(out.file_content_hash.len(), 64);
}

/// C1-13
#[test]
fn an_all_malformed_scan_stream_degrades_that_file_to_unparsed_and_never_throws() {
    let _g = env_guard();
    let body = "export function ok() {}\n";
    let (_dir, abs) = tmp_file("m.ts", body);
    with_vars(
        &[
            ("CORT_SCAN_BACKEND", Some("cli")),
            (
                "FAKE_AG_MODE",
                Some(&format!(
                    "emit:{}",
                    base64_encode(b"garbage\nalso garbage\n")
                )),
            ),
        ],
        || {
            let out = extract_file(ExtractFileArgs {
                bin: fake_ag().to_str().unwrap(),
                project_id: "p",
                file_path: "m.ts",
                abs_path: abs.to_str().unwrap(),
                source: body,
                timeout_ms: None,
            })
            .expect("must not throw");
            assert!(out.unparsed);
            assert_eq!(out.malformed, 2);
            assert_eq!(out.chunks[0].chunk_source, "unparsed");
        },
    );
}

/// C1-14
#[test]
fn a_90_percent_malformed_scan_stream_still_indexes_the_surviving_record_scan_never_aborts() {
    let _g = env_guard();
    let body = "export function ok() {}\n";
    let (_dir, abs) = tmp_file("p.ts", body);
    let good = serde_json::json!({
        "text": "export function ok() {}",
        "message": "chunk:function",
        "language": "TypeScript",
        "range": { "start": { "line": 0, "column": 0 }, "end": { "line": 0, "column": 23 } },
        "metaVariables": { "single": { "NAME": { "text": "ok" } } },
    });
    let stream = format!("{}{}\n", "junk\n".repeat(19), good);
    with_vars(
        &[
            ("CORT_SCAN_BACKEND", Some("cli")),
            (
                "FAKE_AG_MODE",
                Some(&format!("emit:{}", base64_encode(stream.as_bytes()))),
            ),
        ],
        || {
            let out = extract_file(ExtractFileArgs {
                bin: fake_ag().to_str().unwrap(),
                project_id: "p",
                file_path: "p.ts",
                abs_path: abs.to_str().unwrap(),
                source: body,
                timeout_ms: None,
            })
            .expect("must not throw");
            assert!(
                !out.unparsed,
                "95% malformed must NOT abort the index — that rule is run-only"
            );
            assert_eq!(out.malformed, 19);
            assert_eq!(out.chunks.len(), 1);
            assert_eq!(out.chunks[0].symbol_name.as_deref(), Some("ok"));
        },
    );
}

/// C1-15
#[test]
fn a_scan_that_times_out_degrades_that_file_to_unparsed_instead_of_aborting() {
    let _g = env_guard();
    let body = "export function big() {}\n".repeat(500);
    let (_dir, abs) = tmp_file("huge.ts", &body);
    with_vars(
        &[
            ("CORT_SCAN_BACKEND", Some("cli")),
            ("FAKE_AG_MODE", Some("hang")),
        ],
        || {
            let out = extract_file(ExtractFileArgs {
                bin: fake_ag().to_str().unwrap(),
                project_id: "p",
                file_path: "huge.ts",
                abs_path: abs.to_str().unwrap(),
                source: &body,
                timeout_ms: Some(200),
            })
            .expect("timeout must degrade, not throw");
            assert!(
                out.unparsed,
                "a timed-out scan must degrade, never abort the index"
            );
            assert_eq!(out.chunks.len(), 1);
            assert_eq!(out.chunks[0].chunk_source, "unparsed");
            assert_eq!(out.chunks[0].content, body);
            assert!(out.edges.is_empty());
            assert!(
                out.file_content_hash.len() == 64
                    && out
                        .file_content_hash
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
        },
    );
}

/// C1-16
#[test]
fn a_spawn_failure_still_propagates_only_timeout_degrades_to_unparsed() {
    // CLI-backend property: the crate backend never spawns, so a bad binary cannot fail it --
    // which is the point of the direct wiring. Pinned here so the loud failure survives on the
    // path that still has a subprocess.
    with_var("CORT_SCAN_BACKEND", Some("cli"), || {
        let body = "export function x() {}\n";
        let (_dir, abs) = tmp_file("x.ts", body);
        let err = extract_file(ExtractFileArgs {
            bin: "/nonexistent/ast-grep-binary",
            project_id: "p",
            file_path: "x.ts",
            abs_path: abs.to_str().unwrap(),
            source: body,
            timeout_ms: None,
        })
        .expect_err(
            "environment-wide failures must stay loud; per-file timeouts are the only silent degrade",
        );
        assert_eq!(err.code, "ast_grep_spawn_failed");
    });
}

const CONST_FN: &str = concat!(
    "import { helper } from './helper';\n",
    "export const alpha = (a: number) => helper(a) + 1;\n",
    "const beta = function () { return helper(2); };\n",
    "const gamma = helper;                       // not a function value\n",
    "const rows = [1, 2, 3].map((n) => helper(n)); // data, not a named function\n",
    "export const handler = createHandler(\"x\", async (req: Request) => { return helper(1); });\n",
);

fn const_fn_chunks() -> (tempfile::TempDir, cort::chunker::ExtractResult) {
    let (dir, abs) = tmp_file("cf.ts", CONST_FN);
    let out = extract_real(&abs, "cf.ts", CONST_FN);
    (dir, out)
}

/// C1-17
#[test]
fn const_bound_arrow_and_function_expressions_become_function_chunks() {
    let (_dir, out) = const_fn_chunks();
    assert!(!out.unparsed);
    let mut names: Vec<String> = out
        .chunks
        .iter()
        .filter_map(|c| c.symbol_name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["alpha", "beta", "handler"]);
    for name in ["alpha", "beta", "handler"] {
        let c = out
            .chunks
            .iter()
            .find(|x| x.symbol_name.as_deref() == Some(name))
            .unwrap();
        assert_eq!(c.chunk_type, "function");
        assert_eq!(c.chunk_source, "ast");
    }
}

/// C1-18
#[test]
fn collection_transforms_and_bare_aliases_do_not_become_chunks() {
    let (_dir, out) = const_fn_chunks();
    let names: Vec<Option<&str>> = out
        .chunks
        .iter()
        .map(|c| c.symbol_name.as_deref())
        .collect();
    assert!(
        !names.contains(&Some("rows")),
        "x.map(n => ...) must not make `rows` a symbol"
    );
    assert!(
        !names.contains(&Some("gamma")),
        "an alias to a function must not make `gamma` a symbol"
    );
}

/// C1-19
#[test]
fn calls_inside_a_const_bound_handler_get_the_handler_as_their_source_symbol() {
    let (_dir, out) = const_fn_chunks();
    let inside: Vec<&Edge> = out
        .edges
        .iter()
        .filter(|e| e.source_symbol.as_deref() == Some("handler") && e.rel_type == "calls")
        .collect();
    assert!(
        inside.iter().any(|e| e.raw_target == "helper"),
        "handler body must resolve to its caller chunk"
    );
}

/// Standard base64 (same alphabet as JS Buffer.toString('base64') / Python b64encode).
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        let n = if i + 2 < bytes.len() {
            3
        } else if i + 1 < bytes.len() {
            2
        } else {
            1
        };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if n > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if n > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

// ---------------------------------------------------------------------------
// schema v4: the call form travels in the rule message, and a call edge is
// pinned to the line that names the callee.
// ---------------------------------------------------------------------------

fn extract_emitted(body: &str, stream: &str) -> cort::chunker::ExtractResult {
    let _g = env_guard();
    let (_dir, abs) = tmp_file("emit.rs", body);
    let mut out: Option<cort::chunker::ExtractResult> = None;
    with_vars(
        &[
            ("CORT_SCAN_BACKEND", Some("cli")),
            (
                "FAKE_AG_MODE",
                Some(&format!("emit:{}", base64_encode(stream.as_bytes()))),
            ),
        ],
        || {
            out = Some(
                extract_file(ExtractFileArgs {
                    bin: fake_ag().to_str().unwrap(),
                    project_id: "p",
                    file_path: "emit.rs",
                    abs_path: abs.to_str().unwrap(),
                    source: body,
                    timeout_ms: None,
                })
                .expect("a scan stream must never throw"),
            );
        },
    );
    out.expect("the fake ran")
}

/// One `edge:` record whose matched node starts on line 10 and whose `$CALLEE` sits on line 11
/// (ast-grep's lines are 0-based, so the stored numbers are 11 and 12).
fn edge_record(message: &str, callee: &str, node_line: u64, name_line: u64) -> String {
    serde_json::json!({
        "text": "builder\n    .foo()",
        "message": message,
        "language": "Rust",
        "range": {
            "start": { "line": node_line, "column": 0 },
            "end": { "line": name_line, "column": 12 }
        },
        "metaVariables": {
            "single": {
                "CALLEE": {
                    "text": callee,
                    "range": {
                        "start": { "line": name_line, "column": 6 },
                        "end": { "line": name_line, "column": 9 }
                    }
                }
            }
        },
    })
    .to_string()
}

#[test]
fn an_edge_tag_names_its_rel_type_and_defaults_the_form_to_bare() {
    use cort::chunker::parse_edge_tag;
    for (raw, rel, form) in [
        ("calls", "calls", CallForm::Bare),
        ("imports", "imports", CallForm::Bare),
        ("exports", "exports", CallForm::Bare),
        ("calls:receiver", "calls", CallForm::Receiver),
        ("calls:scoped", "calls", CallForm::Scoped),
    ] {
        assert_eq!(parse_edge_tag(raw), Some((rel.to_string(), form)), "{raw}");
    }
    // Forms are stable strings, because they are stored in a CHECK-constrained column and printed
    // in `lean` output where an agent reads them without a schema in front of it.
    assert_eq!(CallForm::Receiver.as_str(), "receiver");
    assert_eq!(CallForm::default(), CallForm::Bare);
}

#[test]
fn an_edge_tag_this_build_cannot_read_drops_the_edge_instead_of_guessing_a_form() {
    use cort::chunker::parse_edge_tag;
    for raw in [
        "calls:reciever",
        "calls:",
        "calls:receiver:extra",
        "bogus",
        "exports:bogus",
        "",
    ] {
        assert_eq!(parse_edge_tag(raw), None, "{raw} must not be guessed");
    }
}

#[test]
fn a_mislabelled_edge_record_is_counted_as_malformed_and_yields_no_edge() {
    // A typo in a pack rule must cost one reported gap, not one whole project index: the schema
    // CHECK would have rejected the insert mid-transaction and taken every chunk with it.
    let out = extract_emitted(
        "x\n    .foo()\n",
        &edge_record("edge:calls:reciever", "foo", 10, 11),
    );
    assert!(
        !out.unparsed,
        "the surviving shape of the file is still fine"
    );
    assert!(out.edges.is_empty(), "{:?}", out.edges);
    assert_eq!(out.malformed, 1);
}

#[test]
fn a_call_edge_is_pinned_to_the_line_that_names_the_callee() {
    let out = extract_emitted(
        "x\n    .foo()\n",
        &edge_record("edge:calls:receiver", "foo", 10, 11),
    );
    assert_eq!(out.malformed, 0);
    assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
    let edge = &out.edges[0];
    assert_eq!(edge.rel_type, "calls");
    assert_eq!(edge.call_form, CallForm::Receiver);
    assert_eq!(edge.raw_target, "foo");
    assert_eq!(
        edge.start_line, 12,
        "the name's own line, not the first line of the matched node"
    );
}

#[test]
fn an_import_edge_keeps_the_matched_node_line_because_it_has_no_callee() {
    let record = serde_json::json!({
        "text": "use crate::lib::T;",
        "message": "edge:imports",
        "language": "Rust",
        "range": {
            "start": { "line": 0, "column": 0 },
            "end": { "line": 0, "column": 18 }
        },
        "metaVariables": { "single": { "SRC": { "text": "crate::lib::T" } } },
    })
    .to_string();
    let out = extract_emitted("use crate::lib::T;\n", &record);
    assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
    assert_eq!(out.edges[0].start_line, 1);
    assert_eq!(out.edges[0].call_form, CallForm::Bare);
    assert_eq!(out.edges[0].raw_target, "crate::lib::T");
}

/// The pack side of v4: three Rust call shapes, three forms. This is the only test that reads the
/// real grammar, because `method_call_expression` does not exist in the Rust grammar ast-grep
/// 0.45.2 ships -- a rule written against it matches nothing and looks exactly like a file with no
/// method calls in it.
#[test]
fn the_rust_pack_tags_each_call_shape_with_the_form_it_is() {
    let body = concat!(
        "pub struct T;\n",
        "impl T { pub fn take(&self) -> u32 { 1 } }\n",
        "pub fn free() -> u32 { 2 }\n",
        "pub fn go(t: &T) -> u32 { let _ = crate::free(); t.take() + T::take(t) }\n",
    );
    let (_dir, abs) = tmp_file("forms.rs", body);
    let out = extract_real(&abs, "forms.rs", body);
    let mut got: Vec<(String, String)> = out
        .edges
        .iter()
        .map(|e| (e.call_form.as_str().to_string(), e.raw_target.clone()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            // The head, receiver included: `graph::receiver_binds` needs it, and it is what makes
            // the edge checkable at a glance instead of at a guess.
            ("receiver".to_string(), "t.take".to_string()),
            ("scoped".to_string(), "T::take".to_string()),
            ("scoped".to_string(), "crate::free".to_string()),
        ],
        "{out:?}"
    );
    let receiver = out
        .edges
        .iter()
        .find(|e| e.call_form == CallForm::Receiver)
        .expect("t.take()");
    assert_eq!(
        receiver.source_symbol.as_deref(),
        Some("go"),
        "the receiver call must still be attributed to its enclosing function"
    );
    assert_eq!(receiver.start_line, 4);
}

/// `type` is a fourth call form, not a fourth rel type in disguise. It ranks last because
/// `insertion_rank` decides which row survives a duplicate key, and a call is a stronger claim about
/// a line than a type mention on the same line.
#[test]
fn a_type_reference_parses_as_its_own_form_and_rel_type() {
    assert_eq!(CallForm::Type.as_str(), "type");
    assert_eq!(CallForm::parse("type"), Some(CallForm::Type));
    assert_eq!(CallForm::Type.insertion_rank(), 3);
    assert!(CallForm::Type.insertion_rank() > CallForm::Bare.insertion_rank());

    assert!(EDGE_REL_TYPES.contains(&"references"));
    assert_eq!(
        parse_edge_tag("references:type"),
        Some(("references".to_string(), CallForm::Type)),
        "the pack rule's own message is the only channel that can supply the form"
    );
}

/// `chunk_id` is project:file:start_line with no chunk type (`chunker::chunk_id_for`), so a type
/// declared on the same line as one of its own methods collides. Which one survived used to depend
/// on the order ast-grep emitted records in, i.e. on a directory listing. The loss is accepted, but
/// it must be the same loss every time: the method is the chunk `impact` can hold a seed for, so it
/// wins.
#[test]
fn a_type_sharing_a_line_with_its_method_loses_deterministically() {
    let source = "pub trait T { fn f(&self) {} }\n";
    let (_dir, abs) = tmp_file("t.rs", source);
    let r = extract_real(&abs, "t.rs", source);
    let kinds: Vec<&str> = r.chunks.iter().map(|c| c.chunk_type.as_str()).collect();
    assert_eq!(
        kinds,
        ["method"],
        "the method survives the id collision, every time: {kinds:?}"
    );
}
