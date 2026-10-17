use cort_evals::cost_audit::audit;
use serde_json::json;

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
    let report = audit(dir.path(), "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z");
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
