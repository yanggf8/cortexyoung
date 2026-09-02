//! The §6 funnel, gated on the mistakes that actually happened when it was run by hand.

use cort_evals::adopt::{format_utc, mine, parse_since, runs_cort_impact, DEFAULT_FOLLOW_CALLS};
use serde_json::{json, Value};
use std::path::Path;

/// The exact text the hook injects, so the fixture cannot drift from the product by paraphrase.
fn injected(symbol: &str) -> String {
    format!(
        "cort has an index for this project. `cort impact --symbol '{symbol}' --depth 1 \
         --coverage -f lean` answers who calls it in one call, and `--coverage` lists what the \
         enumeration could not see -- which a grep cannot tell you."
    )
}

fn bash(ts: &str, id: &str, command: &str) -> String {
    json!({
        "type": "assistant",
        "timestamp": ts,
        "message": { "role": "assistant", "content": [
            { "type": "tool_use", "id": id, "name": "Bash", "input": { "command": command } }
        ]},
    })
    .to_string()
}

fn injection(ts: &str, tool_use_id: &str, symbol: &str) -> String {
    json!({
        "type": "attachment",
        "timestamp": ts,
        "attachment": {
            "type": "hook_additional_context",
            "hookName": "PreToolUse:Bash",
            "hookEvent": "PreToolUse",
            "toolUseID": tool_use_id,
            "content": [injected(symbol)],
        },
    })
    .to_string()
}

/// The SessionStart skill preamble: same attachment type, no `hookName`. §6 warns about it because
/// a scan for the type alone counts it as an interception.
fn session_start_preamble(ts: &str) -> String {
    json!({
        "type": "attachment",
        "timestamp": ts,
        "attachment": {
            "type": "hook_additional_context",
            "content": ["<EXTREMELY_IMPORTANT>\nYou have superpowers.\n"],
        },
    })
    .to_string()
}

