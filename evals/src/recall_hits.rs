//! How many hits do real `cort memory recall` calls return?
//!
//! Walks Claude Code transcripts, pairs each `Bash` call containing `cort memory recall` with its
//! `tool_result` (by `tool_use_id`) and buckets the length of the `hits` array, split by whether
//! the recall was a full or a `partial` match. A call whose result is missing, truncated or not
//! JSON is counted as `unparsed`, never dropped.

use crate::time;
use cort::memory::{recall_selection, MemoryEntry};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const BUCKETS: [&str; 5] = ["0", "1", "2-5", "6-20", "21+"];
const DAY_MS: i64 = 86_400_000;

fn bucket(hits: usize) -> usize {
    match hits {
        0 => 0,
        1 => 1,
        2..=5 => 2,
        6..=20 => 3,
        _ => 4,
    }
}

/// Transcript files as `(project directory name, path)`, session files and subagent files alike.
fn files(dir: &Path, project: Option<&str>) -> Result<Vec<(String, PathBuf)>, String> {
    let mut projects = Vec::new();
    if let Some(name) = project {
        if Path::new(name).components().count() != 1
            || !matches!(
                Path::new(name).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err("--project must be a directory name".into());
        }
        projects.push(dir.join(name));
    } else {
        for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            if entry
                .file_type()
                .map_err(|e| format!("{}: {e}", entry.path().display()))?
                .is_dir()
            {
                projects.push(entry.path());
            }
        }
    }
    let mut out = Vec::new();
    for project in projects {
        let name = project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut found = Vec::new();
        jsonl_in(&project, &mut found)?;
        // `<project>/<session>/subagents/*.jsonl`: a subagent's recalls are recalls too.
        for entry in fs::read_dir(&project).map_err(|e| format!("{}: {e}", project.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", project.display()))?;
            let sub = entry.path().join("subagents");
            if entry
                .file_type()
                .map_err(|e| format!("{}: {e}", entry.path().display()))?
                .is_dir()
                && sub.is_dir()
            {
                jsonl_in(&sub, &mut found)?;
            }
        }
        out.extend(found.into_iter().map(|p| (name.clone(), p)));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

fn jsonl_in(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        if entry.path().extension().is_some_and(|s| s == "jsonl")
            && entry
                .file_type()
                .map_err(|e| format!("{}: {e}", entry.path().display()))?
                .is_file()
        {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// The text of a `tool_result`: a string, or the concatenated `text` parts of an array.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// `(hits, partial)` of a recall result, or `None` when it is not a JSON recall answer.
fn parse_recall(text: &str) -> Option<(usize, bool)> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;
    let hits = value["hits"].as_array()?.len();
    Some((hits, value["match"] == "partial"))
}

#[derive(Default)]
struct Project {
    recalls: u64,
    hits: Vec<usize>,
}

fn median(hits: &mut [usize]) -> Value {
    if hits.is_empty() {
        return Value::Null;
    }
    hits.sort_unstable();
    let mid = hits.len() / 2;
    if hits.len() % 2 == 1 {
        json!(hits[mid])
    } else {
        json!((hits[mid - 1] + hits[mid]) as f64 / 2.0)
    }
}

fn bucket_map(counts: &[u64; 5]) -> Value {
    let mut map = Map::new();
    for (name, n) in BUCKETS.iter().zip(counts) {
        map.insert((*name).to_string(), json!(n));
    }
    Value::Object(map)
}

const MARK: &str = "cort memory recall";

/// For each char: is it outside quotes, backticks and heredoc bodies? Only plain text can hold a
/// command, so a `cort memory recall` that is not plain is prose (`grep 'cort memory recall'`,
/// a commit message in a heredoc, a `sed` script) and is not an invocation.
fn plain_mask(cs: &[char]) -> Vec<bool> {
    let mut plain = vec![true; cs.len()];
    let mut heredocs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => {
                i += 2;
                continue;
            }
            q @ ('\'' | '`') => {
                plain[i] = false;
                i += 1;
                while i < cs.len() && cs[i] != q {
                    plain[i] = false;
                    i += 1;
                }
                if i < cs.len() {
                    plain[i] = false;
                }
                i += 1;
                continue;
            }
            '"' => {
                plain[i] = false;
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    if cs[i] == '\\' && i + 1 < cs.len() {
                        plain[i] = false;
                        i += 1;
                    }
                    plain[i] = false;
                    i += 1;
                }
                if i < cs.len() {
                    plain[i] = false;
                }
                i += 1;
                continue;
            }
            '<' if cs.get(i + 1) == Some(&'<') && cs.get(i + 2) != Some(&'<') => {
                let mut j = i + 2;
                if cs.get(j) == Some(&'-') {
                    j += 1;
                }
                while matches!(cs.get(j), Some(' ' | '\t')) {
                    j += 1;
                }
                let quote = cs.get(j).copied().filter(|c| matches!(c, '\'' | '"'));
                if quote.is_some() {
                    j += 1;
                }
                let mut word = String::new();
                while let Some(&c) = cs.get(j) {
                    let ends = match quote {
                        Some(q) => c == q,
                        None => c.is_whitespace() || "|;&<>()".contains(c),
                    };
                    if ends {
                        break;
                    }
                    word.push(c);
                    j += 1;
                }
                if !word.is_empty() {
                    heredocs.push(word);
                }
                // step over the closing quote of `<<'EOF'`
                i = if quote.is_some() { j + 1 } else { j };
                continue;
            }
            '\n' => {
                i += 1;
                for terminator in std::mem::take(&mut heredocs) {
                    while i < cs.len() {
                        let end = cs[i..]
                            .iter()
                            .position(|&c| c == '\n')
                            .map_or(cs.len(), |p| i + p);
                        let line: String = cs[i..end].iter().collect();
                        for p in &mut plain[i..end] {
                            *p = false;
                        }
                        i = (end + 1).min(cs.len());
                        if line.trim() == terminator {
                            break;
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    plain
}

/// Is the `cort` at `p` where a shell would run a command? After the start, `;`, `&`, `|`, `(`
/// (so `$(` and `&&`/`||` too), a newline, `do `/`then `, or a path ending in `/`.
fn at_command_position(cs: &[char], p: usize) -> bool {
    let mut q = p;
    if q > 0 && cs[q - 1] == '/' {
        while q > 0 && !cs[q - 1].is_whitespace() && !";&|()".contains(cs[q - 1]) {
            q -= 1;
        }
    }
    let mut r = q;
    while r > 0 && matches!(cs[r - 1], ' ' | '\t') {
        r -= 1;
    }
    if r == 0 || matches!(cs[r - 1], ';' | '&' | '|' | '(' | '\n') {
        return true;
    }
    if r == q {
        return false;
    }
    let before: String = cs[..r].iter().collect();
    ["do", "then"].iter().any(|k| {
        before.strip_suffix(k).is_some_and(|rest| {
            rest.chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    })
}

fn skip_blanks(cs: &[char], i: &mut usize) {
    loop {
        match (cs.get(*i), cs.get(*i + 1)) {
            (Some(' ' | '\t' | '\r'), _) => *i += 1,
            (Some('\\'), Some('\n')) => *i += 2,
            _ => break,
        }
    }
}

/// One shell word starting at `i`: quotes removed, `\"` and `\\` unescaped inside double quotes.
/// The flag says the word holds an expansion (`$`, backtick) that cannot be replayed. `None` at
/// the end of the command or at an operator.
fn read_word(cs: &[char], i: &mut usize) -> Option<(String, bool)> {
    skip_blanks(cs, i);
    let first = *cs.get(*i)?;
    if "|;&<>)\n".contains(first) {
        return None;
    }
    let (mut word, mut expands, mut any) = (String::new(), false, false);
    while let Some(&c) = cs.get(*i) {
        match c {
            '\'' => {
                any = true;
                *i += 1;
                while let Some(&d) = cs.get(*i).filter(|&&d| d != '\'') {
                    word.push(d);
                    *i += 1;
                }
                *i += 1;
            }
            '"' => {
                any = true;
                *i += 1;
                while let Some(&d) = cs.get(*i).filter(|&&d| d != '"') {
                    if d == '\\' && matches!(cs.get(*i + 1), Some('"' | '\\')) {
                        word.push(cs[*i + 1]);
                        *i += 2;
                        continue;
                    }
                    expands |= d == '$' || d == '`';
                    word.push(d);
                    *i += 1;
                }
                *i += 1;
            }
            '\\' => {
                if cs.get(*i + 1) == Some(&'\n') {
                    break;
                }
                any = true;
                if let Some(&n) = cs.get(*i + 1) {
                    word.push(n);
                }
                *i += 2;
            }
            _ if c.is_whitespace() || "|;&<>)".contains(c) => break,
            _ => {
                any = true;
                expands |= c == '$' || c == '`';
                word.push(c);
                *i += 1;
            }
        }
    }
    // `2>&1` and friends: a number glued to a redirection is not a word.
    if matches!(cs.get(*i), Some('<' | '>')) && word.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    any.then_some((word, expands))
}

/// The query of the invocation whose arguments start at `i`. Only `--root` and `--harness` take a
/// value; `--json`/`--help` do not, and after `--` the next word is the query.
fn query_at(cs: &[char], mut i: usize) -> Option<String> {
    let mut after_dashes = false;
    loop {
        let (word, expands) = read_word(cs, &mut i)?;
        if !after_dashes {
            match word.as_str() {
                "--" => {
                    after_dashes = true;
                    continue;
                }
                "--root" | "--harness" => {
                    read_word(cs, &mut i)?;
                    continue;
                }
                w if w.len() > 1 && w.starts_with('-') => continue,
                _ => {}
            }
        }
        return Some(word).filter(|w| !expands && !w.trim().is_empty());
    }
}

/// Every `cort memory recall` invocation in a shell command, as the query it was given (`None`
/// when none can be read: no argument, or an expansion like `"$q"`). A mention that is not at
/// command position or sits in quotes, backticks or a heredoc body is not an invocation.
pub fn invocations(command: &str) -> Vec<Option<String>> {
    let cs: Vec<char> = command.chars().collect();
    let mark: Vec<char> = MARK.chars().collect();
    let plain = plain_mask(&cs);
    let mut out = Vec::new();
    let mut p = 0;
    while p + mark.len() <= cs.len() {
        let end = p + mark.len();
        if cs[p..end] == mark[..]
            && plain[p]
            && cs
                .get(end)
                .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
            && at_command_position(&cs, p)
        {
            out.push(query_at(&cs, end));
            p = end;
        } else {
            p += 1;
        }
    }
    out
}

/// The query of the first invocation in `command`, if it has one that can be read.
pub fn extract_query(command: &str) -> Option<String> {
    invocations(command).into_iter().next().flatten()
}

/// Active ledger entries from a `cort memory list` JSON file. `recorded_at` is the listing's
/// ISO-seconds string, not the ledger's nanoseconds: it exists only so the entry deserialises,
/// and the replay uses these entries for counting, never for ordering by time.
pub fn load_ledger(path: &Path) -> Result<Vec<MemoryEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: not JSON: {e}", path.display()))?;
    let entries = value["entries"]
        .as_array()
        .ok_or_else(|| format!("{}: no top-level `entries` array", path.display()))?;
    entries
        .iter()
        .map(|e| {
            let field = |k: &str| e[k].as_str().map(str::to_string);
            let (Some(id), Some(key), Some(claim)) = (field("id"), field("key"), field("claim"))
            else {
                return Err(format!("{}: entry without id/key/claim", path.display()));
            };
            serde_json::from_value(json!({
                "id": id, "key": key, "claim": claim,
                "recorded_at": field("recorded_at_utc").unwrap_or_default(),
                "source": "", "source_line": 0, "source_sha256": "", "unsourced": true
            }))
            .map_err(|e| format!("{}: {e}", path.display()))
        })
        .collect()
}

pub fn scan(
    dir: &Path,
    project: Option<&str>,
    days: Option<u64>,
    now_ms: i64,
    ledger: Option<&[MemoryEntry]>,
) -> Result<Value, String> {
    let since_ms = match days {
        Some(d) => Some(
            i64::try_from(d)
                .ok()
                .and_then(|d| d.checked_mul(DAY_MS))
                .and_then(|span| now_ms.checked_sub(span))
                .ok_or("--days is too large")?,
        ),
        None => None,
    };
    let paths = files(dir, project)?;
    let mut full = [0u64; 5];
    let mut partial = [0u64; 5];
    let mut unparsed = 0u64;
    let mut undated = 0u64;
    let mut by_project: HashMap<String, Project> = HashMap::new();
    let mut r_full = [0u64; 5];
    let mut r_partial = [0u64; 5];
    let mut r_query_unparsed = 0u64;
    let mut r_queries: HashSet<String> = HashSet::new();
    let mut not_invoked = 0u64;
    for (name, path) in &paths {
        let name = name.clone();
        let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // tool_use id -> slot in `settled`, removed once its result arrives
        let mut settled: Vec<Option<(usize, bool)>> = Vec::new();
        let mut order: HashMap<String, usize> = HashMap::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|e| format!("{}: {e}", path.display()))?;
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let Some(content) = value["message"]["content"].as_array() else {
                continue;
            };
            if value["type"] == "assistant" {
                for item in content.iter().filter(|v| v["type"] == "tool_use") {
                    let command = item["input"]["command"].as_str().unwrap_or("");
                    let Some(id) = item["id"].as_str() else {
                        continue;
                    };
                    if item["name"] != "Bash" || !command.contains(MARK) {
                        continue;
                    }
                    let calls = invocations(command);
                    if let Some(since) = since_ms {
                        match value["timestamp"].as_str().map(time::parse_since) {
                            Some(Ok(t)) if t >= since => {}
                            Some(Ok(_)) => continue,
                            _ => {
                                undated += 1;
                                continue;
                            }
                        }
                    }
                    if calls.is_empty() {
                        not_invoked += 1;
                        continue;
                    }
                    if let Some(entries) = ledger {
                        for query in calls {
                            match query {
                                None => r_query_unparsed += 1,
                                Some(query) => {
                                    let (ids, partial) = recall_selection(&query, entries);
                                    let counts = if partial { &mut r_partial } else { &mut r_full };
                                    counts[bucket(ids.len())] += 1;
                                    r_queries.insert(query);
                                }
                            }
                        }
                    }
                    order.insert(id.to_string(), settled.len());
                    settled.push(None);
                }
            } else if value["type"] == "user" {
                for item in content.iter().filter(|v| v["type"] == "tool_result") {
                    let Some(id) = item["tool_use_id"].as_str() else {
                        continue;
                    };
                    if let Some(slot) = order.remove(id) {
                        settled[slot] = parse_recall(&result_text(&item["content"]));
                    }
                }
            }
        }
        let entry = by_project.entry(name).or_default();
        for result in settled {
            entry.recalls += 1;
            match result {
                None => unparsed += 1,
                Some((hits, is_partial)) => {
                    entry.hits.push(hits);
                    let counts = if is_partial { &mut partial } else { &mut full };
                    counts[bucket(hits)] += 1;
                }
            }
        }
    }
    let full_total: u64 = full.iter().sum();
    let partial_total: u64 = partial.iter().sum();
    let full_le_5: u64 = full[..3].iter().sum();
    let mut projects = Map::new();
    let mut names: Vec<_> = by_project.keys().cloned().collect();
    names.sort();
    for name in names {
        if let Some(mut p) = by_project.remove(&name) {
            projects.insert(
                name,
                json!({"recalls": p.recalls, "median_hits": median(&mut p.hits)}),
            );
        }
    }
    let replay = ledger.map(|entries| {
        let full_total: u64 = r_full.iter().sum();
        json!({
            "ledger_entries": entries.len(),
            "reading": "today's ledger snapshot answers past queries: entries recorded after a query are counted, entries superseded since are not; counts only, not order",
            "calls": r_full.iter().sum::<u64>() + r_partial.iter().sum::<u64>(),
            "distinct_queries": r_queries.len(),
            "query_unparsed": r_query_unparsed,
            "full": bucket_map(&r_full),
            "partial": bucket_map(&r_partial),
            "share_full_le_5": if full_total == 0 { Value::Null } else { json!(r_full[..3].iter().sum::<u64>() as f64 / full_total as f64) },
        })
    });
    let mut report = json!({
        "report": "recall-hits",
        "window": {
            "days": days,
            "since": since_ms.map(time::format_utc),
            "now": time::format_utc(now_ms),
            "undated_skipped": undated,
        },
        "project": project,
        "files_read": paths.len(),
        "recalls": full_total + partial_total + unparsed,
        "not_invoked": not_invoked,
        "unparsed": unparsed,
        "full": bucket_map(&full),
        "partial": bucket_map(&partial),
        "share_full_le_5": if full_total == 0 { Value::Null } else { json!(full_le_5 as f64 / full_total as f64) },
        "by_project": Value::Object(projects),
    });
    if let Some(replay) = replay {
        report["replay"] = replay;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, command: &str, ts: &str) -> Value {
        json!({"type":"assistant", "timestamp":ts, "message":{"content":[
            {"type":"tool_use", "id":id, "name":"Bash", "input":{"command":command}}]}})
    }
    fn result(id: &str, content: Value) -> Value {
        json!({"type":"user", "message":{"content":[
            {"type":"tool_result", "tool_use_id":id, "content":content}]}})
    }
    fn answer(hits: usize, matched: &str) -> String {
        json!({"match":matched, "hits": vec![json!({"id":"x"}); hits]}).to_string()
    }
    fn write_lines(path: &Path, lines: &[Value]) {
        let body = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, body + "\n").unwrap();
    }
    fn now() -> i64 {
        time::parse_since("2026-11-14T00:00:00Z").unwrap()
    }

    #[test]
    fn buckets_recalls_and_counts_unparsed() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj-a");
        fs::create_dir_all(&project).unwrap();
        let ts = "2026-11-13T00:00:00Z";
        write_lines(
            &project.join("s1.jsonl"),
            &[
                call("a", "cort memory recall \"x\"", ts),
                result("a", json!(answer(3, "full"))),
                call("b", "cd r && cort memory recall y", ts),
                result("b", json!([{"type":"text","text":answer(5, "partial")}])),
                call("c", "cort memory recall z", ts),
                result("c", json!(answer(40, "full"))),
                call("d", "cort memory recall w", ts),
                result("d", json!("Error: truncated {\"hits\": [")),
                call("e", "cort memory record --key k --claim c", ts),
                result("e", json!(answer(9, "full"))),
            ],
        );
        let report = scan(tmp.path(), Some("proj-a"), None, now(), None).unwrap();
        assert!(report.get("replay").is_none());
        assert_eq!(report["recalls"], 4);
        assert_eq!(report["unparsed"], 1);
        assert_eq!(report["full"]["2-5"], 1);
        assert_eq!(report["full"]["21+"], 1);
        assert_eq!(report["full"]["0"], 0);
        assert_eq!(report["partial"]["2-5"], 1);
        assert_eq!(report["partial"]["21+"], 0);
        assert_eq!(report["share_full_le_5"], 0.5);
        assert_eq!(report["by_project"]["proj-a"]["recalls"], 4);
        assert_eq!(report["by_project"]["proj-a"]["median_hits"], 5);
    }

    #[test]
    fn days_window_and_project_name_are_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj-a");
        fs::create_dir_all(&project).unwrap();
        write_lines(
            &project.join("s1.jsonl"),
            &[
                call("old", "cort memory recall a", "2026-10-01T00:00:00Z"),
                result("old", json!(answer(1, "full"))),
                call("new", "cort memory recall a", "2026-11-13T00:00:00Z"),
                result("new", json!(answer(0, "full"))),
            ],
        );
        let report = scan(tmp.path(), None, Some(7), now(), None).unwrap();
        assert_eq!(report["recalls"], 1);
        assert_eq!(report["full"]["0"], 1);
        assert_eq!(report["full"]["1"], 0);
        assert!(scan(tmp.path(), Some("../x"), None, now(), None).is_err());
    }

    #[test]
    fn extracts_the_query_word() {
        let q = |c: &str| extract_query(c);
        assert_eq!(
            q("cort memory recall \"a b\" | head"),
            Some("a b".to_string())
        );
        assert_eq!(q("cort memory recall 'x' 2>&1"), Some("x".to_string()));
        assert_eq!(
            q(". env.sh; cort memory recall foo | cut -c1-500"),
            Some("foo".to_string())
        );
        assert_eq!(
            q("cort memory recall --harness codex \"k m\""),
            Some("k m".to_string())
        );
        assert_eq!(q("cort memory recall"), None);
        assert_eq!(q("cort memory recall | head"), None);
        assert_eq!(q("cort memory recall 2>&1"), None);
        assert_eq!(q("cort memory list"), None);
    }

    #[test]
    fn only_command_position_counts_as_an_invocation() {
        let n = |c: &str| invocations(c).len();
        // prose, quoted text, heredoc bodies, sed scripts and backticks are not invocations
        assert_eq!(n("echo see `cort memory recall` for details"), 0);
        assert_eq!(n("grep -l 'cort memory recall' *.jsonl"), 0);
        assert_eq!(n("echo \"run cort memory recall x\""), 0);
        assert_eq!(
            n("git commit -F - <<'EOF'\nnote\ncort memory recall x\nEOF"),
            0
        );
        assert_eq!(n("cat <<-EOF\n\tcort memory recall x\n\tEOF\n"), 0);
        assert_eq!(n("sed 's/cort memory recall/x/' f"), 0);
        // command positions
        assert_eq!(n("cort memory recall a"), 1);
        assert_eq!(n("cd r && cort memory recall a"), 1);
        assert_eq!(n("x || cort memory recall a"), 1);
        assert_eq!(n("(cort memory recall a)"), 1);
        assert_eq!(n("echo $(cort memory recall a)"), 1);
        assert_eq!(n("echo hi\ncort memory recall a"), 1);
        assert_eq!(n("for q in a; do cort memory recall a; done"), 1);
        assert_eq!(n("if x; then cort memory recall a; fi"), 1);
        assert_eq!(n("/home/u/.local/bin/cort memory recall a"), 1);
        // after a heredoc ends, commands count again
        assert_eq!(
            n("cat <<'E'\ncort memory recall x\nE\ncort memory recall y"),
            1
        );
        // several invocations in one command
        assert_eq!(
            invocations("cort memory recall a; cort memory recall \"b c\""),
            vec![Some("a".to_string()), Some("b c".to_string())]
        );
    }

    #[test]
    fn query_extraction_handles_flags_escapes_and_expansions() {
        let q = extract_query;
        // shell expansions cannot be replayed
        assert_eq!(
            invocations("for q in \"a\" \"b\"; do cort memory recall \"$q\"; done"),
            vec![None]
        );
        assert_eq!(q("cort memory recall `date`"), None);
        assert_eq!(q("cort memory recall '$x'"), Some("$x".to_string()));
        // only --root and --harness take a value; --json and --help do not
        assert_eq!(
            q("cort memory recall --json \"a b\""),
            Some("a b".to_string())
        );
        assert_eq!(q("cort memory recall --help \"a\""), Some("a".to_string()));
        assert_eq!(
            q("cort memory recall --root /r --harness codex q1"),
            Some("q1".to_string())
        );
        assert_eq!(q("cort memory recall --root=/r q1"), Some("q1".to_string()));
        // after `--` the next word is the query, even if it looks like a flag
        assert_eq!(q("cort memory recall -- \"q\""), Some("q".to_string()));
        assert_eq!(q("cort memory recall -- --odd"), Some("--odd".to_string()));
        // escapes inside double quotes, and line continuations between words
        assert_eq!(
            q("cort memory recall \"say \\\"hi\\\" there\""),
            Some("say \"hi\" there".to_string())
        );
        assert_eq!(q("cort memory recall \"a\\\\b\""), Some("a\\b".to_string()));
        assert_eq!(
            q("cort memory recall \\\n  \"two words\""),
            Some("two words".to_string())
        );
    }

    #[test]
    fn scan_separates_not_invoked_and_replays_every_invocation() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj-a");
        let sub = project.join("sess1/subagents");
        fs::create_dir_all(&sub).unwrap();
        let ts = "2026-11-13T00:00:00Z";
        let ledger_path = tmp.path().join("ledger.json");
        fs::write(
            &ledger_path,
            json!({"entries":[{"id":"i1","key":"k","claim":"alpha beta","recorded_at_utc":"2026-11-01T00:00:00Z"}]}).to_string(),
        )
        .unwrap();
        write_lines(
            &project.join("s1.jsonl"),
            &[
                call("a", "grep -l 'cort memory recall' *.jsonl", ts),
                call(
                    "b",
                    "cort memory recall alpha; cort memory recall nomatch",
                    ts,
                ),
                result("b", json!(answer(1, "full"))),
            ],
        );
        write_lines(
            &sub.join("agent-1.jsonl"),
            &[call("c", "cort memory recall beta", ts)],
        );
        let ledger = load_ledger(&ledger_path).unwrap();
        let report = scan(tmp.path(), Some("proj-a"), None, now(), Some(&ledger)).unwrap();
        assert_eq!(report["files_read"], 2);
        assert_eq!(report["not_invoked"], 1);
        // one observed tool_result per command, two replayed invocations in it
        assert_eq!(report["recalls"], 2);
        assert_eq!(report["replay"]["calls"], 3);
        assert_eq!(report["replay"]["full"]["0"], 1);
        assert_eq!(report["replay"]["full"]["1"], 2);
        assert_eq!(report["by_project"]["proj-a"]["recalls"], 2);
    }

    #[test]
    fn replay_matches_recorded_queries_against_the_ledger() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj-a");
        fs::create_dir_all(&project).unwrap();
        let ts = "2026-11-13T00:00:00Z";
        let ledger_path = tmp.path().join("ledger.json");
        let entries: Vec<Value> = (1..=7)
            .map(|i| {
                let claim = if i <= 2 {
                    "alpha red blue"
                } else {
                    "alpha green"
                };
                json!({"id": format!("id{i}"), "key": format!("k{i}"), "claim": claim,
                       "recorded_at_utc": "2026-11-01T00:00:00Z"})
            })
            .collect();
        fs::write(&ledger_path, json!({"entries": entries}).to_string()).unwrap();
        write_lines(
            &project.join("s1.jsonl"),
            &[
                call("a", "cort memory recall \"red blue\" | head", ts),
                call("b", "cort memory recall alpha 2>&1", ts),
                call("c", "cort memory recall 'red zebra'", ts),
                call("d", "cort memory recall", ts),
                call("e", "cort memory recall alpha | cut -c1-9", ts),
            ],
        );
        let ledger = load_ledger(&ledger_path).unwrap();
        assert_eq!(ledger.len(), 7);
        let report = scan(tmp.path(), Some("proj-a"), None, now(), Some(&ledger)).unwrap();
        let replay = &report["replay"];
        assert_eq!(replay["distinct_queries"], 3);
        assert_eq!(replay["calls"], 4);
        assert!(replay["reading"]
            .as_str()
            .is_some_and(|r| r.contains("entries recorded after a query are counted")));
        assert_eq!(replay["query_unparsed"], 1);
        assert_eq!(replay["full"]["2-5"], 1);
        assert_eq!(replay["full"]["6-20"], 2);
        assert_eq!(replay["partial"]["2-5"], 1);
        assert_eq!(replay["share_full_le_5"], 1.0 / 3.0);
        assert_eq!(report["recalls"], 5);
    }
}
