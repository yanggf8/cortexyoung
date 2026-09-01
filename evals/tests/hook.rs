//! The rule is judged on real traffic, so the fixtures here are verbatim commands taken from the
//! session transcripts that carried the deployed skill (2026-08-31 20:10 onward). Inventing
//! plausible-looking greps would measure the rule against my imagination of agent behaviour, which
//! is exactly the error the demand re-measurement was written to correct.

use cort_evals::hook::{commands_of_line, declares_callable_in, suggests_impact};

#[test]
fn the_one_call_site_search_in_the_corpus_fires() {
    // The agent hand-rolled "drop the declaration line" with a second grep -- that is cort's
    // DECLARATION_KEYWORDS, rebuilt inline and worse. If the rule misses this, it has no purpose.
    let hit = suggests_impact(
        "grep -rn 'rate_limit(' /home/yanggf/a/ft/crates/api/src/routes/*.rs | grep -v 'pub async fn'",
    )
    .expect("the call-site shape must fire");
    assert_eq!(hit.symbol, "rate_limit");
}

#[test]
fn orientation_searches_from_the_same_corpus_stay_with_rg() {
    // Alternations are text hunts: several names OR'd together is not one symbol.
    assert!(suggests_impact(
        r"grep -n 'thinking\|MAX_TOKENS\|max_tokens\|LLM_TIMEOUT_S\|model' crates/cct2/src/llm.rs"
    )
    .is_none());
    // Transcripts, logs and state files are not project source.
    assert!(suggests_impact("grep -a 'cct2' ~/.nullclaw/skill-traces.jsonl | tail -4").is_none());
    assert!(
        suggests_impact("rg -l 'muse-spark' ~/.claude/projects --glob '*.jsonl' 2>/dev/null")
            .is_none(),
        "a hyphenated string in a transcript tree is not a symbol"
    );
    assert!(suggests_impact(
        "grep -inE 'quota|429|rate.?limit|credit|exhaust' /home/yanggf/.claude/"
    )
    .is_none());
    // CJK prose.
    assert!(
        suggests_impact(r"grep -rn '雙模型\|單模型\|no text block' crates/cct2/src/").is_none()
    );
}

#[test]
fn the_shape_of_the_symbol_is_what_decides_not_the_tool() {
    // Qualified Rust methods are the form cort asks for.
    assert_eq!(
        suggests_impact("rg 'Tally::add' rust/src").unwrap().symbol,
        "Tally::add"
    );
    // No path means the working tree, which in an agent session is the project.
    assert_eq!(
        suggests_impact("rg resolve_targets").unwrap().symbol,
        "resolve_targets"
    );
    // Short names are noise on any corpus.
    assert!(suggests_impact("rg fs src/").is_none());
    // A flag that takes a value must not be read as the pattern.
    assert_eq!(
        suggests_impact("rg --glob '*.rs' receiver_binds src/")
            .unwrap()
            .symbol,
        "receiver_binds"
    );
    // `-e` names the pattern explicitly.
    assert_eq!(
        suggests_impact("grep -rn -e cause_of rust/src/coverage.rs")
            .unwrap()
            .symbol,
        "cause_of"
    );
    // Not a search at all.
    assert!(suggests_impact("cargo test --locked").is_none());
    assert!(suggests_impact("sed -n '1,20p' src/lib.rs").is_none());
}

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

