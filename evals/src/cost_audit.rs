//! Read-only cost census of Claude Code's local JSONL sessions.
//!
//! Assistant messages are snapshots: one message id can occur many times as content is appended.
//! Count usage once per id and tool results once per tool-use id. This is a descriptive census,
//! not a counterfactual token-savings estimate.

use crate::stream::estimate_tokens;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct ToolStats {
    calls: u64,
    results: u64,
    bytes: u64,
    estimated_tokens: u64,
    exact_repeat_reads: u64,
    exact_repeat_read_bytes: u64,
    exact_repeat_read_estimated_tokens: u64,
}

#[derive(Default)]
struct Usage {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
}

impl Usage {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            input: value.get("input_tokens")?.as_u64()?,
            cache_creation: value.get("cache_creation_input_tokens")?.as_u64()?,
            cache_read: value.get("cache_read_input_tokens")?.as_u64()?,
            output: value.get("output_tokens")?.as_u64()?,
        })
    }

    fn add(&mut self, other: &Self) {
        self.input += other.input;
        self.cache_creation += other.cache_creation;
        self.cache_read += other.cache_read;
        self.output += other.output;
    }
}

#[derive(Clone)]
struct Call {
    name: String,
    repeat_read: bool,
    bash_leading_command: Option<String>,
}

#[derive(Default)]
struct Census {
    files_read: u64,
    files_unreadable: u64,
    lines_unparsed: u64,
    sessions_in_window: u64,
    sidechain_files: u64,
    assistant_snapshots: u64,
    unique_assistant_messages: u64,
    usage_messages: u64,
    messages_without_usage: u64,
    tool_results_unmatched: u64,
    total: Usage,
    tools: BTreeMap<String, ToolStats>,
    bash_leading_commands: BTreeMap<String, ToolStats>,
    session_costs: Vec<SessionCost>,
}

#[derive(Default)]
struct SessionCost {
    model_tokens: u64,
    cache_read_tokens: u64,
    tool_result_tokens: u64,
    assistant_messages: u64,
}

fn distribution(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    let n = values.len();
    if n == 0 {
        return json!({"files": 0});
    }
    let total: u64 = values.iter().sum();
    let top_one = n.div_ceil(100);
    let top_ten = n.div_ceil(10);
    json!({
        "files": n,
        "total": total,
        "p50": values[(n - 1) * 50 / 100],
        "p90": values[(n - 1) * 90 / 100],
        "p99": values[(n - 1) * 99 / 100],
        "top_one_percent_files": top_one,
        "top_one_percent_total": values[n - top_one..].iter().sum::<u64>(),
        "top_ten_percent_files": top_ten,
        "top_ten_percent_total": values[n - top_ten..].iter().sum::<u64>(),
    })
}

fn bash_leading_command(script: &str) -> String {
    let mut start = script.trim_start();
    // A common wrapper changes directory before the one command whose output is wanted.
    if start.starts_with("cd ") {
        if let Some((_, rest)) = start.split_once("&&") {
            start = rest.trim_start();
        }
    }
    let word = start.split_whitespace().next().unwrap_or("");
    // Whitelist prevents an absolute executable path, environment assignment, or user text from
    // entering the report. This is the leading command only, not a shell-semantic classifier.
    match word {
        "rg" | "grep" | "ag" | "ast-grep" | "cat" | "sed" | "head" | "tail" | "ls" | "find"
        | "fd" | "cargo" | "git" | "make" | "npm" | "pnpm" | "yarn" | "bun" | "go" | "pytest"
        | "mvn" | "gradle" | "jq" | "awk" | "wc" | "cort" | "claudecat" | "curl" | "wget"
        | "docker" | "python" | "python3" | "node" | "bash" | "sh" | "zsh" | "echo" | "printf"
        | "cp" | "mv" | "rm" | "mkdir" | "touch" | "tee" => word.to_string(),
        _ => "other".to_string(),
    }
}

fn files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, depth - 1, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .or_else(|| part.get("text").and_then(Value::as_str))
            })
            .collect::<String>(),
        _ => String::new(),
    }
}

