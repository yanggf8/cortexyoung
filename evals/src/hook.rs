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

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What the rule concluded about one command.
#[derive(Debug, Clone, PartialEq)]
pub struct HookHit {
    /// The symbol to hand to `cort impact --symbol`.
    pub symbol: String,
    /// Why it fired, for the report; a hit nobody can explain is not evidence.
    pub reason: &'static str,
}

/// Paths whose contents are not project source. A search into any of them is orientation over
/// logs, transcripts or build output -- `rg` is the right tool and always will be.
const NON_SOURCE_MARKERS: [&str; 12] = [
    ".log",
    ".jsonl",
    ".json",
    ".history",
    "_history",
    "node_modules",
    "/.claude",
    "/.codex",
    "/.nullclaw",
    "/target/",
    "/dist/",
    "/build/",
];

/// Extensions and directory names that say "this is project source".
/// `src/` and `/src` are both listed on purpose: a relative target (`rg x src/`) is the common
/// shape in an agent session and does not carry a leading slash.
const SOURCE_MARKERS: [&str; 12] = [
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", "/src", "src/", "crates/", "lib/", "app/", "./",
];

/// The languages that have an `edge:calls` rule pack. Anything else, `impact` cannot answer, so a
/// suggestion there is worse than silence: it looks answerable.
const SOURCE_EXTENSIONS: [&str; 6] = [".rs", ".ts", ".tsx", ".js", ".jsx", ".py"];

/// `-A`, `-B`, `-C` and their long forms, including the glued `-A10` / `-B2` forms and the combined
/// short cluster `-nB2`.
fn is_context_flag(token: &str) -> bool {
    if token == "--context" || token.starts_with("--context=") {
        return true;
    }
    if token.starts_with("--after-context") || token.starts_with("--before-context") {
        return true;
    }
    if !token.starts_with('-') || token.starts_with("--") {
        return false;
    }
    token.chars().skip(1).any(|c| matches!(c, 'A' | 'B' | 'C'))
}

/// A redirection is shell plumbing, not a place to search. Counting `2>/dev/null` as a directory
/// let a two-named-file read pass the cross-file test.
fn is_redirection(token: &str) -> bool {
    token.contains('>') || token.starts_with('<') || token == "/dev/null"
}

/// `-rn` carries `-r`. Long flags are excluded: `--recursive` is matched by name.
fn is_short_cluster_with(token: &str, flag: char) -> bool {
    token.starts_with('-') && !token.starts_with("--") && token.chars().skip(1).any(|c| c == flag)
}

/// Does the command name a file extension at all? `foo.rs`, `*.zig`, `Cargo.toml` do; a bare
/// directory (`backend/src`) does not, and a bare directory stays eligible.
fn names_an_extension(targets: &str) -> bool {
    targets.split_whitespace().any(|t| {
        t.rsplit('/')
            .next()
            .and_then(|base| base.rsplit_once('.'))
            .map(|(head, ext)| {
                !head.is_empty()
                    && !ext.is_empty()
                    && ext.chars().all(|c| c.is_ascii_alphanumeric())
                    && ext.len() <= 5
            })
            .unwrap_or(false)
    })
}

/// Regex metacharacters. A pattern carrying any of them is a text hunt, not a symbol.
/// `:` is deliberately absent: `Type::method` is the qualified form cort wants.
fn has_regex_meta(pattern: &str) -> bool {
    pattern.chars().any(|c| {
        matches!(
            c,
            '|' | '\\' | '[' | ']' | '*' | '+' | '?' | '^' | '$' | '.' | '{' | '}' | ' ' | '\t'
        )
    })
}

/// A bare identifier, or a qualified `Type::method`, with the call parenthesis optionally attached
/// (`rate_limit(` is the exact shape a hand-rolled call-site search takes). Returns the symbol with
/// the parenthesis stripped.
fn symbol_of_pattern(pattern: &str) -> Option<String> {
    let core = pattern.strip_suffix('(').unwrap_or(pattern);
    if core.is_empty() || has_regex_meta(core) {
        return None;
    }
    // Non-ASCII is prose, not an identifier: the corpus contains CJK searches.
    if !core.is_ascii() {
        return None;
    }
    let segments: Vec<&str> = core.split("::").collect();
    if segments.len() > 2 {
        return None;
    }
    for seg in &segments {
        let mut chars = seg.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => return None,
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
    }
    // One-and-two-character names are noise on any corpus (`n`, `fs`, `ok`).
    if core.len() < 3 {
        return None;
    }
    Some(core.to_string())
}

