//! The adoption funnel, as a command rather than as a protocol document.
//!
//! `docs/2026-08-31-recall-wip.md` §6 specifies how to pair a hook injection with what the agent
//! did next, and until now that specification was prose executed by hand. The first hand-run of it
//! after the hook was wired reported a flat zero across every stage, because the window start is
//! recorded in local time and transcript timestamps are UTC: the cutoff had been placed eight hours
//! in the future. Nothing in the method was wrong and the reading was still false, which is the
//! failure mode this repo has now hit three times -- the skill relies on the model remembering, the
//! hook relied on a person remembering to wire it, and the mining relied on a person remembering
//! the timezone. `parse_since` refuses an offset-less timestamp for that reason.
//!
//! Two things this module will not do. It does not infer adoption from a command that merely
//! mentions `cort impact` -- a session writing the mining script is not a session using the tool --
//! so a command counts only when a shell segment actually *executes* it. And it does not decide
//! whether an adoption was correct: every injection is emitted as a row carrying its triggering
//! command and whatever followed, to be adjudicated the way the demand screen's hits are.

use cort::hook::{first_segment, suggests_impact_shape, tokenize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The marker that makes an injection *ours*. Several PreToolUse hooks may be installed in the same
/// harness (`mos hook` is one on the machine this was written for), so the event name alone
/// attributes nothing; the injected text names the query it is recommending, and that is the only
/// part of it no other hook writes.
const INJECTION_MARKER: &str = "cort impact --symbol '";

/// Milliseconds since the epoch for an RFC 3339 timestamp that carries an explicit offset.
///
/// A bare `2026-09-02 09:24` is refused rather than assumed to be UTC or local. Either assumption
/// is silently wrong half the time, and the wrong one produced a full-zero funnel that read exactly
/// like a broken hook.
pub fn parse_since(raw: &str) -> Result<i64, String> {
    let s = raw.trim();
    let refuse = |why: &str| -> String {
        format!(
            "--since {raw}: {why}\nit must carry an explicit UTC offset, e.g. \
             2026-09-02T09:24:00+08:00 or 2026-09-02T01:24:00Z -- transcript timestamps are UTC \
             and a local wall-clock time without an offset silently reads as the wrong instant"
        )
    };
    let (civil, offset_min) = if let Some(head) = s.strip_suffix('Z').or(s.strip_suffix('z')) {
        (head.to_string(), 0i64)
    } else {
        // The offset sign has to be looked for after the date, or `2026-09-02` splits on its own
        // dashes and the year becomes an offset.
        let Some(time_at) = s.find(['T', 't']) else {
            return Err(refuse("no time-of-day"));
        };
        let Some(sign_at) = s[time_at..].find(['+', '-']).map(|i| i + time_at) else {
            return Err(refuse("no offset"));
        };
        let (head, off) = s.split_at(sign_at);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let digits = &off[1..];
        let (oh, om) = match digits.split_once(':') {
            Some((h, m)) => (h.to_string(), m.to_string()),
            None if digits.len() == 4 => (digits[..2].to_string(), digits[2..].to_string()),
            None => return Err(refuse("malformed offset")),
        };
        let oh: i64 = oh.parse().map_err(|_| refuse("malformed offset"))?;
        let om: i64 = om.parse().map_err(|_| refuse("malformed offset"))?;
        // No real zone is past +/-14:00. An unchecked offset silently shifts the window, which is
        // the same class of error as the one that made this function refuse bare local times.
        if oh > 14 || om > 59 || (oh == 14 && om > 0) {
            return Err(refuse("offset out of range"));
        }
        (head.to_string(), sign * (oh * 60 + om))
    };
    let ms = civil_to_ms(&civil).ok_or_else(|| refuse("malformed date-time"))?;
    Ok(ms - offset_min * 60_000)
}

/// `YYYY-MM-DDTHH:MM[:SS[.fff]]` read as if it were UTC. The caller applies the offset.
fn civil_to_ms(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(['T', 't'])?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if d.next().is_some() || !(1..=12).contains(&m) || day < 1 || day > days_in_month(y, m) {
        return None;
    }
    let mut t = time.split(':');
    let hh: i64 = t.next()?.parse().ok()?;
    let mm: i64 = t.next()?.parse().ok()?;
    let (ss, frac) = match t.next() {
        Some(rest) => match rest.split_once('.') {
            Some((sec, f)) => (sec.parse::<i64>().ok()?, millis_of_fraction(f)?),
            None => (rest.parse::<i64>().ok()?, 0),
        },
        None => (0, 0),
    };
    if t.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some((days_from_civil(y, m, day) * 86_400 + hh * 3600 + mm * 60 + ss) * 1000 + frac)
}

/// `2026-02-31` is not a date. Accepting it silently moves the window by a day.
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// `None` for anything that is not all digits: `.5x` is not a fraction, and reading it as `.500`
/// would be inventing a timestamp the caller never wrote.
fn millis_of_fraction(f: &str) -> Option<i64> {
    if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = f.as_bytes();
    let mut ms = 0i64;
    for i in 0..3 {
        ms = ms * 10 + digits.get(i).map_or(0, |b| (b - b'0') as i64);
    }
    Some(ms)
}

/// Howard Hinnant's days-from-civil. Proleptic Gregorian, no table, no dependency: the alternative
/// was a date crate for one conversion in a dev-only binary.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The UTC instant, for the report to echo back. A window the reader cannot check is a window they
/// have to trust, and trusting it is what went wrong.
pub fn format_utc(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (hh, mm, ss) = (rem / 3_600_000, (rem / 60_000) % 60, (rem / 1000) % 60);
    // Inverse of days_from_civil.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn ts_ms(v: &Value) -> Option<i64> {
    let raw = v.get("timestamp").and_then(Value::as_str)?;
    parse_since(raw).ok()
}

/// Everything the shell would actually execute, with heredoc bodies removed.
///
/// Written because the first real run of this module mis-scored itself: the session was writing
/// this file's tests with `cat > adopt.rs <<'RS'`, one fixture line inside the heredoc read
/// `"cd /repo && cort impact --symbol 'a,b'"`, and splitting the raw command on `&&` turned a
/// string literal into an executed segment. A heredoc body is data being written to a file or a
/// program's stdin -- counting it as a tool call is the same category error as counting a `grep`
/// for "cort impact" in the docs.
fn shell_without_heredocs(command: &str) -> String {
    let mut out = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for line in command.lines() {
        if let Some(tag) = pending.first() {
            // The terminator is the tag alone on its line; `<<-` also allows leading tabs.
            if line.trim() == tag.as_str() {
                pending.remove(0);
            }
            continue;
        }
        out.push(line);
        let mut rest = line;
        while let Some(at) = rest.find("<<") {
            let after = rest[at + 2..].trim_start_matches('-');
            // `<<<` is a here-string, which is one line and has no body to skip.
            if after.starts_with('<') {
                rest = &rest[at + 2..];
                continue;
            }
            let quote = after.starts_with('\'') || after.starts_with('"');
            let body = if quote { &after[1..] } else { after };
            let tag: String = body
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !tag.is_empty() {
                pending.push(tag);
            }
            rest = &rest[at + 2..];
        }
    }
    out.join("\n")
}

/// Every place the shell would start a new command, honouring quotes.
///
/// The first version reached for `first_segment`, which returns only the text before the first `;`
/// or `|` -- so `cd repo; cort impact --symbol x` was read as `cd repo` and the adoption was
/// missed. Splitting the raw string instead would find `cort impact` inside
/// `echo "run cort impact"`, so the split has to be quote-aware. Both mistakes move the funnel in
/// opposite directions and neither announces itself.
fn command_starts(line: &str) -> Vec<&str> {
    let (mut single, mut double) = (false, false);
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < line.len() {
        let c = bytes[i] as char;
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ';' | '|' | '&' if !single && !double => {
                out.push(&line[start..i]);
                // `&&` and `||` are two bytes; a single `&` or `|` is one.
                i += if i + 1 < line.len() && bytes[i + 1] == bytes[i] {
                    2
                } else {
                    1
                };
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&line[start..]);
    out
}

/// Does some segment of this command line actually run `cort impact`? Returns the `--symbol` value
/// when one is given. A command that merely contains the string -- a heredoc writing this very
/// module, a doc being edited -- is not an execution and must not be counted as one.
pub fn runs_cort_impact(command: &str) -> Option<Option<String>> {
    let command = shell_without_heredocs(command);
    for segment in command.lines().flat_map(command_starts) {
        let tokens = tokenize(segment.trim());
        let mut idx = 0;
        while idx < tokens.len() && tokens[idx].contains('=') && !tokens[idx].starts_with('-') {
            idx += 1;
        }
        let Some(head) = tokens.get(idx) else {
            continue;
        };
        if head.rsplit('/').next().unwrap_or(head) != "cort" {
            continue;
        }
        if tokens.get(idx + 1).map(String::as_str) != Some("impact") {
            continue;
        }
        let symbol = tokens
            .iter()
            .position(|t| t == "--symbol")
            .and_then(|i| tokens.get(i + 1))
            .cloned();
        return Some(symbol);
    }
    None
}

/// The denominator has to be what the *hook* evaluates, or the funnel divides by the wrong number.
///
/// `suggests_impact` skips leading `VAR=value` assignments and a `sudo`-style prefix before it
/// looks at the verb; this predicate did not, so `LC_ALL=C rg helper src` was dropped from
/// `searches` and from `shape_would_fire` while the installed hook fired on it. Same input, two
/// answers, and the disagreement was invisible because both numbers stayed plausible.
fn is_search(command: &str) -> bool {
    let tokens = tokenize(first_segment(command.trim()));
    let mut idx = 0;
    while idx < tokens.len() && tokens[idx].contains('=') && !tokens[idx].starts_with('-') {
        idx += 1;
    }
    if tokens.get(idx).map(String::as_str) == Some("sudo") {
        idx += 1;
    }
    tokens
        .get(idx)
        .map(|t| {
            let base = t.rsplit('/').next().unwrap_or(t);
            base == "rg" || base == "grep" || base == "egrep"
        })
        .unwrap_or(false)
}

/// The symbol the injected line recommends a seed for.
fn injected_symbol(text: &str) -> Option<String> {
    let at = text.find(INJECTION_MARKER)? + INJECTION_MARKER.len();
    let rest = &text[at..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

struct Injection {
    project: String,
    session: String,
    ts: i64,
    symbol: String,
    tool_use_id: Option<String>,
}

struct BashCall {
    id: Option<String>,
    ts: i64,
    command: String,
}

/// Transcript files, excluding the subagent sidechains §6 excludes: a subagent's transcript is not
/// a session anyone steered, and counting it doubles the sessions that spawned one.
fn session_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
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
            out.extend(session_files(&path, depth - 1));
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && !path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .starts_with("agent-")
        {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Mine the funnel from the Claude Code transcript tree. Codex is not read: it carries the skill
/// and no hook, so there is no injection stage there to measure.
/// How many Bash calls after the intercepted one may still count as taking the suggestion.
///
/// Unbounded was wrong. The first version accepted *any* later `cort impact` in the session, so an
/// audit command run an hour afterwards was scored as adoption of an injection it had nothing to do
/// with, and two injections could both claim the same one call. Adoption means the agent acted on
/// what it was just told; a few calls is the whole of that window, and anything past it is reported
/// as `impact_later_in_session` for a human to judge rather than counted.
pub const DEFAULT_FOLLOW_CALLS: usize = 5;

pub fn mine(
    claude_dir: &Path,
    since_ms: i64,
    usage_db: Option<&Path>,
    max_rows: usize,
    follow_calls: usize,
    exclude: &[String],
) -> Value {
    let mut sessions = 0usize;
    let mut searches = 0usize;
    let mut would_fire = 0usize;
    let mut injection_count = 0usize;
    let mut rows: Vec<Value> = Vec::new();
    let mut per_project: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut files_unreadable = 0usize;
    let mut lines_unparsed = 0usize;
    let mut records_without_timestamp = 0usize;
    let mut excluded_sessions = 0usize;
    let (mut adopted_same, mut adopted_other) = (0usize, 0usize);

    for file in session_files(claude_dir, 6) {
        let project = file
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        if exclude.iter().any(|e| e == &project) {
            excluded_sessions += 1;
            continue;
        }
        let session = file
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let Ok(text) = std::fs::read_to_string(&file) else {
            files_unreadable += 1;
            continue;
        };
        let mut calls: Vec<BashCall> = Vec::new();
        let mut here: Vec<Injection> = Vec::new();
        let (mut s_searches, mut s_fire) = (0usize, 0usize);
        let mut in_window = false;
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                lines_unparsed += 1;
                continue;
            };
            let Some(ts) = ts_ms(&v) else {
                // A record with no readable timestamp cannot be placed in or out of the window,
                // so it is counted rather than dropped -- a truncated transcript must arrive as
                // missing data and not as a confident `not_adopted`.
                //
                // Only records that could have carried something we read are counted. The first
                // version counted every line, and a transcript's ordinary furniture --
                // `file-history-snapshot`, `bridge-session`, `last-prompt` -- never carries a
                // timestamp, so it reported 4,210 "missing" records on a tree with nothing wrong
                // with it. A health counter that is loud when everything is fine is not a health
                // counter.
                if v.get("message").is_some() || v.get("attachment").is_some() {
                    records_without_timestamp += 1;
                }
                continue;
            };
            if ts < since_ms {
                continue;
            }
            in_window = true;
            if let Some(items) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
            {
                for item in items {
                    if item.get("type").and_then(Value::as_str) != Some("tool_use")
                        || item.get("name").and_then(Value::as_str) != Some("Bash")
                    {
                        continue;
                    }
                    let Some(command) = item
                        .get("input")
                        .and_then(|i| i.get("command"))
                        .and_then(Value::as_str)
                    else {
                        continue;
                    };
                    if is_search(command) {
                        s_searches += 1;
                        if suggests_impact_shape(command).is_some() {
                            s_fire += 1;
                        }
                    }
                    calls.push(BashCall {
                        id: item.get("id").and_then(Value::as_str).map(str::to_string),
                        ts,
                        command: command.to_string(),
                    });
                }
            }
            // The injection. `hookName` must be checked: the SessionStart hook that installs a
            // skill uses the same attachment type, and a naive scan for the type counts its
            // preamble as an interception.
            let Some(att) = v.get("attachment") else {
                continue;
            };
            if att.get("type").and_then(Value::as_str) != Some("hook_additional_context")
                || !att
                    .get("hookName")
                    .and_then(Value::as_str)
                    .map(|n| n.starts_with("PreToolUse"))
                    .unwrap_or(false)
            {
                continue;
            }
            let content = match att.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            let Some(symbol) = injected_symbol(&content) else {
                continue;
            };
            here.push(Injection {
                project: project.clone(),
                session: session.clone(),
                ts,
                symbol,
                tool_use_id: att
                    .get("toolUseID")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
        if !in_window {
            continue;
        }
        sessions += 1;
        searches += s_searches;
        would_fire += s_fire;
        injection_count += here.len();
        {
            let entry = per_project.entry(project.clone()).or_default();
            bump(entry, "sessions", 1);
            bump(entry, "searches", s_searches as i64);
            bump(entry, "injections", here.len() as i64);
        }

        // One call can only be taken once. Without this, two injections a few lines apart both
        // claimed the same later `cort impact` and the funnel reported two adoptions from one act.
        //
        // Claiming is resolved in timestamp order, not file order. A compacted or resumed
        // transcript can carry records out of order, and in file order a later injection could take
        // the call that belongs to the earlier one -- a swap that leaves the totals identical and
        // both rows wrong. The set is still per file: two transcripts describing one real session
        // could each claim the same call, which is `A8` in the review and is not fixed here.
        let mut ordered: Vec<&Injection> = here.iter().collect();
        ordered.sort_by_key(|i| i.ts);
        let mut claimed: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for inj in ordered {
            // The harness names the tool call it attached the context to, which pairs the injection
            // with its trigger exactly. `parentUuid` adjacency was the fallback before this field
            // was confirmed on real records; it is not needed and would be looser.
            let exact = inj.tool_use_id.as_ref().and_then(|id| {
                calls
                    .iter()
                    .position(|c| c.id.as_deref() == Some(id.as_str()))
            });
            let trigger_at = exact.or_else(|| calls.iter().rposition(|c| c.ts <= inj.ts));
            let paired_exactly = exact.is_some();
            // Falling back to 0 was wrong. When neither the `toolUseID` nor any earlier call can be
            // found -- a truncated or compacted transcript -- the window became the session's first
            // five calls, which happened *before* the injection. A call that precedes the
            // suggestion cannot be an adoption of it, so with no trigger the window opens at the
            // first call the injection could possibly have caused.
            let start = match trigger_at {
                Some(i) => i + 1,
                None => calls
                    .iter()
                    .position(|c| c.ts >= inj.ts)
                    .unwrap_or(calls.len()),
            };
            let end = (start + follow_calls).min(calls.len());
            let follow = (start..end)
                .find(|i| !claimed.contains(i) && runs_cort_impact(&calls[*i].command).is_some());
            if let Some(i) = follow {
                claimed.insert(i);
            }
            // Beyond the window there may still be an `impact` call. It is reported, never counted:
            // "the agent used the tool later in a long session" is not "the agent took this
            // suggestion", and only a reader looking at the row can tell the two apart.
            let later = follow.is_none()
                && (end..calls.len()).any(|i| runs_cort_impact(&calls[i].command).is_some());
            let symbol_of = |i: usize| runs_cort_impact(&calls[i].command).unwrap_or(None);
            let verdict = match follow {
                None => "not_adopted",
                Some(i) => match symbol_of(i) {
                    Some(s) if s.split(',').any(|part| part.trim() == inj.symbol) => {
                        adopted_same += 1;
                        "adopted_same_symbol"
                    }
                    _ => {
                        adopted_other += 1;
                        "adopted_other_symbol"
                    }
                },
            };
            bump(
                per_project.entry(inj.project.clone()).or_default(),
                verdict,
                1,
            );
            if rows.len() < max_rows {
                rows.push(json!({
                    "project": inj.project,
                    "session": inj.session,
                    "at": format_utc(inj.ts),
                    "symbol": inj.symbol,
                    "verdict": verdict,
                    "paired_by": if paired_exactly { "toolUseID" } else { "nearest_earlier_call" },
                    "triggering_command": trigger_at.map(|i| truncate(&calls[i].command)),
                    "followed_by": follow.map(|i| truncate(&calls[i].command)),
                    "impact_later_in_session": later,
                }));
            }
        }
    }

    // Whether the db counts can be set beside `injections` at all.
    //
    // They were printed side by side unconditionally, and with `--exclude` in play that produced
    // `injections=0` next to `hit=2` on a machine where nothing was wrong: the transcript side had
    // been filtered to a population the db side knows nothing about. A shortfall there reads as
    // "a second cache was in play", which is the one conclusion this field exists to support, so
    // manufacturing it is worse than omitting the field. The db carries cort's own `project_id`
    // hash while transcripts carry the harness's directory name, and this report does not map
    // between them -- so with any exclusion the comparison is refused rather than approximated.
    // The transcript side of this comparison is `--claude-dir`, so the db side must be restricted to
    // the rows Claude Code's hook wrote. Every harness that wires `cort hook-suggest` shares one
    // usage.db, and a Grok or Codex fire has no Claude transcript to match -- counted in, it would
    // raise `injections_recorded` while every guard still read green.
    const MINED_HARNESS: &str = "claude-code";
    let cross_check = usage_db.map(|path| {
        match cort::usage::hook_outcomes_at(path, since_ms, Some(MINED_HARNESS)) {
            Ok(counts) => {
                let get = |k: &str| counts.get(k).and_then(Value::as_i64).unwrap_or(0);
                // `hit_stale` is an injection too. Comparing against `hit` alone would make an
                // injection onto a behind-head index look like a lost row.
                let recorded = get("hit") + get("hit_stale");
                let legacy = get("legacy_unsplit");
                let mut blockers: Vec<String> = Vec::new();
                if !exclude.is_empty() {
                    blockers.push(
                    "--exclude filtered the transcript side; the db side is every project on this \
                     machine and cannot be filtered the same way"
                        .to_string(),
                );
                }
                if legacy > 0 {
                    blockers.push(format!(
                        "{legacy} rows predate outcome recording (`legacy_unsplit`) and cannot be \
                     attributed to either side"
                    ));
                }
                let unspecified = get("unspecified");
                if unspecified > 0 {
                    blockers.push(format!(
                        "{unspecified} rows predate harness recording (`unspecified`); they were \
                     written when only one harness was wired, but the row does not say so and this \
                     report will not assume it"
                    ));
                }
                let other = get("other_harness");
                if other > 0 {
                    blockers.push(format!(
                    "{other} rows were written by a harness other than `{MINED_HARNESS}` and have \
                     no counterpart in the transcripts this window read"
                ));
                }
                json!({
                    "outcomes": counts,
                    "injections_recorded": recorded,
                    "comparable_to_injections": blockers.is_empty(),
                    "not_comparable_because": blockers,
                })
            }
            Err(e) => json!({ "unreadable": e.to_string() }),
        }
    });

    json!({
        "method": "adopt-mine-v1",
        "window": {
            "since_ms": since_ms,
            "since_utc": format_utc(since_ms),
        },
        "sessions_in_window": sessions,
        "searches": searches,
        "shape_would_fire": would_fire,
        "injections": injection_count,
        "adopted_same_symbol": adopted_same,
        "adopted_other_symbol": adopted_other,
        "not_adopted": injection_count.saturating_sub(adopted_same + adopted_other),
        "usage_db_cross_check": cross_check,
        "follow_calls_window": follow_calls,
        "excluded_projects": exclude,
        "excluded_sessions": excluded_sessions,
        "files_unreadable": files_unreadable,
        "lines_unparsed": lines_unparsed,
        "records_without_timestamp": records_without_timestamp,
        "by_project": per_project,
        "injection_rows": rows,
        "reading": "`injections` is how often the hook actually put the query in front of the \
                    agent, and it is the only stage that means anything on its own: `searches` is \
                    the population, and `shape_would_fire` is the offline matcher's opinion of the \
                    SHAPE half only -- since 2026-09-04 the shipped rule also asks the index \
                    whether it holds a seed or a raw edge naming the symbol, and that half cannot \
                    be replayed here because the index state at the time of each historical search \
                    is not recoverable. It is therefore an upper bound on what would ship today, \
                    and the gap against `injections` now has TWO causes that must not be added \
                    together: `no_index` is the opportunity the gate declined (index the project \
                    and the hook would help), while `no_evidence` is a correct refusal (the \
                    project is indexed and holds nothing about that symbol). Only the first is a \
                    missed chance, \
                    which will disagree with `injections` wherever a project has no index -- that \
                    difference is the opportunity the gate declined, not a bug. Adoption is \
                    reported per injection and must be adjudicated row by row: `adopted_other_symbol` \
                    is usually the agent moving on rather than taking the suggestion, and a session \
                    that was auditing the hook adopts it for reasons no user shares -- pass \
                    `--exclude` for the project cort is developed in, or the funnel is measuring \
                    its own audit. Adoption is only counted inside `follow_calls_window` calls of \
                    the intercepted one; `impact_later_in_session` marks a row where the tool was \
                    used further on, which is a fact for a reader and not an adoption. Nothing here \
                    says the enumeration that followed was correct; `verify-impact` grades an edge. \
                    The adoption test reads shell syntax, not shell semantics, and the residue is \
                    named rather than hidden: a short-circuited branch (`false && cort impact`) is \
                    counted though it never ran, and `sh -c`, `xargs`, `env`, an alias, a command \
                    substitution or a prefix redirection are missed though they did. Read \
                    `followed_by` on each row; that is what it is printed for.",
        "cross_check_reading": "`injections_recorded` is `hit + hit_stale`: the injections cort \
                    itself recorded, from a file the transcript scan never reads. Compare it to \
                    `injections` ONLY when `comparable_to_injections` is true -- otherwise the two \
                    have different populations and a difference between them means nothing. When \
                    they are comparable, a shortfall means a second CORT_CACHE_DIR was in play, \
                    which is how the 09-01 baseline was lost. Two limits on the word `independent`: \
                    this report reads one cache (`--usage-db`) and cannot discover others, and a \
                    row is only written when the hook ran to completion, so a hook killed by the \
                    harness timeout is missing from both sides at once rather than from one.",
    })
}

fn bump(map: &mut Map<String, Value>, key: &str, by: i64) {
    let now = map.get(key).and_then(Value::as_i64).unwrap_or(0) + by;
    map.insert(key.to_string(), json!(now));
}

fn truncate(s: &str) -> String {
    let cleaned = s.replace('\n', " ");
    if cleaned.chars().count() <= 160 {
        cleaned
    } else {
        cleaned.chars().take(160).collect::<String>() + "…"
    }
}
