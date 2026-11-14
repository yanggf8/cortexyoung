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

fn files(dir: &Path, project: Option<&str>) -> Result<Vec<PathBuf>, String> {
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
        for entry in fs::read_dir(&project).map_err(|e| format!("{}: {e}", project.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", project.display()))?;
            if entry.path().extension().is_some_and(|s| s == "jsonl")
                && entry
                    .file_type()
                    .map_err(|e| format!("{}: {e}", entry.path().display()))?
                    .is_file()
            {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    Ok(out)
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

/// The first shell word after `cort memory recall`: double-quoted, single-quoted or bare. Flags
/// before it are skipped together with their value (`--harness x`; `--k=v` is one word). `None`
/// when nothing query-like follows (end of command, a pipe, a redirection).
pub fn extract_query(command: &str) -> Option<String> {
    const MARK: &str = "cort memory recall";
    let rest = &command[command.find(MARK)? + MARK.len()..];
    let mut chars = rest.chars().peekable();
    let mut skip_value = false;
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let first = *chars.peek()?;
        if matches!(first, '|' | ';' | '&' | '>' | '<' | ')' | '\n') {
            return None;
        }
        let word: String = if first == '"' || first == '\'' {
            chars.next();
            let mut w = String::new();
            for c in chars.by_ref() {
                if c == first {
                    break;
                }
                w.push(c);
            }
            if skip_value {
                skip_value = false;
                continue;
            }
            return Some(w).filter(|w| !w.trim().is_empty());
        } else {
            let mut w = String::new();
            while let Some(c) = chars.next_if(|c| !c.is_whitespace() && !"|;&<>)".contains(*c)) {
                w.push(c);
            }
            if chars.peek().is_some_and(|c| matches!(c, '>' | '<'))
                && w.chars().all(|c| c.is_ascii_digit())
            {
                return None; // `2>&1`, not a query
            }
            w
        };
        if skip_value {
            skip_value = false;
            continue;
        }
        if word.starts_with("--") {
            skip_value = !word.contains('=');
            continue;
        }
        return Some(word).filter(|w| !w.is_empty());
    }
}

/// Active ledger entries from a `cort memory list` JSON file.
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
    for path in &paths {
        let name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
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
                    if item["name"] != "Bash" || !command.contains("cort memory recall") {
                        continue;
                    }
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
                    if let Some(entries) = ledger {
                        match extract_query(command) {
                            None => r_query_unparsed += 1,
                            Some(query) => {
                                let (ids, partial) = recall_selection(&query, entries);
                                let counts = if partial { &mut r_partial } else { &mut r_full };
                                counts[bucket(ids.len())] += 1;
                                r_queries.insert(query);
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
            "queries": r_queries.len(),
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
        assert_eq!(replay["queries"], 3);
        assert_eq!(replay["query_unparsed"], 1);
        assert_eq!(replay["full"]["2-5"], 1);
        assert_eq!(replay["full"]["6-20"], 2);
        assert_eq!(replay["partial"]["2-5"], 1);
        assert_eq!(replay["share_full_le_5"], 1.0 / 3.0);
        assert_eq!(report["recalls"], 5);
    }
}