/// Adjudicating the first probe run (31 fires over the exposed corpus) turned up three shapes the
/// rule had no business firing on. Each is pinned by a verbatim command from that run.
#[test]
fn the_three_shapes_the_first_probe_run_got_wrong() {
    // 1. A language cort does not index. `~/nullclaw/src/cron.zig` matched on the `src/` marker,
    //    but there is no Zig rule pack, so `impact` has nothing to say about it. Three of the 31.
    assert!(suggests_impact(r#"grep -n "timeout" ~/nullclaw/src/cron.zig | head -40"#).is_none());
    assert!(suggests_impact(
        r#"grep -rn "reloadJobs(" ~/nullclaw/src/*.zig ~/nullclaw/src/**/*.zig 2>/dev/null"#
    )
    .is_none());

    // 2. Config and manifests are not source. This fired through the `crates/` marker.
    assert!(suggests_impact("grep name backend-rust/crates/core/Cargo.toml | head -1").is_none());

    // 3. Context flags mean the agent is reading the code, not enumerating its callers. Whoever
    //    asks for 10 lines around a match wants the body, and `cort context` is that verb.
    assert!(suggests_impact(
        r#"grep -n -B2 -A10 "getVisitorAnalytics" backend/src/services/analyticsService.ts"#
    )
    .is_none());
    assert!(
        suggests_impact(r#"grep -n -A 18 "validate" crates/worker/src/auth_handlers.rs"#).is_none()
    );

    // Still fires: the shapes adjudicated as genuine caller-set work in the same run.
    assert_eq!(
        suggests_impact(
            "grep -rn 'deliver_news' crates/news/src/*.rs | grep -v '^crates/news/src/deliver.rs'"
        )
        .unwrap()
        .symbol,
        "deliver_news"
    );
    assert_eq!(
        suggests_impact(
            r#"grep -rn "ensureSeedUserPasswords" backend/src frontend/src 2>/dev/null; echo "---(empty = fully removed)---""#
        )
        .unwrap()
        .symbol,
        "ensureSeedUserPasswords",
        "verifying a deletion is complete is the exact task the goal sentence names"
    );
    assert_eq!(
        suggests_impact(
            r#"grep -rn "updatePaymentStatus" frontend/src --include="*.tsx" | grep -v "registrationService""#
        )
        .unwrap()
        .symbol,
        "updatePaymentStatus"
    );
}

/// Second adjudication (23 fires). Eleven of the fourteen remaining false positives shared one
/// shape: the search names a single concrete file the agent already had open. A caller set is
/// cross-file by definition, so that shape cannot be one -- it is "where does X appear in this
/// file", which is reading.
#[test]
fn a_search_inside_one_named_file_is_reading_not_enumerating() {
    assert!(suggests_impact("grep -n 'confidence' crates/cct2/src/merge.rs | head -30").is_none());
    assert!(suggests_impact(
        r#"grep -n "AI_SUBSTAGE_CACHE_VARIANT" crates/news/src/summarize.rs | head -2"#
    )
    .is_none());
    assert!(suggests_impact(r#"grep -n "sales" crates/worker/src/lib.rs | head -20"#).is_none());

    // Cross-file shapes survive: a directory, a glob, or a recursive flag.
    assert_eq!(
        suggests_impact(
            r#"grep -rn "ensureSeedUserPasswords" backend/src frontend/src 2>/dev/null"#
        )
        .unwrap()
        .symbol,
        "ensureSeedUserPasswords"
    );
    assert_eq!(
        suggests_impact("grep -rn 'SkillStatus::Degraded' crates/*/src/*.rs | sort -u")
            .unwrap()
            .symbol,
        "SkillStatus::Degraded"
    );
    assert_eq!(
        suggests_impact(
            "grep -rn 'rate_limit(' /home/yanggf/a/ft/crates/api/src/routes/*.rs | grep -v 'pub async fn'"
        )
        .unwrap()
        .symbol,
        "rate_limit"
    );
    // Two named files is still a scoped read, not an enumeration.
    assert!(suggests_impact(
        r#"grep -n "DUCKDB_PATH" backend/src/database/duckdb-connection.ts backend/src/database/duckdb-pool.ts"#
    )
    .is_none());
}

/// A shell redirection is not a search target. `2>/dev/null` was being counted as a directory,
/// which let a two-file read through the cross-file test in the third probe run.
#[test]
fn a_redirection_is_not_a_search_target() {
    assert!(suggests_impact(
        r#"grep -n "DUCKDB_PATH" backend/src/database/duckdb-connection.ts backend/src/database/duckdb-pool.ts 2>/dev/null"#
    )
    .is_none());
    assert_eq!(
        suggests_impact(r#"grep -rn "ensureSeedUserPasswords" backend/src 2>/dev/null"#)
            .unwrap()
            .symbol,
        "ensureSeedUserPasswords",
        "stripping redirections must not strip the real directory beside them"
    );
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
