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

/// The callee name out of a possibly-qualified symbol: `Tally::add` -> `add`, `t.run` -> `run`.
///
/// This mirrors `cort::chunker::bare_name`. The duplication is deliberate and the other direction is
/// not: an evaluator that links the product it grades cannot be used to contradict it.
pub fn last_segment(symbol: &str) -> &str {
    symbol
        .rsplit([':', '.'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(symbol)
}

/// Does this line name one of the symbols the dependent is supposed to depend on?
///
/// The point of the column is that one line of source is now enough to check one edge, so the
/// independent checker has to look at exactly that line and nothing else. The same honest
/// limitation as the body check applies: this matches text, so a comment naming the symbol also
/// "confirms" it. It is a screen against fabricated edges, not proof of a call.
pub fn call_site_verdict(line_text: &str, nearer: &[String]) -> Option<String> {
    nearer
        .iter()
        .find(|name| contains_word(line_text, last_segment(name)))
        .cloned()
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

    // Which names a row at hop h is allowed to mention on its call-site line. `parents[h]` is the
    // set the *body* check uses for that hop (parents[1] is the seed itself), so the line check has
    // to be graded against parents[1..=h]: wider, because `impact` picks the earliest call site into
    // "the seeds plus anything nearer", and it records which edge carried the hop only implicitly.
    // Grading narrower would call true edges unfounded -- the first cut did exactly that and read
    // `line_precision=0.0` on a venue where every dependent is real.
    let mut nearer_by_hop: HashMap<i64, Vec<String>> = HashMap::new();
    {
        let mut acc: Vec<String> = Vec::new();
        for hop in 1..=depth {
            if let Some(names) = parents.get(&hop) {
                acc.extend(names.iter().cloned());
            }
            nearer_by_hop.insert(hop, acc.clone());
        }
    }

    let mut body_cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut rows = Vec::new();
    let mut by_hop: Map<String, Value> = Map::new();
    let mut sites_total = 0i64;
    let mut sites_confirmed = 0i64;

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

        // The v4 column: `impact` now names the line it based this row on, so the claim is checkable
        // against one line instead of one function. Absent for a dependency that came in through an
        // import, and for any index built before call sites were recorded.
        let site = d.get("call_site_line").and_then(Value::as_i64);
        let call_site = site.map(|line| {
            let text = lines
                .get(line as usize - 1)
                .map(String::as_str)
                .unwrap_or("")
                .to_string();
            let nearer = nearer_by_hop
                .get(&hop)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let via = call_site_verdict(&text, nearer);
            sites_total += 1;
            if via.is_some() {
                sites_confirmed += 1;
            }
            json!({
                "line": line,
                "form": d.get("call_form").and_then(Value::as_str),
                "confirmed": via.is_some(),
                "via": via,
                "text": text.trim().chars().take(120).collect::<String>(),
            })
        });

        rows.push(json!({
            "hop": hop, "file": file, "symbol": name,
            "confirmed": matched.is_some(), "via": matched,
            "call_site": call_site,
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
        // Kept beside `precision`, never folded into it: the body-level figure is what every
        // recorded run from 2026-08-28 onwards measured, and a stricter check that also renames the
        // old one destroys the comparison it is supposed to strengthen.
        "call_sites": sites_total,
        "call_sites_confirmed": sites_confirmed,
        "line_precision": if sites_total == 0 {
            Value::Null
        } else {
            json!(((sites_confirmed as f64 / sites_total as f64) * 1000.0).round() / 1000.0)
        },
    }))
}
