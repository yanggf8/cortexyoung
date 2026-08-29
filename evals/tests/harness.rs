//! Tests for the eval harness itself.
//!
//! The harness decides whether `cort` gets built further, so it is not allowed to be the untested
//! part of the tree. These are the ports of `evals/harness.test.mjs` (deleted with the rest of the
//! JS, because this repo is pure Rust) plus the checks added when the first live cell exposed a
//! baseline arm that had never been contained at all.

use cort_evals::arms::{
    allowed_tools, arm_binaries, arm_held, build_args, build_env, build_prompt, build_row,
    is_cort_command, make_jail, resolve_binary, shell_binaries, AGENT_ARMS, REQUIRED_FIELDS,
};
use cort_evals::grade::{
    grade_answer, load_tasks, Graded, Task, ANSWER_CONTRACT, GATE_COVERAGE, GATE_PRECISION,
};
use cort_evals::stream::{estimate_tokens, parse_stream, ToolCall};
use cort_evals::summary::{summarize, ARMS, METRICS};
use cort_evals::verify::contains_word;
use serde_json::{json, Value};

fn task() -> Task {
    Task {
        id: "t1".into(),
        prompt: "Which functions reach leaf within 3 hops?".into(),
        venue: "/tmp/venue-under-test".into(),
        seed_symbol: "leaf".into(),
        expected_symbols: vec!["mid".into(), "top".into(), "entry".into()],
        by_hop: [
            (1i64, vec!["mid".to_string()]),
            (2, vec!["top".to_string()]),
            (3, vec!["entry".to_string()]),
        ]
        .into_iter()
        .collect(),
    }
}

fn bash(command: &str) -> ToolCall {
    ToolCall {
        name: "Bash".into(),
        command: command.into(),
    }
}

