//! Paired full-task comparisons for two separate token-saving hypotheses.
//!
//! The input is a measured row file, not a simulator. A compact-output row can never be pooled
//! with a handoff row: they change different things and a pooled mean would prove neither.

use serde_json::{json, Value};
use std::collections::HashSet;

const USAGE_FIELDS: [&str; 4] = [
    "input_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "output_tokens",
];

fn nonempty<'a>(v: &'a Value, field: &str, where_at: &str) -> Result<&'a str, String> {
    v.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{where_at}.{field} must be a nonempty string"))
}

fn measured_u64(v: &Value, field: &str, where_at: &str) -> Result<u64, String> {
    v.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{where_at}.{field} must be a measured nonnegative integer"))
}

#[derive(Default)]
struct Totals {
    cost_usd: f64,
    usage: [u64; 4],
    turns: u64,
    successes: u64,
}

impl Totals {
    fn add(&mut self, row: &Value, where_at: &str) -> Result<bool, String> {
        let cost = row
            .get("cost_usd")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or_else(|| format!("{where_at}.cost_usd must be measured and nonnegative"))?;
        let turns = measured_u64(row, "turns", where_at)?;
        let success = row
            .get("success")
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("{where_at}.success must be graded true or false"))?;
        nonempty(row, "grade_evidence", where_at)?;
        let trace_hash = nonempty(row, "trace_sha256", where_at)?;
        if trace_hash.len() != 64 || !trace_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{where_at}.trace_sha256 must be 64 hex characters"));
        }
        let usage = row
            .get("usage")
            .ok_or_else(|| format!("{where_at}.usage is missing"))?;
        let measured: Vec<u64> = USAGE_FIELDS
            .iter()
            .map(|field| measured_u64(usage, field, &format!("{where_at}.usage")))
            .collect::<Result<_, _>>()?;
        self.cost_usd += cost;
        self.turns += turns;
        self.successes += u64::from(success);
        for (sum, n) in self.usage.iter_mut().zip(measured) {
            *sum += n;
        }
        Ok(success)
    }

    fn total_tokens(&self) -> u64 {
        self.usage.iter().sum()
    }

    fn json(&self) -> Value {
        json!({
            "cost_usd": self.cost_usd,
            "total_tokens": self.total_tokens(),
            "turns": self.turns,
            "successes": self.successes,
            "usage": {
                "input_tokens": self.usage[0],
                "cache_creation_input_tokens": self.usage[1],
                "cache_read_input_tokens": self.usage[2],
                "output_tokens": self.usage[3],
            }
        })
    }
}

/// The input file declares exactly one hypothesis and pairs the same task under the same model
/// and venue head. Rows with null usage, missing provenance, or protocol violations are refused.
pub fn summarize(doc: &Value) -> Result<Value, String> {
    let hypothesis = nonempty(doc, "hypothesis", "trial")?;
    if !matches!(hypothesis, "handoff" | "compact_output") {
        return Err("trial.hypothesis must be handoff or compact_output".to_string());
    }
    if doc.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err("trial.schema must be 1".to_string());
    }
    let pairs = doc
        .get("pairs")
        .and_then(Value::as_array)
        .ok_or_else(|| "trial.pairs must be an array".to_string())?;
    let mut seen = HashSet::new();
    let (mut baseline, mut treatment) = (Totals::default(), Totals::default());
    let (mut both_success, mut baseline_only, mut treatment_only) = (0u64, 0u64, 0u64);
    let mut cheaper_pairs = 0u64;
    for (i, pair) in pairs.iter().enumerate() {
        let at = format!("pairs[{i}]");
        let task = nonempty(pair, "task_id", &at)?;
        let task_version = nonempty(pair, "task_version", &at)?;
        if !seen.insert((task.to_string(), task_version.to_string())) {
            return Err(format!("duplicate task/version {task}/{task_version}"));
        }
        let b = pair
            .get("baseline")
            .ok_or_else(|| format!("{at}.baseline is missing"))?;
        let t = pair
            .get("treatment")
            .ok_or_else(|| format!("{at}.treatment is missing"))?;
        for field in ["model", "venue_head"] {
            let bv = nonempty(b, field, &format!("{at}.baseline"))?;
            let tv = nonempty(t, field, &format!("{at}.treatment"))?;
            if bv != tv {
                return Err(format!("{at}: {field} differs between arms"));
            }
        }
        if t.get("intervention_delivered").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "{at}.treatment.intervention_delivered must be true"
            ));
        }
        let bs = baseline.add(b, &format!("{at}.baseline"))?;
        let ts = treatment.add(t, &format!("{at}.treatment"))?;
        match (bs, ts) {
            (true, true) => both_success += 1,
            (true, false) => baseline_only += 1,
            (false, true) => treatment_only += 1,
            (false, false) => {}
        }
        if t["cost_usd"].as_f64().unwrap() < b["cost_usd"].as_f64().unwrap() {
            cheaper_pairs += 1;
        }
    }
    let paired = pairs.len() as u64;
    let state = if paired < 20 {
        "insufficient_pairs"
    } else if treatment.successes < baseline.successes {
        "quality_regressed"
    } else if treatment.cost_usd >= baseline.cost_usd
        || treatment.total_tokens() >= baseline.total_tokens()
    {
        "cost_not_lower"
    } else {
        "eligible_for_review"
    };
    Ok(json!({
        "hypothesis": hypothesis,
        "pairs": paired,
        "baseline": baseline.json(),
        "treatment": treatment.json(),
        "both_success": both_success,
        "baseline_only_success": baseline_only,
        "treatment_only_success": treatment_only,
        "cheaper_pairs": cheaper_pairs,
        "state": state,
        "reading": "One hypothesis per report. A pair must use the same task/version, model and venue head and provide measured provider usage, cost, turns, trace hash and grading evidence. Fewer than 20 pairs never pass the screen. eligible_for_review means only that summed tokens and cost fell with no aggregate success-count loss; inspect per-task outcomes, order effects and independent labels before claiming causal savings. The handoff must include its summary and extra calls in treatment usage. Do not combine this report with another hypothesis."
    }))
}
