//! Does a shell search the agent just ran have a better answer in the index?
//!
//! The routing skill's trigger is prospective -- "about to rename / delete / migrate a symbol" --
//! and measured over the sessions that carried it, it never fired: 409 searches, zero `cort` calls.
//! That asks the model to recognise a future action before the search that would tell it what the
//! action is. This module is the retrospective shape instead: given the search that actually ran,
//! say whether `cort impact` answers it. A harness hook can decide that at the moment itself, with
//! nothing depending on the model remembering anything.
//!
//! It lives in the product crate so there is exactly one rule. `cort-evals hook-probe` replays this
//! same function over transcripts to measure its false-positive rate; a second copy over there would
//! mean the measured rule and the installed rule could drift, which is the failure this move exists
//! to prevent.
//!
//! Calibrated by hand-adjudicating every fire over 192 real searches from 13 sessions
//! (`docs/`, and the probe's own output). Four rounds took it from 31 fires to 10, six of them
//! genuine. The rejections are as load-bearing as the fires: a rule that also fires on ordinary
//! orientation greps trains the agent to ignore it, which is how the over-broad version of the
//! skill's prose failed.

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
pub fn is_redirection(token: &str) -> bool {
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
/// Public so the probe reads a command exactly the way the rule does; two readers would drift.
pub fn tokenize(command: &str) -> Vec<String> {
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
pub fn first_segment(command: &str) -> &str {
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
