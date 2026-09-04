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

/// What this project's index has to say about one symbol, asked as cheaply as it can be.
///
/// `RawOnly` is the variant this whole design turns on. `raw_edges` is rebuilt across files and
/// therefore outlives the `chunks` row it pointed at (schema F-01), so a symbol whose definition was
/// just deleted still has a surviving caller's raw edge naming it. Gating on `Seed` alone would
/// silence the hook on exactly the deletion-verification search the goal sentence names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// A `chunks` row: `impact` can seed on it.
    Seed,
    /// No chunk, but a `raw_edges` row names it: deleted, or an external type this project uses.
    RawOnly,
    /// Nothing in the index names it at all: a concept, a field, a domain word.
    Neither,
    /// This project has no index, so there was nothing to ask.
    NoIndex,
    /// The lookup could not run -- a replay with no recoverable state, or a database that would not
    /// open or answer. Fires: a question that was never put is not a negative answer.
    Unknown,
}

/// Ask one project's index what it holds about `symbol`, in at most two queries.
///
/// `chunks` first: a seed is the strong answer and the common one, and the query is covered by
/// `idx_chunks_symbol (project_id, symbol_name)`.
///
/// `raw_edges` second, matched **exactly** the way `coverage::extracted_but_unresolved` matches
/// (`coverage.rs`, the `rel_type IN ('calls','references')` query): the same relation filter, and
/// the leaf passed for all three parameters. Both halves matter and a first draft got both wrong.
/// Without the filter the query also counts `imports`, whose raw targets are module routes: a Rust
/// `use crate::gone::tide;` stores `crate::gone::tide`, whose leaf matches the `LIKE '%:'` arm, so a
/// bare `tide` search would fire on a project that merely imports the name and never calls it. (A
/// JS/TS `./tide` specifier does *not* leak -- it ends `/tide`, matching neither arm. Worth checking
/// rather than assuming; a first draft of this comment asserted the opposite.) Measured on the hook corpus, filtering moves 7
/// of 110 fires out of the firing set and costs nothing: an `imports` raw edge never becomes a
/// relationship at all, so `impact` could not have answered those anyway.
///
/// Using the same splitter and parameters as the coverage screen is deliberate: two answers to
/// "which name is this" is how a gate and a report start disagreeing. One shared looseness comes
/// with it -- `LIKE` treats `_` as a single-character wildcard, so every snake_case leaf is a
/// pattern rather than a literal. Coverage has always had that; neither place has measured it.
///
/// That second query is a **scan** of the project's raw edges, not an indexed seek: the only index
/// on the table is `(project_id, file_path)` and the `LIKE` terms lead with a wildcard. Measured on
/// this repository's ~15,850 rows: ~2.95ms for an absent symbol. Affordable only because this runs
/// after the shape gate, on about one search in twenty. If it stops being affordable the fix is a
/// stored `raw_target_leaf` column with its own index -- not a tighter match, which would cost the
/// deletion case.
///
/// Errors are returned. A caller that read a storage failure as "absent" would silence the hook on
/// a disk problem, which is the opposite of this crate's rule that storage failures degrade loudly.
pub fn evidence_in(
    db: &rusqlite::Connection,
    project_id: &str,
    symbol: &str,
) -> rusqlite::Result<Evidence> {
    use rusqlite::OptionalExtension;
    let seed = db
        .query_row(
            "SELECT 1 FROM chunks WHERE project_id = ?1 AND symbol_name = ?2 LIMIT 1",
            rusqlite::params![project_id, symbol],
            |_| Ok(()),
        )
        .optional()?;
    if seed.is_some() {
        return Ok(Evidence::Seed);
    }
    let leaf = crate::chunker::bare_name(symbol);
    let edge = db
        .query_row(
            "SELECT 1 FROM raw_edges
              WHERE project_id = ?1 AND rel_type IN ('calls', 'references')
                AND (raw_target = ?2 OR raw_target LIKE '%:' || ?3 OR raw_target LIKE '%.' || ?4)
              LIMIT 1",
            rusqlite::params![project_id, leaf, leaf, leaf],
            |_| Ok(()),
        )
        .optional()?;
    Ok(if edge.is_some() {
        Evidence::RawOnly
    } else {
        Evidence::Neither
    })
}

