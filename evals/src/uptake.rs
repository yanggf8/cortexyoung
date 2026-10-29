//! Mechanical candidates for uptake of Claude Code's SessionStart memory context.
//! Text-bearing detail is written only to an explicitly private directory.
use crate::{demand, time};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const UPTAKE_METHOD: &str = "uptake-v1";
const OLD_HEADER: &str = "cort memory: local replica (may be behind the shared database; `cort memory recall \"<topic>\"` refreshes)";
const HEADER: &str = cort::hook_recall::HEADER;
const NOTES_HEADER: &str = "unreviewed harness notes from other machines (not checked against source; read one with `cort memory notes show <id>`):";
const NOTICE_PREFIX: &str = "cort memory: the shared database could not be read at session start (";

fn is_memory_context(text: &str) -> bool {
    text.starts_with(HEADER) || text.starts_with(OLD_HEADER)
}
const CORRECTIONS: &[&str] = &[
    "不是",
    "不對",
    "錯了",
    "你忘了",
    "之前說過",
    "我們決定",
    "記錯",
    "no,",
    "wrong",
    "we decided",
    "i told you",
];
const REVIEWER_PROMPT: &str = include_str!("../prompts/memory-uptake-reviewer.md");
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[derive(Clone, Debug)]
struct Entry {
    key: String,
    claim: String,
    source: String,
    line: String,
    evidence_status: String,
    partial: bool,
    tokens: BTreeSet<String>,
}
#[derive(Clone, Debug)]
struct Note {
    id: String,
    file_name: String,
}
#[derive(Default)]
struct Injection {
    entries: Vec<Entry>,
    notes: Vec<Note>,
    entries_left_out: bool,
    notes_left_out: bool,
}
#[derive(Default)]
struct Counts {
    windows: usize,
    sessions: BTreeSet<String>,
    session_entries: BTreeSet<(String, String)>,
    u1_consumption: usize,
    u1_consumption_windows: usize,
    u1_status: usize,
    u1_status_windows: usize,
    u1_contribution: usize,
    u1_contribution_windows: usize,
    u2: usize,
    u2_windows: usize,
    u2_lookup: usize,
    u2_lookup_windows: usize,
    u3: usize,
    u3_windows: usize,
    m1: usize,
    m1_windows: usize,
    m2: usize,
    m2_windows: usize,
    m2_listed: usize,
    m2_listed_windows: usize,
    m3: usize,
    m3_windows: usize,
    partial_entries: usize,
    entries_left_out: usize,
    notes_left_out: usize,
    missing_instructions: usize,
    u2_entries: BTreeSet<(String, String)>,
    m1_entries: BTreeSet<(String, String)>,
}
impl Counts {
    fn add(&mut self, other: &Self) {
        self.windows += other.windows;
        self.sessions.extend(other.sessions.iter().cloned());
        self.session_entries
            .extend(other.session_entries.iter().cloned());
        self.u1_consumption += other.u1_consumption;
        self.u1_consumption_windows += other.u1_consumption_windows;
        self.u1_status += other.u1_status;
        self.u1_status_windows += other.u1_status_windows;
        self.u1_contribution += other.u1_contribution;
        self.u1_contribution_windows += other.u1_contribution_windows;
        self.u2 += other.u2;
        self.u2_windows += other.u2_windows;
        self.u2_lookup += other.u2_lookup;
        self.u2_lookup_windows += other.u2_lookup_windows;
        self.u3 += other.u3;
        self.u3_windows += other.u3_windows;
        self.m1 += other.m1;
        self.m1_windows += other.m1_windows;
        self.m2 += other.m2;
        self.m2_windows += other.m2_windows;
        self.m2_listed += other.m2_listed;
        self.m2_listed_windows += other.m2_listed_windows;
        self.m3 += other.m3;
        self.m3_windows += other.m3_windows;
        self.partial_entries += other.partial_entries;
        self.entries_left_out += other.entries_left_out;
        self.notes_left_out += other.notes_left_out;
        self.missing_instructions += other.missing_instructions;
        self.u2_entries.extend(other.u2_entries.iter().cloned());
        self.m1_entries.extend(other.m1_entries.iter().cloned());
    }
    fn json(&self) -> Value {
        let rate = |events: usize, windows: usize| json!({"events":events,"windows":windows,"n":self.windows,"rate": if self.windows < 5 { Value::String("-".into()) } else { json!(windows as f64 / self.windows as f64) }});
        json!({"windows":self.windows,"distinct_sessions":self.sessions.len(),
            "distinct_session_entries":self.session_entries.len(),
            "u1":{"consumption":rate(self.u1_consumption,self.u1_consumption_windows),"status":rate(self.u1_status,self.u1_status_windows),"contribution":rate(self.u1_contribution,self.u1_contribution_windows)},
            "u2":rate(self.u2,self.u2_windows),"u2_lookup":rate(self.u2_lookup,self.u2_lookup_windows),"u3":rate(self.u3,self.u3_windows),
            "m1":rate(self.m1,self.m1_windows),"m2":rate(self.m2,self.m2_windows),"m2_listed_note":rate(self.m2_listed,self.m2_listed_windows),"m3":rate(self.m3,self.m3_windows),
            "u2_distinct_session_entries":self.u2_entries.len(),"m1_distinct_session_entries":self.m1_entries.len(),
            "partial_entries":self.partial_entries,"entries_left_out_windows":self.entries_left_out,
            "notes_left_out_windows":self.notes_left_out,"no_instruction_file_windows":self.missing_instructions})
    }
}
fn hash(salt: &[u8; 32], value: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(value.as_bytes());
    h.finalize()[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn new_salt() -> Result<[u8; 32], String> {
    let mut salt = [0; 32];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut salt))
        .map_err(|e| format!("random salt: {e}"))?;
    Ok(salt)
}
/// Private detail is readable text, so it is owner-only, including a file an earlier run left
/// with wider permissions.
pub(crate) fn private_file(path: &Path, create_new: bool) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).mode(0o600);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    let file = options.open(path)?;
    owner_only(path)?;
    Ok(file)
}
fn owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}
/// Refuse unless git positively says `dir` is outside every repository.
fn outside_git(dir: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|e| format!("git worktree check: {e}"))?;
    if out.status.success() {
        // `false` is the inside of a .git directory: still a repository, still refused.
        return Err("--private directory is inside a git worktree".into());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.contains("not a git repository") {
        return Err(format!(
            "--private: git could not confirm the directory is outside a git worktree: {}",
            stderr.trim()
        ));
    }
    Ok(())
}
/// The check runs on the nearest existing ancestor before anything is created, so a refusal
/// leaves nothing behind, and again on the created directory, which a symlink may have moved.
pub(crate) fn private_dir(path: &Path) -> Result<PathBuf, String> {
    let mut probe = path;
    let existing = loop {
        if probe.exists() {
            break probe.to_path_buf();
        }
        match probe.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => probe = parent,
            _ => break PathBuf::from("."),
        }
    };
    let existing = fs::canonicalize(&existing).map_err(|e| format!("private directory: {e}"))?;
    outside_git(&existing)?;
    fs::create_dir_all(path).map_err(|e| format!("private directory: {e}"))?;
    let dir = fs::canonicalize(path).map_err(|e| format!("private directory: {e}"))?;
    outside_git(&dir)?;
    Ok(dir)
}
fn salt_for(dir: Option<&Path>) -> Result<[u8; 32], String> {
    let Some(dir) = dir else {
        return new_salt();
    };
    let path = dir.join("salt");
    if path.exists() {
        owner_only(&path).map_err(|e| format!("salt: {e}"))?;
        let raw = fs::read_to_string(&path).map_err(|e| format!("salt: {e}"))?;
        let raw = raw.trim().as_bytes();
        if raw.len() != 64 || !raw.iter().all(u8::is_ascii_hexdigit) {
            return Err("salt must contain 64 hex characters".into());
        }
        let nibble = |c: u8| (c as char).to_digit(16).map_or(0, |d| d as u8);
        let mut out = [0; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = nibble(raw[i * 2]) << 4 | nibble(raw[i * 2 + 1]);
        }
        return Ok(out);
    }
    let salt = new_salt()?;
    let mut file = private_file(&path, true).map_err(|e| format!("salt: {e}"))?;
    for b in salt {
        write!(file, "{b:02x}").map_err(|e| format!("salt: {e}"))?;
    }
    writeln!(file).map_err(|e| format!("salt: {e}"))?;
    Ok(salt)
}
fn tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_tick = false;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '`' {
            if in_tick && i > start {
                out.insert(text[start..i].to_ascii_lowercase());
            }
            in_tick = !in_tick;
            start = i + 1;
        }
    }
    for word in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.len() >= 7
            && word.bytes().all(|b| b.is_ascii_hexdigit())
            && word
                .bytes()
                .any(|b| (b'a'..=b'f').contains(&b.to_ascii_lowercase()))
        {
            out.insert(word.to_ascii_lowercase());
        }
    }
    out
}
fn parse_injection(text: &str) -> Injection {
    let mut result = Injection::default();
    let mut notes = false;
    for line in text.lines().skip(1) {
        if line == NOTES_HEADER {
            notes = true;
            continue;
        }
        if line.starts_with("... ") && line.ends_with(" entries left out") {
            result.entries_left_out = true;
            continue;
        }
        if line.starts_with("... ") && line.ends_with(" more notes") {
            result.notes_left_out = true;
            continue;
        }
        if notes {
            if let Some(body) = line.strip_prefix("- ") {
                if let Some((id, rest)) = body.split_once(" [") {
                    if let Some((_, rest)) = rest.split_once("] ") {
                        if let Some((name, _)) = rest.rsplit_once(" (") {
                            result.notes.push(Note {
                                id: id.into(),
                                file_name: name.into(),
                            });
                        }
                    }
                }
            }
        } else if let Some(body) = line.strip_prefix("- [") {
            if let Some((key, rest)) = body.split_once("] ") {
                let partial = line.ends_with(" ...");
                let (claim, source, source_line, status) =
                    if let Some((claim, tail)) = rest.rsplit_once(" (") {
                        let tail = tail.trim_end_matches(')');
                        if let Some((location, status)) = tail.rsplit_once(", ") {
                            if let Some((source, source_line)) = location.rsplit_once(':') {
                                (claim, source, source_line, status)
                            } else {
                                (rest, "", "", "")
                            }
                        } else {
                            (rest, "", "", "")
                        }
                    } else {
                        (rest.trim_end_matches(" ..."), "", "", "")
                    };
                let mut entry_tokens = tokens(line);
                if !source.is_empty() {
                    entry_tokens.insert(format!("{source}:{source_line}").to_ascii_lowercase());
                }
                result.entries.push(Entry {
                    key: key.into(),
                    claim: claim.into(),
                    source: source.into(),
                    line: source_line.into(),
                    evidence_status: status.into(),
                    partial,
                    tokens: entry_tokens,
                });
            }
        }
    }
    result
}

