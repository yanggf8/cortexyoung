//! Demand side: how often does a real session need a caller set at all?
//!
//! The cost side is measured (`run-agents`, `summarize`): when a multi-hop relationship walk is
//! asked for, an agent holding `cort` reaches the labelled answer for roughly a seventh of the
//! tool payload of an agent holding a shell, and the shell arm gets it wrong in six cells out of
//! ten. Cost per use is settled. What was never measured on data that still exists is **how often
//! the walk is wanted** — and the earlier answer to that ("zero relational questions out of 1,565
//! real prompts") was computed from transcripts Claude Code's retention has since deleted, so it
//! cannot be re-checked.
//!
//! This module is that re-check, made repeatable. It reads the local agent transcripts of both
//! drivers and counts genuine user instructions, because a count of *questions* is the wrong
//! denominator for a tool whose real job is to cheapen work the agent does unprompted. The unit
//! here is the instruction, split into the two ways a caller set gets wanted:
//!
//! * `ask`  — the answer *is* the relationship ("who calls X", blast radius), and
//! * `task` — the instruction cannot be done correctly without enumerating call sites first
//!   (rename, delete dead code, migrate, extract, replace, move).
//!
//! Everything else in this file exists to stop the mistake this measurement made the first time.
//! The dominant instruction pattern in these logs is `review` followed by a **pasted agent
//! report**, and that report is thick with the words "refactor", "impact" and "callers". Naive
//! needle matching over raw user messages reported a 7.9% relational rate on this corpus; after
//! the paste is stripped the same corpus reports 0.2%. `own_words` is the whole difference between
//! those two numbers, which is why it is the most heavily tested function here.
//!
//! Every hit is emitted with the needles that fired and an excerpt of the instruction, and `--show`
//! prints them: a demand number nobody can audit is exactly how the unverifiable conclusion got
//! written down in the first place. The excerpt is **redacted and truncated** (`scrub`), because
//! `report.json` is committed next to the eval rows and this repo publishes no developer paths —
//! `AGENTS.md` forbids them, and the `runs/` policy keeps venue text out of the tree. So the tool,
//! not the person running it, is what makes the artefact safe to check in.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Recorded into the artefact: the needle lists and `own_words` will change, and a rate means
/// nothing without the version that produced it.
pub const DEMAND_METHOD: &str = "demand-v1";

/// A message longer than this is not somebody typing an instruction; it is a document or a pasted
/// report arriving as one message. Inherited from the archived measurement so the two samples stay
/// comparable.
pub const MAX_PROMPT_CHARS: usize = 2000;

/// Below this many characters what remains is a verb, not an instruction.
pub const MIN_INSTRUCTION_CHARS: usize = 8;

/// The answer itself is a relationship set.
pub const NEEDLES_ASK: &[&str] = &[
    "誰呼叫",
    "呼叫者",
    "誰在用",
    "誰會用到",
    "影響範圍",
    "波及",
    "呼叫鏈",
    "調用鏈",
    "上游",
    "下游",
    "依賴鏈",
    "相依關係",
    "相依圖",
    "依賴圖",
    "跨檔案",
    "blast radius",
    "who calls",
    "what calls",
    "callers",
    "call sites",
    "usages of",
    "where is used",
    "depends on",
    "dependency graph",
    "dependency chain",
];

/// Doable only after the call sites are known. Deliberately broader than "the user typed
/// 'refactor'": these are the verbs whose failure mode is a missed caller — the thing `cort`
/// removes — whether or not anybody asks who the callers are.
pub const NEEDLES_TASK: &[&str] = &[
    "改名",
    "重構",
    "移除",
    "刪掉",
    "刪除",
    "淘汰",
    "抽取",
    "提取",
    "遷移",
    "替換",
    "換成",
    "搬",
    "rename",
    "refactor",
    "delete",
    "deleted",
    "migrate",
    "delete",
    "dead code",
    "all callers",
    "every usage",
    "extract",
    "move to",
    "replace",
];

/// From here on the text is somebody else's output arriving inside the user's message: blockquotes
/// and fenced blocks (pasted reports), table rows, the two bullet glyphs the agents actually emit,
/// and the first-person voice a user instruction does not need.
const PASTE_MARKERS: &[&str] = &[
    "\n>",
    "\n```",
    "\n|",
    "\u{25cf}",
    "\u{273d}",
    "I've",
    "I have ",
    "Here's ",
    "Here are ",
    "Summary of",
    "Summary:",
    "\u{2705}",
    "\u{274c}",
    "\u{250c}",
    "## ",
    "Review verdict",
    "Implementation Complete",
    "Work Summary",
    "Pushed ",
    "Changes Made",
    "Task Report",
];