/// Split a command line into whitespace-separated tokens, honouring single and double quotes.
fn tokenize(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in command.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                    started = true;
                } else if c.is_whitespace() {
                    if started || !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                        started = false;
                    }
                } else {
                    cur.push(c);
                }
            }
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The first pipeline segment. `grep -rn 'x(' src/*.rs | grep -v 'pub fn'` is one search whose
/// second stage is the agent hand-rolling "drop the declaration line" -- which is `cort`'s
/// `DECLARATION_KEYWORDS`. The stage that matters is the first one.
fn first_segment(command: &str) -> &str {
    let mut depth_single = false;
    let mut depth_double = false;
    for (i, c) in command.char_indices() {
        match c {
            '\'' if !depth_double => depth_single = !depth_single,
            '"' if !depth_single => depth_double = !depth_double,
            '|' | ';' if !depth_single && !depth_double => return &command[..i],
            _ => {}
        }
    }
    command
}

/// Does `cort impact` answer this search better than the search does?
///
/// Fires only on the narrow shape it can actually beat: one bare symbol, searched in project
/// source. Everything else -- alternations, phrases, logs, transcripts, build output -- stays with
/// `rg`, which is what the routing skill already says and what the traffic shows the agent doing
/// correctly hundreds of times.
pub fn suggests_impact(command: &str) -> Option<HookHit> {
    let segment = first_segment(command.trim());
    let tokens = tokenize(segment);
    let mut idx = 0;
    // Skip leading `VAR=value` assignments and a `sudo`-style prefix.
    while idx < tokens.len() && tokens[idx].contains('=') && !tokens[idx].starts_with('-') {
        idx += 1;
    }
    let tool = tokens.get(idx)?.rsplit('/').next()?.to_string();
    if tool != "rg" && tool != "grep" && tool != "egrep" {
        return None;
    }
    idx += 1;

    // The pattern is the first non-flag token. `-e PATTERN` names it explicitly.
    let mut pattern: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    while idx < tokens.len() {
        let t = &tokens[idx];
        if t == "-e" || t == "--regexp" {
            idx += 1;
            pattern = tokens.get(idx).cloned();
        } else if t.starts_with('-') && pattern.is_none() {
            // A flag that takes a value we must not read as the pattern.
            if t == "--glob" || t == "-g" || t == "--type" || t == "-t" {
                idx += 1;
            }
        } else if pattern.is_none() {
            pattern = Some(t.clone());
        } else if !is_redirection(t) {
            rest.push(t.clone());
        }
        idx += 1;
    }
    let symbol = symbol_of_pattern(&pattern?)?;

    // A context flag means the agent wants to read the body around the match, not enumerate who
    // reaches it. `cort context` is that verb, and suggesting `impact` there is a wrong answer
    // dressed as a helpful one. Adjudicated on the first probe run: every `-A`/`-B`/`-C` fire was a
    // false positive.
    if tokens.iter().any(|t| is_context_flag(t)) {
        return None;
    }

    let targets = rest.join(" ");
    if NON_SOURCE_MARKERS.iter().any(|m| targets.contains(m)) {
        return None;
    }
    // If the command names any file extension at all, at least one has to be a language the rule
    // pack actually indexes. Without this a Zig or Go file under `src/` fires, and `impact` has
    // nothing to say about a language it never parsed -- the worst kind of suggestion, because it
    // looks answerable.
    if names_an_extension(&targets) && !SOURCE_EXTENSIONS.iter().any(|e| targets.contains(e)) {
        return None;
    }
    // A caller set is cross-file by definition. A search that names concrete files and nothing
    // recursive or glob-shaped is asking "where does this appear in the file I already have open",
    // which is reading. Eleven of the fourteen false positives left after the first two fixes were
    // exactly this shape.
    let recursive = tokens
        .iter()
        .any(|t| t == "-r" || t == "-R" || t == "--recursive" || is_short_cluster_with(t, 'r'));
    let has_glob = targets.contains('*') || targets.contains('?');
    let concrete_dirs = rest
        .iter()
        .any(|t| !t.starts_with('-') && !names_an_extension(t) && !t.contains('*'));
    if !targets.trim().is_empty() && !recursive && !has_glob && !concrete_dirs {
        return None;
    }

    // No path at all means the current directory, which in an agent session is the project.
    let reason = if targets.trim().is_empty() {
        "bare symbol, search scoped to the working tree"
    } else if SOURCE_MARKERS.iter().any(|m| targets.contains(m)) {
        "bare symbol, search scoped to project source"
    } else {
        return None;
    };
    Some(HookHit { symbol, reason })
}

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

