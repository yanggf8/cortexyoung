//! Tests for the ast-grep test double itself.
//!
//! The double is load-bearing: `hang`, `streams`, `empty`, `emit:` and `preflight-*` are what let
//! the pathological parser paths be tested at all. If it drifts from the real CLI's shapes, those
//! tests start asserting a fiction — so pin the shapes here instead of trusting them by inspection.

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake_ast_grep"))
}

fn run(mode: &str, args: &[&str]) -> Output {
    Command::new(fixture())
        .args(args)
        .env("FAKE_AG_MODE", mode)
        .output()
        .expect("fixture spawns")
}

fn code(out: &Output) -> i32 {
    out.status
        .code()
        .expect("the fixture always exits with a code")
}

#[test]
fn version_mode_reports_the_pinned_version_by_default() {
    let out = run("", &["--version"]);
    assert_eq!(code(&out), 0);
    assert_eq!(out.stdout, b"ast-grep 0.45.2\n");

    let old = run("version:0.44.9", &["--version"]);
    assert_eq!(old.stdout, b"ast-grep 0.44.9\n");
}

#[test]
fn streams_writes_both_pipes_and_exits_nonzero() {
    let out = run("streams", &["run"]);
    assert_eq!(code(&out), 1);
    assert_eq!(out.stdout, b"OUT\n");
    assert_eq!(out.stderr, b"ERR\n");
}

#[test]
fn empty_mode_is_indistinguishable_from_a_real_zero_match() {
    // ast-grep 0.45.2 prints nothing and exits 1 for both "no matches" and "bad pattern", which is
    // why the design requires a pre-flight instead of reading the exit code.
    let out = run("empty", &["run"]);
    assert_eq!(code(&out), 1);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn emit_decodes_base64_byte_for_byte_including_newlines() {
    let hello = run("emit:aGVsbG8=", &["run"]);
    assert_eq!(code(&hello), 0);
    assert_eq!(hello.stdout, b"hello");

    let lines = run("emit:YQpiCg==", &["run"]);
    assert_eq!(lines.stdout, b"a\nb\n");

    let json_stream = run("emit:eyJtZXNzYWdlIjoiY2h1bms6ZnVuY3Rpb24ifQo=", &["scan"]);
    assert_eq!(json_stream.stdout, b"{\"message\":\"chunk:function\"}\n");

    let nothing = run("emit:", &["run"]);
    assert_eq!(code(&nothing), 0);
    assert!(
        nothing.stdout.is_empty(),
        "an empty payload is not an error"
    );
}

#[test]
fn a_malformed_emit_payload_fails_loudly_instead_of_emitting_nothing() {
    // Trailing bits that are not a whole character: a silent empty stdout here would make a
    // malformed-stream test pass for the wrong reason.
    let out = run("emit:YQ===", &["run"]);
    assert_eq!(code(&out), 2);
    assert!(out.stdout.is_empty());
}

#[test]
fn preflight_modes_separate_a_bad_pattern_from_a_good_one() {
    let bad = run("preflight-bad", &["run", "--debug-query=ast"]);
    assert_eq!(code(&bad), 0);
    let bad_stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(bad_stderr.contains("Debug AST:"));
    assert!(bad_stderr.contains("Pattern contains an ERROR node"));

    let good = run("preflight-ok", &["run", "--debug-query=ast"]);
    assert_eq!(code(&good), 0);
    let good_stderr = String::from_utf8_lossy(&good.stderr);
    assert!(good_stderr.contains("Debug AST:"));
    assert!(!good_stderr.contains("ERROR node"));
}

#[test]
fn an_unknown_mode_is_a_quiet_success_like_a_real_no_match_run() {
    let out = run("nonsense", &["scan", "file.ts"]);
    assert_eq!(code(&out), 0);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}
