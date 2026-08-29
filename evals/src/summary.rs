//! Aggregate cells into the stop/go verdict.
//!
//! The rule the JS version broke: `mean([null])` is NaN in JS and NaN serialises back to null, so
//! three rounds of unmeasured metrics still produced a verdict. Here a metric that was not
//! measured stays absent from the average, is counted in `metrics_missing`, and the verdict refuses
//! to compare on it — `strict` mode refuses the whole aggregate outright.

use serde_json::{json, Map, Value};

pub const ARMS: [&str; 3] = ["rg+Read", "ast-grep+Read", "cort"];
/// The metrics the current runner actually measures. `stale_reads` is deliberately **not** here:
/// it was defined as "the agent acted on content that no longer matched disk when it answered",
/// which needs a disk-vs-answer comparison the runner never implemented, and the archived rows
/// carry it as null. Listing it made `--strict` unconditionally fatal, i.e. a check that can never
/// pass is not a check. The per-arm `stale_reads` field stays (null + counted as missing) so
/// historical rows still aggregate honestly.
pub const METRICS: [&str; 4] = ["total_tokens", "tool_return_tokens", "turns", "read_calls"];

const AVERAGED: [(&str, &str); 4] = [
    ("mean_total_tokens", "total_tokens"),
    ("mean_tool_return_tokens", "tool_return_tokens"),
    ("mean_turns", "turns"),
    ("mean_read_calls", "read_calls"),
];

fn as_number(row: &Value, key: &str) -> Option<f64> {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }
}

/// Which arm `cort` is judged against. Rounds 1-3 compared against `ast-grep+Read`; the current
/// runner only drives `rg+Read`, so a gate that could only see the old baseline reported
/// "metric-missing" on perfectly good data. Preference order, first arm actually present wins.
pub const BASELINE_PREFERENCE: [&str; 2] = ["ast-grep+Read", "rg+Read"];

pub fn summarize(rows: &[Value], strict: bool) -> Result<Value, String> {
    let mut by_arm = Map::new();

    for arm in ARMS {
        let mine: Vec<&Value> = rows
            .iter()
            .filter(|r| r.get("arm").and_then(Value::as_str) == Some(arm))
            .collect();
        if mine.is_empty() {
            continue;
        }
        let mut obj = Map::new();
        let mut missing = Map::new();
        obj.insert("runs".into(), json!(mine.len()));

        let successes = mine
            .iter()
            .filter(|r| r.get("success") == Some(&Value::Bool(true)))
            .count();
        obj.insert(
            "success_rate".into(),
            json!((successes as f64 / mine.len() as f64 * 1000.0).round() / 1000.0),
        );

        for (out, key) in AVERAGED {
            let values: Vec<f64> = mine.iter().filter_map(|r| as_number(r, key)).collect();
            let miss = mine.len() - values.len();
            if values.is_empty() {
                obj.insert(out.into(), Value::Null);
            } else {
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                obj.insert(out.into(), json!((mean * 1000.0).round() / 1000.0));
            }
            if miss > 0 {
                missing.insert(key.into(), json!(miss));
            }
        }

        let stale: Vec<f64> = mine
            .iter()
            .filter_map(|r| as_number(r, "stale_reads"))
            .collect();
        // Reported, never gated: no driver produces this field yet (see METRICS). Making an
        // unmeasurable field a strict failure turns `--strict` into a check that can never pass,
        // which quietly trains people to stop running it.
        obj.insert(
            "stale_reads".into(),
            if stale.len() == mine.len() {
                json!(stale.iter().sum::<f64>())
            } else {
                Value::Null
            },
        );
        obj.insert("stale_reads_measured".into(), json!(stale.len()));

        obj.insert("metrics_missing".into(), Value::Object(missing.clone()));
        if strict && !missing.is_empty() {
            return Err(format!(
                "{arm}: refusing to summarise unmeasured metrics: {missing:?}"
            ));
        }
        by_arm.insert(arm.into(), Value::Object(obj));
    }

    let baseline_arm = BASELINE_PREFERENCE
        .iter()
        .find(|arm| by_arm.contains_key(**arm))
        .map(|s| s.to_string());
    let base = baseline_arm
        .as_ref()
        .and_then(|arm| by_arm.get(arm).and_then(Value::as_object));
    let cort = by_arm.get("cort").and_then(Value::as_object);
    let comparable = base
        .zip(cort)
        .map(|(b, c)| {
            matches!(
                (b.get("mean_total_tokens"), c.get("mean_total_tokens")),
                (Some(Value::Number(_)), Some(Value::Number(_)))
            )
        })
        .unwrap_or(false);

    let beats = comparable
        && cort
            .unwrap()
            .get("mean_total_tokens")
            .and_then(Value::as_f64)
            .unwrap()
            < base
                .unwrap()
                .get("mean_total_tokens")
                .and_then(Value::as_f64)
                .unwrap()
        && cort
            .unwrap()
            .get("success_rate")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            >= base
                .unwrap()
                .get("success_rate")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);

    Ok(json!({
        "by_arm": by_arm,
        "verdict": {
            "baseline_arm": baseline_arm,
            "cort_beats_ast_grep": beats,
            "reason": if comparable {
                "compared on mean_total_tokens + success_rate"
            } else {
                "metric-missing: no comparison possible"
            },
            "next_action": if beats {
                "continue to deferred features"
            } else {
                "STOP: do not add features until cort wins"
            },
        }
    }))
}
