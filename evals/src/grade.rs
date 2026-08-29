//! Score one agent answer against a task's labels.
//!
//! Free prose cannot be scored for precision: you can tell whether an expected name appears, but
//! not which of the other identifiers in the paragraph were meant as an answer. Both arms are
//! therefore asked, in identical words, to end with a machine-readable block; the tool set is the
//! only thing that differs between arms.

use serde_json::Value;
use std::collections::BTreeMap;

pub const ANSWER_CONTRACT: &str =
    "End your reply with a fenced block in exactly this form, one line per function,\n\
the symbol name and its hop distance from the seed separated by a tab:\n\
\n\
```answer\n\
symbolName\t1\n\
otherSymbol\t2\n\
```\n\
\n\
List every function you found and nothing else. If you are unsure of a distance, still list\n\
the symbol.";

/// Fixed before any cell ran, so a disappointing number cannot move it afterwards.
pub const GATE_COVERAGE: f64 = 0.9;
pub const GATE_PRECISION: f64 = 0.7;

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub prompt: String,
    pub venue: String,
    pub seed_symbol: String,
    pub expected_symbols: Vec<String>,
    pub by_hop: BTreeMap<i64, Vec<String>>,
}

pub fn load_tasks(path: &str) -> Result<Vec<Task>, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let doc: Value = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
    let items = doc
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{path}: no tasks array"))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let mut by_hop = BTreeMap::new();
        if let Some(map) = item.get("by_hop").and_then(Value::as_object) {
            for (hop, names) in map {
                if let Ok(h) = hop.parse::<i64>() {
                    if let Some(list) = names.as_array() {
                        by_hop.insert(
                            h,
                            list.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect(),
                        );
                    }
                }
            }
        }
        out.push(Task {
            id: str_field(item, "id"),
            prompt: str_field(item, "prompt"),
            venue: str_field(item, "venue"),
            seed_symbol: str_field(item, "seed_symbol"),
            expected_symbols: item
                .get("expected_symbols")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            by_hop,
        });
    }
    Ok(out)
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The last ```answer block wins, so an agent that repeats itself is graded on its final answer.
fn last_block(text: &str) -> Option<String> {
    let mut found: Option<String> = None;
    let mut rest = text;
    while let Some(start) = rest.find("```answer") {
        let after = &rest[start + "```answer".len()..];
        let body = match after.find("\n```") {
            Some(end) => &after[..end + 1],
            None => break,
        };
        let consumed = body.len();
        found = Some(body.to_string());
        rest = &after[consumed..];
    }
    found
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// `^([A-Za-z_$][\w$]*)(?:\s+h?(\d+))?\s*$` after stripping a leading list marker.
fn parse_line(raw: &str) -> Option<(String, Option<i64>)> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .unwrap_or(trimmed)
        .trim();
    if stripped.is_empty() {
        return None;
    }
    let chars: Vec<char> = stripped.chars().collect();
    let mut i = 0usize;
    if !matches!(chars[0], c if c.is_ascii_alphabetic() || c == '_' || c == '$') {
        return None;
    }
    while i < chars.len() && is_ident_char(chars[i]) {
        i += 1;
    }
    let symbol: String = chars[..i].iter().collect();
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    if i == chars.len() {
        return Some((symbol, None));
    }
    if chars[i] == 'h' {
        i += 1;
    }
    let digits_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    let hop: i64 = chars[digits_start..i]
        .iter()
        .collect::<String>()
        .parse()
        .ok()?;
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    if i != chars.len() {
        return None;
    }
    Some((symbol, Some(hop)))
}

fn round3(n: f64, d: f64) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    ((n / d) * 1000.0).round() / 1000.0
}

#[derive(Debug, Clone, PartialEq)]
pub struct WrongHop {
    pub symbol: String,
    pub said: Option<i64>,
    pub actual: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graded {
    pub answer_block: bool,
    pub answered_symbols: Vec<String>,
    pub covered_symbols: Vec<String>,
    pub spurious_symbols: Vec<String>,
    pub coverage: f64,
    pub precision: f64,
    pub hop_accuracy: f64,
    pub wrong_hop: Vec<WrongHop>,
    pub success: bool,
}

pub fn grade_answer(answer_text: &str, task: &Task) -> Graded {
    let body = last_block(answer_text);
    let rows: Vec<(String, Option<i64>)> = body
        .as_deref()
        .map(|b| b.lines().filter_map(parse_line).collect())
        .unwrap_or_default();

    // First mention wins, so repeating a name cannot inflate or deflate either score. Insertion
    // order is kept, because `answered_symbols` is read by a human deciding whether the arm's
    // answer was reasonable.
    let mut order: Vec<String> = Vec::new();
    let mut said: BTreeMap<String, Option<i64>> = BTreeMap::new();
    for (symbol, hop) in rows {
        said.entry(symbol.clone()).or_insert_with(|| {
            order.push(symbol.clone());
            hop
        });
    }

    let mut hop_of: BTreeMap<String, Option<i64>> = BTreeMap::new();
    for (hop, names) in &task.by_hop {
        for n in names {
            hop_of.insert(n.clone(), Some(*hop));
        }
    }

    let answered: Vec<String> = order;
    let covered: Vec<String> = task
        .expected_symbols
        .iter()
        .filter(|s| said.contains_key(s.as_str()))
        .cloned()
        .collect();
    let spurious: Vec<String> = answered
        .iter()
        .filter(|s| !task.expected_symbols.iter().any(|e| e == *s))
        .cloned()
        .collect();

    let wrong_hop: Vec<WrongHop> = covered
        .iter()
        .filter(|s| said.get(s.as_str()).copied() != hop_of.get(s.as_str()).copied())
        .map(|s| WrongHop {
            symbol: s.clone(),
            said: said.get(s.as_str()).copied().flatten(),
            actual: hop_of.get(s.as_str()).copied().flatten(),
        })
        .collect();

    let coverage = round3(covered.len() as f64, task.expected_symbols.len() as f64);
    let precision = round3(covered.len() as f64, answered.len() as f64);
    let hop_accuracy = round3(
        (covered.len() - wrong_hop.len()) as f64,
        covered.len() as f64,
    );

    Graded {
        answer_block: body.is_some(),
        coverage,
        precision,
        hop_accuracy,
        answered_symbols: answered,
        covered_symbols: covered,
        spurious_symbols: spurious,
        wrong_hop,
        success: coverage >= GATE_COVERAGE && precision >= GATE_PRECISION,
    }
}
