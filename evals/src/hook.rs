//! The routing rule, as a matcher rather than as advice.
//!
//! The skill asks the agent to recognise, *before* it acts, that it is about to rename or delete
//! something. Measured, that never fired: across the sessions that actually carried the new trigger
//! the agent ran hundreds of text searches and called `cort` zero times. A prospective trigger asks
//! the model to predict its own next action, which it cannot do before the search that would tell it
//! what the action is.
//!
//! So this module tries the other shape: look at the search the agent *did* run and decide whether
//! `cort impact` would have answered it. That is a decision a harness hook can make at the moment
//! itself, with no reliance on the model remembering anything.
//!
//! This file is the offline half. `probe` replays the rule over transcripts already on disk and
//! reports every fire with its source command, so the false-positive rate is read off real traffic
//! before anything is installed. Promote the matcher into `rust/` only once those numbers hold: a
//! hook that fires on ordinary orientation greps would train the agent to ignore it, which is the
//! failure the demand screen already documented for over-broad skill prose.

use cort::hook::is_redirection;
use cort::hook::{judge, search_from_grep_fields, search_from_shell, Search};
pub use cort::hook::{suggests_impact, HookHit};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Does this line declare `name` as something `impact` could hold a seed for? Returns the keyword
/// that decided it, so a verdict can be quoted rather than trusted.
///
/// Narrower than `recall::declared_names`, deliberately. That scanner accepts every declaration
/// keyword including `const`, `static` and `let`, which is right for counting what a rule could
/// bind to and wrong here: a constant and a struct field are declared and are not callable, and
/// treating them as seeds is what made `TIMEOUT_S` and `trace_file` pass the first index check.
pub fn declares_callable_in(line: &str, name: &str) -> Option<&'static str> {
    for (kw, tag) in [
        ("fn ", "fn"),
        ("function ", "function"),
        ("def ", "def"),
        ("fn(", "fn"),
    ] {
        let mut rest = line;
        while let Some(at) = rest.find(kw) {
            let after = &rest[at + kw.len()..];
            let ident: String = after
                .chars()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if ident == name {
                return Some(tag);
            }
            rest = &rest[at + kw.len()..];
        }
    }
    // `const f = (…) => …` / `const f = function …`: a binding whose value is callable. The rule
    // pack indexes these as chunks (README limitation #4), so the check has to as well.
    if let Some(at) = line.find(name) {
        let before = line[..at].trim_end();
        let after = &line[at + name.len()..];
        let bound = before.ends_with("const") || before.ends_with("let") || before.ends_with("var");
        let callable = after.contains("=>") || after.contains("= function");
        if bound && callable {
            return Some("const-arrow");
        }
    }
    None
}

/// What the index-free declaration check could establish.
///
/// Three outcomes, not two-plus-`None`, because the two ways of failing to check are not the same
/// finding and were reported as one. `unchecked_tree_unreadable` was printed for a Godot project
/// whose directory was right there and perfectly readable -- `walk_source` simply reads none of the
/// extensions it contains. Calling that "unreadable" sends the reader to look for a permissions or
/// path problem that does not exist, and hides the real one: this screen has nothing to say about
/// that language at all, which is also why `impact` would have nothing to say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclCheck {
    /// The name is declared as something callable somewhere in the searched tree.
    Declared,
    /// The tree was read and the name is not declared callable in it.
    NotDeclared,
    /// The path is not a directory: gone, renamed, or never resolvable from this session's cwd.
    TreeMissing,
    /// The directory is there and readable, and holds no file this scanner reads.
    NoSourceRead,
}

impl DeclCheck {
    /// The verdict string the report carries, kept as one function so the counter keys and the
    /// per-fire label can never drift apart.
    pub fn verdict(self) -> &'static str {
        match self {
            DeclCheck::Declared => "confirmed_function",
            DeclCheck::NotDeclared => "rejected_not_a_function",
            DeclCheck::TreeMissing => "unchecked_tree_missing",
            DeclCheck::NoSourceRead => "unchecked_no_source_this_screen_reads",
        }
    }
}

