use cort_evals::cost_audit::audit;
use serde_json::json;
use std::process::Command;

#[test]
fn repeated_assistant_snapshots_do_not_double_count_usage_or_calls() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let first = json!({
        "type":"assistant", "timestamp":"2026-10-01T00:00:00Z",
        "message": {"id":"m1", "usage": {
            "input_tokens":10,"cache_creation_input_tokens":2,
            "cache_read_input_tokens":3,"output_tokens":4
        }, "content":[{"type":"tool_use","id":"c1","name":"Read",
            "input":{"file_path":"src/lib.rs"}}]}
    });
    let result = json!({
        "type":"user", "timestamp":"2026-10-01T00:00:01Z",
        "message":{"content":[{"type":"tool_result","tool_use_id":"c1",
            "content":"abcd"}]}
    });
    let second = json!({
        "type":"assistant", "timestamp":"2026-10-01T00:00:02Z",
        "message": {"id":"m2", "usage": {
            "input_tokens":5,"cache_creation_input_tokens":0,
            "cache_read_input_tokens":7,"output_tokens":1
        }, "content":[{"type":"tool_use","id":"c2","name":"Read",
            "input":{"file_path":"src/lib.rs"}}]}
    });
    let second_result = json!({
        "type":"user", "timestamp":"2026-10-01T00:00:03Z",
        "message":{"content":[{"type":"tool_result","tool_use_id":"c2",
            "content":"abcd"}]}
    });
    std::fs::write(
        session,
        format!("{first}\n{first}\n{result}\n{second}\n{second_result}\n"),
    )
    .unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    assert_eq!(report["assistant_snapshots"], 3);
    assert_eq!(report["unique_assistant_messages"], 2);
    assert_eq!(report["model_usage"]["input_tokens"], 15);
    assert_eq!(report["tools"]["Read"]["calls"], 2);
    assert_eq!(report["tools"]["Read"]["results"], 2);
    assert_eq!(report["tools"]["Read"]["exact_repeat_reads"], 1);
    assert_eq!(
        report["tools"]["Read"]["exact_repeat_read_estimated_tokens"],
        1
    );
}

#[test]
fn fractional_timestamps_obey_daily_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let at = |id: &str, timestamp: &str, tokens: u64| {
        json!({
            "type":"assistant", "timestamp":timestamp,
            "message":{"id":id, "content":[], "usage":{
                "input_tokens":tokens, "cache_creation_input_tokens":0,
                "cache_read_input_tokens":0, "output_tokens":0
            }}
        })
    };
    std::fs::write(
        session,
        format!(
            "{}\n{}\n{}\n{}\n",
            at("before", "2026-09-30T23:59:59.999Z", 1),
            at("start", "2026-10-01T00:00:00.100Z", 10),
            at("last", "2026-10-01T23:59:59.999Z", 30),
            at("after", "2026-10-02T00:00:00.100Z", 20),
        ),
    )
    .unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    assert_eq!(report["unique_assistant_messages"], 2);
    assert_eq!(report["model_usage"]["input_tokens"], 40);
}

fn assistant(
    id: &str,
    timestamp: &str,
    model: Option<&str>,
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("id".into(), json!(id));
    if let Some(model) = model {
        message.insert("model".into(), json!(model));
    }
    message.insert("content".into(), json!([]));
    message.insert(
        "usage".into(),
        json!({
            "input_tokens": input,
            "cache_creation_input_tokens": cache_creation,
            "cache_read_input_tokens": cache_read,
            "output_tokens": output,
        }),
    );
    json!({"type":"assistant", "timestamp":timestamp, "message":message})
}

fn assert_by_model_sums_match_totals(report: &serde_json::Value) {
    let by_model = report["model_usage"]["by_model"]
        .as_object()
        .expect("by_model must be an object");
    for field in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ] {
        let sum: u64 = by_model
            .values()
            .map(|m| {
                m[field]
                    .as_u64()
                    .unwrap_or_else(|| panic!("{field} missing from a by_model entry"))
            })
            .sum();
        assert_eq!(
            sum,
            report["model_usage"][field].as_u64().unwrap(),
            "{field}: by_model sum must equal the total"
        );
    }
}

#[test]
fn by_model_splits_tokens_and_does_not_recount_repeated_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let first = assistant("m1", "2026-10-01T00:00:00Z", Some("model-a"), 10, 2, 3, 4);
    let second = assistant("m2", "2026-10-01T00:00:02Z", Some("model-a"), 5, 0, 7, 1);
    let third = assistant("m3", "2026-10-01T00:00:03Z", Some("model-b"), 20, 1, 0, 9);
    std::fs::write(session, format!("{first}\n{first}\n{second}\n{third}\n")).unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    let by_model = report["model_usage"]["by_model"].as_object().unwrap();
    assert_eq!(by_model.len(), 2);
    assert_eq!(
        report["model_usage"]["by_model"]["model-a"],
        json!({
            "messages": 2,
            "input_tokens": 15,
            "cache_creation_input_tokens": 2,
            "cache_read_input_tokens": 10,
            "output_tokens": 5,
        })
    );
    assert_eq!(
        report["model_usage"]["by_model"]["model-b"],
        json!({
            "messages": 1,
            "input_tokens": 20,
            "cache_creation_input_tokens": 1,
            "cache_read_input_tokens": 0,
            "output_tokens": 9,
        })
    );
    assert_eq!(report["model_usage"]["input_tokens"], 35);
    assert_by_model_sums_match_totals(&report);
}