fn tree(sessions: &[(&str, &str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("adopt-").tempdir().unwrap();
    for (project, name, body) in sessions {
        let p = dir.path().join(project);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join(format!("{name}.jsonl")), body).unwrap();
    }
    dir
}

fn run(dir: &Path, since: &str) -> Value {
    mine(
        dir,
        parse_since(since).expect("fixture window parses"),
        None,
        50,
        DEFAULT_FOLLOW_CALLS,
        &[],
    )
}

#[test]
fn a_window_without_an_offset_is_refused_rather_than_guessed() {
    // The hand-run that produced a full-zero funnel passed a local wall-clock time and got it read
    // as UTC, placing the cutoff eight hours in the future. Both readings are defensible, which is
    // exactly why neither may be assumed.
    let err = parse_since("2026-09-02 09:24").unwrap_err();
    assert!(err.contains("explicit UTC offset"), "{err}");
    let err = parse_since("2026-09-02T09:24:00").unwrap_err();
    assert!(err.contains("explicit UTC offset"), "{err}");

    // The same instant, written two ways, is one number.
    let taipei = parse_since("2026-09-02T09:24:00+08:00").unwrap();
    let utc = parse_since("2026-09-02T01:24:00Z").unwrap();
    assert_eq!(taipei, utc);
    assert_eq!(format_utc(taipei), "2026-09-02T01:24:00Z");
    // And a negative offset moves the other way.
    assert_eq!(
        parse_since("2026-09-01T21:24:00-04:00").unwrap(),
        utc
    );
}

#[test]
fn a_mention_of_the_command_is_not_an_execution_of_it() {
    assert_eq!(
        runs_cort_impact("cort impact --symbol helper --depth 1"),
        Some(Some("helper".to_string()))
    );
    assert_eq!(
        runs_cort_impact("cd /repo && cort impact --symbol 'a,b' -f lean"),
        Some(Some("a,b".to_string()))
    );
    // The session that wrote this test is not a session that used the tool.
    assert_eq!(
        runs_cort_impact("python3 - <<'PY'\nif \"cort impact\" in cmd:\n    print(1)\nPY"),
        None
    );
    assert_eq!(runs_cort_impact("grep -rn 'cort impact' docs/"), None);
    assert_eq!(runs_cort_impact("cort status"), None);
}

#[test]
fn an_injection_pairs_with_its_trigger_and_with_what_followed() {
    let body = [
        session_start_preamble("2026-09-02T01:00:00.000Z"),
        bash("2026-09-02T01:27:10.000Z", "toolu_1", "grep -rn 'helper(' src --include=*.ts"),
        injection("2026-09-02T01:27:12.193Z", "toolu_1", "helper"),
        bash("2026-09-02T01:28:00.000Z", "toolu_2", "cort impact --symbol helper --depth 1 --coverage -f lean"),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");

    assert_eq!(r["sessions_in_window"], json!(1));
    assert_eq!(r["searches"], json!(1));
    assert_eq!(r["rule_would_fire"], json!(1));
    // The preamble shares the attachment type and is not an interception.
    assert_eq!(r["injections"], json!(1), "{r:#}");
    assert_eq!(r["adopted_same_symbol"], json!(1));
    assert_eq!(r["not_adopted"], json!(0));

    let row = &r["injection_rows"][0];
    assert_eq!(row["symbol"], json!("helper"));
    assert_eq!(row["verdict"], json!("adopted_same_symbol"));
    assert_eq!(row["at"], json!("2026-09-02T01:27:12Z"));
    // Paired by toolUseID, so the row names the search that was intercepted rather than whichever
    // command happened to be nearest.
    assert!(
        row["triggering_command"]
            .as_str()
            .unwrap()
            .contains("grep -rn"),
        "{row:#}"
    );
    assert!(row["followed_by"].as_str().unwrap().starts_with("cort impact"));
    // The db was not passed, and a missing source is reported as absent rather than as zero.
    assert_eq!(r["usage_db_cross_check"], Value::Null);
}

#[test]
fn an_injection_nobody_acted_on_is_not_an_adoption() {
    let body = [
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src"),
        injection("2026-09-02T02:00:01.000Z", "toolu_1", "helper"),
        // Mentions the command inside a script; never runs it.
        bash(
            "2026-09-02T02:01:00.000Z",
            "toolu_2",
            "python3 - <<'PY'\nprint(\"cort impact --symbol helper\")\nPY",
        ),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["injections"], json!(1));
    assert_eq!(r["adopted_same_symbol"], json!(0));
    assert_eq!(r["not_adopted"], json!(1));
    assert_eq!(r["injection_rows"][0]["followed_by"], Value::Null);
}

#[test]
fn the_window_excludes_what_happened_before_the_hook_was_wired() {
    let body = [
        bash("2026-09-01T10:00:00.000Z", "toolu_0", "grep -rn 'old(' src"),
        injection("2026-09-01T10:00:01.000Z", "toolu_0", "old"),
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src"),
        injection("2026-09-02T02:00:01.000Z", "toolu_1", "helper"),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let all = run(dir.path(), "2026-09-01T00:00:00Z");
    assert_eq!(all["injections"], json!(2));
    let after = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(after["injections"], json!(1));
    assert_eq!(after["searches"], json!(1));
    assert_eq!(after["injection_rows"][0]["symbol"], json!("helper"));
}

#[test]
fn a_subagent_sidechain_is_not_a_session_anyone_steered() {
    let body = [
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src"),
        injection("2026-09-02T02:00:01.000Z", "toolu_1", "helper"),
    ]
    .join("\n");
    let dir = tree(&[
        ("-home-u-repo", "s1", &body),
        ("-home-u-repo", "agent-abc", &body),
    ]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["sessions_in_window"], json!(1), "{r:#}");
    assert_eq!(r["injections"], json!(1));
}

#[test]
fn a_project_with_no_index_shows_up_as_the_gap_between_the_rule_and_the_hook() {
    // The offline matcher fires on the search; no injection followed, because the gate declined a
    // project cort has never indexed. The report must let those two numbers disagree rather than
    // reconcile them, since that difference *is* the opportunity the gate passed on.
    let body = [
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src --include=*.ts"),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-unindexed", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["rule_would_fire"], json!(1));
    assert_eq!(r["injections"], json!(0));
    assert_eq!(r["by_project"]["-home-u-unindexed"]["searches"], json!(1));
}

/// The false positive this module scored on its own session, kept as a fixture.
///
/// A heredoc that *writes* a test containing `cd /repo && cort impact ...` is a file being created,
/// not a tool being used. Splitting the raw command on `&&` promoted a string literal to an
/// executed segment, and the funnel reported an adoption that never happened.
#[test]
fn a_heredoc_body_is_written_not_executed() {
    let writing_this_file = "cat > /repo/evals/tests/adopt.rs <<'RS'\n\
        assert_eq!(\n\
            runs_cort_impact(\"cd /repo && cort impact --symbol 'a,b' -f lean\"),\n\
            Some(Some(\"a,b\".to_string()))\n\
        );\n\
        RS";
    assert_eq!(runs_cort_impact(writing_this_file), None);

    // An unquoted tag, a `<<-` tag, and a command that runs *after* the body closes.
    let after = "cat > f <<-EOF\ncort impact --symbol ghost\nEOF\ncort impact --symbol real";
    assert_eq!(
        runs_cort_impact(after),
        Some(Some("real".to_string())),
        "the body is skipped; the command after the terminator still counts"
    );

    // A here-string has no body to skip, so the line itself is still read.
    assert_eq!(
        runs_cort_impact("cort impact --symbol x <<< 'seed'"),
        Some(Some("x".to_string()))
    );
}

/// Codex's first finding: adoption had no bound, so any later `cort impact` in the session counted.
#[test]
fn adoption_is_bounded_to_what_the_agent_did_next() {
    let mut lines = vec![
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src"),
        injection("2026-09-02T02:00:01.000Z", "toolu_1", "helper"),
    ];
    // Six unrelated calls, then an impact run long after the suggestion.
    for i in 0..6 {
        lines.push(bash(
            &format!("2026-09-02T02:1{i}:00.000Z"),
            &format!("toolu_f{i}"),
            "cargo test",
        ));
    }
    lines.push(bash(
        "2026-09-02T03:00:00.000Z",
        "toolu_late",
        "cort impact --symbol helper --depth 1",
    ));
    let dir = tree(&[("-home-u-repo", "s1", &lines.join("\n"))]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["not_adopted"], json!(1), "{r:#}");
    assert_eq!(r["adopted_same_symbol"], json!(0));
    // The later call is a fact the reader gets, not a number the funnel claims.
    assert_eq!(r["injection_rows"][0]["impact_later_in_session"], json!(true));
    assert_eq!(r["injection_rows"][0]["followed_by"], Value::Null);
}

/// Two injections must not both claim the same single `cort impact`.
#[test]
fn one_impact_call_can_only_be_taken_once() {
    let body = [
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'alpha(' src"),
        injection("2026-09-02T02:00:01.000Z", "toolu_1", "alpha"),
        bash("2026-09-02T02:00:02.000Z", "toolu_2", "grep -rn 'beta(' src"),
        injection("2026-09-02T02:00:03.000Z", "toolu_2", "beta"),
        bash("2026-09-02T02:00:04.000Z", "toolu_3", "cort impact --symbol beta --depth 1"),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["injections"], json!(2));
    assert_eq!(
        r["adopted_same_symbol"].as_i64().unwrap() + r["adopted_other_symbol"].as_i64().unwrap(),
        1,
        "one call, one adoption: {r:#}"
    );
}

/// Codex's fifth finding: the denominator must be what the hook evaluates.
#[test]
fn a_leading_env_assignment_does_not_hide_a_search() {
    let body = [bash(
        "2026-09-02T02:00:00.000Z",
        "toolu_1",
        "LC_ALL=C rg 'helper(' src --include=*.ts",
    )]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["searches"], json!(1), "the hook skips VAR=value, so must this: {r:#}");
    assert_eq!(r["rule_would_fire"], json!(1));
}

/// Codex's sixth finding, the false-negative half: `;` starts a new command.
#[test]
fn a_command_after_a_semicolon_still_counts_as_run() {
    assert_eq!(
        runs_cort_impact("cd repo; cort impact --symbol x --depth 1"),
        Some(Some("x".to_string()))
    );
    assert_eq!(
        runs_cort_impact("echo start | tee log; cort impact --symbol y"),
        Some(Some("y".to_string()))
    );
    // ...and the false-positive half: a quoted mention is still not an execution.
    assert_eq!(runs_cort_impact("echo 'run cort impact --symbol z'"), None);
    assert_eq!(runs_cort_impact("git commit -m \"add; cort impact wiring\""), None);
}

/// Codex's ninth finding: refusing offset-less input fixed one silent window error, not all of them.
#[test]
fn an_impossible_instant_is_refused_too() {
    assert!(parse_since("2026-02-31T00:00:00Z").is_err(), "2026-02-31 is not a date");
    assert!(parse_since("2026-09-02T00:00:00+99:00").is_err(), "no zone is +99");
    assert!(parse_since("2026-09-02T00:00:00.5xZ").is_err(), "junk is not a fraction");
    // The real leap day is a date.
    assert!(parse_since("2024-02-29T00:00:00Z").is_ok());
    assert!(parse_since("2026-02-28T23:59:59.250Z").is_ok());
}

/// The project cort is developed in is audit traffic; the report must be able to drop it and say so.
#[test]
fn an_excluded_project_is_dropped_and_named() {
    let body = [
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src"),
        injection("2026-09-02T02:00:01.000Z", "toolu_1", "helper"),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-cortexyoung", "s1", &body), ("-home-u-other", "s2", &body)]);
    let all = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(all["injections"], json!(2));

    let kept = mine(
        dir.path(),
        parse_since("2026-09-02T00:00:00Z").unwrap(),
        None,
        50,
        DEFAULT_FOLLOW_CALLS,
        &["-home-u-cortexyoung".to_string()],
    );
    assert_eq!(kept["injections"], json!(1));
    assert_eq!(kept["excluded_sessions"], json!(1));
    assert_eq!(kept["excluded_projects"][0], json!("-home-u-cortexyoung"));
}

/// A transcript that cannot be read must arrive as missing data, not as a confident zero.
#[test]
fn unreadable_records_are_counted_not_swallowed() {
    let body = [
        "{ this is not json".to_string(),
        // Carries a message and no timestamp: real missing data.
        json!({"type": "assistant", "message": {"content": []}}).to_string(),
        // A transcript's ordinary furniture never carries a timestamp and is not missing data.
        json!({"type": "file-history-snapshot", "snapshot": {}}).to_string(),
        json!({"type": "last-prompt"}).to_string(),
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "grep -rn 'helper(' src"),
    ]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["lines_unparsed"], json!(1));
    assert_eq!(
        r["records_without_timestamp"],
        json!(1),
        "only records that could have carried what we read count as missing: {r:#}"
    );
}

/// A call that happened *before* the injection cannot be an adoption of it.
///
/// With no `toolUseID` and no earlier call to anchor on -- a truncated or compacted transcript --
/// the window used to open at index 0, which is the start of the session. The first five calls of
/// a session are exactly the ones the injection could not have caused.
#[test]
fn the_window_never_opens_before_the_injection() {
    // An impact call early in the session, then an injection with no resolvable trigger.
    let injection_without_trigger = json!({
        "type": "attachment",
        "timestamp": "2026-09-02T02:30:00.000Z",
        "attachment": {
            "type": "hook_additional_context",
            "hookName": "PreToolUse:Bash",
            "content": [injected("helper")],
        },
    })
    .to_string();
    let body = [
        bash("2026-09-02T02:00:00.000Z", "toolu_1", "cort impact --symbol helper --depth 1"),
        injection_without_trigger,
    ]
    .join("\n");
    let dir = tree(&[("-home-u-repo", "s1", &body)]);
    let r = run(dir.path(), "2026-09-02T00:00:00Z");
    assert_eq!(r["injections"], json!(1));
    assert_eq!(
        r["not_adopted"],
        json!(1),
        "a call that preceded the suggestion is not an adoption of it: {r:#}"
    );
    assert_eq!(r["injection_rows"][0]["paired_by"], json!("nearest_earlier_call"));
    assert_eq!(r["injection_rows"][0]["followed_by"], Value::Null);
}

/// `command_starts` indexes bytes; every byte it splits on is ASCII, so a multibyte command must
/// neither panic nor lose a segment.
#[test]
fn a_multibyte_command_is_split_without_panicking() {
    assert_eq!(
        runs_cort_impact("echo '改名前先確認'; cort impact --symbol 名前"),
        Some(Some("名前".to_string()))
    );
    assert_eq!(runs_cort_impact("echo '確認; cort impact --symbol x'"), None);
    // A redirection carries `&` and must not lose the command that follows.
    assert_eq!(
        runs_cort_impact("build 2>&1 && cort impact --symbol y"),
        Some(Some("y".to_string()))
    );
}