pub fn declares_function(root: &Path, symbol: &str) -> DeclCheck {
    if !root.is_dir() {
        return DeclCheck::TreeMissing;
    }
    // `Type::method` is declared as `method`.
    let bare = symbol.rsplit("::").next().unwrap_or(symbol);
    let mut files = 0usize;
    let mut found = false;
    walk_source(root, 6, &mut files, &mut |text| {
        if found {
            return;
        }
        for line in text.lines() {
            if declares_callable_in(line, bare).is_some() {
                found = true;
                return;
            }
        }
    });
    if files == 0 {
        return DeclCheck::NoSourceRead;
    }
    if found {
        DeclCheck::Declared
    } else {
        DeclCheck::NotDeclared
    }
}

/// Bounded: 4,000 files is well past any tree in the corpus and keeps a probe over a home directory
/// from turning into a filesystem crawl.
const MAX_SCANNED_FILES: usize = 4000;

fn walk_source(dir: &Path, depth: usize, files: &mut usize, f: &mut impl FnMut(&str)) {
    if depth == 0 || *files >= MAX_SCANNED_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        if path.is_dir() {
            walk_source(&path, depth - 1, files, f);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| ["rs", "ts", "tsx", "js", "jsx", "py"].contains(&e))
            .unwrap_or(false)
        {
            if *files >= MAX_SCANNED_FILES {
                return;
            }
            *files += 1;
            if let Ok(text) = std::fs::read_to_string(&path) {
                f(&text);
            }
        }
    }
}

/// The directory a search covered, resolved against the session's working directory. Globs and
/// filenames are trimmed back to the enclosing directory; the first one that exists wins.
///
/// Takes the parsed targets rather than a command line, so a structured search and a shell one are
/// resolved by the same code without either being re-rendered into the other's spelling.
pub fn root_of_targets(targets: &[String], cwd: Option<&str>) -> Option<PathBuf> {
    for t in targets {
        if t.starts_with('-') || is_redirection(t) {
            continue;
        }
        let trimmed = t.split('*').next().unwrap_or(t);
        let mut candidate = PathBuf::from(trimmed);
        if candidate.extension().is_some() || trimmed.ends_with('/') {
            candidate = candidate
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
        }
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            match cwd {
                Some(c) => Path::new(c).join(&candidate),
                None => continue,
            }
        };
        if resolved.is_dir() {
            return Some(resolved);
        }
    }
    cwd.map(PathBuf::from).filter(|p| p.is_dir())
}

/// The session's working directory, needed to resolve a relative search target. Claude Code writes
/// `"cwd"` on its records; Codex writes it once in its session header.
pub fn cwd_of_line(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    fn find(v: &Value) -> Option<String> {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(s)) = map.get("cwd") {
                    if s.starts_with('/') {
                        return Some(s.clone());
                    }
                }
                map.values().find_map(find)
            }
            Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    find(&v)
}

/// Every executed shell command on one transcript line, in either transcript dialect: Claude Code
/// writes `"command":"..."`, Codex writes `"command":["bash","-lc","..."]`.
pub fn commands_of_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return out;
    };
    collect_commands(&v, &mut out);
    out
}

fn collect_commands(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if let Some(cmd) = map.get("command") {
                match cmd {
                    Value::String(s) => out.push(s.clone()),
                    Value::Array(items) => {
                        // `["bash","-lc","<script>"]`: the script is the last element.
                        if let Some(Value::String(s)) = items.last() {
                            out.push(s.clone());
                        }
                    }
                    _ => {}
                }
            }
            for (k, child) in map {
                if k != "command" {
                    collect_commands(child, out);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_commands(child, out);
            }
        }
        _ => {}
    }
}

