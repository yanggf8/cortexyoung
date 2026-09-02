//! The rule is judged on real traffic, so the fixtures here are verbatim commands taken from the
//! session transcripts that carried the deployed skill (2026-08-31 20:10 onward). Inventing
//! plausible-looking greps would measure the rule against my imagination of agent behaviour, which
//! is exactly the error the demand re-measurement was written to correct.

use cort::hook::{judge, search_from_grep_fields, suggests_impact};

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

/// The hook end of the rule: what the harness actually receives. Exercised through the built
/// binary, because the contract being tested is "what lands on stdout", not a function's return.
mod hook_command {
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn run(payload: &str, cache: &std::path::Path, cwd: &std::path::Path) -> String {
        let bin = env!("CARGO_BIN_EXE_cort");
        let mut child = Command::new(bin)
            .arg("hook-suggest")
            .current_dir(cwd)
            .env("CORT_CACHE_DIR", cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cort runs");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("write");
        let out = child.wait_with_output().expect("wait");
        assert!(
            out.status.success(),
            "the hook must never fail the tool call"
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// An unindexed project is the common case on a machine with many checkouts. Suggesting a query
    /// that can only answer `no_seed_resolved` spends the agent's turn to say nothing, and the first
    /// time it happens the suggestion stops being worth reading.
    #[test]
    fn an_unindexed_project_gets_no_suggestion_even_on_a_matching_search() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = tmp.path().join("cache");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).expect("mkdir");
        let out = run(
            r#"{"tool_name":"Bash","tool_input":{"command":"grep -rn deliver_news crates/news/src/*.rs"}}"#,
            &cache,
            &project,
        );
        assert!(!out.contains("additionalContext"), "{out}");
    }

    #[test]
    fn a_payload_that_is_not_a_search_is_silent_and_still_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for payload in [
            r#"{"tool_name":"Bash","tool_input":{"command":"cargo test --locked"}}"#,
            r#"{"tool_name":"Bash","tool_input":{}}"#,
            "not json at all",
            "",
        ] {
            let out = run(payload, tmp.path(), tmp.path());
            assert!(!out.contains("additionalContext"), "{payload} -> {out}");
        }
    }
}

// --- Structured searches ------------------------------------------------------------------------
//
// Same discipline as above: every fixture is a verbatim `Grep` tool call taken from
// ~/.kimi-code/sessions/**/wire.jsonl, so Kimi's parser is judged against traffic that happened
// rather than against a guess at what a structured search looks like. What these pin is that a
// second *parser* still reaches the one shared `judge` -- and reaches the same verdict its shell
// twin does.

fn grep_tool(pattern: &str, path: Option<&str>) -> Option<cort::hook::HookHit> {
    judge(&search_from_grep_fields(pattern, path, None, false).expect("a non-empty pattern parses"))
}

#[test]
fn a_bare_symbol_grep_tool_call_fires_the_same_as_its_shell_twin() {
    // {"pattern":"enumeration_may_be_incomplete","path":"rust/src","output_mode":"content","-n":true}
    let hit = grep_tool("enumeration_may_be_incomplete", Some("rust/src"))
        .expect("a bare symbol scoped to source must fire");
    assert_eq!(hit.symbol, "enumeration_may_be_incomplete");
    assert_eq!(
        hit.symbol,
        suggests_impact("rg -e 'enumeration_may_be_incomplete' rust/src")
            .unwrap()
            .symbol,
        "two parsers, one verdict -- that is the whole point of the split"
    );
}

/// The value on `-C` is exactly what a rendering back into a shell line had to throw away. Kimi's
/// parser keeps it as a flag rather than re-deriving it from text.
#[test]
fn a_context_flag_in_the_fields_silences_it_the_way_a_shell_flag_does() {
    // {"-C":4,"output_mode":"content","path":"frontend/src","pattern":"updatePaymentStatus"}
    let s = search_from_grep_fields("updatePaymentStatus", Some("frontend/src"), None, true)
        .expect("parses");
    assert!(
        judge(&s).is_none(),
        "a `-C` field is `cort context`'s question, not `impact`'s"
    );
    assert!(
        suggests_impact("rg -C 4 -e 'updatePaymentStatus' frontend/src").is_none(),
        "and the shell twin must agree"
    );
}

#[test]
fn a_grep_tool_call_pinned_to_one_file_does_not_fire() {
    // {"-n":true,"output_mode":"content","path":"rust/src/main.rs","pattern":"cmd_hook_install"}
    // A source-language file, so this is stopped by the single-file gate rather than by the
    // extension gate -- which is the case a naive `recursive: true` would have got wrong.
    assert!(
        grep_tool("cmd_hook_install", Some("rust/src/main.rs")).is_none(),
        "one concrete file is reading, not enumerating a caller set"
    );
}

#[test]
fn an_alternation_pattern_is_not_a_symbol_in_either_form() {
    // {"pattern":"MAP_ORIGIN|MAP_STEP|MAP_COLS","path":"game/scripts", ...}
    assert!(grep_tool("MAP_ORIGIN|MAP_STEP|MAP_COLS", Some("game/scripts")).is_none());
}

/// `type` narrows which languages are read, not where, so it is not a target -- folding it in would
/// make a tree-wide search read as if it named a concrete path and stop firing.
#[test]
fn a_type_filtered_tree_wide_call_still_fires() {
    // {"pattern":"custom_topic_raw_listing","type":"rust"}
    let hit = grep_tool("custom_topic_raw_listing", None).expect("scoped to the working tree");
    assert_eq!(hit.symbol, "custom_topic_raw_listing");
}

/// A `glob` is a target: it is what makes a search cross-file, which is the shape `impact` answers.
#[test]
fn a_glob_field_counts_as_a_cross_file_target() {
    let s = search_from_grep_fields("visitorTracking", None, Some("*.ts"), false).expect("parses");
    assert_eq!(judge(&s).unwrap().symbol, "visitorTracking");
}

/// No shell, so no quoting, so no class of pattern the parser has to refuse. The only `None` left
/// is the one that was never a search.
#[test]
fn the_only_unparseable_structured_call_is_an_empty_pattern() {
    assert!(search_from_grep_fields("", Some("rust/src"), None, false).is_none());
    assert!(search_from_grep_fields("it's a \"quote\"", None, None, false).is_some());
}