/// The shape test cannot know whether a name is a function. Every false positive left after three
/// rounds of adjudication was the same mistake: `confidence` is a struct field, `TIMEOUT_S` a
/// constant, `trace_file` a local. `impact` has nothing to say about any of them -- it would answer
/// `no_seed_resolved` -- so suggesting it is a wrong answer wearing a helpful face.
///
/// In production the hook asks the index. Offline there is no index for five of the six projects in
/// the corpus, so this is the index-free equivalent of the same question, reusing the declaration
/// scanner `recall` already ships: is this name declared as a function anywhere in the tree the
/// search covered? `None` means the tree could not be read, which must be reported as unchecked and
/// never silently counted as a pass -- that is the false-safe shape the coverage screen exists to
/// refuse.
pub fn declares_function(root: &Path, symbol: &str) -> Option<bool> {
    if !root.is_dir() {
        return None;
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
        return None;
    }
    Some(found)
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
pub fn search_root(command: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let tokens = tokenize(first_segment(command.trim()));
    let mut seen_pattern = false;
    for t in tokens.iter().skip(1) {
        if t.starts_with('-') || is_redirection(t) {
            continue;
        }
        if !seen_pattern {
            seen_pattern = true;
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
    let mut searches = 0usize;
    let mut commands_seen = 0usize;
    let mut fires: Vec<Value> = Vec::new();
    let mut passed_over: Vec<Value> = Vec::new();
    let mut symbols: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let (mut confirmed, mut rejected, mut unchecked) = (0usize, 0usize, 0usize);

    for (_kind, dir) in dirs {
        for file in jsonl_files(dir, 8) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let session = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            let mut cwd: Option<String> = None;
            for line in text.lines() {
                if let Some(found) = cwd_of_line(line) {
                    cwd = Some(found);
                }
                for cmd in commands_of_line(line) {
                    commands_seen += 1;
                    let head = first_segment(cmd.trim());
                    let tokens = tokenize(head);
                    let is_search = tokens
                        .first()
                        .map(|t| {
                            let base = t.rsplit('/').next().unwrap_or(t);
                            base == "rg" || base == "grep" || base == "egrep"
                        })
                        .unwrap_or(false);
                    if !is_search {
                        continue;
                    }
                    searches += 1;
                    match suggests_impact(&cmd) {
                        Some(hit) => {
                            let verdict = match search_root(&cmd, cwd.as_deref())
                                .and_then(|root| declares_function(&root, &hit.symbol))
                            {
                                Some(true) => "confirmed_function",
                                Some(false) => "rejected_not_a_function",
                                None => "unchecked_tree_unreadable",
                            };
                            match verdict {
                                "confirmed_function" => confirmed += 1,
                                "rejected_not_a_function" => rejected += 1,
                                _ => unchecked += 1,
                            }
                            *symbols.entry(hit.symbol.clone()).or_insert(0) += 1;
                            if fires.len() < max_examples {
                                fires.push(json!({
                                    "session": session,
                                    "symbol": hit.symbol,
                                    "reason": hit.reason,
                                    "index_check": verdict,
                                    "command": truncate(&cmd),
                                }));
                            }
                        }
                        None => {
                            if passed_over.len() < max_examples {
                                passed_over.push(json!({
                                    "session": session,
                                    "command": truncate(&cmd),
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
        "method": "hook-probe-v1",
        "commands_seen": commands_seen,
        "searches": searches,
        "fired": fired,
        "fire_rate_of_searches": rate(fired, searches),
        "index_check": {
            "confirmed_function": confirmed,
            "rejected_not_a_function": rejected,
            "unchecked_tree_unreadable": unchecked,
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
                    Treat a rejection as 'no seed here', never as 'do not suggest'.",
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
