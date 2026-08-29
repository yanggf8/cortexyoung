//! One `claude -p --output-format stream-json --verbose` transcript -> the row the gate needs.
//!
//! Rounds 1-3 recorded `tool_return_tokens` and `read_calls` as null in all 30 cells and still
//! printed a verdict. Every metric here is either measured or the parse is an error — never null.

use serde_json::Value;

/// Same estimator for both arms. ASCII is ~4 characters per token; a wide (non-ASCII) character is
/// its own token. Dividing CJK by 4 would under-price the payload of whichever arm reads source
/// with Chinese comments in it, i.e. it would flatter the baseline.
pub fn estimate_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut wide = 0usize;
    for ch in text.chars() {
        if (ch as u32) < 128 {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    ascii.div_ceil(4) + wide
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub turns: i64,
    pub hit_turn_cap: bool,
    pub tool_calls: Vec<ToolCall>,
    pub read_calls: i64,
    pub tool_return_tokens: i64,
    pub tool_return_bytes: i64,
    pub input_tokens: i64,
    pub cache_creation: i64,
    pub cache_read: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub permission_denials: Vec<Value>,
    pub cost_usd: Value,
    pub session_id: String,
    pub answer_text: String,
}

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|b| match b {
                Value::String(s) => s.clone(),
                other => other
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn number(usage: &Value, key: &str) -> Result<i64, String> {
    let v = usage
        .get(key)
        .ok_or_else(|| format!("usage.{key} is not a number: refusing to record a null metric"))?;
    if let Some(i) = v.as_i64() {
        return Ok(i);
    }
    if let Some(u) = v.as_u64() {
        return i64::try_from(u)
            .map_err(|_| format!("usage.{key} is not a number: refusing to record a null metric"));
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() && f == f.trunc() {
            return Ok(f as i64);
        }
    }
    Err(format!(
        "usage.{key} is not a number: refusing to record a null metric"
    ))
}

/// Parse an NDJSON transcript. `Err` means the cell produced no usable measurement, which is never
/// recorded as a zero.
pub fn parse_stream(ndjson: &str) -> Result<Parsed, String> {
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut return_bytes = 0i64;
    let mut return_tokens = 0i64;
    let mut result: Option<Value> = None;

    for line in ndjson.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("transcript line is not JSON: {e}"))?;
        match event.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                if let Some(blocks) = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            tool_calls.push(ToolCall {
                                name: block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                command: block
                                    .get("input")
                                    .and_then(|i| i.get("command"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                }
            }
            Some("user") => {
                if let Some(blocks) = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                            continue;
                        }
                        let text = tool_result_text(block.get("content").unwrap_or(&Value::Null));
                        return_bytes += text.len() as i64;
                        return_tokens += estimate_tokens(&text) as i64;
                    }
                }
            }
            Some("result") => result = Some(event),
            _ => {}
        }
    }

    let result = result
        .ok_or_else(|| "transcript has no result event: the cell did not finish".to_string())?;
    let usage = result
        .get("usage")
        .filter(|u| !u.is_null())
        .ok_or_else(|| {
            "result event carries no usage: refusing to record a null metric".to_string()
        })?;

    let input_tokens = number(usage, "input_tokens")?;
    let cache_creation = number(usage, "cache_creation_input_tokens")?;
    let cache_read = number(usage, "cache_read_input_tokens")?;
    let output_tokens = number(usage, "output_tokens")?;

    Ok(Parsed {
        turns: number(&result, "num_turns").unwrap_or(0),
        hit_turn_cap: result.get("subtype").and_then(Value::as_str) == Some("error_max_turns"),
        read_calls: tool_calls.iter().filter(|c| c.name == "Read").count() as i64,
        tool_return_tokens: return_tokens,
        tool_return_bytes: return_bytes,
        input_tokens,
        cache_creation,
        cache_read,
        output_tokens,
        total_tokens: input_tokens + cache_creation + cache_read + output_tokens,
        permission_denials: result
            .get("permission_denials")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        cost_usd: result.get("total_cost_usd").cloned().unwrap_or(Value::Null),
        session_id: result
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        answer_text: result
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tool_calls,
    })
}
