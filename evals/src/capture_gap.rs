//! Aggregate Claude Code turns that write private notes and/or record ledger entries.
//! Bash-written memory files are not counted as private edits.

use crate::{demand, time, uptake};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Turn {
    timestamp: String,
    text: String,
    ledger: bool,
    private: bool,
    included: bool,
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

pub fn scan(
    dir: &Path,
    project: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
    show: bool,
) -> Result<Value, String> {
    // `parse_since` refuses a bare date or a local time without an offset, as every other
    // windowed report here does: transcript timestamps are UTC.
    let start = since.map(time::parse_since).transpose()?;
    let end = until.map(time::parse_since).transpose()?;
    if let (Some(s), Some(e)) = (start, end) {
        if s >= e {
            return Err("--since must precede --until".to_string());
        }
    }
    let paths = files(dir, project)?;
    let mut counts = [0u64; 4];
    let mut unparsed = 0u64;
    // Under a window, a turn whose opening line carries no usable timestamp is counted here,
    // never guessed into or out of the share (as `memory-uptake`'s `undated_skipped`).
    let mut undated = 0u64;
    for path in &paths {
        let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut turn: Option<Turn> = None;
        let finish = |turn: Turn, counts: &mut [u64; 4]| {
            if !turn.included {
                return;
            }
            let index = match (turn.ledger, turn.private) {
                (true, false) => 0,
                (true, true) => 1,
                (false, true) => 2,
                (false, false) => 3,
            };
            counts[index] += 1;
            if show && turn.private {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                eprintln!(
                    "{} {} {}",
                    turn.timestamp,
                    stem.chars().take(8).collect::<String>(),
                    demand::excerpt(&demand::scrub_user(&turn.text))
                );
            }
        };
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|e| format!("{}: {e}", path.display()))?;
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => {
                    unparsed += 1;
                    continue;
                }
            };
            if let Some((_, text)) = demand::claude_user_line(&line) {
                if let Some(previous) = turn.take() {
                    finish(previous, &mut counts);
                }
                let timestamp = value["timestamp"].as_str().unwrap_or("").to_string();
                let included = if start.is_none() && end.is_none() {
                    true
                } else if let Ok(t) = time::parse_since(&timestamp) {
                    start.is_none_or(|s| t >= s) && end.is_none_or(|e| t < e)
                } else {
                    undated += 1;
                    false
                };
                turn = Some(Turn {
                    timestamp,
                    text,
                    included,
                    ..Turn::default()
                });
            } else if value["type"] == "assistant" && value["isSidechain"] != true {
                // A sidechain line is a subagent's, not the agent answering the user; its user
                // lines are already refused by `claude_user_line`.
                if let (Some(turn), Some(content)) =
                    (&mut turn, value["message"]["content"].as_array())
                {
                    for item in content.iter().filter(|v| v["type"] == "tool_use") {
                        let name = item["name"].as_str().unwrap_or("");
                        let input = &item["input"];
                        if name == "Bash"
                            && uptake::command_purpose(input["command"].as_str().unwrap_or("")).2
                        {
                            turn.ledger = true;
                        }
                        if matches!(name, "Write" | "Edit" | "MultiEdit") {
                            if let Some(p) = input["file_path"].as_str().map(Path::new) {
                                if p.file_name().is_some_and(|n| n != "MEMORY.md")
                                    && cort::notes_kick::is_memory_edit(p, dir)
                                {
                                    turn.private = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(turn) = turn {
            finish(turn, &mut counts);
        }
    }
    let memory = counts[1] + counts[2];
    Ok(
        json!({"report":"capture-gap", "since":since, "until":until, "project":project.map(redact_project), "turns":counts.iter().sum::<u64>(), "ledger_only":counts[0], "both":counts[1], "private_only":counts[2], "neither":counts[3], "memory_turns":memory, "private_only_share":{"numerator":counts[2], "denominator":memory, "value":if memory == 0 { None } else { Some(counts[2] as f64 / memory as f64) }}, "files_read":paths.len(), "lines_unparsed":unparsed, "undated_skipped":undated}),
    )
}

/// A Claude Code project directory name is its working directory with `/` turned into `-`, so it
/// carries the account name; the report says `<user>` instead, like `demand`'s excerpts.
fn redact_project(name: &str) -> String {
    let Some(user) = demand::home_user().filter(|u| !u.is_empty()) else {
        return name.to_string();
    };
    name.split('-')
        .map(|part| if part == user { "<user>" } else { part })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn user(source: &str, timestamp: &str, text: &str) -> Value {
        json!({"type":"user", "promptSource":source, "timestamp":timestamp, "message":{"content":text}})
    }
    fn tool(name: &str, input: Value) -> Value {
        json!({"type":"assistant", "message":{"content":[{"type":"tool_use", "name":name, "input":input}]}})
    }
    fn write_lines(path: &Path, lines: &[Value]) {
        fs::write(
            path,
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\nnot json\n",
        )
        .unwrap();
    }

    #[test]
    fn classifies_turns_and_filters_projects_and_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let memory = project.join("memory");
        fs::create_dir_all(memory.join("nested")).unwrap();
        fs::create_dir_all(project.join("subagents")).unwrap();
        fs::create_dir_all(tmp.path().join("other")).unwrap();
        let private = |name: &str, path: PathBuf| tool(name, json!({"file_path":path}));
        let ledger = || {
            tool(
                "Bash",
                json!({"command":"true && cort memory record --key x"}),
            )
        };
        let lines = vec![
            ledger(),
            user("typed", "2026-10-27T00:00:00Z", "secret message one"),
            ledger(),
            user("system", "2026-10-27T01:00:00Z", "not a turn"),
            json!({"type":"user", "promptSource":"typed", "message":{"content":[{"type":"tool_result"}]}}),
            user("queued", "2026-10-28T00:00:00+00:00", "secret message two"),
            private("Write", memory.join("note.md")),
            user("typed", "2026-10-29T00:00:00Z", "third"),
            private("MultiEdit", memory.join("another.md")),
            tool("Bash", json!({"command":"cort memory resolve x"})),
            user("typed", "2026-10-30T00:00:00Z", "fourth"),
            private("Edit", memory.join("MEMORY.md")),
            private("Write", memory.join("nested").join("note.md")),
            tool("Bash", json!({"command":"echo 'cort memory record x'"})),
            {
                let mut sidechain = private("Write", memory.join("sidechain.md"));
                sidechain["isSidechain"] = json!(true);
                sidechain
            },
            json!({"type":"user", "promptSource":"typed", "message":{"content":"undated"}}),
        ];
        write_lines(&project.join("abcdefgh-more.jsonl"), &lines);
        write_lines(
            &project.join("subagents/ignored.jsonl"),
            &[user("typed", "2026-10-28T00:00:00Z", "ignored")],
        );
        write_lines(
            &tmp.path().join("other/other.jsonl"),
            &[user("typed", "2026-10-28T00:00:00Z", "other")],
        );
        let report = scan(tmp.path(), Some("project"), None, None, false).unwrap();
        assert_eq!(
            (
                report["turns"].as_u64(),
                report["ledger_only"].as_u64(),
                report["both"].as_u64(),
                report["private_only"].as_u64(),
                report["neither"].as_u64()
            ),
            (Some(5), Some(1), Some(1), Some(1), Some(2))
        );
        // Without a window nothing is dated or skipped.
        assert_eq!(report["undated_skipped"], 0);
        assert_eq!(report["private_only_share"]["value"], 0.5);
        assert_eq!(report["files_read"], 1);
        assert_eq!(report["lines_unparsed"], 1);
        assert!(!report.to_string().contains("secret message"));
        let window = scan(
            tmp.path(),
            Some("project"),
            Some("2026-10-28T00:00:00Z"),
            Some("2026-10-30T08:00:00+08:00"),
            false,
        )
        .unwrap();
        assert_eq!(window["turns"], 2);
        assert_eq!(window["undated_skipped"], 1);
        assert_eq!(
            scan(tmp.path(), None, None, None, false).unwrap()["turns"],
            6
        );
        for (since, until) in [
            ("2026-10-30T00:00:00Z", "2026-10-28T00:00:00Z"),
            ("2026-10-28T00:00:00Z", "2026-10-28T00:00:00Z"),
        ] {
            assert!(scan(tmp.path(), None, Some(since), Some(until), false).is_err());
        }
        if let Some(user) = demand::home_user().filter(|u| !u.is_empty()) {
            let redacted = redact_project(&format!("-home-{user}-a-repo"));
            assert_eq!(redacted, "-home-<user>-a-repo");
        }
        let empty = scan(
            tmp.path(),
            Some("project"),
            Some("2026-10-30T00:00:00Z"),
            None,
            false,
        )
        .unwrap();
        assert!(empty["private_only_share"]["value"].is_null());
        // A bare date has no offset and is refused, like every other windowed report.
        assert!(scan(tmp.path(), None, Some("2026-10-28"), None, false).is_err());
        assert!(scan(tmp.path(), None, Some("2026-02-30T00:00:00Z"), None, false).is_err());
    }
}