/// A message whose entire content is "look at this report" is not an instruction that needs a
/// caller set: the vocabulary belongs to the pasted body, not to the request.
const BARE_DIRECTIVES: &[&str] = &[
    "review",
    "review works",
    "corroborate",
    "validate",
    "advise me",
    "verify root cause",
];

/// The corpus is zh-TW, but a simplified `影响范围` is the same question. Normalising the handful
/// of variant characters that appear in the needles costs one pass and removes a bias that points
/// the wrong way: a missed needle *under-counts* demand, which is the conclusion this project would
/// most like to get wrong.
const VARIANT_PAIRS: &[(char, char)] = &[
    ('\u{8c01}', '\u{8ab0}'), // 谁 -> 誰
    ('\u{54cd}', '\u{97ff}'), // 响 -> 響
    ('\u{8303}', '\u{7bc4}'), // 范 -> 範
    ('\u{56f4}', '\u{570d}'), // 围 -> 圍
    ('\u{8c03}', '\u{8abf}'), // 调 -> 調
    ('\u{5173}', '\u{95dc}'), // 关 -> 關
    ('\u{7cfb}', '\u{4fc2}'), // 系 -> 係
    ('\u{8fc1}', '\u{9077}'), // 迁 -> 遷
    ('\u{6362}', '\u{63db}'), // 换 -> 換
    ('\u{5220}', '\u{522a}'), // 删 -> 刪
    ('\u{6784}', '\u{69cb}'), // 构 -> 構
    ('\u{94fe}', '\u{93c8}'), // 链 -> 鏈
    ('\u{56fe}', '\u{5716}'), // 图 -> 圖
    ('\u{4e49}', '\u{7fa9}'), // 义 -> 義
];