fn scan_file(path: &Path, since: &str, until: &str, census: &mut Census) {
    let Ok(file) = std::fs::File::open(path) else {
        census.files_unreadable += 1;
        return;
    };
    census.files_read += 1;
    let mut in_window = false;
    let mut usages: HashMap<String, Usage> = HashMap::new();
    let mut messages: HashSet<String> = HashSet::new();
    let mut calls: HashMap<String, Call> = HashMap::new();
    let mut results: HashSet<String> = HashSet::new();
    let mut reads: HashSet<String> = HashSet::new();
    let mut edit_generation = 0u64;
    let mut file_tool_tokens = 0u64;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            census.lines_unparsed += 1;
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            census.lines_unparsed += 1;
            continue;
        };
        let Some(ts) = v.get("timestamp").and_then(Value::as_str) else {
            continue;
        };
        // RFC3339 UTC timestamps in Claude's transcript sort lexically. Both boundaries are
        // required in UTC so offsets cannot make the comparison lie.
        if ts < since || ts >= until {
            continue;
        }
        in_window = true;
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) == Some("assistant") {
            census.assistant_snapshots += 1;
            if let Some(id) = v
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
            {
                messages.insert(id.to_string());
                if let Some(usage) = v
                    .get("message")
                    .and_then(|m| m.get("usage"))
                    .and_then(Usage::from_value)
                {
                    usages.insert(id.to_string(), usage);
                }
            }
            if let Some(items) = content.as_array() {
                for item in items {
                    if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let Some(id) = item.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if calls.contains_key(id) {
                        continue;
                    }
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let bash_leading_command = if name == "Bash" {
                        Some(bash_leading_command(
                            item.get("input")
                                .and_then(|i| i.get("command"))
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                        ))
                    } else {
                        None
                    };
                    let repeat_read = if name == "Read" {
                        let key = format!(
                            "{}:{}",
                            edit_generation,
                            item.get("input").unwrap_or(&Value::Null)
                        );
                        !reads.insert(key)
                    } else {
                        false
                    };
                    if matches!(
                        name.as_str(),
                        "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
                    ) {
                        edit_generation += 1;
                    }
                    census.tools.entry(name.clone()).or_default().calls += 1;
                    calls.insert(
                        id.to_string(),
                        Call {
                            name,
                            repeat_read,
                            bash_leading_command,
                        },
                    );
                }
            }
        } else if v.get("type").and_then(Value::as_str) == Some("user") {
            if let Some(items) = content.as_array() {
                for item in items {
                    if item.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = item.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    if !results.insert(id.to_string()) {
                        continue;
                    }
                    let text = result_text(item.get("content").unwrap_or(&Value::Null));
                    let bytes = text.len() as u64;
                    let tokens = estimate_tokens(&text) as u64;
                    file_tool_tokens += tokens;
                    let (name, repeat) = if let Some(call) = calls.get(id) {
                        (call.name.as_str(), call.repeat_read)
                    } else {
                        census.tool_results_unmatched += 1;
                        ("unmatched", false)
                    };
                    let stats = census.tools.entry(name.to_string()).or_default();
                    stats.results += 1;
                    stats.bytes += bytes;
                    stats.estimated_tokens += tokens;
                    if repeat {
                        stats.exact_repeat_reads += 1;
                        stats.exact_repeat_read_bytes += bytes;
                        stats.exact_repeat_read_estimated_tokens += tokens;
                    }
                    if let Some(prefix) = calls
                        .get(id)
                        .and_then(|call| call.bash_leading_command.as_deref())
                    {
                        let by_prefix = census
                            .bash_leading_commands
                            .entry(prefix.to_string())
                            .or_default();
                        by_prefix.results += 1;
                        by_prefix.bytes += bytes;
                        by_prefix.estimated_tokens += tokens;
                    }
                }
            }
        }
    }
    if in_window {
        census.sessions_in_window += 1;
        if path
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "subagents")
        {
            census.sidechain_files += 1;
        }
    }
    census.unique_assistant_messages += messages.len() as u64;
    census.usage_messages += usages.len() as u64;
    census.messages_without_usage += messages.len().saturating_sub(usages.len()) as u64;
    let mut session = SessionCost {
        tool_result_tokens: file_tool_tokens,
        assistant_messages: messages.len() as u64,
        ..SessionCost::default()
    };
    for usage in usages.values() {
        census.total.add(usage);
        session.model_tokens +=
            usage.input + usage.cache_creation + usage.cache_read + usage.output;
        session.cache_read_tokens += usage.cache_read;
    }
    if in_window {
        census.session_costs.push(session);
    }
}