/// Why the hook stayed quiet. Separate variants because the mining has to tell a heuristic problem
/// from an index problem from a missing index, and a single `None` collapses all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilenceReason {
    /// Not the narrow shape where `impact` beats `rg`.
    NoShape,
    /// Right shape, no index for this project -- a missed opportunity, not a refusal.
    NoIndex,
    /// Right shape, indexed project, and the index holds neither a seed nor a raw edge naming the
    /// symbol. `impact` would answer `seeds=0 dependents=0` and nothing else.
    NoEvidence,
}

/// The whole routing decision for one search.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Fire(HookHit),
    Silent(SilenceReason),
}

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
pub fn suggests_impact_shape(command: &str) -> Option<HookHit> {
    match judge(&search_from_shell(command)?, |_| Evidence::Unknown) {
        Verdict::Fire(hit) => Some(hit),
        Verdict::Silent(_) => None,
    }
}

/// One search in the terms the decision needs, with every harness-specific spelling already
/// resolved.
///
/// The split exists because parsing and deciding are not the same job and do not have the same
/// number of right answers. Parsing is per-harness by necessity, and the two spellings a *hook
/// payload* arrives in each get their own constructor below: a shell line (`search_from_shell` --
/// Claude Code, Codex and Grok all send `tool_input.command` as a string, byte-identical fields,
/// `docs/2026-09-02-hook-wiring-correction.md` §12) and Kimi's `Grep` fields
/// (`search_from_grep_fields`). Codex's `["bash","-lc",…]` is a third spelling but not a third one
/// here: it is the *rollout transcript* dialect, which only `cort-evals hook-probe` reads, so its
/// extraction lives in `evals/src/hook.rs` and hands the recovered script to `search_from_shell`.
/// Adding an array arm to the payload parser would be an arm that cannot fire -- the mistake the
/// `/.kimi-code/` branch in `harness_of` was deleted for.
/// Deciding is one function, not out of tidiness but because its only justification is a measured
/// number: `cort-evals hook-probe` grades exactly this predicate, so a second copy would leave the
/// calibration describing something other than what ships. The data says the split is drawn in the
/// right place -- replayed over 4,436 real searches the same `judge` fires on 4.73% of shell
/// searches and 4.47% of structured ones (`docs/2026-09-02-hook-wiring-correction.md` §15).
#[derive(Debug, Clone)]
pub struct Search {
    /// The pattern exactly as the agent issued it.
    pub pattern: String,
    /// What the search was pointed at: paths and globs, plus (from a shell line) the flag tokens
    /// that trailed the pattern, which the source/extension tests read as text.
    pub targets: Vec<String>,
    /// The agent asked for the lines around each match (`-A`/`-B`/`-C`). That is `cort context`'s
    /// question, not `impact`'s.
    pub wants_context: bool,
    /// The search descends through directories rather than reading named files.
    pub recursive: bool,
}

/// A search written as a shell command line -- Claude Code's and Codex's surface, and Kimi's
/// minority one.
pub fn search_from_shell(command: &str) -> Option<Search> {
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
    let mut targets: Vec<String> = Vec::new();
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
            targets.push(t.clone());
        }
        idx += 1;
    }
    Some(Search {
        pattern: pattern?,
        targets,
        wants_context: tokens.iter().any(|t| is_context_flag(t)),
        recursive: tokens
            .iter()
            .any(|t| t == "-r" || t == "-R" || t == "--recursive" || is_short_cluster_with(t, 'r')),
    })
}