fn normalize_variants(text: &str) -> String {
    text.chars()
        .map(|c| {
            VARIANT_PAIRS
                .iter()
                .find(|(from, _)| *from == c)
                .map_or(c, |(_, to)| *to)
        })
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Substring match for phrase needles, with word boundaries on both ends when the needle starts or
/// ends with ASCII. `verify::contains_word` handles single identifiers; these are multi-word.
fn needle_hit(haystack_lower: &str, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let ascii_edges = needle
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && needle
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    let mut from = 0usize;
    while let Some(offset) = haystack_lower[from..].find(&needle) {
        let start = from + offset;
        let end = start + needle.len();
        if !ascii_edges {
            return true;
        }
        // Boundary on the leading edge only: `rename` has to catch "renamed" and "rename it to",
        // and the trailing check bought nothing that the hand audit of `matches` does not cover.
        let before_ok =
            start == 0 || !is_word_char(haystack_lower[..start].chars().next_back().unwrap());
        if before_ok {
            return true;
        }
        from = end;
    }
    false
}

pub fn matched_needles(text: &str, needles: &[&'static str]) -> Vec<&'static str> {
    let lower = normalize_variants(text).to_lowercase();
    needles
        .iter()
        .cloned()
        .filter(|n| needle_hit(&lower, n))
        .collect()
}

/// Drop everything that is pasted agent output rather than the user's own words. Returns "" when
/// nothing of theirs survives — that is what a pure `review <report>` message must score as.
///
/// All offsets are converted to char counts: these messages mix CJK and ASCII, so a byte slice at
/// a marker offset would panic on a character boundary.
pub fn own_words(raw: &str) -> String {
    let trimmed = raw.trim();
    let total = trimmed.chars().count();
    if total == 0 {
        return String::new();
    }
    // Prefixing the newline lets a marker that *opens* the message (a pasted blockquote, a table
    // row, a fenced block) cut at position zero instead of being missed because its leading \n
    // was trimmed away.
    let probe = format!("\n{trimmed}");
    let mut cut = total;
    for marker in PASTE_MARKERS {
        if let Some(index) = probe.find(marker) {
            // probe[..index] is "\n" followed by the user's own leading text, so the character
            // count minus that synthetic newline is where the user's words stop.
            cut = cut.min(probe[..index].chars().count().saturating_sub(1));
        }
    }
    let head = trimmed.chars().take(cut).collect::<String>();
    let mut head = head.trim();
    // A short opening line followed by a multi-line body is the shape of "review this", where the
    // body is somebody else's report. The markers above catch the voices they know; this catches
    // the ones they do not ("我已完成您要求的架構", "Why You're Getting the Warning"), and it errs
    // by *discarding* the user's message rather than by crediting paste to them.
    if head.lines().count() > 1 {
        let first = head.lines().next().unwrap_or("").trim();
        let words = first.split_whitespace().count();
        if !first.is_empty() && first.chars().count() <= 40 && words <= 5 {
            head = first;
        }
    }
    if head.chars().count() < MIN_INSTRUCTION_CHARS {
        return String::new();
    }
    let lowered = head.to_lowercase();
    if BARE_DIRECTIVES.iter().any(|d| *d == lowered) {
        return String::new();
    }
    for directive in BARE_DIRECTIVES {
        let prefix = format!("{} ", directive);
        if !lowered.starts_with(&prefix) {
            continue;
        }
        let tail = head
            .chars()
            .skip(prefix.chars().count())
            .collect::<String>();
        let tail = tail.trim();
        if tail.chars().count() < MIN_INSTRUCTION_CHARS {
            return String::new();
        }
        // Whatever follows the verb is itself pasted output, e.g. "review \u{25cf} all changes
        // complete …" once the verb has been removed.
        if PASTE_MARKERS
            .iter()
            .any(|m| !m.trim().is_empty() && tail.starts_with(m.trim()))
        {
            return String::new();
        }
        // Re-run the whole routine on the tail: a second paste marker can sit behind the verb.
        let stripped = own_words(tail);
        if stripped.is_empty() {
            return String::new();
        }
        return stripped;
    }
    head.to_string()
}

pub fn classify(text: &str) -> Option<(&'static str, Vec<&'static str>)> {
    let ask = matched_needles(text, NEEDLES_ASK);
    if !ask.is_empty() {
        return Some(("ask", ask));
    }
    let task = matched_needles(text, NEEDLES_TASK);
    if !task.is_empty() {
        return Some(("task", task));
    }
    None
}

/// Injection and command traffic is not a human typing: system prompts, `<...>` envelopes, slash
/// commands and the AGENTS.md block all arrive as `role: user`.
fn is_injection(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with('<') || head.starts_with('/') || head.starts_with("# AGENTS.md instructions")
}

fn text_of_content(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = Vec::new();
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str);
                if matches!(kind, Some("text") | Some("input_text")) {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        out.push(t.to_string());
                    }
                }
            }
            (!out.is_empty()).then(|| out.join("\n"))
        }
        _ => None,
    }
}

fn project_of_cwd(cwd: &str) -> String {
    let leaf = Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    // A session whose cwd is the account's home directory has no project: label it structurally
    // rather than publishing the username as if it were a repo name.
    match home_user() {
        Some(user) if leaf == user => "<user>".to_string(),
        _ => leaf,
    }
}

/// One Claude Code transcript line. `content` must be a plain string: a `user` line whose content
/// is a list is a `tool_result` coming back, never a human instruction.
pub fn claude_user_line(line: &str) -> Option<(String, String)> {
    let e: Value = serde_json::from_str(line).ok()?;
    if e.get("type").and_then(Value::as_str)? != "user" {
        return None;
    }
    if e.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if e.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let source = e.get("promptSource").and_then(Value::as_str)?;
    if source != "typed" && source != "queued" {
        return None;
    }
    let message = e.get("message")?;
    if !matches!(message.get("content"), Some(Value::String(_))) {
        return None;
    }
    let text = text_of_content(message.get("content")?)?;
    if text.is_empty() || text.chars().count() > MAX_PROMPT_CHARS || is_injection(&text) {
        return None;
    }
    let project = e
        .get("cwd")
        .and_then(Value::as_str)
        .map(project_of_cwd)
        .unwrap_or_else(|| "unknown".to_string());
    Some((project, text))
}

