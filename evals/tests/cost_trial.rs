use cort_evals::cost_trial::summarize;
use serde_json::{json, Value};

fn row(cost: f64, success: bool) -> Value {
    json!({
        "model":"same-model", "venue_head":"same-head", "success":success,
        "cost_usd":cost, "turns":3, "grade_evidence":"independent label v1",
        "trace_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "usage": {
            "input_tokens":10,"cache_creation_input_tokens":20,
            "cache_read_input_tokens":30,"output_tokens":40
        },
        "intervention_delivered":true
    })
}

fn trial(hypothesis: &str) -> Value {
    json!({
        "schema":1,"hypothesis":hypothesis,
        "pairs":[{"task_id":"task-a","task_version":"v1",
            "baseline":row(1.0,true),"treatment":row(0.5,true)}]
    })
}

#[test]
fn a_single_pair_is_preliminary_even_when_cheap_and_correct() {
    let report = summarize(&trial("handoff")).unwrap();
    assert_eq!(report["hypothesis"], "handoff");
    assert_eq!(report["state"], "insufficient_pairs");
    assert_eq!(report["baseline"]["total_tokens"], 100);
    assert_eq!(report["cheaper_pairs"], 1);
}

#[test]
fn a_trial_cannot_pool_hypotheses_or_omit_provenance() {
    assert!(summarize(&trial("handoff+compact_output")).is_err());
    let mut doc = trial("compact_output");
    doc["pairs"][0]["treatment"]["trace_sha256"] = Value::Null;
    assert!(summarize(&doc).unwrap_err().contains("trace_sha256"));
}

#[test]
fn a_different_model_or_null_token_metric_refuses_the_pair() {
    let mut doc = trial("handoff");
    doc["pairs"][0]["treatment"]["model"] = json!("different-model");
    assert!(summarize(&doc).unwrap_err().contains("model differs"));
    let mut doc = trial("handoff");
    doc["pairs"][0]["treatment"]["usage"]["cache_read_input_tokens"] = Value::Null;
    assert!(summarize(&doc)
        .unwrap_err()
        .contains("cache_read_input_tokens"));
}

#[test]
fn lower_cost_with_more_failures_never_passes_review() {
    let mut doc = trial("handoff");
    doc["pairs"] = Value::Array(
        (0..20)
            .map(|n| {
                json!({"task_id":format!("task-{n}"),"task_version":"v1",
                    "baseline":row(1.0,true),"treatment":row(0.5,false)})
            })
            .collect(),
    );
    let report = summarize(&doc).unwrap();
    assert_eq!(report["state"], "quality_regressed");
    assert_eq!(report["baseline_only_success"], 20);
}
