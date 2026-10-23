use cort::transcript::*;
use serde_json::{json, Value};

fn line(v: Value) -> String {
    format!("{v}\n")
}
fn boundary(uuid: &str) -> String {
    line(
        json!({"type":"system","subtype":"compact_boundary","uuid":uuid,"sessionId":"s1",
        "timestamp":"2026-10-22T23:37:47.345Z","cwd":"/w/p",
        "compactMetadata":{"trigger":"auto","preTokens":358791,"postTokens":19780}}),
    )
}
fn tool(name: &str, input: Value) -> String {
    line(
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":name,"input":input}]}}),
    )
}
fn read(p: &str) -> String {
    tool("Read", json!({"file_path":p}))
}
fn bash(c: &str) -> String {
    tool("Bash", json!({"command":c}))
}
fn human(t: &str) -> String {
    line(json!({"type":"user","origin":{"kind":"human"},"message":{"content":t}}))
}
fn comps(s: &str) -> Vec<Compaction> {
    compactions(&parse_transcript(s).steps)
}
fn filler(n: usize) -> String {
    (0..n).map(|i| bash(&format!("filler {i}"))).collect()
}

#[test]
fn boundary_row_is_parsed_with_metadata() {
    let p = parse_transcript(&boundary("u1"));
    match &p.steps[0] {
        Step::Boundary(m) => {
            assert_eq!(m.uuid, "u1");
            assert_eq!(m.session_id, "s1");
            assert_eq!(m.cwd.as_deref(), Some("/w/p"));
            assert_eq!(m.trigger, "auto");
            assert_eq!(m.pre_tokens, Some(358791));
            assert_eq!(m.post_tokens, Some(19780));
            assert_eq!(m.time_ms, 1_792_712_267_345);
        }
        _ => panic!("expected boundary"),
    }
}

#[test]
fn written_file_counts_as_known_for_reread() {
    let s = format!(
        "{}{}{}",
        tool("Write", json!({"file_path":"a.rs"})),
        boundary("u1"),
        read("a.rs")
    );
    assert_eq!(comps(&s)[0].post.rereads, vec!["a.rs"]);
}

#[test]
fn reread_counts_each_path_once() {
    let s = format!(
        "{}{}{}{}",
        read("a.rs"),
        boundary("u1"),
        read("a.rs"),
        read("a.rs")
    );
    assert_eq!(comps(&s)[0].post.rereads, vec!["a.rs"]);
}

#[test]
fn first_touch_inside_window_is_not_a_reread() {
    let s = format!("{}{}{}", boundary("u1"), read("a.rs"), read("a.rs"));
    assert!(comps(&s)[0].post.rereads.is_empty());
}

#[test]
fn repeated_command_is_exact_match_only() {
    let s = format!(
        "{}{}{}{}",
        bash("git status"),
        boundary("u1"),
        bash("git status "),
        bash("git status")
    );
    assert_eq!(comps(&s)[0].post.repeated_commands, vec!["git status"]);
}

#[test]
fn transcript_lookup_matches_full_input() {
    let long = format!("{}/home/x/.claude/projects/p/a.jsonl", "x".repeat(300));
    let s = format!("{}{}", boundary("u1"), bash(&format!("cat {long}")));
    assert_eq!(comps(&s)[0].post.transcript_lookups.len(), 1);
}

fn is_h(v: Value) -> bool {
    is_human(&v)
}

#[test]
fn origin_human_counts() {
    assert!(is_h(
        json!({"type":"user","origin":{"kind":"human"},"message":{"content":"hi"}})
    ));
}

#[test]
fn task_notification_and_peer_are_not_human() {
    for k in ["task-notification", "peer"] {
        assert!(!is_h(
            json!({"type":"user","origin":{"kind":k},"message":{"content":"hi"}})
        ));
    }
}

#[test]
fn legacy_line_starting_with_lt_is_not_human() {
    assert!(!is_h(
        json!({"type":"user","message":{"content":"<system-reminder>x"}})
    ));
    assert!(is_h(json!({"type":"user","message":{"content":"plain"}})));
    assert!(!is_h(
        json!({"type":"user","isMeta":true,"message":{"content":"plain"}})
    ));
}

#[test]
fn compact_summary_is_not_human() {
    assert!(!is_h(
        json!({"type":"user","isCompactSummary":true,"message":{"content":"summary"}})
    ));
    assert!(!is_h(
        json!({"type":"user","isCompactSummary":true,"origin":{"kind":"human"},"message":{"content":"summary"}})
    ));
}