/// One Codex rollout line: `payload` is a `message` with `role: user`.
pub fn codex_user_line(line: &str) -> Option<String> {
    let e: Value = serde_json::from_str(line).ok()?;
    let payload = e.get("payload")?;
    if payload.get("type").and_then(Value::as_str)? != "message" {
        return None;
    }
    if payload.get("role").and_then(Value::as_str)? != "user" {
        return None;
    }
    let text = text_of_content(payload.get("content")?)?;
    if text.is_empty() || text.chars().count() > MAX_PROMPT_CHARS || is_injection(&text) {
        return None;
    }
    Some(text)
}

/// Older rollouts have no `session_meta` at all, which is why the project stays "unknown" rather
/// than being guessed from the file name.
pub fn codex_cwd_line(line: &str) -> Option<String> {
    let e: Value = serde_json::from_str(line).ok()?;
    if e.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    e.get("payload")?.get("cwd")?.as_str().map(project_of_cwd)
}

fn jsonl_files(dir: &Path, depth: usize) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 1 {
                out.extend(jsonl_files(&path, depth - 1));
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    out
}

/// Longest excerpt kept per hit, in characters.
pub const MAX_EXCERPT_CHARS: usize = 160;

/// Strip anything that looks like a path on somebody's machine, and any URL. The instruction text
/// is what makes a committed artefact auditable, but an absolute path or a personal worker hostname
/// is exactly what this repo promises not to carry.
///
/// A slash only starts a path when it opens a token (`/home/dev/x`, `~/x`, `https://host/y`), so
/// ordinary prose like "rg/grep" and repo-relative paths like `evals/Cargo.toml` survive intact:
/// over-redacting would destroy the very thing the excerpt is here to prove.
const TRAILING_PUNCT: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\u{201d}'];
const LEADING_PUNCT: &[char] = &['(', '[', '{', '"', '\u{201c}', '\u{300c}', '<'];

/// One whitespace-separated token: a URL or an absolute path becomes a placeholder, and the
/// punctuation glued to it survives so the sentence still reads.
fn scrub_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    let lead = chars
        .iter()
        .take_while(|c| LEADING_PUNCT.contains(c))
        .count();
    let trail = chars
        .iter()
        .rev()
        .take_while(|c| TRAILING_PUNCT.contains(c))
        .count();
    let core_end = chars.len().saturating_sub(trail);
    if lead >= core_end {
        return token.to_string();
    }
    let prefix: String = chars[..lead].iter().collect();
    let core: String = chars[lead..core_end].iter().collect();
    let suffix: String = chars[core_end..].iter().collect();
    let replaced = if core.contains("://") {
        "<url>"
    } else if core.starts_with('/') || core.starts_with("~/") {
        "<path>"
    } else {
        core.as_str()
    };
    format!("{prefix}{replaced}{suffix}")
}

