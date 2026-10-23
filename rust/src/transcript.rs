//! Claude Code transcript parser and compaction windows.
//!
//! Pure: no file IO and no store. A caller reads a transcript file and hands the text in; the
//! counts that go to the measurement store are always derived from [`WindowDetails`], so the
//! rules exist exactly once.

use crate::measure_store::WindowCounts;
use serde_json::Value;
use std::collections::HashSet;

pub const WINDOW: usize = 50;
pub const CORRECTION_TERMS: &[&str] = &[
    "不對",
    "不是",
    "錯了",
    "講錯",
    "搞錯",
    "wrong",
    "that's not",
    "no,",
];
pub const KNOWN_FILE_TOOLS: &[&str] = &["Read", "Write", "Edit", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryMeta {
    pub uuid: String,
    pub session_id: String,
    pub time_ms: i64,
    pub cwd: Option<String>,
    pub trigger: String,
    pub pre_tokens: Option<i64>,
    pub post_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Tool {
        name: String,
        file_path: Option<String>,
        command: Option<String>,
        input: String,
    },
    Human {
        text: String,
    },
    Boundary(BoundaryMeta),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub steps: Vec<Step>,
    pub skipped_lines: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowDetails {
    pub tool_calls: usize,
    pub rereads: Vec<String>,
    pub repeated_commands: Vec<String>,
    pub transcript_lookups: Vec<String>,
    pub human_messages: usize,
    pub corrections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compaction {
    pub boundary: BoundaryMeta,
    pub pre: WindowDetails,
    pub post: WindowDetails,
    pub post_complete: bool,
}

impl WindowDetails {
    pub fn counts(&self) -> WindowCounts {
        WindowCounts {
            tool_calls: self.tool_calls as i64,
            rereads: self.rereads.len() as i64,
            repeated_commands: self.repeated_commands.len() as i64,
            transcript_lookups: self.transcript_lookups.len() as i64,
            human_messages: self.human_messages as i64,
            corrections: self.corrections.len() as i64,
        }
    }
}

pub fn is_correction(text: &str) -> bool {
    let lower = text.to_lowercase();
    CORRECTION_TERMS.iter().any(|t| lower.contains(t))
}

/// Text of a message's content when it is a string or an array of text blocks with no
/// `tool_result`; `None` otherwise.
fn plain_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => out.push(item.get("text").and_then(Value::as_str)?.to_string()),
                    _ => return None,
                }
            }
            Some(out.join("\n"))
        }
        _ => None,
    }
}

fn human_text(line: &Value) -> Option<String> {
    if line.get("type").and_then(Value::as_str) != Some("user")
        || line.get("isCompactSummary").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let content = line.pointer("/message/content")?;
    if let Some(origin) = line.get("origin") {
        if origin.get("kind").and_then(Value::as_str) != Some("human") {
            return None;
        }
        return plain_text(content);
    }
    if line.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let text = content.as_str()?;
    if text.starts_with('<') {
        return None;
    }
    Some(text.to_string())
}