#[test]
fn assistant_messages_without_a_model_count_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let known = assistant("m1", "2026-10-01T00:00:00Z", Some("model-a"), 4, 0, 0, 1);
    let missing = assistant("m2", "2026-10-01T00:00:01Z", None, 6, 0, 0, 2);
    std::fs::write(session, format!("{known}\n{missing}\n")).unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    let by_model = report["model_usage"]["by_model"].as_object().unwrap();
    assert_eq!(by_model.len(), 2);
    assert_eq!(
        report["model_usage"]["by_model"]["unknown"],
        json!({
            "messages": 1,
            "input_tokens": 6,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 2,
        })
    );
    assert_eq!(
        report["model_usage"]["by_model"]["model-a"]["input_tokens"],
        4
    );
    assert_by_model_sums_match_totals(&report);
}

#[test]
fn messages_outside_the_window_do_not_enter_by_model() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let outside = assistant(
        "m1",
        "2026-09-30T23:59:59.999Z",
        Some("model-old"),
        1,
        0,
        0,
        0,
    );
    let after = assistant(
        "m2",
        "2026-10-02T00:00:00.100Z",
        Some("model-old"),
        2,
        0,
        0,
        0,
    );
    let inside = assistant("m3", "2026-10-01T12:00:00Z", Some("model-new"), 30, 0, 0, 7);
    std::fs::write(session, format!("{outside}\n{after}\n{inside}\n")).unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    let by_model = report["model_usage"]["by_model"].as_object().unwrap();
    assert_eq!(by_model.len(), 1);
    assert!(by_model.contains_key("model-new"));
    assert!(!by_model.contains_key("model-old"));
    assert_eq!(
        report["model_usage"]["by_model"]["model-new"]["messages"],
        1
    );
    assert_eq!(
        report["model_usage"]["by_model"]["model-new"]["input_tokens"],
        30
    );
    assert_by_model_sums_match_totals(&report);
}

/// The kept usage and its model are one unit: a later snapshot of the same id replaces both, and a
/// later snapshot whose usage cannot be read replaces neither. Identical repeats cannot tell the two
/// apart, so the snapshots here disagree on the model.
#[test]
fn a_later_snapshot_moves_usage_and_model_together() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let early = assistant("m1", "2026-10-01T00:00:00Z", Some("model-a"), 10, 2, 3, 4);
    let kept = assistant("m1", "2026-10-01T00:00:01Z", Some("model-b"), 5, 0, 7, 1);
    let mut unreadable = assistant(
        "m1",
        "2026-10-01T00:00:02Z",
        Some("model-c"),
        99,
        99,
        99,
        99,
    );
    unreadable["message"]["usage"]
        .as_object_mut()
        .unwrap()
        .remove("output_tokens");
    std::fs::write(session, format!("{early}\n{kept}\n{unreadable}\n")).unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    assert_eq!(
        report["model_usage"]["by_model"],
        json!({
            "model-b": {
                "messages": 1,
                "input_tokens": 5,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 7,
                "output_tokens": 1,
            }
        })
    );
    assert_by_model_sums_match_totals(&report);
}

#[test]
fn by_model_token_sums_equal_the_totals_across_model_names() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.jsonl");
    let a = assistant(
        "m1",
        "2026-10-01T00:00:00Z",
        Some("deepseek-v4-1-flash"),
        11,
        2,
        13,
        5,
    );
    let b = assistant(
        "m2",
        "2026-10-01T00:00:01Z",
        Some("deepseek-v4-1-flash-economy"),
        7,
        3,
        2,
        1,
    );
    let synthetic = assistant(
        "m3",
        "2026-10-01T00:00:02Z",
        Some("<synthetic>"),
        0,
        0,
        0,
        0,
    );
    let unknown = assistant("m4", "2026-10-01T00:00:03Z", None, 9, 1, 4, 6);
    std::fs::write(session, format!("{a}\n{b}\n{synthetic}\n{unknown}\n")).unwrap();
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z").unwrap();
    let by_model = report["model_usage"]["by_model"].as_object().unwrap();
    assert_eq!(by_model.len(), 4);
    assert_eq!(by_model["<synthetic>"]["messages"], 1);
    assert_eq!(report["model_usage"]["output_tokens"], 12);
    let messages: u64 = by_model
        .values()
        .map(|m| m["messages"].as_u64().unwrap())
        .sum();
    assert_eq!(messages, report["usage_messages"].as_u64().unwrap());
    assert_by_model_sums_match_totals(&report);
}

#[test]
fn cli_accepts_a_fractional_window_within_one_second() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cort-evals"))
        .args([
            "cost-audit",
            "--since",
            "2026-10-01T00:00:00Z",
            "--until",
            "2026-10-01T00:00:00.100Z",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["window"]["until_utc"], "2026-10-01T00:00:00.100Z");
}