/// Strip anything that looks like a path on somebody's machine, and any URL. The instruction text
/// is what makes a committed artefact auditable, but an absolute path or a personal worker hostname
/// is exactly what this repo promises not to carry.
///
/// Only token-leading slashes count, so "rg/grep" and `evals/Cargo.toml` survive: over-redacting
/// would destroy the very thing the excerpt exists to let you check.
pub fn scrub(text: &str) -> String {
    text.split_whitespace()
        .map(scrub_token)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The developer's own account name, used as a redaction target: `project_of_cwd` labels the home
/// directory with its basename, which is a username rather than a repo.
fn home_user() -> Option<String> {
    std::env::var("HOME").ok().and_then(|h| {
        Path::new(&h)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    })
}

/// Replace the account name wherever it appears as a whole token, keeping any punctuation glued to
/// it. Only exact matches count, so a repo or a symbol that merely contains the name is untouched.
pub fn scrub_user(text: &str) -> String {
    let Some(user) = home_user().filter(|u| !u.is_empty()) else {
        return text.to_string();
    };
    text.split_whitespace()
        .map(|token| {
            let core = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if core == user {
                token.replace(user.as_str(), "<user>")
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Excerpt that is safe to commit: paths, URLs and the account name out, then truncated on a
/// character boundary.
pub fn excerpt(text: &str) -> String {
    scrub_user(&scrub(text))
        .chars()
        .take(MAX_EXCERPT_CHARS)
        .collect()
}

#[derive(Default)]
struct Tally {
    usable: usize,
    skipped_pure_paste: usize,
    excluded: usize,
    per_project: BTreeMap<String, (usize, usize, usize)>,
    matches: Vec<Value>,
}

impl Tally {
    fn add(&mut self, project: &str, raw: &str) {
        let own = own_words(raw);
        if own.is_empty() {
            self.skipped_pure_paste += 1;
            return;
        }
        self.usable += 1;
        let entry = self.per_project.entry(project.to_string()).or_default();
        entry.0 += 1;
        if let Some((class, needles)) = classify(&own) {
            if class == "ask" {
                entry.1 += 1;
            } else {
                entry.2 += 1;
            }
            self.matches.push(json!({
                "project": project,
                "class": class,
                "needles": needles,
                "instruction": excerpt(&own),
            }));
        }
    }

    fn report(&self, files: usize, unreadable: usize, exclude: &[String]) -> Value {
        let ask = self.matches.iter().filter(|m| m["class"] == "ask").count();
        let task = self.matches.iter().filter(|m| m["class"] == "task").count();
        let share = |n: usize| {
            if self.usable == 0 {
                json!(null)
            } else {
                json!((n as f64 * 10000.0 / self.usable as f64).round() / 10000.0)
            }
        };
        json!({
            "method": DEMAND_METHOD,
            "max_prompt_chars": MAX_PROMPT_CHARS,
            "files": { "read": files, "unreadable": unreadable },
            "usable_instructions": self.usable,
            // The count of messages that were only pasted agent output. This is the number that
            // made a naive keyword pass report 7.9%: it must stay visible in the artefact.
            "dropped_as_pure_paste": self.skipped_pure_paste,
            "excluded_instructions": self.excluded,
            "ask": { "count": ask, "share_of_instructions": share(ask) },
            "task": { "count": task, "share_of_instructions": share(task) },
            "task_per_ask": if ask > 0 { json!((task as f64 / ask as f64 * 10.0).round() / 10.0) } else { json!(null) },
            // Echoed back through the same redaction as everything else: this is operator
            // input, and a username typed into --exclude must not come back as a username.
            "excluded_projects": exclude.iter().map(|e| scrub_user(e)).collect::<Vec<_>>(),
            "by_project": self.per_project.iter().map(|(p, (u, a, t))| json!({
                "project": p, "usable": u, "ask": a, "task": t
            })).collect::<Vec<_>>(),
            "matches": self.matches,
            "reading": format!(
                "{} usable user instructions produced {} ask and {} task hits. Each hit carries its \
                 needles and the instruction verbatim: reject the false positives by hand before \
                 quoting either number.",
                self.usable, ask, task
            ),
        })
    }
}

/// Scan both transcript trees. `exclude` names projects the caller decides are not code work; they
/// are counted and listed in the artefact rather than silently dropped.
pub fn scan(
    claude_dir: Option<&Path>,
    codex_dir: Option<&Path>,
    exclude: &[String],
) -> Result<Value, String> {
    if claude_dir.is_none() && codex_dir.is_none() {
        return Err(
            "demand needs at least one readable transcript tree (--claude-dir / --codex-dir)"
                .to_string(),
        );
    }
    let mut tally = Tally::default();
    let mut files = 0usize;
    let mut unreadable = 0usize;

    if let Some(dir) = claude_dir {
        for file in jsonl_files(dir, 2) {
            files += 1;
            let Ok(raw) = std::fs::read_to_string(&file) else {
                unreadable += 1;
                continue;
            };
            for line in raw.lines() {
                if let Some((project, text)) = claude_user_line(line) {
                    if exclude.iter().any(|x| x == &project) {
                        tally.excluded += 1;
                        continue;
                    }
                    tally.add(&project, &text);
                }
            }
        }
    }
    if let Some(dir) = codex_dir {
        for file in jsonl_files(dir, 4) {
            files += 1;
            let Ok(raw) = std::fs::read_to_string(&file) else {
                unreadable += 1;
                continue;
            };
            let mut project = "unknown".to_string();
            for line in raw.lines() {
                if let Some(cwd) = codex_cwd_line(line) {
                    project = cwd;
                }
                if let Some(text) = codex_user_line(line) {
                    if exclude.iter().any(|x| x == &project) {
                        tally.excluded += 1;
                        continue;
                    }
                    tally.add(&project, &text);
                }
            }
        }
    }
    if tally.usable == 0 {
        return Err(format!(
            "no usable user instructions in {} files — that is not a measurement, refuse to report a rate",
            files
        ));
    }
    Ok(tally.report(files, unreadable, exclude))
}