/// A search issued as a structured tool call: Kimi's `Grep`, whose 834 calls against 32 shell greps
/// make it that harness's real search surface (Claude Code's split is the other way round, 244 to
/// 2,546). Its own parser rather than a rendering back into shell, because the fields already say
/// what a shell line would have to be re-parsed to recover -- and a round trip through a tokenizer
/// only loses things: the value on `-C`, and any pattern carrying both quote characters.
///
/// `file_type` (rg's `--type`) is deliberately not a target. It narrows which languages are
/// searched, not where, so folding it in would make a tree-wide type-filtered search read as if it
/// named a concrete path. The known gap that leaves is a type naming a language the rule pack
/// cannot answer; the shell side has the same gap for the same reason.
pub fn search_from_grep_fields(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    wants_context: bool,
) -> Option<Search> {
    if pattern.is_empty() {
        return None;
    }
    let mut targets: Vec<String> = Vec::new();
    if let Some(p) = path.filter(|p| !p.is_empty()) {
        targets.push(p.to_string());
    }
    if let Some(g) = glob.filter(|g| !g.is_empty()) {
        targets.push(g.to_string());
    }
    Some(Search {
        pattern: pattern.to_string(),
        targets,
        wants_context,
        // A structured grep descends when it is pointed at a directory and does not when it is
        // pointed at one file -- which is the distinction the single-file gate below is made of.
        recursive: path.is_none_or(|p| p.is_empty() || !names_an_extension(p)),
    })
}

/// Does `cort impact` answer this search better than the search does?
///
/// Fires only on the narrow shape it can actually beat: one bare symbol, searched in project
/// source. Everything else -- alternations, phrases, logs, transcripts, build output -- stays with
/// `rg`, which is what the routing skill already says and what the traffic shows the agent doing
/// correctly hundreds of times.
pub fn judge(search: &Search, evidence: impl FnOnce(&str) -> Evidence) -> Verdict {
    let Some(symbol) = symbol_of_pattern(&search.pattern) else {
        return Verdict::Silent(SilenceReason::NoShape);
    };

    // A context flag means the agent wants to read the body around the match, not enumerate who
    // reaches it. `cort context` is that verb, and suggesting `impact` there is a wrong answer
    // dressed as a helpful one. Adjudicated on the first probe run: every `-A`/`-B`/`-C` fire was a
    // false positive.
    if search.wants_context {
        return Verdict::Silent(SilenceReason::NoShape);
    }

    let targets = search.targets.join(" ");
    if NON_SOURCE_MARKERS.iter().any(|m| targets.contains(m)) {
        return Verdict::Silent(SilenceReason::NoShape);
    }
    // If the search names any file extension at all, at least one has to be a language the rule
    // pack actually indexes. Without this a Zig or Go file under `src/` fires, and `impact` has
    // nothing to say about a language it never parsed -- the worst kind of suggestion, because it
    // looks answerable.
    if names_an_extension(&targets) && !SOURCE_EXTENSIONS.iter().any(|e| targets.contains(e)) {
        return Verdict::Silent(SilenceReason::NoShape);
    }
    // A caller set is cross-file by definition. A search that names concrete files and nothing
    // recursive or glob-shaped is asking "where does this appear in the file I already have open",
    // which is reading. Eleven of the fourteen false positives left after the first two fixes were
    // exactly this shape.
    let has_glob = targets.contains('*') || targets.contains('?');
    let concrete_dirs = search
        .targets
        .iter()
        .any(|t| !t.starts_with('-') && !names_an_extension(t) && !t.contains('*'));
    if !targets.trim().is_empty() && !search.recursive && !has_glob && !concrete_dirs {
        return Verdict::Silent(SilenceReason::NoShape);
    }

    // No path at all means the current directory, which in an agent session is the project.
    let reason = if targets.trim().is_empty() {
        "bare symbol, search scoped to the working tree"
    } else if SOURCE_MARKERS.iter().any(|m| targets.contains(m)) {
        "bare symbol, search scoped to project source"
    } else {
        return Verdict::Silent(SilenceReason::NoShape);
    };
    // Only now, with every shape check passed, is the index asked -- and the closure is what opens
    // it. This ordering is the budget: the shape gate turns down about 95% of searches and none of
    // them may cost a database open or a `git rev-parse`.
    match evidence(&symbol) {
        Evidence::Neither => Verdict::Silent(SilenceReason::NoEvidence),
        Evidence::NoIndex => Verdict::Silent(SilenceReason::NoIndex),
        Evidence::Seed | Evidence::RawOnly | Evidence::Unknown => {
            Verdict::Fire(HookHit { symbol, reason })
        }
    }
}