fn stream(tool_commands: &[&str], results: &[&str], subtype: &str, result_text: &str) -> String {
    let mut events: Vec<Value> = Vec::new();
    for command in tool_commands {
        events.push(json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Bash","input":{"command":command}}]}}));
    }
    for text in results {
        events.push(json!({"type":"user","message":{"content":[
            {"type":"tool_result","content":text}]}}));
    }
    events.push(json!({"type":"assistant","message":{"content":[
        {"type":"tool_use","name":"Read","input":{"file_path":"src/a.ts"}}]}}));
    events.push(
        json!({"type":"result","subtype":subtype,"num_turns":4,"result":result_text,
        "total_cost_usd":0.1,"session_id":"s","permission_denials":[],
        "usage":{"input_tokens":100,"cache_creation_input_tokens":10,
                 "cache_read_input_tokens":20,"output_tokens":5}}),
    );
    events
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn good_answer() -> String {
    "prose...\n\n```answer\nmid\t1\ntop\t2\nentry\t3\n```".to_string()
}

#[test]
fn estimator_prices_cjk_as_one_token_not_a_quarter() {
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
    assert_eq!(estimate_tokens("查询使用者"), 5);
    // '查' is its own token and the space + four letters round up to two. Dividing everything by
    // 4 would price this at 2 and flatter whichever arm reads commented source.
    assert_eq!(estimate_tokens("查 abcd"), 3);
}

#[test]
fn parse_stream_measures_per_tool_payload() {
    let parsed = parse_stream(&stream(
        &[&format!("{} impact --symbol leaf -f lean", "cort")],
        &["h1\tsrc/c.ts\tmid\t2\n"],
        "success",
        &good_answer(),
    ))
    .expect("parses");
    assert!(parsed.tool_return_bytes > 0, "bytes must be measured");
    assert!(parsed.tool_return_tokens > 0);
    assert_eq!(parsed.read_calls, 1);
    assert_eq!(parsed.total_tokens, 135);
    assert_eq!(parsed.turns, 4);
    assert!(!parsed.hit_turn_cap);
}

#[test]
fn parse_stream_flags_the_turn_cap_without_failing_the_cell() {
    let parsed = parse_stream(&stream(&[], &[], "error_max_turns", "")).unwrap();
    assert!(parsed.hit_turn_cap);
}

#[test]
fn parse_stream_refuses_to_produce_a_null_metric() {
    assert!(
        parse_stream(r#"{"type":"assistant","message":{"content":[]}}"#)
            .unwrap_err()
            .contains("no result event")
    );
    assert!(parse_stream(r#"{"type":"result","num_turns":1}"#)
        .unwrap_err()
        .contains("no usage"));
    let nullish = json!({"type":"result","num_turns":1,"usage":{
        "input_tokens":null,"cache_creation_input_tokens":0,
        "cache_read_input_tokens":0,"output_tokens":0}})
    .to_string();
    assert!(parse_stream(&nullish).unwrap_err().contains("not a number"));
}

#[test]
fn is_cort_command_counts_the_rust_binary_not_the_deleted_js_entry_point() {
    assert!(is_cort_command(
        "/home/x/.local/share/cortexyoung/cort/cort impact --symbol leaf"
    ));
    assert!(is_cort_command("/home/x/.cargo/bin/cort read src/a.ts"));
    assert!(is_cort_command("cort status ."));
    assert!(!is_cort_command("rg -n leaf src"));
    assert!(!is_cort_command(""));
    // The regression this pins: the old filter looked for 'cort.js' and counted 0 for every cell.
    assert!(!"/home/x/cargo/bin/cort impact".contains("cort.js"));
}

#[test]
fn the_cort_arm_is_whitelisted_for_exactly_the_command_its_prompt_hands_over() {
    let entry = allowed_tools("cort")
        .into_iter()
        .find(|t| t.starts_with("Bash("))
        .unwrap();
    assert!(
        entry.starts_with("Bash(") && entry.ends_with(":*)"),
        "matcher shape: {entry}"
    );
    let allowed = entry[5..entry.len() - 3].trim_end_matches(':').to_string();
    assert!(
        allowed.ends_with("/cort"),
        "whitelist prefix must be the binary: {allowed}"
    );
    let command = build_prompt(&task(), "cort")
        .lines()
        .find(|l| l.starts_with(&allowed))
        .unwrap_or_default()
        .to_string();
    assert!(
        !command.is_empty(),
        "guidance must give a copy-able command"
    );
    assert!(
        command.starts_with(&allowed),
        "{command} is outside the whitelist {allowed}"
    );

    assert!(allowed_tools("rg+Read").contains(&"Bash(rg:*)".to_string()));
    assert!(
        !allowed_tools("rg+Read").iter().any(|t| t.contains("cort")),
        "the baseline arm must not hold the tool under test"
    );
    assert!(build_prompt(&task(), "rg+Read").contains(ANSWER_CONTRACT));
    for arm in AGENT_ARMS {
        assert!(!allowed_tools(arm).iter().any(|t| t.contains("cort.js")));
    }
    assert!(
        !build_prompt(&task(), "rg+Read").contains("offline code-intelligence CLI"),
        "the baseline gets no cort guidance"
    );
}

#[test]
fn build_args_runs_in_the_venue_because_project_id_comes_from_cwd() {
    let invocation = build_args(&task(), "cort", 40);
    assert_eq!(invocation.cwd, "/tmp/venue-under-test");
    assert!(invocation.args.contains(&"--strict-mcp-config".to_string()));
    let at = invocation
        .args
        .iter()
        .position(|a| a == "--max-turns")
        .unwrap();
    assert_eq!(invocation.args[at + 1], "40");
}

#[test]
fn grade_answer_scores_coverage_precision_and_hop_distance() {
    let g: Graded = grade_answer(&good_answer(), &task());
    assert_eq!(g.coverage, 1.0);
    assert_eq!(g.precision, 1.0);
    assert_eq!(g.hop_accuracy, 1.0);
    assert!(g.success);

    let noisy = grade_answer("```answer\nmid\t1\nunrelated\t9\n```", &task());
    assert_eq!(noisy.coverage, 0.333);
    assert_eq!(noisy.precision, 0.5);
    assert!(!noisy.success);

    let wrong_hop = grade_answer("```answer\nmid\t2\ntop\t2\nentry\t3\n```", &task());
    assert!(wrong_hop.success, "the gate is coverage + precision only");
    assert_eq!(wrong_hop.hop_accuracy, 0.667);
    assert_eq!(
        wrong_hop
            .wrong_hop
            .iter()
            .map(|w| w.symbol.clone())
            .collect::<Vec<_>>(),
        vec!["mid"]
    );

    let none = grade_answer("no block at all", &task());
    assert!(!none.answer_block);
    assert!(!none.success);
    assert_eq!((GATE_COVERAGE, GATE_PRECISION), (0.9, 0.7));
}

#[test]
fn grade_answer_takes_the_last_block_and_strips_list_markers() {
    let text = "```answer\nmid\t1\n```\nsecond thoughts:\n```answer\n- top\t2\n* entry\t3\n```";
    let g = grade_answer(text, &task());
    assert_eq!(g.answered_symbols, vec!["top", "entry"]);
}

#[test]
fn build_row_carries_every_required_field_and_rejects_unmeasured_ones() {
    let parsed = parse_stream(&stream(
        &[&format!(
            "{} impact --symbol leaf --depth 3 -f lean",
            "cort"
        )],
        &["h1\tsrc/c.ts\tmid\t2\n"],
        "success",
        &good_answer(),
    ))
    .unwrap();
    let row = build_row("cort", &task(), &parsed, "deadbee", Some(true)).unwrap();
    for field in REQUIRED_FIELDS {
        let value = row
            .get(field)
            .unwrap_or_else(|| panic!("row missing {field}"));
        assert_ne!(value, &Value::Null, "{field} must never be null");
    }
    assert_eq!(row["cort_calls"], 1);
    assert_eq!(row["rg_calls"], 0);
    assert_eq!(row["read_calls"], 1);
    assert_eq!(row["arm_held"], true);
    assert_eq!(row["jailed"], json!(true));
    assert_eq!(row["success"], true);
    assert_eq!(row["venue_head"], "deadbee");
    assert_eq!(row["estimator"], "ascii/4 + non-ascii*1 (v1)");

    let mut broken = parsed.clone();
    broken.tool_return_tokens = 0;
    let row2 = build_row("cort", &task(), &broken, "x", Some(true)).unwrap();
    assert_eq!(
        row2["tool_return_tokens"], 0,
        "a genuinely empty payload is a real zero"
    );
}

#[test]
fn arm_held_flags_exactly_the_leak_the_first_live_cell_showed() {
    let leaked = vec![
        bash("grep -rn \"getLastNTradingDays\" --include=*.ts . | grep -v node_modules"),
        bash("sed -n '1400,1500p' src/routes/report-routes.ts"),
    ];
    assert!(
        !arm_held("rg+Read", &leaked),
        "grep+sed must not pass as the rg arm"
    );
    assert_eq!(shell_binaries(&leaked), vec!["grep", "sed"]);

    assert!(arm_held(
        "rg+Read",
        &[
            bash("rg -n symbol src"),
            ToolCall {
                name: "Read".into(),
                command: String::new()
            }
        ]
    ));
    // The rg arm is configured by bare name, so containment means "reached it through the jail".
    // Hard-coding an absolute path outside the jail is exactly how an arm escapes containment, and
    // is reported as not held rather than waved through on a matching basename.
    assert!(
        !arm_held("rg+Read", &[bash("/usr/local/bin/rg -n symbol src")]),
        "an absolute path around the jail is not containment"
    );

    let cort = arm_binaries("cort")[0].clone();
    assert!(arm_held(
        "cort",
        &[bash(&format!("{cort} impact --symbol leaf -f lean"))]
    ));
    assert!(
        arm_held("cort", &[bash("cort status .")],),
        "a bare name resolves through the jail"
    );
    assert!(
        !arm_held("cort", &[bash("/opt/cort impact --symbol leaf")]),
        "an absolute path to some *other* cort build is not containment"
    );
    assert!(
        !arm_held("cort", &[bash("rg -n leaf src")]),
        "the cort arm must not quietly use rg"
    );
}

#[test]
fn the_jail_exposes_only_the_permitted_binary() {
    let root = std::env::temp_dir().join(format!("cort-jail-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fake = root.join("src-bin");
    std::fs::create_dir_all(&fake).unwrap();
    let fake_tool = fake.join("rg");
    std::fs::write(&fake_tool, "#!/bin/sh\nexit 0\n").unwrap();

    let jail = root.join("jail");
    make_jail(
        jail.to_str().unwrap(),
        &[fake_tool.to_str().unwrap().to_string()],
    )
    .unwrap();
    assert!(
        jail.join("rg").exists(),
        "the permitted binary must be reachable"
    );
    assert!(!jail.join("grep").exists(), "grep must not be reachable");
    assert!(!jail.join("sed").exists(), "sed must not be reachable");

    assert_eq!(arm_binaries("rg+Read"), vec!["rg".to_string()]);
    let cort_bins = arm_binaries("cort");
    assert!(cort_bins[0].ends_with("/cort"), "{cort_bins:?}");
    // cort shells out to ast-grep (its only parser) and git (staleness). A jail without either
    // measures a broken tool, which is what the first jailed live cell did.
    for needed in ["git", "ast-grep"] {
        assert!(
            cort_bins
                .iter()
                .any(|b| b == needed || b.ends_with(&format!("/{needed}"))),
            "the cort arm's jail must carry {needed}: {cort_bins:?}"
        );
    }
    assert!(resolve_binary("definitely-not-a-binary-here").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn build_env_sets_path_config_and_cache_from_the_parent() {
    let env = build_env("/tmp/cc-eval", "/tmp/cort-exp", Some("/tmp/jails/cort"));
    let get = |k: &str| {
        env.iter()
            .find(|(name, _)| name == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("PATH").as_deref(), Some("/tmp/jails/cort"));
    assert_eq!(get("CORT_CACHE_DIR").as_deref(), Some("/tmp/cort-exp"));
    assert_eq!(get("CLAUDE_CONFIG_DIR").as_deref(), Some("/tmp/cc-eval"));

    let unjailed = build_env("/tmp/cc-eval", "/tmp/cort-exp", None);
    assert_ne!(
        unjailed
            .iter()
            .find(|(n, _)| n == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap(),
        String::new()
    );
}

#[test]
fn summarize_never_averages_nulls_into_a_verdict() {
    let hist = vec![
        json!({"arm":"cort","success":true,"total_tokens":388000,"tool_return_tokens":null,"turns":20,"read_calls":null}),
        json!({"arm":"ast-grep+Read","success":true,"total_tokens":200000,"tool_return_tokens":null,"turns":9,"read_calls":null}),
    ];
    let out = summarize(&hist, false).unwrap();
    assert_eq!(
        out["by_arm"]["cort"]["mean_tool_return_tokens"],
        Value::Null,
        "unmeasured stays null, never 0"
    );
    assert_eq!(out["by_arm"]["cort"]["stale_reads"], Value::Null);
    assert_eq!(
        out["by_arm"]["cort"]["metrics_missing"]["tool_return_tokens"],
        1
    );
    assert_eq!(
        out["verdict"]["reason"],
        "compared on mean_total_tokens + success_rate"
    );

    assert!(
        summarize(&hist, true).is_err(),
        "strict mode refuses unmeasured metrics"
    );

    let measured = vec![
        json!({"arm":"cort","success":true,"total_tokens":50,"tool_return_tokens":5,"turns":2,"read_calls":0,"stale_reads":0}),
        json!({"arm":"ast-grep+Read","success":true,"total_tokens":500,"tool_return_tokens":50,"turns":8,"read_calls":5,"stale_reads":1}),
    ];
    let win = summarize(&measured, true).unwrap();
    assert_eq!(win["verdict"]["cort_beats_ast_grep"], true);
    assert_eq!(
        win["verdict"]["next_action"],
        "continue to deferred features"
    );

    let tie = vec![
        json!({"arm":"cort","success":true,"total_tokens":500,"tool_return_tokens":5,"turns":2,"read_calls":0,"stale_reads":0}),
        json!({"arm":"ast-grep+Read","success":true,"total_tokens":500,"tool_return_tokens":50,"turns":8,"read_calls":5,"stale_reads":0}),
    ];
    assert_eq!(
        summarize(&tie, true).unwrap()["verdict"]["cort_beats_ast_grep"],
        false
    );

    assert_eq!(ARMS.len(), 3);
    assert_eq!(METRICS.len(), 4);
    assert!(
        !METRICS.contains(&"stale_reads"),
        "a metric nothing measures cannot be part of the strict gate"
    );
}

#[test]
fn the_shipped_task_file_still_loads_and_its_labels_are_hop_consistent() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tasks-graph.json");
    let tasks = load_tasks(path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tasks.len(), 5);
    for t in &tasks {
        assert!(!t.seed_symbol.is_empty(), "{} has no seed", t.id);
        assert_eq!(
            t.expected_symbols.len(),
            t.by_hop.values().map(Vec::len).sum::<usize>(),
            "{}: by_hop does not cover expected_symbols exactly",
            t.id
        );
        assert!(
            t.by_hop.keys().max().copied().unwrap_or(0) >= 3,
            "{}: task is not actually multi-hop",
            t.id
        );
    }
}

#[test]
fn word_matching_adjudicates_edges_without_a_regex_crate() {
    assert!(contains_word(
        "return handleReportsStatus(req)",
        "handleReportsStatus"
    ));
    assert!(!contains_word(
        "return handleReportsStatusV2(req)",
        "handleReportsStatus"
    ));
    // Honest limitation of the adjudicator: it matches text, so a mention inside a comment also
    // "confirms" an edge. verify-impact is a soundness screen against fabricated dependents, not
    // proof of a call — worth remembering whenever a precision figure of 1.0 is quoted.
    assert!(contains_word(
        "// handleReportsStatus",
        "handleReportsStatus"
    ));
    assert!(contains_word("const x = logInfo(msg);", "logInfo"));
    assert!(!contains_word("x = mylogInfo(msg)", "logInfo"));
    assert!(!contains_word("", "leaf"));
    assert!(!contains_word("leaf()", ""));
}

#[test]
fn the_gate_judges_against_whichever_baseline_actually_ran() {
    // Rounds 1-3 compared cort against `ast-grep+Read`; the current runner drives `rg+Read`.
    // A gate that can only name the old baseline reports "no comparison possible" on good data,
    // which is how a verdict goes missing without anybody noticing.
    let three_arm = vec![
        json!({"arm":"cort","success":true,"total_tokens":100,"tool_return_tokens":10,"turns":2,"read_calls":0,"stale_reads":0}),
        json!({"arm":"ast-grep+Read","success":true,"total_tokens":90,"tool_return_tokens":9,"turns":2,"read_calls":1,"stale_reads":0}),
        json!({"arm":"rg+Read","success":true,"total_tokens":80,"tool_return_tokens":8,"turns":2,"read_calls":1,"stale_reads":0}),
    ];
    let t = summarize(&three_arm, true).unwrap();
    assert_eq!(t["verdict"]["baseline_arm"], "ast-grep+Read");
    assert_eq!(
        t["verdict"]["cort_beats_ast_grep"], false,
        "100 is not cheaper than 90"
    );

    let two_arm = vec![
        json!({"arm":"cort","success":true,"total_tokens":100,"tool_return_tokens":10,"turns":2,"read_calls":0,"stale_reads":0}),
        json!({"arm":"rg+Read","success":true,"total_tokens":900,"tool_return_tokens":90,"turns":9,"read_calls":4,"stale_reads":0}),
    ];
    let t2 = summarize(&two_arm, true).unwrap();
    assert_eq!(t2["verdict"]["baseline_arm"], "rg+Read");
    assert_eq!(t2["verdict"]["cort_beats_ast_grep"], true);

    let alone = vec![
        json!({"arm":"cort","success":true,"total_tokens":100,"tool_return_tokens":10,"turns":2,"read_calls":0,"stale_reads":0}),
    ];
    let t3 = summarize(&alone, false).unwrap();
    assert_eq!(t3["verdict"]["baseline_arm"], Value::Null);
    assert_eq!(
        t3["verdict"]["reason"],
        "metric-missing: no comparison possible"
    );
}

#[test]
fn a_rate_limited_cell_is_an_error_never_a_zero() {
    // Observed live: the five-hour window rejected the call, the CLI still emitted a `result`
    // event with subtype success and all-zero usage, and the row went out as
    // total_tokens=0 coverage=0 — three tasks' worth of cells that had not run at all.
    let refused = [
        json!({"type":"rate_limit_event","rate_limit_info":{
            "status":"rejected","rateLimitType":"five_hour","overageStatus":"rejected",
            "isUsingOverage":false}}),
        json!({"type":"result","subtype":"success","is_error":true,"num_turns":1,
               "terminal_reason":"api_error","result":"You've hit your session limit",
               "permission_denials":[],"total_cost_usd":0,
               "usage":{"input_tokens":0,"cache_creation_input_tokens":0,
                        "cache_read_input_tokens":0,"output_tokens":0}}),
    ]
    .iter()
    .map(|e| e.to_string())
    .collect::<Vec<_>>()
    .join("\n");

    let err = parse_stream(&refused).unwrap_err();
    assert!(err.contains("cell never ran"), "{err}");
    assert!(err.contains("five_hour"), "{err}");
    assert!(err.contains("session limit"), "{err}");
}

#[test]
fn a_non_rate_limited_api_error_is_also_refused() {
    let raw = json!({"type":"result","subtype":"success","is_error":true,"num_turns":2,
        "terminal_reason":"api_error","result":"upstream blew up","permission_denials":[],
        "usage":{"input_tokens":0,"cache_creation_input_tokens":0,
                 "cache_read_input_tokens":0,"output_tokens":0}})
    .to_string();
    let err = parse_stream(&raw).unwrap_err();
    assert!(err.contains("cell errored without measuring"), "{err}");
}

#[test]
fn a_turn_capped_cell_is_still_a_real_measurement() {
    // Distinct from the two above: the agent did work and ran out of turns. That is a result, and
    // must be recorded as one (with hit_turn_cap) rather than dropped.
    let parsed = parse_stream(&stream(
        &[],
        &["h1\tsrc/c.ts\tmid\t2\n"],
        "error_max_turns",
        "",
    ))
    .unwrap();
    assert!(parsed.hit_turn_cap);
    assert!(parsed.tool_return_bytes > 0);
}