#[test]
fn tool_result_is_not_human() {
    assert!(!is_h(
        json!({"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}})
    ));
}

#[test]
fn correction_terms_match() {
    for t in [
        "這不對",
        "不是這樣",
        "錯了",
        "你講錯",
        "搞錯了",
        "that is wrong",
        "That's not it",
        "no, stop",
        "Wrong!",
    ] {
        assert!(is_correction(t), "{t}");
    }
    assert!(!is_correction("looks good"));
}

#[test]
fn post_window_stops_at_next_boundary_and_is_complete() {
    let s = format!(
        "{}{}{}{}{}",
        boundary("u1"),
        filler(3),
        boundary("u2"),
        filler(2),
        ""
    );
    let c = comps(&s);
    assert_eq!(c[0].post.tool_calls, 3);
    assert!(c[0].post_complete);
    assert_eq!(c[1].post.tool_calls, 2);
    assert!(!c[1].post_complete);
}

#[test]
fn short_post_window_is_incomplete() {
    let s = format!("{}{}", boundary("u1"), filler(10));
    let c = comps(&s);
    assert_eq!(c[0].post.tool_calls, 10);
    assert!(!c[0].post_complete);
}

#[test]
fn full_post_window_is_complete() {
    let s = format!("{}{}{}", boundary("u1"), filler(50), human("after"));
    let c = comps(&s);
    assert_eq!(c[0].post.tool_calls, 50);
    assert!(c[0].post_complete);
    assert_eq!(c[0].post.human_messages, 0);
    let s = format!("{}{}", boundary("u1"), filler(60));
    assert_eq!(comps(&s)[0].post.tool_calls, 50);
}

#[test]
fn pre_window_stops_at_previous_boundary() {
    let s = format!(
        "{}{}{}{}{}",
        filler(4),
        boundary("u1"),
        filler(3),
        human("nope, wrong"),
        boundary("u2")
    );
    let c = comps(&s);
    assert_eq!(c[0].pre.tool_calls, 4);
    assert_eq!(c[1].pre.tool_calls, 3);
    assert_eq!(c[1].pre.human_messages, 1);
    assert_eq!(c[1].pre.corrections.len(), 1);
    let s = format!("{}{}", filler(60), boundary("u1"));
    assert_eq!(comps(&s)[0].pre.tool_calls, 50);
}

#[test]
fn empty_pre_window_when_boundaries_adjacent() {
    let s = format!("{}{}{}", boundary("u1"), boundary("u2"), filler(1));
    let c = comps(&s);
    assert_eq!(c[1].pre.tool_calls, 0);
    assert_eq!(c[1].pre.human_messages, 0);
}

#[test]
fn partial_last_line_is_ignored() {
    let s = format!("{}{{\"type\":\"assis", boundary("u1"));
    let p = parse_transcript(&s);
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.skipped_lines, 0);
}

#[test]
fn malformed_line_is_skipped_and_counted() {
    let s = format!("{}not json\n{}", boundary("u1"), read("a"));
    let p = parse_transcript(&s);
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.skipped_lines, 1);
}

#[test]
fn sidechain_lines_are_skipped() {
    let s = line(
        json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"a"}}]}}),
    );
    assert!(parse_transcript(&s).steps.is_empty());
}

#[test]
fn counts_derive_from_details() {
    let s = format!("{}{}{}", read("a"), boundary("u1"), read("a"));
    let c = comps(&s)[0].post.counts();
    assert_eq!((c.tool_calls, c.rereads), (1, 1));
}

#[test]
fn short_windows_are_symmetric_for_humans() {
    let s = format!(
        "{}{}{}{}{}",
        boundary("u1"),
        human("a"),
        read("x"),
        boundary("u2"),
        human("b")
    );
    let c = comps(&s);
    assert_eq!(c[0].post.human_messages, 1);
    assert_eq!(c[1].pre.human_messages, 1);
    // file start behaves like a previous boundary
    let s = format!("{}{}{}", human("a"), read("x"), boundary("u1"));
    assert_eq!(comps(&s)[0].pre.human_messages, 1);
}

#[test]
fn full_pre_window_starts_at_the_fiftieth_tool() {
    let s = format!(
        "{}{}{}{}",
        human("early"),
        filler(50),
        human("late"),
        boundary("u1")
    );
    assert_eq!(comps(&s)[0].pre.human_messages, 1);
}

fn ms_of(ts: &str) -> i64 {
    let l = line(
        json!({"type":"system","subtype":"compact_boundary","uuid":"u","sessionId":"s","timestamp":ts,
        "compactMetadata":{"trigger":"auto"}}),
    );
    match &parse_transcript(&l).steps[0] {
        Step::Boundary(m) => m.time_ms,
        _ => panic!(),
    }
}

#[test]
fn timestamp_calendar_is_validated() {
    assert_eq!(ms_of("1970-01-01T00:00:00Z"), 0);
    assert_eq!(ms_of("1970-01-02T00:00:00.5Z"), 86_400_500);
    for ok in ["2028-02-29T00:00:00Z", "2000-02-29T00:00:00Z"] {
        assert!(ms_of(ok) > 0, "{ok}");
    }
    for bad in [
        "2026-02-29T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-10T00:00:00Z",
        "2026-10-00T00:00:00Z",
    ] {
        assert_eq!(ms_of(bad), 0, "{bad}");
    }
}

#[test]
fn timestamp_time_fields_are_validated() {
    for bad in [
        "2026-10-22T24:00:00Z",
        "2026-10-22T-1:00:00Z",
        "2026-10-22T+1:00:00Z",
        "2026-10-22T10:60:00Z",
        "2026-10-22T10:-5:00Z",
        "2026-10-22T10:00:60Z",
        "2026-10-22T10:00:00.Z",
        "2026-10-22T10:00:00.1x2Z",
        "2026-10-22T10:00:00.-5Z",
        "2026-10-22T10:00:00",
        "2026-10-22 10:00:00Z",
    ] {
        assert_eq!(ms_of(bad), 0, "{bad}");
    }
}