pub fn is_human(line: &Value) -> bool {
    human_text(line).is_some()
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Fixed-width ASCII digits only (no sign, no whitespace).
fn digits(s: &str, width: usize) -> Option<i64> {
    if s.len() != width || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// `2026-10-22T23:37:47.345Z` to epoch milliseconds; 0 when it is not a valid UTC timestamp.
fn parse_time_ms(ts: &str) -> i64 {
    fn go(ts: &str) -> Option<i64> {
        let ts = ts.strip_suffix('Z')?;
        let (date, time) = ts.split_once('T')?;
        let mut d = date.split('-');
        let y = digits(d.next()?, 4)?;
        let m = digits(d.next()?, 2)?;
        let day = digits(d.next()?, 2)?;
        if d.next().is_some() || !(1..=12).contains(&m) || day < 1 || day > days_in_month(y, m) {
            return None;
        }
        let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
        if time.contains('.') && frac.is_empty() {
            return None;
        }
        let mut t = hms.split(':');
        let h = digits(t.next()?, 2)?;
        let mi = digits(t.next()?, 2)?;
        let s = digits(t.next()?, 2)?;
        if t.next().is_some() || h > 23 || mi > 59 || s > 59 {
            return None;
        }
        let ms = if frac.is_empty() {
            0
        } else {
            if !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut f: String = frac.chars().take(3).collect();
            while f.len() < 3 {
                f.push('0');
            }
            f.parse::<i64>().ok()?
        };
        days_from_civil(y, m, day)
            .checked_mul(24)?
            .checked_add(h)?
            .checked_mul(60)?
            .checked_add(mi)?
            .checked_mul(60_000)?
            .checked_add(s * 1000 + ms)
    }
    go(ts).unwrap_or(0)
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn boundary_meta(line: &Value) -> BoundaryMeta {
    let meta = line.get("compactMetadata");
    BoundaryMeta {
        uuid: str_field(line, "uuid").unwrap_or_default(),
        session_id: str_field(line, "sessionId").unwrap_or_default(),
        time_ms: line
            .get("timestamp")
            .and_then(Value::as_str)
            .map(parse_time_ms)
            .unwrap_or(0),
        cwd: str_field(line, "cwd"),
        trigger: meta
            .and_then(|m| str_field(m, "trigger"))
            .unwrap_or_default(),
        pre_tokens: meta
            .and_then(|m| m.get("preTokens"))
            .and_then(Value::as_i64),
        post_tokens: meta
            .and_then(|m| m.get("postTokens"))
            .and_then(Value::as_i64),
    }
}

pub fn parse_transcript(jsonl: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let ends_with_newline = jsonl.ends_with('\n');
    let lines: Vec<&str> = jsonl.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    for (i, raw) in lines.iter().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Value>(raw) else {
            // A writer may be mid-line: an unterminated last line is not damage.
            if i != last || ends_with_newline {
                parsed.skipped_lines += 1;
            }
            continue;
        };
        if line.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        match line.get("type").and_then(Value::as_str) {
            Some("system")
                if line.get("subtype").and_then(Value::as_str) == Some("compact_boundary") =>
            {
                parsed.steps.push(Step::Boundary(boundary_meta(&line)));
            }
            Some("assistant") => {
                let Some(items) = line.pointer("/message/content").and_then(Value::as_array) else {
                    continue;
                };
                for item in items {
                    if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let input = item.get("input").cloned().unwrap_or(Value::Null);
                    parsed.steps.push(Step::Tool {
                        name: str_field(item, "name").unwrap_or_default(),
                        file_path: str_field(&input, "file_path")
                            .or_else(|| str_field(&input, "notebook_path")),
                        command: str_field(&input, "command"),
                        input: input.to_string(),
                    });
                }
            }
            Some("user") => {
                if let Some(text) = human_text(&line) {
                    parsed.steps.push(Step::Human { text });
                }
            }
            _ => {}
        }
    }
    parsed
}

/// Everything known strictly before `start`: file paths touched and Bash commands run.
fn known_before(steps: &[Step], start: usize) -> (HashSet<&str>, HashSet<&str>) {
    let mut files = HashSet::new();
    let mut cmds = HashSet::new();
    for step in &steps[..start] {
        if let Step::Tool {
            name,
            file_path,
            command,
            ..
        } = step
        {
            if KNOWN_FILE_TOOLS.contains(&name.as_str()) {
                if let Some(p) = file_path {
                    files.insert(p.as_str());
                }
            }
            if name == "Bash" {
                if let Some(c) = command {
                    cmds.insert(c.as_str());
                }
            }
        }
    }
    (files, cmds)
}

/// Details for `steps[start..=end]` (inclusive; `end < start` means empty).
fn details(steps: &[Step], start: usize, end: usize) -> WindowDetails {
    let mut d = WindowDetails::default();
    if end < start {
        return d;
    }
    let (known_files, known_cmds) = known_before(steps, start);
    let mut seen_files = HashSet::new();
    let mut seen_cmds = HashSet::new();
    for step in &steps[start..=end] {
        match step {
            Step::Tool {
                name,
                file_path,
                command,
                input,
            } => {
                d.tool_calls += 1;
                if name == "Read" {
                    if let Some(p) = file_path {
                        if known_files.contains(p.as_str()) && seen_files.insert(p.as_str()) {
                            d.rereads.push(p.clone());
                        }
                    }
                }
                if name == "Bash" {
                    if let Some(c) = command {
                        if known_cmds.contains(c.as_str()) && seen_cmds.insert(c.as_str()) {
                            d.repeated_commands.push(c.clone());
                        }
                    }
                }
                if input.contains(".claude/projects/") && input.contains(".jsonl") {
                    d.transcript_lookups.push(input.clone());
                }
            }
            Step::Human { text } => {
                d.human_messages += 1;
                if is_correction(text) {
                    d.corrections.push(text.clone());
                }
            }
            Step::Boundary(_) => {}
        }
    }
    d
}

pub fn compactions(steps: &[Step]) -> Vec<Compaction> {
    let mut out = Vec::new();
    for (b, step) in steps.iter().enumerate() {
        let Step::Boundary(meta) = step else { continue };

        // Post window: after the boundary, up to the WINDOW-th tool, cut at the next boundary.
        let mut tools = 0;
        let mut post_end = b; // inclusive; == b means empty
        for (i, s) in steps.iter().enumerate().skip(b + 1) {
            match s {
                Step::Boundary(_) => break,
                Step::Tool { .. } => {
                    tools += 1;
                    post_end = i;
                    if tools == WINDOW {
                        break;
                    }
                }
                Step::Human { .. } => post_end = i,
            }
        }
        let next_boundary = steps[b + 1..]
            .iter()
            .any(|s| matches!(s, Step::Boundary(_)));
        let post_full = tools == WINDOW;
        let post = details(steps, b + 1, post_end);

        // Pre window: the last WINDOW tools before the boundary, cut at the previous boundary.
        let mut pre_tools = 0;
        let mut pre_start = 0; // after the previous boundary, or file start
        for i in (0..b).rev() {
            match &steps[i] {
                Step::Boundary(_) => {
                    pre_start = i + 1;
                    break;
                }
                Step::Tool { .. } => {
                    pre_tools += 1;
                    if pre_tools == WINDOW {
                        pre_start = i;
                        break;
                    }
                }
                Step::Human { .. } => {}
            }
        }
        let pre = if b == 0 || pre_start >= b {
            WindowDetails::default()
        } else {
            details(steps, pre_start, b - 1)
        };

        out.push(Compaction {
            boundary: meta.clone(),
            pre,
            post,
            post_complete: post_full || next_boundary,
        });
    }
    out
}