/// Count observed cost channels. Tool tokens use the same estimator for every tool; model usage
/// is reported separately because cache reads replay old context and cannot be added to a tool
/// result as if each byte entered the model only once.
pub fn audit(dir: &Path, since: &str, until: &str) -> Value {
    let mut paths = Vec::new();
    files(dir, 6, &mut paths);
    paths.sort();
    let mut c = Census::default();
    for path in paths {
        scan_file(&path, since, until, &mut c);
    }
    let tools: BTreeMap<_, _> = c
        .tools
        .iter()
        .map(|(name, s)| {
            (
                name.clone(),
                json!({
                    "calls": s.calls,
                    "results": s.results,
                    "result_bytes": s.bytes,
                    "estimated_result_tokens": s.estimated_tokens,
                    "exact_repeat_reads": s.exact_repeat_reads,
                    "exact_repeat_read_bytes": s.exact_repeat_read_bytes,
                    "exact_repeat_read_estimated_tokens": s.exact_repeat_read_estimated_tokens,
                }),
            )
        })
        .collect();
    let estimated_tool_tokens: u64 = c.tools.values().map(|s| s.estimated_tokens).sum();
    let bash_leading_commands: BTreeMap<_, _> = c
        .bash_leading_commands
        .iter()
        .map(|(name, s)| {
            (
                name.clone(),
                json!({
                    "results": s.results,
                    "result_bytes": s.bytes,
                    "estimated_result_tokens": s.estimated_tokens,
                }),
            )
        })
        .collect();
    json!({
        "window": {"since_utc": since, "until_utc": until},
        "files_read": c.files_read,
        "files_unreadable": c.files_unreadable,
        "lines_unparsed": c.lines_unparsed,
        "session_files_in_window": c.sessions_in_window,
        "sidechain_files_in_window": c.sidechain_files,
        "assistant_snapshots": c.assistant_snapshots,
        "unique_assistant_messages": c.unique_assistant_messages,
        "usage_messages": c.usage_messages,
        "messages_without_usage": c.messages_without_usage,
        "tool_results_unmatched": c.tool_results_unmatched,
        "model_usage": {
            "input_tokens": c.total.input,
            "cache_creation_input_tokens": c.total.cache_creation,
            "cache_read_input_tokens": c.total.cache_read,
            "output_tokens": c.total.output,
        },
        "estimated_tool_result_tokens": estimated_tool_tokens,
        "session_file_distribution": {
            "model_tokens": distribution(c.session_costs.iter().map(|s| s.model_tokens).collect()),
            "cache_read_tokens": distribution(c.session_costs.iter().map(|s| s.cache_read_tokens).collect()),
            "tool_result_tokens": distribution(c.session_costs.iter().map(|s| s.tool_result_tokens).collect()),
            "assistant_messages": distribution(c.session_costs.iter().map(|s| s.assistant_messages).collect()),
        },
        "tools": tools,
        "bash_leading_commands": bash_leading_commands,
        "reading": "Descriptive local Claude Code census. Assistant usage is deduplicated by message id within each transcript file; result payloads by tool-use id. Session-file distribution is per JSONL file, including subagent sidechains, not per user task. Tool-result tokens are an ASCII/4 + non-ASCII*1 estimate, not provider billing tokens. Model usage is provider-reported, but sums include repeated context across turns. Bash leading command is a whitelisted first word after an optional cd && wrapper, not a shell-semantic classifier; compound commands can include other work. Exact repeat Read means identical Read input after no observed Edit/Write tool in the same file; Bash writes and external changes are not detected, so it is a candidate, not proven avoidable cost. This report does not estimate savings or grade task success."
    })
}

#[cfg(test)]
mod tests {
    use super::{bash_leading_command, distribution};

    #[test]
    fn leading_command_skips_a_cd_wrapper_and_redacts_unfamiliar_words() {
        assert_eq!(bash_leading_command("cd repo && rg foo"), "rg");
        assert_eq!(bash_leading_command("/private/path/tool arg"), "other");
    }

    #[test]
    fn distribution_exposes_a_one_file_heavy_tail() {
        let mut values = vec![1; 99];
        values.push(1000);
        let d = distribution(values);
        assert_eq!(d["files"], 100);
        assert_eq!(d["total"], 1099);
        assert_eq!(d["top_one_percent_files"], 1);
        assert_eq!(d["top_one_percent_total"], 1000);
    }
}