/// Every structured search tool call on one transcript line.
///
/// Not every harness searches through a shell. Kimi writes `{"name":"Grep","args":{…}}` inside a
/// `tool.call` event and Claude Code writes `{"name":"Grep","input":{…}}` in a tool_use block; both
/// are the same question asked with fields instead of a command line. The fields are handed to
/// `cort::hook::search_from_grep_fields` -- the product crate's own parser for that surface -- so
/// this file extracts and never decides. Parsing is per-harness; the verdict is not.
pub fn structured_searches_of_line(line: &str) -> Vec<Search> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return out;
    };
    collect_structured(&v, &mut out);
    out
}

fn collect_structured(v: &Value, out: &mut Vec<Search>) {
    match v {
        Value::Object(map) => {
            // `args` is Kimi's spelling, `input` Claude Code's. A `Grep` entry with neither is the
            // tool *declaration* in a tools snapshot, not a call, and must not be counted.
            if map.get("name").and_then(Value::as_str) == Some("Grep") {
                if let Some(args) = map
                    .get("args")
                    .or_else(|| map.get("input"))
                    .and_then(Value::as_object)
                {
                    let field = |k: &str| {
                        args.get(k)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                    };
                    if let Some(pattern) = field("pattern") {
                        let wants_context =
                            ["-A", "-B", "-C"].iter().any(|k| args.contains_key(*k));
                        if let Some(s) = search_from_grep_fields(
                            pattern,
                            field("path"),
                            field("glob"),
                            wants_context,
                        ) {
                            out.push(s);
                        }
                    }
                }
            }
            for child in map.values() {
                collect_structured(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_structured(child, out);
            }
        }
        _ => {}
    }
}

fn jsonl_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth == 0 {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(jsonl_files(&path, depth - 1));
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    out
}

/// Replay the rule over transcripts. Reports what fired, with the command each fire came from, so
/// the precision number is adjudicated rather than asserted.
pub fn probe(dirs: &[(&str, PathBuf)], max_examples: usize) -> Value {
    let (mut shell_searches, mut structured_searches) = (0usize, 0usize);
    let (mut fired_shell, mut fired_structured) = (0usize, 0usize);
    let mut commands_seen = 0usize;
    let mut fires: Vec<Value> = Vec::new();
    let mut passed_over: Vec<Value> = Vec::new();
    let mut symbols: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let (mut confirmed, mut rejected, mut tree_missing, mut no_source) =
        (0usize, 0usize, 0usize, 0usize);

    for (_kind, dir) in dirs {
        for file in jsonl_files(dir, 8) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            // The path relative to the scanned root, not the file name. Every Kimi session's
            // transcript is called `wire.jsonl`, so a bare file name made all 178 of them
            // indistinguishable -- and a fire nobody can trace back to its session cannot be
            // adjudicated, which is the only thing this report is for.
            let session = file
                .strip_prefix(dir)
                .unwrap_or(&file)
                .to_string_lossy()
                .to_string();
            let mut cwd: Option<String> = None;
            for line in text.lines() {
                if let Some(found) = cwd_of_line(line) {
                    cwd = Some(found);
                }
                // Two surfaces, one verdict. A shell search arrives as a command line and a
                // structured one as fields; each has its own parser in the product crate, and both
                // hand the same `Search` to the same `judge`. They are counted separately because
                // they are not the same population -- Kimi is 834 structured to 32 shell, Claude
                // Code 244 to 2,546 -- and a merged rate would describe a corpus that does not
                // exist on any single machine.
                let mut candidates: Vec<(&'static str, Search, String)> = Vec::new();
                for cmd in commands_of_line(line) {
                    commands_seen += 1;
                    if let Some(search) = search_from_shell(&cmd) {
                        shell_searches += 1;
                        candidates.push(("shell", search, cmd));
                    }
                }
                for search in structured_searches_of_line(line) {
                    structured_searches += 1;
                    let shown = format!(
                        "Grep pattern={} targets={:?}",
                        search.pattern, search.targets
                    );
                    candidates.push(("structured", search, shown));
                }

                for (source, search, shown) in candidates {
                    match judge(&search) {
                        Some(hit) => {
                            let check = match root_of_targets(&search.targets, cwd.as_deref()) {
                                Some(root) => declares_function(&root, &hit.symbol),
                                None => DeclCheck::TreeMissing,
                            };
                            let verdict = check.verdict();
                            match check {
                                DeclCheck::Declared => confirmed += 1,
                                DeclCheck::NotDeclared => rejected += 1,
                                DeclCheck::TreeMissing => tree_missing += 1,
                                DeclCheck::NoSourceRead => no_source += 1,
                            }
                            match source {
                                "structured" => fired_structured += 1,
                                _ => fired_shell += 1,
                            }
                            *symbols.entry(hit.symbol.clone()).or_insert(0) += 1;
                            if fires.len() < max_examples {
                                fires.push(json!({
                                    "session": session,
                                    "source": source,
                                    "symbol": hit.symbol,
                                    "reason": hit.reason,
                                    "index_check": verdict,
                                    "command": truncate(&shown),
                                }));
                            }
                        }
                        None => {
                            if passed_over.len() < max_examples {
                                passed_over.push(json!({
                                    "session": session,
                                    "source": source,
                                    "command": truncate(&shown),
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    let fired: usize = symbols.values().sum();
    json!({
        "method": "hook-probe-v2",
        "commands_seen": commands_seen,
        "searches": shell_searches + structured_searches,
        "searches_shell": shell_searches,
        "searches_structured": structured_searches,
        "fired": fired,
        "fired_shell": fired_shell,
        "fired_structured": fired_structured,
        "fire_rate_of_searches": rate(fired, shell_searches + structured_searches),
        "fire_rate_shell": rate(fired_shell, shell_searches),
        "fire_rate_structured": rate(fired_structured, structured_searches),
        "index_check": {
            "confirmed_function": confirmed,
            "rejected_not_a_function": rejected,
            "unchecked_tree_missing": tree_missing,
            "unchecked_no_source_this_screen_reads": no_source,
        },
        "confirmed_callable_in_searched_tree": confirmed,
        "distinct_symbols": symbols.len(),
        "symbols": symbols,
        "fires": fires,
        "passed_over_examples": passed_over,
        "index_check_reading": "`index_check` is reported, NOT applied. Gating on it was measured and \
                    is wrong in both directions. It rejects the highest-value case there is: a symbol \
                    being checked for complete removal has no declaration left in the tree, which is \
                    the whole reason the agent is searching -- `ensureSeedUserPasswords`, verbatim \
                    `echo \"---(empty = fully removed)---\"`, is rejected by it. It also rejects a \
                    caller search run from a directory that does not contain the definition \
                    (`updatePaymentStatus` under `frontend/src`). In production the hook can ask a \
                    real index instead of the text, which fixes the second case and not the first. \
                    Treat a rejection as 'no seed here', never as 'do not suggest'. The two unchecked \
                    counts are separate on purpose: `unchecked_tree_missing` is a path this session \
                    could not resolve, while `unchecked_no_source_this_screen_reads` is a directory \
                    that is right there and holds nothing this scanner opens -- a Godot tree, say. \
                    They were one count called `unchecked_tree_unreadable`, which sent the reader \
                    hunting a permissions problem that did not exist.",
        "reading": "`fired` is how often the rule would have suggested `cort impact` on traffic that \
                    already happened. It is an upper bound on usefulness and says nothing about whether \
                    the suggestion was right: read the `fires` array and adjudicate each command by \
                    hand, the same way the demand screen's hits are adjudicated. A fire on an \
                    orientation grep is a false positive, and a hook with those trains the agent to \
                    ignore it.",
    })
}

fn rate(n: usize, d: usize) -> Value {
    if d == 0 {
        Value::Null
    } else {
        json!(((n as f64 / d as f64) * 10000.0).round() / 10000.0)
    }
}

fn truncate(s: &str) -> String {
    let cleaned = s.replace('\n', " ");
    if cleaned.chars().count() <= 160 {
        cleaned
    } else {
        cleaned.chars().take(160).collect::<String>() + "…"
    }
}
