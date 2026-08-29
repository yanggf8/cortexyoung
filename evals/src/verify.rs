//! Independent check of an `impact` answer: the graph supplies the hypothesis, the file text decides.
//!
//! For every dependent at hop k, its own body (start_line..end_line, straight off disk) must contain
//! a word-boundary reference to at least one symbol from hop k-1. This is what stopped a label file
//! from being circular: cort proposes, the source code adjudicates.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// `\b<name>\b` without pulling in a regex crate.
pub fn contains_word(haystack: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = name.chars().collect();
    if bytes.len() < needle.len() {
        return false;
    }
    for start in 0..=(bytes.len() - needle.len()) {
        if bytes[start..start + needle.len()] != needle[..] {
            continue;
        }
        let before_ok = start == 0 || !is_word_char(bytes[start - 1]);
        let after_index = start + needle.len();
        let after_ok = after_index == bytes.len() || !is_word_char(bytes[after_index]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

pub fn run_cort_json(cort: &str, repo: &str, args: &[&str]) -> Result<Value, String> {
    let out = std::process::Command::new(cort)
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("{cort}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cort exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cort output is not JSON: {e}"))
}

pub fn verify_impact(cort: &str, repo: &str, symbol: &str, depth: i64) -> Result<Value, String> {
    let payload = run_cort_json(
        cort,
        repo,
        &["impact", "--symbol", symbol, "--depth", &depth.to_string()],
    )?;
    let empty: Vec<Value> = Vec::new();
    let deps: Vec<Value> = payload
        .get("dependents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| empty.clone());
    let dependents: Vec<&Value> = deps.iter().collect();

    // parents[1] = the seed; parents[k] = the distinct symbols reported at hop k-1.
    let mut parents: HashMap<i64, Vec<String>> = HashMap::new();
    parents.insert(1, vec![symbol.to_string()]);
    for h in 2..=depth {
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for d in &dependents {
            if d.get("hop").and_then(Value::as_i64) == Some(h - 1) {
                if let Some(n) = d.get("symbol_name").and_then(Value::as_str) {
                    if !n.is_empty() && seen.insert(n.to_string()) {
                        names.push(n.to_string());
                    }
                }
            }
        }
        parents.insert(h, names);
    }

    let mut body_cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut rows = Vec::new();
    let mut by_hop: Map<String, Value> = Map::new();

    for d in &dependents {
        let file = d
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let start = d.get("start_line").and_then(Value::as_i64).unwrap_or(0);
        let end = d.get("end_line").and_then(Value::as_i64).unwrap_or(0);
        let hop = d.get("hop").and_then(Value::as_i64).unwrap_or(0);
        let name = d
            .get("symbol_name")
            .and_then(Value::as_str)
            .map(str::to_string);

        if !body_cache.contains_key(&file) {
            let abs = Path::new(repo).join(&file);
            let text =
                std::fs::read_to_string(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
            body_cache.insert(file.clone(), text.split('\n').map(str::to_string).collect());
        }
        let lines = &body_cache[&file];
        let from = (start.max(1) as usize) - 1;
        let to = (end as usize).min(lines.len());
        let body = if from < to {
            lines[from..to].join("\n")
        } else {
            String::new()
        };

        let mut matched: Option<String> = None;
        for candidate in parents.get(&hop).cloned().unwrap_or_default() {
            if contains_word(&body, &candidate) {
                matched = Some(candidate);
                break;
            }
        }

        let entry = by_hop
            .entry(hop.to_string())
            .or_insert_with(|| json!({ "total": 0, "confirmed": 0 }));
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(
                "total".into(),
                json!(obj["total"].as_i64().unwrap_or(0) + 1),
            );
            if matched.is_some() {
                obj.insert(
                    "confirmed".into(),
                    json!(obj["confirmed"].as_i64().unwrap_or(0) + 1),
                );
            }
        }

        rows.push(json!({
            "hop": hop, "file": file, "symbol": name,
            "confirmed": matched.is_some(), "via": matched,
        }));
    }

    let confirmed = rows
        .iter()
        .filter(|r| r["confirmed"] == json!(true))
        .count();
    let unconfirmed: Vec<String> = rows
        .iter()
        .filter(|r| r["confirmed"] != json!(true))
        .map(|r| {
            let label = r
                .get("symbol")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| r["file"].as_str().unwrap_or("?").to_string());
            format!("h{}:{}", r["hop"].as_i64().unwrap_or(0), label)
        })
        .collect();

    Ok(json!({
        "symbol": symbol,
        "depth": depth,
        "seed_count": payload.get("seeds").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "dependents": rows.len(),
        "by_hop": by_hop,
        "confirmed": confirmed,
        "precision": if rows.is_empty() {
            Value::Null
        } else {
            json!(((confirmed as f64 / rows.len() as f64) * 1000.0).round() / 1000.0)
        },
        "unconfirmed": unconfirmed,
    }))
}