fn attachment(e: &Value) -> Option<&Value> {
    (e.get("type")?.as_str()? == "attachment")
        .then(|| e.get("attachment"))
        .flatten()
        .filter(|a| {
            a.get("type").and_then(Value::as_str) == Some("hook_additional_context")
                && a.get("hookEvent").and_then(Value::as_str) == Some("SessionStart")
        })
}
fn stamp(e: &Value) -> Option<i64> {
    time::parse_since(e.get("timestamp")?.as_str()?).ok()
}
fn hook_source(e: &Value) -> Option<&str> {
    if !session_hook_success(e) {
        return None;
    }
    e.get("attachment")?
        .get("hookName")?
        .as_str()?
        .strip_prefix("SessionStart:")
}
fn hook_success(e: &Value) -> bool {
    e.get("type").and_then(Value::as_str) == Some("attachment")
        && e["attachment"]["type"].as_str() == Some("hook_success")
}
fn session_hook_success(e: &Value) -> bool {
    hook_success(e) && e["attachment"]["hookEvent"].as_str() == Some("SessionStart")
}
/// A token test on the hook's command line: the program's file name is `cort` and the next word
/// is `hook-recall`, whatever directory the installer put it in.
fn runs_hook_recall(e: &Value) -> bool {
    let words: Vec<_> = e["attachment"]["command"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    words
        .windows(2)
        .any(|w| program(w[0]) == "cort" && w[1] == "hook-recall")
}
fn row_type(e: &Value) -> &str {
    e.get("type").and_then(Value::as_str).unwrap_or("")
}
/// Every string leaf of a JSON value, so a token is matched as text and never through JSON
/// escaping.
fn leaves(v: &Value) -> String {
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::String(s) => {
                out.push_str(s);
                out.push('\n');
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(v, &mut out);
    out
}
/// One turn's speaker, text, and tool id: the call's own id on a `tool_call`, the id it answers on
/// a `tool_result`. Text is trimmed at the end and empty turns are dropped here, so the scanner's
/// turn counter and the packet's turn numbers are one sequence.
type Part = (&'static str, String, Option<String>);
fn event_texts(e: &Value) -> Vec<Part> {
    let content = &e["message"]["content"];
    let id = |v: &Value| v.as_str().map(String::from);
    let mut out: Vec<Part> = Vec::new();
    match row_type(e) {
        "user" => {
            if let Some((_, raw)) = demand::claude_user_line(&e.to_string()) {
                out.push(("user", demand::own_words(&raw), None));
            } else if let Some(s) = content.as_str() {
                // Text the harness put in a user row: summaries, command output, reminders.
                out.push(("harness", s.into(), None));
            } else if let Some(parts) = content.as_array() {
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("tool_result") => out.push((
                            "tool_result",
                            leaves(&part["content"]),
                            id(&part["tool_use_id"]),
                        )),
                        Some("text") => {
                            if let Some(s) = part["text"].as_str() {
                                out.push(("harness", s.into(), None));
                            }
                        }
                        _ => {}
                    }
                }
            } else if !content.is_null() {
                out.push(("harness", leaves(content), None));
            }
        }
        "assistant" => {
            if let Some(parts) = content.as_array() {
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text" | "thinking") => {
                            if let Some(s) = part
                                .get("text")
                                .or_else(|| part.get("thinking"))
                                .and_then(Value::as_str)
                            {
                                let kind = if part["type"] == "thinking" {
                                    "thinking"
                                } else {
                                    "assistant_text"
                                };
                                out.push((kind, s.into(), None));
                            }
                        }
                        // The same parts `tool_calls` keeps, so the two stay aligned.
                        Some("tool_use") => {
                            if let (Some(name), Some(input)) = (
                                part["name"].as_str().filter(|n| !n.is_empty()),
                                part.get("input"),
                            ) {
                                out.push((
                                    "tool_call",
                                    format!("{name} {}", leaves(input)),
                                    id(&part["id"]),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            } else if let Some(s) = content.as_str() {
                out.push(("assistant_text", s.into(), None));
            }
        }
        _ => {}
    }
    out.into_iter()
        .filter_map(|(kind, text, id)| {
            let text = text.trim_end();
            (!text.is_empty()).then(|| (kind, text.to_string(), id))
        })
        .collect()
}
/// What the model could read in a row, lowercased once per file. Hook stdout is harness metadata
/// (a hook's context arrives as its own attachment), and our own earlier injection is the same
/// source as this one, not independent knowledge.
fn visible_text(e: &Value) -> String {
    let text = match row_type(e) {
        "assistant" => event_texts(e)
            .into_iter()
            .map(|(_, t, _)| t)
            .collect::<Vec<_>>()
            .join("\n"),
        "user" => leaves(&e["message"]["content"]),
        "system" => e["content"].as_str().unwrap_or("").to_string(),
        "attachment" if hook_success(e) => String::new(),
        "attachment" => match attachment(e) {
            Some(a) => a["content"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .filter(|s| !is_memory_context(s) && !s.starts_with(NOTICE_PREFIX))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            None => leaves(&e["attachment"]),
        },
        _ => String::new(),
    };
    text.to_ascii_lowercase()
}
fn tool_calls(e: &Value) -> Vec<(&str, &Value, &str)> {
    if row_type(e) != "assistant" {
        return Vec::new();
    }
    e["message"]["content"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    (p.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .then(|| {
                            Some((
                                p.get("name")?.as_str().filter(|n| !n.is_empty())?,
                                p.get("input")?,
                                p.get("id").and_then(Value::as_str).unwrap_or(""),
                            ))
                        })
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}
fn tool_results(e: &Value) -> Vec<(&str, String)> {
    if row_type(e) != "user" {
        return Vec::new();
    }
    e["message"]["content"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
                .filter_map(|p| Some((p.get("tool_use_id")?.as_str()?, leaves(&p["content"]))))
                .collect()
        })
        .unwrap_or_default()
}
/// The instruction files, lowercased, and whether neither exists in the window's directory.
fn instruction_text(cwd: &str) -> (String, bool) {
    let mut out = String::new();
    let mut found = false;
    for name in ["CLAUDE.md", "AGENTS.md"] {
        if let Ok(s) = fs::read_to_string(Path::new(cwd).join(name)) {
            found = true;
            out.push_str(&s);
            out.push('\n');
        }
    }
    (out.to_ascii_lowercase(), !found)
}
const READERS: &[&str] = &[
    "cat", "sed", "grep", "rg", "head", "tail", "less", "awk", "nl", "bat",
];
const WRAPPERS: &[&str] = &["env", "nice", "timeout", "time", "sudo", "command", "exec"];
fn program(word: &str) -> &str {
    Path::new(word)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(word)
}
fn prefix_word(word: &str) -> bool {
    let assignment = word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.bytes().all(|b| b == b'_' || b.is_ascii_alphanumeric())
    });
    let number = word.trim_end_matches(['s', 'm', 'h']);
    assignment
        || WRAPPERS.contains(&word)
        || word.starts_with('-')
        || (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
}
/// The words of each simple command in a shell line, with leading environment assignments,
/// wrappers and their options dropped, so the first word is the program that runs. Quotes and
/// backslash escapes are honoured, so a separator or a command name inside quotes stays one
/// argument; `$(`, backticks and parentheses only split, they are not parsed.
fn segments(command: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let flush_word = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    };
    let flush_segment = |words: &mut Vec<String>, out: &mut Vec<Vec<String>>| {
        let skip = words.iter().take_while(|w| prefix_word(w)).count();
        if skip < words.len() {
            out.push(words.split_off(skip));
        }
        words.clear();
    };
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => word.extend(chars.next()),
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, ';' | '|' | '&' | '\n' | '(' | ')' | '`') => {
                flush_word(&mut word, &mut words);
                flush_segment(&mut words, &mut out);
            }
            (None, c) if c.is_whitespace() => flush_word(&mut word, &mut words),
            (None, '$') if word.is_empty() => {}
            (None, c) => word.push(c),
        }
    }
    flush_word(&mut word, &mut words);
    flush_segment(&mut words, &mut out);
    out
}
fn command_purpose(command: &str) -> (bool, bool, bool) {
    let mut flags = (false, false, false);
    for words in segments(command) {
        let arg = |i: usize| words.get(i).map(String::as_str);
        if program(&words[0]) != "cort" || arg(1) != Some("memory") {
            continue;
        }
        match arg(2) {
            Some("recall") => flags.0 = true,
            Some("status") => flags.1 = true,
            Some("record" | "resolve") => flags.2 = true,
            Some("notes") if matches!(arg(3), Some("show" | "list")) => flags.0 = true,
            _ => {}
        }
    }
    flags
}
fn source_read(name: &str, input: &Value, source: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    match name {
        "Read" => input["file_path"]
            .as_str()
            .is_some_and(|p| Path::new(p).ends_with(source)),
        "Bash" => segments(input["command"].as_str().unwrap_or(""))
            .iter()
            .any(|words| {
                READERS.contains(&program(&words[0]))
                    && words[1..]
                        .iter()
                        .any(|w| !w.starts_with('-') && Path::new(w).ends_with(source))
            }),
        _ => false,
    }
}
fn private_memory_read(name: &str, input: &Value, claude_dir: &Path) -> Option<PathBuf> {
    if name != "Read" {
        return None;
    }
    let raw = Path::new(input["file_path"].as_str()?);
    // A note read then deleted no longer canonicalizes; its recorded path still counts.
    let path = fs::canonicalize(raw).unwrap_or_else(|_| raw.to_path_buf());
    let relative = path.strip_prefix(claude_dir).ok()?;
    relative
        .components()
        .any(|c| c.as_os_str() == "memory")
        .then_some(path)
}
/// `text` is already lowercase; candidate tokens are too.
fn mark_tokens(text: &str, candidates: &[(String, String)], seen: &mut BTreeSet<String>) {
    for (_, token) in candidates {
        if text.contains(token.as_str()) {
            seen.insert(token.clone());
        }
    }
}
fn reset_at(rows: &[Value], start: usize) -> usize {
    (0..start)
        .rev()
        .find(|&i| {
            rows[i].get("isCompactSummary").and_then(Value::as_bool) == Some(true)
                || matches!(hook_source(&rows[i]), Some("startup" | "clear"))
        })
        .unwrap_or(0)
}
fn entry_id(salt: &[u8; 32], cwd: &str, key: &str) -> String {
    hash(salt, &format!("{cwd}\0{key}"))
}
fn cut_text(text: &str, limit: usize, lines: Option<usize>) -> (String, bool) {
    let line_end = lines
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| text.match_indices('\n').nth(n).map(|(i, _)| i))
        .unwrap_or(text.len());
    let end = line_end.min(text.len());
    if end == text.len() && text.len() <= limit {
        return (text.into(), false);
    }
    let budget = limit.saturating_sub(" …[cut]".len());
    let mut boundary = end.min(budget);
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (format!("{} …[cut]", &text[..boundary]), true)
}
const PACKET_CAP: usize = 60_000;
/// No kept turn is cut below this, so every kept turn can still be quoted.
const TURN_FLOOR: usize = 200;
/// The packet keeps every turn up to the last candidate (or the first 40) and every user turn
/// after it, plus the result each candidate tool call got, matched by tool id. Past the byte cap
/// it drops tool results first, then tool calls, then the agent's own words, latest first within
/// each class; the injection, user turns, candidate turns and their results are never dropped.
/// If those alone still exceed the cap, the longest of them are cut in place, down to
/// `TURN_FLOOR` bytes each; only then may the packet stay above the cap.
fn packet_turns(mut turns: Vec<Value>, candidates: &BTreeSet<usize>) -> (Vec<Value>, usize, bool) {
    let mut elided = 0;
    let mut truncated = false;
    let turn_of = |t: &Value| t["turn"].as_u64().unwrap_or(0) as usize;
    let len_of = |t: &Value| t["text"].as_str().unwrap_or("").len();
    // A candidate tool call keeps the result it got back: that is what the agent then saw.
    let calls: BTreeSet<String> = turns
        .iter()
        .filter(|t| t["speaker"] == "tool_call" && candidates.contains(&turn_of(t)))
        .filter_map(|t| t["_id"].as_str().map(String::from))
        .collect();
    let mut protected = candidates.clone();
    for t in &turns {
        if t["speaker"] == "tool_result" && t["_id"].as_str().is_some_and(|id| calls.contains(id)) {
            protected.insert(turn_of(t));
        }
    }
    // A candidate call without an id keeps the next result, unless another call comes first.
    for (i, t) in turns.iter().enumerate() {
        if t["speaker"] == "tool_call" && t["_id"].is_null() && candidates.contains(&turn_of(t)) {
            if let Some(r) = turns[i + 1..]
                .iter()
                .take_while(|r| r["speaker"] != "tool_call")
                .find(|r| r["speaker"] == "tool_result")
            {
                protected.insert(turn_of(r));
            }
        }
    }
    let boundary = candidates.last().copied().unwrap_or(40);
    turns.retain(|t| {
        let keep =
            turn_of(t) <= boundary || t["speaker"] == "user" || protected.contains(&turn_of(t));
        elided += usize::from(!keep);
        keep
    });
    let mut bytes: usize = turns.iter().map(len_of).sum();
    if bytes > PACKET_CAP {
        truncated = true;
        for class in [
            &["tool_result", "harness"][..],
            &["tool_call"],
            &["assistant_text", "thinking"],
        ] {
            for i in (0..turns.len()).rev() {
                if bytes <= PACKET_CAP {
                    break;
                }
                let speaker = turns[i]["speaker"].as_str().unwrap_or("");
                if class.contains(&speaker) && !protected.contains(&turn_of(&turns[i])) {
                    bytes -= len_of(&turns[i]);
                    turns.remove(i);
                    elided += 1;
                }
            }
        }
        // Still over: cut the longest kept turns in place, never the injection.
        while bytes > PACKET_CAP {
            let Some(i) = (0..turns.len())
                .filter(|&i| turns[i]["speaker"] != "injection" && len_of(&turns[i]) > TURN_FLOOR)
                .max_by_key(|&i| len_of(&turns[i]))
            else {
                break;
            };
            let old = len_of(&turns[i]);
            let target = old.saturating_sub(bytes - PACKET_CAP).max(TURN_FLOOR);
            let (cut, _) = cut_text(turns[i]["text"].as_str().unwrap_or(""), target, None);
            bytes = bytes - old + cut.len();
            turns[i]["text"] = json!(cut);
        }
    }
    for t in &mut turns {
        if let Some(o) = t.as_object_mut() {
            o.remove("_id");
        }
    }
    (turns, elided, truncated)
}
struct Window<'a> {
    rows: &'a [Value],
    visible: &'a [String],
    start: usize,
    end: usize,
    session: &'a str,
    source: &'a str,
    other_attachment: &'a [&'a str],
}
fn scan_window(
    w: &Window,
    injection: &Injection,
    salt: &[u8; 32],
    claude_dir: &Path,
    want_detail: bool,
    injected_text: &str,
) -> (Counts, Option<Value>, Option<Value>) {
    let (rows, start, session) = (w.rows, w.start, w.session);
    let mut counts = Counts {
        windows: 1,
        ..Counts::default()
    };
    counts.sessions.insert(session.into());
    counts.partial_entries = injection.entries.iter().filter(|e| e.partial).count();
    counts.entries_left_out = usize::from(injection.entries_left_out);
    counts.notes_left_out = usize::from(injection.notes_left_out);
    let cwd = rows[start]["cwd"].as_str().unwrap_or("");
    let (instructions, absent) = instruction_text(cwd);
    counts.missing_instructions = usize::from(absent);
    let entry_ids: Vec<String> = injection
        .entries
        .iter()
        .map(|e| entry_id(salt, cwd, &e.key))
        .collect();
    let mut candidates: Vec<(String, String)> = Vec::new();
    for (entry, id) in injection.entries.iter().zip(&entry_ids) {
        counts.session_entries.insert((session.into(), id.clone()));
        for token in &entry.tokens {
            if token.len() >= 4 {
                candidates.push((id.clone(), token.clone()));
            }
        }
    }
    for note in &injection.notes {
        let id = hash(salt, &format!("note\0{}", note.id));
        // Every harness names its own memory index MEMORY.md, so that name attributes nothing.
        if !note.file_name.eq_ignore_ascii_case("MEMORY.md") {
            candidates.push((id.clone(), note.file_name.to_ascii_lowercase()));
        }
        candidates.push((id, note.id.to_ascii_lowercase()));
    }
    // Visible before the exposure: the instruction files, the other hooks' SessionStart context,
    // and every row since the last context reset.
    let mut seen = BTreeSet::new();
    mark_tokens(&instructions, &candidates, &mut seen);
    for text in w.other_attachment {
        mark_tokens(&text.to_ascii_lowercase(), &candidates, &mut seen);
    }
    for text in &w.visible[reset_at(rows, start)..start] {
        mark_tokens(text, &candidates, &mut seen);
    }
    let mut reused = BTreeSet::new();
    let mut assistant_reused = BTreeSet::new();
    let mut consumption_calls = BTreeSet::new();
    let mut consumed = BTreeSet::new();
    let mut events = Vec::new();
    let mut turn = 0usize;
    let mut packet_turns_raw =
        want_detail.then(|| vec![json!({"turn":0,"speaker":"injection","text":injected_text})]);
    let mut candidate_turns = BTreeSet::new();
    for (idx, e) in rows.iter().enumerate().take(w.end).skip(start + 1) {
        let timestamp = e["timestamp"].as_str().unwrap_or("");
        let mut call_turns = Vec::new();
        for (kind, text, part_id) in event_texts(e) {
            turn += 1;
            if kind == "tool_call" {
                call_turns.push(turn);
            }
            if let Some(packet) = packet_turns_raw.as_mut() {
                // A per-turn cut is marked in the text itself; `truncated` means the packet cap.
                let (text, _) = cut_text(
                    &text,
                    if kind == "tool_result" { 600 } else { 2_000 },
                    (kind == "tool_result").then_some(5),
                );
                // `_id` pairs a call with its result while trimming; it never leaves this file.
                packet.push(json!({"turn":turn,"speaker":kind,"text":text,"_id":part_id}));
            }
            let matching_text = if kind == "tool_call" {
                text.split_once(' ').map_or("", |(_, input)| input)
            } else {
                &text
            };
            let lower = matching_text.to_ascii_lowercase();
            let mut hits = Vec::new();
            if matches!(kind, "assistant_text" | "thinking" | "tool_call") {
                for (id, token) in &candidates {
                    if !seen.contains(token)
                        && lower.contains(token.as_str())
                        && reused.insert((id.clone(), token.clone()))
                    {
                        if kind == "tool_call" {
                            counts.u2_lookup += 1;
                            hits.push("u2_lookup");
                        } else {
                            counts.u2 += 1;
                            assistant_reused.insert(id.clone());
                            counts.u2_entries.insert((session.into(), id.clone()));
                            hits.push("u2");
                        }
                    }
                }
            }
            if kind == "user" && CORRECTIONS.iter().any(|needle| lower.contains(needle)) {
                counts.m3 += 1;
                hits.push("m3");
            }
            if want_detail && !hits.is_empty() {
                candidate_turns.insert(turn);
                events
                    .push(json!({"timestamp":timestamp,"turn":turn,"kind":kind,"candidates":hits}));
            }
        }
        // A recall answer names the entries it returned; only those count as consumed.
        for (use_id, result) in tool_results(e) {
            if consumption_calls.contains(use_id) {
                let lower = result.to_ascii_lowercase();
                for (entry, id) in injection.entries.iter().zip(&entry_ids) {
                    if lower.contains(&entry.key.to_ascii_lowercase()) {
                        consumed.insert(id.clone());
                    }
                }
            }
        }
        for (call_index, (name, input, use_id)) in tool_calls(e).into_iter().enumerate() {
            let call_turn = call_turns.get(call_index).copied().unwrap_or(turn);
            let mut hits = Vec::new();
            if name == "Bash" {
                let (use_it, status, contribution) =
                    command_purpose(input["command"].as_str().unwrap_or(""));
                if use_it {
                    counts.u1_consumption += 1;
                    consumption_calls.insert(use_id.to_string());
                    hits.push("u1_consumption");
                }
                if status {
                    counts.u1_status += 1;
                    hits.push("u1_status");
                }
                if contribution {
                    counts.u1_contribution += 1;
                    hits.push("u1_contribution");
                }
            }
            for (entry, id) in injection.entries.iter().zip(&entry_ids) {
                if source_read(name, input, &entry.source) {
                    counts.u3 += 1;
                    hits.push("u3");
                    if !consumed.contains(id) && !assistant_reused.contains(id) {
                        counts.m1 += 1;
                        counts.m1_entries.insert((session.into(), id.clone()));
                        hits.push("m1");
                    }
                }
            }
            if let Some(path) = private_memory_read(name, input, claude_dir) {
                counts.m2 += 1;
                hits.push("m2");
                if injection.notes.iter().any(|n| path.ends_with(&n.file_name)) {
                    counts.m2_listed += 1;
                    hits.push("m2_listed_note");
                }
            }
            if want_detail && !hits.is_empty() {
                candidate_turns.insert(call_turn);
                events.push(json!({"timestamp":timestamp,"turn":call_turn,"kind":"tool_call","tool":name,"candidates":hits}));
            }
        }
        if row_type(e) != "assistant" {
            mark_tokens(&w.visible[idx], &candidates, &mut seen);
        }
    }
    counts.u1_consumption_windows = usize::from(counts.u1_consumption > 0);
    counts.u1_status_windows = usize::from(counts.u1_status > 0);
    counts.u1_contribution_windows = usize::from(counts.u1_contribution > 0);
    counts.u2_windows = usize::from(counts.u2 > 0);
    counts.u2_lookup_windows = usize::from(counts.u2_lookup > 0);
    counts.u3_windows = usize::from(counts.u3 > 0);
    counts.m1_windows = usize::from(counts.m1 > 0);
    counts.m2_windows = usize::from(counts.m2 > 0);
    counts.m2_listed_windows = usize::from(counts.m2_listed > 0);
    counts.m3_windows = usize::from(counts.m3 > 0);
    if !want_detail {
        return (counts, None, None);
    }
    let (turns, elided_turns, truncated) =
        packet_turns(packet_turns_raw.unwrap_or_default(), &candidate_turns);
    let packet = json!({"source":w.source,"entries":injection.entries.iter().zip(&entry_ids).map(|(e,id)| json!({"id":id,"key":e.key,"claim":e.claim})).collect::<Vec<_>>(),
        "turns":turns,"elided_turns":elided_turns,"truncated":truncated});
    let detail = json!({"session":session,"source":w.source,"timestamp":rows[start]["timestamp"],"cwd":cwd,"project_id":hash(salt, cwd),
        "entries":injection.entries.iter().zip(&entry_ids).map(|(e, id)| json!({"id":id,"key":e.key,"claim":e.claim,"source":e.source,"line":e.line,"evidence_status":e.evidence_status,"partial":e.partial,"tokens":e.tokens})).collect::<Vec<_>>(),
        "notes":injection.notes.iter().map(|n| json!({"id":n.id,"opaque_id":hash(salt,&format!("note\0{}",n.id)),"file_name":n.file_name})).collect::<Vec<_>>(),
        "entries_left_out":injection.entries_left_out,"notes_left_out":injection.notes_left_out,"instruction_files_absent":absent,
        "events":events,"counts":counts.json()});
    (counts, Some(detail), Some(packet))
}
/// One SessionStart run: the hook_success rows of one source, until a SessionStart context
/// attachment or a conversation turn closes it.
struct HookEvent {
    members: Vec<usize>,
    source: String,
    confirmed: bool,
}
fn hook_events(rows: &[Value]) -> (Vec<HookEvent>, BTreeMap<usize, usize>) {
    let mut events: Vec<HookEvent> = Vec::new();
    let mut closes = BTreeMap::new();
    let mut open: Option<usize> = None;
    for (i, e) in rows.iter().enumerate() {
        if let Some(source) = hook_source(e) {
            match open {
                Some(j) if events[j].source == source => events[j].members.push(i),
                _ => {
                    events.push(HookEvent {
                        members: vec![i],
                        source: source.into(),
                        confirmed: false,
                    });
                    open = Some(events.len() - 1);
                }
            }
        } else if attachment(e).is_some() {
            if let Some(j) = open.take() {
                closes.insert(i, j);
            }
        } else if matches!(row_type(e), "user" | "assistant") {
            open = None;
        }
    }
    (events, closes)
}
/// Scan Claude Code JSONL. The public value contains aggregates and opaque ids only.
pub fn scan(
    claude_dir: &Path,
    since: Option<&str>,
    private: Option<&Path>,
) -> Result<Value, String> {
    scan_sample(claude_dir, since, private, 0, 0)
}
pub fn scan_sample(
    claude_dir: &Path,
    since: Option<&str>,
    private: Option<&Path>,
    n_rest: usize,
    seed: u64,
) -> Result<Value, String> {
    let since_ms = since.map(time::parse_since).transpose()?;
    if !claude_dir.is_dir() {
        return Err("--claude-dir is not a directory".into());
    }
    let claude_dir = fs::canonicalize(claude_dir).map_err(|e| format!("--claude-dir: {e}"))?;
    let private = private.map(private_dir).transpose()?;
    let salt = salt_for(private.as_deref())?;
    let mut detail_file = match private.as_ref() {
        Some(dir) => Some(
            private_file(&dir.join("windows.jsonl"), false)
                .map_err(|e| format!("windows.jsonl: {e}"))?,
        ),
        None => None,
    };
    let mut files = 0usize;
    let mut unreadable = 0usize;
    let mut malformed = 0usize;
    let mut scanned = 0usize;
    let mut undated_windows = 0usize;
    let mut undated_runs = 0usize;
    let mut unconfirmed = 0usize;
    let mut recall_runs = 0usize;
    let mut by_source: BTreeMap<String, Counts> = BTreeMap::new();
    let mut packets = Vec::new();
    let mut min_ts: Option<i64> = None;
    let mut max_ts: Option<i64> = None;
    // `--since` keeps what is dated at or after it; an undated item is counted, never guessed.
    let in_range = |ts: Option<i64>, undated: &mut usize| match (since_ms, ts) {
        (None, _) => true,
        (Some(_), None) => {
            *undated += 1;
            false
        }
        (Some(s), Some(t)) => t >= s,
    };
    for path in demand::jsonl_files(&claude_dir, 2) {
        if path.components().any(|c| c.as_os_str() == "subagents") {
            continue;
        }
        files += 1;
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        // A file our hook never ran in holds no exposure and no hook run to confirm. The probe
        // stops before the header's first quote, which JSON escapes in the raw line.
        let probe = HEADER.split('"').next().unwrap_or(HEADER);
        let old_probe = OLD_HEADER.split('"').next().unwrap_or(OLD_HEADER);
        if !raw.contains(probe) && !raw.contains(old_probe) && !raw.contains("hook-recall") {
            continue;
        }
        scanned += 1;
        let mut rows: Vec<Value> = Vec::new();
        for line in raw.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                malformed += 1;
                continue;
            };
            // The conversation itself. `api-request-blob` and the other bookkeeping rows copy
            // what was sent, our injection included, and would mark every token as seen.
            if v.get("isSidechain").and_then(Value::as_bool) != Some(true)
                && matches!(row_type(&v), "user" | "assistant" | "attachment" | "system")
            {
                rows.push(v);
            }
        }
        drop(raw);
        let visible: Vec<String> = rows.iter().map(visible_text).collect();
        let (mut events, closes) = hook_events(&rows);
        for (i, e) in rows.iter().enumerate() {
            let Some(a) = attachment(e) else {
                continue;
            };
            let contents: Vec<&str> = a["content"]
                .as_array()
                .map(|v| v.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let event = closes.get(&i).copied();
            if contents.iter().any(|s| s.starts_with(NOTICE_PREFIX)) {
                if let Some(j) = event {
                    events[j].confirmed = true;
                }
            }
            let Some(ours) = contents.iter().find(|s| is_memory_context(s)) else {
                continue;
            };
            if let Some(j) = event {
                events[j].confirmed = true;
            }
            let source = event
                .map(|j| events[j].source.as_str())
                .filter(|s| matches!(*s, "startup" | "resume" | "compact" | "clear"))
                .unwrap_or("unknown")
                .to_string();
            let ts = stamp(e);
            if !in_range(ts, &mut undated_windows) {
                continue;
            }
            if let Some(t) = ts {
                min_ts = Some(min_ts.map_or(t, |m| m.min(t)));
                max_ts = Some(max_ts.map_or(t, |m| m.max(t)));
            }
            // Any SessionStart ends the window, even one that injected nothing: it is a new
            // context either way.
            let end = (i + 1..rows.len())
                .find(|j| attachment(&rows[*j]).is_some() || session_hook_success(&rows[*j]))
                .unwrap_or(rows.len());
            let other = contents
                .iter()
                .filter(|s| **s != *ours)
                .copied()
                .collect::<Vec<_>>();
            let session = e["sessionId"].as_str().unwrap_or("");
            let session_id = hash(&salt, &format!("{}\0{session}", path.display()));
            let window = Window {
                rows: &rows,
                visible: &visible,
                start: i,
                end,
                session: &session_id,
                source: &source,
                other_attachment: &other,
            };
            let (counts, detail, packet) = scan_window(
                &window,
                &parse_injection(ours),
                &salt,
                &claude_dir,
                detail_file.is_some(),
                ours,
            );
            by_source.entry(source).or_default().add(&counts);
            let window_id = hash(&salt, &format!("{}\0{i}", path.display()));
            if let (Some(file), Some(mut detail)) = (detail_file.as_mut(), detail) {
                detail["window_id"] = json!(window_id);
                writeln!(file, "{detail}").map_err(|e| format!("windows.jsonl: {e}"))?;
            }
            if let Some(mut packet) = packet {
                let stratum = if counts.u1_consumption > 0 {
                    "consumption"
                } else if counts.u2
                    + counts.u2_lookup
                    + counts.u3
                    + counts.m1
                    + counts.m2
                    + counts.m3
                    > 0
                {
                    "candidate"
                } else {
                    "rest"
                };
                packet["window_id"] = json!(window_id);
                packet["stratum"] = json!(stratum);
                packets.push(packet);
            }
        }
        // Only an event in which our hook ran can owe an injection; the others ran someone else's.
        // The run is dated by its first dated row, else by the attachment that closed it.
        let closer: BTreeMap<usize, usize> = closes.iter().map(|(a, e)| (*e, *a)).collect();
        for (j, event) in events.iter().enumerate() {
            let ts = event
                .members
                .iter()
                .find_map(|&m| stamp(&rows[m]))
                .or_else(|| closer.get(&j).and_then(|&a| stamp(&rows[a])));
            if event.members.iter().any(|&m| runs_hook_recall(&rows[m]))
                && in_range(ts, &mut undated_runs)
            {
                recall_runs += 1;
                if !event.confirmed {
                    unconfirmed += 1;
                }
            }
        }
    }
    let mut total = Counts::default();
    for c in by_source.values() {
        total.add(c);
    }
    let sources = by_source
        .into_iter()
        .map(|(source, counts)| (source, counts.json()))
        .collect::<BTreeMap<_, _>>();
    let sample = if let Some(dir) = private.as_ref() {
        let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for packet in packets {
            let stratum = packet["stratum"].as_str().unwrap_or("rest").to_string();
            groups.entry(stratum).or_default().push(packet);
        }
        let mut rest = groups.remove("rest").unwrap_or_default();
        let rest_population = rest.len();
        rest.sort_by_cached_key(|packet| {
            let mut h = Sha256::new();
            h.update(seed.to_string().as_bytes());
            h.update([0]);
            h.update(packet["window_id"].as_str().unwrap_or("").as_bytes());
            hex(&h.finalize())
        });
        rest.truncate(n_rest);
        let taken = rest.len();
        let rest_weight = (taken > 0).then(|| rest_population as f64 / taken as f64);
        let mut file = private_file(&dir.join("packets.jsonl"), false)
            .map_err(|e| format!("packets.jsonl: {e}"))?;
        let mut strata = serde_json::Map::new();
        for name in ["consumption", "candidate", "rest"] {
            let mut group = if name == "rest" {
                std::mem::take(&mut rest)
            } else {
                groups.remove(name).unwrap_or_default()
            };
            let population = if name == "rest" {
                rest_population
            } else {
                group.len()
            };
            let sampled = group.len();
            let weight = if name == "rest" {
                rest_weight
            } else {
                (sampled > 0).then_some(1.0)
            };
            strata.insert(
                name.into(),
                json!({"population":population,"sampled":sampled,"weight":weight}),
            );
            for packet in &mut group {
                packet["weight"] = json!(weight);
                writeln!(file, "{packet}").map_err(|e| format!("packets.jsonl: {e}"))?;
            }
        }
        let mut prompt = private_file(&dir.join("reviewer-prompt.md"), false)
            .map_err(|e| format!("reviewer-prompt.md: {e}"))?;
        prompt
            .write_all(REVIEWER_PROMPT.as_bytes())
            .map_err(|e| format!("reviewer-prompt.md: {e}"))?;
        let digest = sha256(REVIEWER_PROMPT.as_bytes());
        json!({"seed":seed,"n_rest":n_rest,"strata":strata,"prompt_sha256":digest})
    } else {
        Value::Null
    };
    Ok(
        json!({"method":UPTAKE_METHOD,"since":since,"scan_bounds":{"first":min_ts.map(time::format_utc),"last":max_ts.map(time::format_utc)},
        "machine":{"id":cort::usage::machine_id(),"source":cort::usage::machine_id_source()},
        "files":{"read":files,"scanned":scanned,"unreadable":unreadable,"malformed_lines_in_scanned":malformed},"undated_skipped":{"windows":undated_windows,"hook_runs":undated_runs},
        "hook_recall_runs":recall_runs,"unconfirmed_hook_runs":unconfirmed,
        "total":total.json(),"by_source":sources,"sample":sample,
        "salt":if private.is_some() { "persistent_private" } else { "ephemeral; ids are not comparable across runs" }}),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn uptake_parses_moved_evidence() {
        let text = format!(
            "{}\n- [key] claim (docs/decision.md:3, source_moved)",
            super::HEADER
        );
        let injection = super::parse_injection(&text);
        assert_eq!(injection.entries.len(), 1);
        assert_eq!(injection.entries[0].evidence_status, "source_moved");
    }
}
