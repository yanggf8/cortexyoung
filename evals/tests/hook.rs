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
