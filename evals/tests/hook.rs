//! What stays here is the offline half: pulling commands out of transcripts, and the index-free
//! stand-in for the seed check. The matcher itself moved to `cort::hook` so the measured rule and
//! the installed rule cannot drift; its fixtures moved with it to `rust/tests/hook.rs`.

use cort_evals::hook::{commands_of_line, declares_callable_in};

#[test]
fn both_transcript_dialects_yield_their_executed_commands() {
    // Claude Code: a string command nested under the tool-use input.
    let claude = r#"{"message":{"content":[{"name":"Bash","input":{"command":"rg foo src/","description":"x"}}]}}"#;
    assert_eq!(commands_of_line(claude), vec!["rg foo src/".to_string()]);
    // Codex: an argv array whose last element is the script.
    let codex = r#"{"payload":{"command":["bash","-lc","rg bar crates/"]}}"#;
    assert_eq!(commands_of_line(codex), vec!["rg bar crates/".to_string()]);
    // A line with no command at all yields nothing rather than an error.
    assert!(commands_of_line(r#"{"type":"user","text":"hello"}"#).is_empty());
    assert!(commands_of_line("not json").is_empty());
}

/// The index check, measured. It answers "is this name a thing `impact` could have a seed for",
/// which is narrower than "is this name declared": a constant and a struct field are declared and
/// are not callable.
#[test]
fn the_index_check_asks_for_a_callable_not_any_declaration() {
    assert!(declares_callable_in("pub const TIMEOUT_S: u64 = 30;", "TIMEOUT_S").is_none());
    assert!(declares_callable_in("    trace_file: PathBuf,", "trace_file").is_none());
    assert!(declares_callable_in("    let trace_file = dir.join(\"t\");", "trace_file").is_none());
    assert!(declares_callable_in("pub struct Confidence;", "Confidence").is_none());

    assert_eq!(
        declares_callable_in("pub async fn deliver_news(x: u8) {}", "deliver_news"),
        Some("fn")
    );
    assert_eq!(
        declares_callable_in(
            "export function updatePaymentStatus(id) {",
            "updatePaymentStatus"
        ),
        Some("function")
    );
    assert_eq!(
        declares_callable_in("def rate_limit(self):", "rate_limit"),
        Some("def")
    );
    assert_eq!(
        declares_callable_in(
            "const ensureSeedUserPasswords = async () => {",
            "ensureSeedUserPasswords"
        ),
        Some("const-arrow")
    );
    // `impl T { pub fn take(&self) -> u32 { 1 } }` -- one line, two items, the callable is found.
    assert_eq!(
        declares_callable_in("impl T { pub fn take(&self) -> u32 { 1 } }", "take"),
        Some("fn")
    );
}

/// A directory that is right there and holds nothing the scanner opens is not an unreadable tree.
///
/// Both used to answer `None` and both were reported as `unchecked_tree_unreadable`, which sent a
/// reader looking for a permissions or path problem that did not exist on a Godot project whose
/// `.gd` files this screen simply does not read. Splitting them cost nothing and the split is not
/// cosmetic: on the local corpus it separates 20 genuinely unresolvable paths from 4 of these.
#[test]
fn a_readable_tree_with_no_source_is_not_an_unreadable_tree() {
    use cort_evals::hook::{declares_function, DeclCheck};

    let d = tempfile::Builder::new()
        .prefix("cort-declcheck-")
        .tempdir()
        .unwrap();

    // A directory holding only files this scanner has no reason to open.
    std::fs::write(d.path().join("world.gd"), "func _ready():\n\tpass\n").unwrap();
    std::fs::write(d.path().join("notes.txt"), "nothing here\n").unwrap();
    assert_eq!(
        declares_function(d.path(), "_ready"),
        DeclCheck::NoSourceRead,
        "the tree is readable; it is this screen that has nothing to read in it"
    );

    // A path that is not a directory at all.
    assert_eq!(
        declares_function(&d.path().join("no-such-dir"), "_ready"),
        DeclCheck::TreeMissing
    );

    // And with something it does read, it answers the actual question again.
    std::fs::write(d.path().join("lib.rs"), "pub fn helper() {}\n").unwrap();
    assert_eq!(declares_function(d.path(), "helper"), DeclCheck::Declared);
    assert_eq!(
        declares_function(d.path(), "absent_symbol"),
        DeclCheck::NotDeclared
    );

    // The verdict strings are the report's keys; drift there is drift in every number read off it.
    assert_eq!(DeclCheck::Declared.verdict(), "confirmed_function");
    assert_eq!(DeclCheck::NotDeclared.verdict(), "rejected_not_a_function");
    assert_eq!(DeclCheck::TreeMissing.verdict(), "unchecked_tree_missing");
    assert_eq!(
        DeclCheck::NoSourceRead.verdict(),
        "unchecked_no_source_this_screen_reads"
    );
}

/// Every report this harness prints names the machine that produced it.
///
/// On 2026-09-03 a hook-attribution table showing 417 Codex fires was set beside this machine's 2
/// and reconciled as if the two were comparable. They were different computers, and neither report
/// said so. The stamp goes on at the print site because that is the last point where the number is
/// still attached to the process that computed it.
#[test]
fn a_printed_report_names_the_machine_that_produced_it() {
    let empty = tempfile::Builder::new()
        .prefix("cort-evals-machine-")
        .tempdir()
        .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cort-evals"))
        .args([
            "hook-probe",
            "--claude-dir",
            empty.path().to_str().unwrap(),
            "--codex-dir",
            empty.path().to_str().unwrap(),
            "--kimi-dir",
            empty.path().to_str().unwrap(),
        ])
        .output()
        .expect("spawn cort-evals");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "not json: {e}; stdout={stdout} stderr={}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    let m = &v["machine"];
    assert!(
        m.get("id").and_then(serde_json::Value::as_str).is_some(),
        "the report does not name its machine: {v}"
    );
    assert!(
        m.get("source")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "the machine id must say where it came from, so a reader knows what it is worth: {v}"
    );
}
