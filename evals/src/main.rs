//! `cort-evals` — the evaluation driver. Dev-only binary; `install.sh` never touches it.
//!
//!   cort-evals run-agents    [--only <task>] [--arms rg+Read,cort] [--max-turns 40]
//!                            [--config-dir /tmp/cc-eval] [--cache-dir /tmp/cort-exp]
//!                            [--out <dir>] [--concurrency 2] [--jail-dir <dir>] [--jail]
//!   cort-evals verify-impact --repo <path> --symbols A,B [--depth 3]
//!   cort-evals summarize     <rows.json>... [--strict]

use cort_evals::arms::{self, build_args, build_env, build_row, make_jail, AGENT_ARMS};
use cort_evals::grade::{load_tasks, Task};
use cort_evals::stream::parse_stream;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

fn at(argv: &[String], name: &str, default: &str) -> String {
    argv.iter()
        .position(|a| a == name)
        .and_then(|i| argv.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn has(argv: &[String], name: &str) -> bool {
    argv.iter().any(|a| a == name)
}

// Options are looked up by exact name, so a token nobody recognises used to be *ignored*: `cort-evals
// run-agents --help` silently started the default 5-task eval into the default output directory,
// which is how a request for help nearly spent a sampling window. The same trap catches `--out=x`,
// which reads as an unknown token and would otherwise be dropped while `--out` fell back to its
// default. Options are therefore whitelisted per subcommand, and `--flag=value` is refused rather
// than quietly discarded.
const RUN_AGENTS_FLAGS: &[&str] = &[
    "--tasks",
    "--only",
    "--arms",
    "--max-turns",
    "--config-dir",
    "--cache-dir",
    "--jail-dir",
    "--jail",
    "--out",
    "--concurrency",
    "--delay-secs",
];
const VERIFY_IMPACT_FLAGS: &[&str] = &["--repo", "--depth", "--symbols"];
const SUMMARIZE_FLAGS: &[&str] = &["--strict"];

const USAGE_TOP: &str = "usage: cort-evals <run-agents|verify-impact|summarize> [options]";
const USAGE_RUN_AGENTS: &str = "usage: cort-evals run-agents [--tasks FILE] [--only ID[,ID...]] [--arms a,b] [--max-turns N] [--config-dir DIR] [--cache-dir DIR] [--jail-dir DIR] [--jail] [--out DIR] [--concurrency N] [--delay-secs N]";
const USAGE_VERIFY_IMPACT: &str =
    "usage: cort-evals verify-impact --repo DIR --symbols A,B [--depth N]";
const USAGE_SUMMARIZE: &str = "usage: cort-evals summarize [--strict] rows.json [rows.json...]";

/// The provider gates a sampling run on a rolling window, so "run these cells after the window
/// resets" is part of the experiment, not shell glue. Seconds rather than a wall-clock time: the
/// caller decides *when* and the runner only waits, which keeps no timezone parsing in here.
fn delay_secs(argv: &[String]) -> Result<u64, String> {
    let raw = at(argv, "--delay-secs", "0");
    raw.parse::<u64>().map_err(|_| {
        format!("--delay-secs must be a non-negative whole number of seconds (got {raw})")
    })
}

fn split_only(only: &str) -> Vec<String> {
    only.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn only_matches(only: &[String], id: &str) -> bool {
    only.is_empty() || only.iter().any(|want| want == id)
}

/// Sort is part of the artefact: rows.json is diffed and re-summarised across rounds, so an
/// order that depends on which thread finished first would make an identical run look like a
/// changed dataset.
fn sort_rows(rows: &mut [Value]) {
    rows.sort_by_key(|r| {
        format!(
            "{} {}",
            r["arm"].as_str().unwrap_or(""),
            r["task"].as_str().unwrap_or("")
        )
    });
}

/// A batch is not one thing. Until now rows.json was written only after every thread joined
/// successfully, so one refused cell threw away the cells that *had* run: six cells in one batch
/// meant one rejection could cost a whole sampling window. The counter is what lets a reader tell
/// "this batch has 4 cells" apart from "this batch lost 2 cells".
fn run_status_json(planned: usize, written: usize, state: &str) -> Value {
    json!({
        "planned_cells": planned,
        "written_cells": written,
        "state": state,
        "complete": planned == written,
        "rows": "rows.json",
        "reading": if planned == written {
            "every planned cell ran and was measured".to_string()
        } else {
            format!(
                "{} of {} planned cells are missing from rows.json: they did not run (refused,                  interrupted, or unmeasurable). Do not read this batch as complete.",
                planned - written,
                planned
            )
        }
    })
}

/// Records the honest batch state on *every* exit path, including the early `return Err(...)`s.
struct RunStatus {
    out_dir: String,
    planned: usize,
    written: Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for RunStatus {
    fn drop(&mut self) {
        let written = self.written.load(std::sync::atomic::Ordering::Relaxed);
        // "exited" rather than "complete": a Drop also runs on the `return Err(...)` paths, and
        // only the row count in rows.json says how much of the batch was actually measured.
        let body = run_status_json(self.planned, written, "exited");
        let path = format!("{}/run-status.json", self.out_dir);
        if let Err(err) = std::fs::write(&path, format!("{body}\n")) {
            eprintln!("could not record run status in {path}: {err}");
        }
    }
}

/// One `rows.json` plus whatever its batch claimed it should contain. Without this, F-18's
/// completeness record is an artefact nobody reads: a 4-of-6 batch would be aggregated as "4
/// cells" and look exactly like an experiment that only ever planned 4.
#[derive(Debug, Clone)]
struct BatchRead {
    rows_path: String,
    rows_count: usize,
    status: Option<Value>,
}

impl BatchRead {
    fn load(rows_path: &str, rows_count: usize) -> Self {
        let status_path = Path::new(rows_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("run-status.json");
        let status = std::fs::read_to_string(&status_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        Self {
            rows_path: rows_path.to_string(),
            rows_count,
            status,
        }
    }

    fn field(&self, key: &str) -> Option<usize> {
        self.status
            .as_ref()
            .and_then(|st| st[key].as_u64())
            .map(|v| v as usize)
    }

    /// What this batch says about itself, for the reader of the aggregate.
    fn report(&self) -> Value {
        match &self.status {
            None => json!({
                "rows": self.rows_path, "cells": self.rows_count,
                "run_status": "absent (predates F-18, nothing to check against)"
            }),
            Some(st) => json!({
                "rows": self.rows_path,
                "cells": self.rows_count,
                "planned_cells": st["planned_cells"],
                "written_cells": st["written_cells"],
                // A sidecar from the first cut of F-18 has no `state` key. Emitting null there
                // would leave a reader to guess between "never ran" and "ran, unrecorded".
                "state": st.get("state").cloned().unwrap_or(json!("unrecorded")),
                "complete": st["complete"],
            }),
        }
    }

    /// A reason to distrust this batch, or None. Pre-F-18 artefacts are deliberately not a
    /// problem: they were written before the counter existed, and calling them suspect would
    /// make every historical repro look broken.
    fn problem(&self) -> Option<String> {
        // Pre-F-18 artefacts are deliberately not a problem: they predate the counter, and
        // calling them suspect would make every historical repro look broken.
        self.status.as_ref()?;
        let planned = self.field("planned_cells");
        let written = self.field("written_cells");
        // Order matters. If rows.json and its sidecar disagree, the sidecar is the untrustworthy
        // one (a SIGKILLed run leaves "running / 0 written" next to four real cells) and reporting
        // "lost cells" would send a reader to look for cells that are sitting right there.
        if let Some(w) = written {
            if self.rows_count != w {
                return Some(format!(
                    "{}: rows.json holds {} cells but its run-status.json claims {} were written \
                     (state {:?}, planned {:?}) -- the batch was interrupted before it could report, \
                     so treat the counts as unreliable",
                    self.rows_path,
                    self.rows_count,
                    w,
                    self.status.as_ref().and_then(|st| st["state"].as_str()),
                    planned
                ));
            }
        }
        if let (Some(p), Some(w)) = (planned, written) {
            if w < p {
                return Some(format!(
                    "{}: {} of {} planned cells never made it into rows.json ({} measured)",
                    self.rows_path,
                    p - w,
                    p,
                    self.rows_count
                ));
            }
        }
        if self.status.as_ref().and_then(|st| st["complete"].as_bool()) == Some(false) {
            return Some(format!(
                "{}: run-status.json says this batch did not complete",
                self.rows_path
            ));
        }
        None
    }
}

fn wants_help(argv: &[String]) -> bool {
    argv.iter().any(|a| a == "--help" || a == "-h")
}

fn check_flags(argv: &[String], known: &[&str], usage: &str) -> Result<(), String> {
    for arg in argv {
        if !arg.starts_with("--") {
            continue;
        }
        let name = match arg.split_once('=') {
            Some(_) => {
                return Err(format!(
                    "{arg} is not supported, options take a separate value\n{usage}"
                ))
            }
            None => arg.as_str(),
        };
        if !known.contains(&name) {
            return Err(format!("unknown option {arg}\n{usage}"));
        }
    }
    Ok(())
}

/// Refuse unknown options before any work happens; print usage and succeed for `--help`.
fn guard_options(argv: &[String], known: &[&str], usage: &str) -> Result<(), String> {
    if wants_help(argv) {
        println!("{usage}");
        std::process::exit(0);
    }
    check_flags(argv, known, usage)
}

fn sanitize(arm: &str) -> String {
    arm.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '+' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn venue_head(repo: &str) -> Result<String, String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse failed in {repo}: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run_cell(
    task: &Task,
    arm: &str,
    max_turns: i64,
    env: &[(String, String)],
) -> Result<String, String> {
    let invocation = build_args(task, arm, max_turns);
    // Resolve the driver against the *parent's* PATH before the jail replaces it: the program is
    // looked up in the child's environment, so a bare "claude" plus PATH=jail is ENOENT — which is
    // exactly how a run came back with `cells: 0`.
    let program = arms::resolve_binary(&invocation.program)
        .ok_or_else(|| format!("{} not found on PATH", invocation.program))?;
    let output = Command::new(program)
        .args(&invocation.args)
        .current_dir(&invocation.cwd)
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("spawn {}: {e}", invocation.program))?;
    // A non-zero exit still leaves a usable transcript when the cell merely hit its turn cap.
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        return Err(format!(
            "{} produced no transcript (exit {:?}): {}",
            invocation.program,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(stdout)
}

fn run_agents(argv: &[String]) -> Result<(), String> {
    guard_options(argv, RUN_AGENTS_FLAGS, USAGE_RUN_AGENTS)?;
    let tasks_path = at(argv, "--tasks", "evals/tasks-graph.json");
    let doc = load_tasks(&tasks_path)?;
    let only = at(argv, "--only", "");
    let only_ids = split_only(&only);
    let tasks: Vec<Task> = doc
        .into_iter()
        .filter(|t| only_matches(&only_ids, &t.id))
        .collect();
    if tasks.is_empty() {
        return Err(format!("no task matched --only {only} in {tasks_path}"));
    }

    let arms_arg = at(argv, "--arms", "rg+Read,cort");
    let arms: Vec<String> = arms_arg.split(',').map(|s| s.trim().to_string()).collect();
    for arm in &arms {
        if !AGENT_ARMS.contains(&arm.as_str()) {
            return Err(format!("unknown arm {arm}"));
        }
    }

    let max_turns: i64 = at(argv, "--max-turns", "40")
        .parse()
        .map_err(|_| "--max-turns must be a number")?;
    let config_dir = at(argv, "--config-dir", "/tmp/cc-eval");
    let cache_dir = at(argv, "--cache-dir", "/tmp/cort-exp");
    let jail_root = at(argv, "--jail-dir", "/tmp/cc-jails");
    // A PATH jail looks like containment, but Claude Code normalises the Bash tool's PATH, so the
    // jail is quietly replaced: a live cell still reached for `grep` (and `ToolSearch`, and an
    // absolute `/usr/bin/grep`) while cort could no longer find `ast-grep`. Opt-in, and never the
    // control — `arm_held` is what decides whether a cell may be read as a comparison.
    let jailed = has(argv, "--jail");
    let out_dir = at(argv, "--out", "evals/runs/2026-08-30-graph");
    let concurrency = at(argv, "--concurrency", "2")
        .parse::<usize>()
        .map_err(|_| "--concurrency must be a number")?
        .max(1);

    if !Path::new(&config_dir).is_dir() {
        return Err(format!(
            "config dir {config_dir} does not exist; the user settings would add ~16k tokens of noise per request"
        ));
    }
    if jailed {
        for arm in &arms {
            make_jail(
                &format!("{jail_root}/{}", sanitize(arm)),
                &arms::arm_binaries(arm),
            )?;
        }
    }

    let delay = delay_secs(argv)?;
    if delay > 0 {
        println!(
            "delaying {delay}s before the first cell ({} tasks x {} arms); nothing has run yet",
            tasks.len(),
            arms.len()
        );
        let mut left = delay;
        while left > 0 {
            let step = left.min(60);
            std::thread::sleep(std::time::Duration::from_secs(step));
            left -= step;
            if left % 600 == 0 {
                eprintln!("delay: {left}s remaining");
            }
        }
        eprintln!("delay: window reached, starting cells");
    }

    // The status file and the first rows.json land before any cell runs, so an interrupted batch
    // is distinguishable from a batch that never started.
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{out_dir}: {e}"))?;
    std::fs::write(format!("{out_dir}/rows.json"), "[]\n")
        .map_err(|e| format!("{out_dir}/rows.json: {e}"))?;
    let head = venue_head(&tasks[0].venue)?;
    let work: Vec<(Task, String)> = tasks
        .iter()
        .cloned()
        .flat_map(|t| arms.iter().map(move |a| (t.clone(), a.clone())))
        .collect();

    let planned = work.len();
    let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let _status = RunStatus {
        out_dir: out_dir.clone(),
        planned,
        written: Arc::clone(&written),
    };
    // Claim "running" before the first cell. A process that is SIGKILLed never reaches Drop, and
    // a half-written batch with no status file at all is indistinguishable from a pre-F-18 run.
    std::fs::write(
        format!("{out_dir}/run-status.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&run_status_json(planned, 0, "running")).unwrap()
        ),
    )
    .map_err(|e| format!("{out_dir}/run-status.json: {e}"))?;
    let queue = Arc::new(Mutex::new(work.into_iter()));
    let sink = Arc::new(Mutex::new(Vec::<Value>::new()));
    let mut threads = Vec::new();
    for _ in 0..concurrency {
        let queue = Arc::clone(&queue);
        let sink = Arc::clone(&sink);
        let written = Arc::clone(&written);
        let out_dir = out_dir.clone();
        let head = head.clone();
        let (config_dir, cache_dir, jail_root) =
            (config_dir.clone(), cache_dir.clone(), jail_root.clone());
        threads.push(std::thread::spawn(move || -> Result<Vec<Value>, String> {
            let mut rows = Vec::new();
            loop {
                let next = {
                    let mut guard = queue.lock().unwrap();
                    guard.next()
                };
                let Some((task, arm)) = next else { break };
                let dir = format!("{}/{}", out_dir, sanitize(&arm));
                std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
                let jail_path = format!("{jail_root}/{}", sanitize(&arm));
                let jail_dir = if jailed {
                    Some(jail_path.as_str())
                } else {
                    None
                };
                let env = build_env(&config_dir, &cache_dir, jail_dir);
                let stdout = run_cell(&task, &arm, max_turns, &env)?;
                std::fs::write(format!("{dir}/{}.stream.jsonl", task.id), &stdout)
                    .map_err(|e| format!("{dir}: {e}"))?;
                let parsed = parse_stream(&stdout).map_err(|e| format!("{}: {e}", task.id))?;
                let row = build_row(&arm, &task, &parsed, &head, Some(jailed))
                    .map_err(|e| format!("{}: {e}", task.id))?;
                eprintln!(
                    "{}/{}: coverage={} precision={} tokens={} tool_return={}",
                    arm,
                    task.id,
                    row["coverage"],
                    row["precision"],
                    row["total_tokens"],
                    row["tool_return_tokens"]
                );
                std::fs::write(
                    format!("{dir}/{}.json", task.id),
                    format!("{}\n", serde_json::to_string_pretty(&row).unwrap()),
                )
                .map_err(|e| format!("{dir}: {e}"))?;
                rows.push(row.clone());
                // Publish the cell the moment it is measured, not at join time.
                {
                    let mut guard = sink.lock().unwrap();
                    guard.push(row);
                    sort_rows(&mut guard);
                    std::fs::write(
                        format!("{out_dir}/rows.json"),
                        format!(
                            "{}\n",
                            serde_json::to_string_pretty(&*guard)
                                .unwrap_or_else(|_| "[]".to_string())
                        ),
                    )
                    .map_err(|e| format!("{out_dir}/rows.json: {e}"))?;
                    written.store(guard.len(), std::sync::atomic::Ordering::Relaxed);
                }
            }
            Ok(rows)
        }));
    }
    // A cell that fails must fail the run. `join()` only reports a panic, so the thread's own
    // Err has to be propagated as well — swallowing it produced a clean-looking run with
    // `cells: 0`, which is the silent-failure class this harness exists to prevent.
    let mut rows: Vec<Value> = Vec::new();
    for t in threads {
        match t.join() {
            Ok(Ok(mut produced)) => rows.append(&mut produced),
            Ok(Err(err)) => return Err(format!("cell failed: {err}")),
            Err(e) => return Err(format!("cell thread panicked: {e:?}")),
        }
    }
    if rows.is_empty() {
        return Err("no rows produced".to_string());
    }
    sort_rows(&mut rows);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{out_dir}: {e}"))?;
    std::fs::write(
        format!("{out_dir}/rows.json"),
        format!("{}\n", serde_json::to_string_pretty(&json!(rows)).unwrap()),
    )
    .map_err(|e| format!("{out_dir}/rows.json: {e}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "gate": { "coverage": cort_evals::grade::GATE_COVERAGE, "precision": cort_evals::grade::GATE_PRECISION },
            "venue_head": head,
            "cells": rows.len(),
            "jailed": jailed,
            "out": out_dir,
        }))
        .unwrap()
    );
    Ok(())
}

fn verify_impact_main(argv: &[String]) -> Result<(), String> {
    guard_options(argv, VERIFY_IMPACT_FLAGS, USAGE_VERIFY_IMPACT)?;
    let repo = at(argv, "--repo", ".");
    let depth: i64 = at(argv, "--depth", "3")
        .parse()
        .map_err(|_| "--depth must be a number")?;
    let symbols: Vec<String> = at(argv, "--symbols", "")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if symbols.is_empty() {
        return Err("verify-impact needs --symbols A,B".to_string());
    }
    let cort = arms::cort_bin();
    let mut report = Vec::new();
    for s in &symbols {
        report.push(cort_evals::verify::verify_impact(&cort, &repo, s, depth)?);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "repo": repo, "depth": depth, "report": report }))
            .unwrap()
    );
    Ok(())
}

fn summarize_main(argv: &[String]) -> Result<(), String> {
    guard_options(argv, SUMMARIZE_FLAGS, USAGE_SUMMARIZE)?;
    // Accepts several row files so a multi-run experiment aggregates in one call, which is what
    // the two-question review actually needs: 5 tasks x 2 arms landed in 5 directories.
    let paths: Vec<String> = argv
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err("summarize needs at least one rows.json path".to_string());
    }
    let mut rows: Vec<Value> = Vec::new();
    let mut batches: Vec<BatchRead> = Vec::new();
    for path in &paths {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let parsed: Vec<Value> = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
        batches.push(BatchRead::load(path, parsed.len()));
        rows.extend(parsed);
    }
    let problems: Vec<String> = batches.iter().filter_map(BatchRead::problem).collect();
    let strict = has(argv, "--strict");
    // Fail closed on an incomplete batch under --strict; otherwise still surface it, because the
    // whole point of the counter is that nobody has to guess whether the numbers are whole.
    if strict && !problems.is_empty() {
        return Err(format!(
            "refusing to aggregate an incomplete batch:\n  {}",
            problems.join("\n  ")
        ));
    }
    let mut out = cort_evals::summary::summarize(&rows, strict)?;
    if let Value::Object(map) = &mut out {
        map.insert(
            "batches".to_string(),
            json!(batches.iter().map(BatchRead::report).collect::<Vec<_>>()),
        );
        map.insert("batch_problems".to_string(), json!(problems));
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    Ok(())
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let result = match argv.first().map(String::as_str) {
        Some("run-agents") => run_agents(&argv[1..]),
        Some("verify-impact") => verify_impact_main(&argv[1..]),
        Some("summarize") => summarize_main(&argv[1..]),
        other => {
            // Asking how to use the tool is never an error and never a run — including at the top
            // level, which is where F-17 stopped. `wants_help` already answered true for a bare
            // `--help`; main() simply never consulted it, so the predicate had a unit test while
            // the binary exited 2. Asserted end to end in tests/harness.rs for exactly that reason.
            if other == Some("help") || wants_help(&argv) {
                println!("{USAGE_TOP}");
                println!("  {USAGE_RUN_AGENTS}");
                println!("  {USAGE_VERIFY_IMPACT}");
                println!("  {USAGE_SUMMARIZE}");
                return;
            }
            eprintln!("{USAGE_TOP}\n(got {:?})", other.unwrap_or(""));
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod option_guard {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn a_help_flag_is_not_mistaken_for_a_run() {
        // Before the guard this invocation started the default eval instead of printing help.
        assert!(wants_help(&v(&["--help"])));
        assert!(wants_help(&v(&["--tasks", "x.json", "-h"])));
        assert!(!wants_help(&v(&["--only", "task-a"])));
    }

    #[test]
    fn unknown_options_are_refused_with_the_subcommand_usage() {
        let err = check_flags(&v(&["--help-x"]), RUN_AGENTS_FLAGS, USAGE_RUN_AGENTS).unwrap_err();
        assert!(err.starts_with("unknown option --help-x"), "{err}");
        assert!(err.contains("usage: cort-evals run-agents"), "{err}");
    }

    #[test]
    fn every_recognised_option_is_listed() {
        // A new `at(argv, "--flag")` that is not whitelisted would be silently unusable.
        for flag in RUN_AGENTS_FLAGS {
            assert!(check_flags(&v(&[flag, "x"]), RUN_AGENTS_FLAGS, USAGE_RUN_AGENTS).is_ok());
        }
        assert!(check_flags(
            &v(&["--strict", "rows.json"]),
            SUMMARIZE_FLAGS,
            USAGE_SUMMARIZE
        )
        .is_ok());
        assert!(check_flags(&v(&[]), VERIFY_IMPACT_FLAGS, USAGE_VERIFY_IMPACT).is_ok());
    }

    #[test]
    fn equals_form_is_refused_rather_than_dropped() {
        // "--out=dir" would otherwise be ignored while --out silently kept its default.
        let err = check_flags(&v(&["--out=dir"]), RUN_AGENTS_FLAGS, USAGE_RUN_AGENTS).unwrap_err();
        assert!(err.contains("separate value"), "{err}");
    }

    #[test]
    fn bare_flags_are_matched_exactly() {
        assert!(check_flags(&v(&["--jail"]), RUN_AGENTS_FLAGS, USAGE_RUN_AGENTS).is_ok());
        // "--jail" is the switch; "--jailed" is nobody's option.
        assert!(check_flags(&v(&["--jailed"]), RUN_AGENTS_FLAGS, USAGE_RUN_AGENTS).is_err());
    }

    #[test]
    fn positionals_stay_positionals() {
        assert!(check_flags(&v(&["a/b/rows.json"]), SUMMARIZE_FLAGS, USAGE_SUMMARIZE).is_ok());
    }
}

#[cfg(test)]
mod sampling_window {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn only_selects_several_tasks_at_once() {
        // Three refused tasks are one invocation, not three shell invocations glued together.
        let ids = split_only("a, b ,,c");
        assert_eq!(ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!(only_matches(&ids, "b"));
        assert!(!only_matches(&ids, "d"));
        assert!(only_matches(&split_only(""), "anything"), "empty means all");
        assert_eq!(split_only("a").len(), 1);
    }

    #[test]
    fn a_refused_typo_selects_nothing_rather_than_everything() {
        let ids = split_only("transitive-chain-lastntradingdys");
        assert!(!only_matches(&ids, "transitive-chain-lastntradingdays"));
    }

    #[test]
    fn delay_defaults_to_no_wait() {
        assert_eq!(delay_secs(&v(&[])).unwrap(), 0);
        assert_eq!(delay_secs(&v(&["--delay-secs", "0"])).unwrap(), 0);
        assert_eq!(delay_secs(&v(&["--delay-secs", "3600"])).unwrap(), 3600);
    }

    #[test]
    fn delay_refuses_nonsense_instead_of_running_immediately() {
        // A silent fallback to "0" would fire the cells straight into the closed window.
        for bad in ["-5", "abc", "10.5", ""] {
            assert!(
                delay_secs(&v(&["--delay-secs", bad])).is_err(),
                "accepted {bad:?}"
            );
        }
    }
}

#[cfg(test)]
mod batch_accounting {
    use super::*;

    fn row(arm: &str, task: &str) -> Value {
        json!({ "arm": arm, "task": task })
    }

    #[test]
    fn a_partial_batch_says_so_instead_of_looking_complete() {
        let full = run_status_json(6, 6, "exited");
        assert_eq!(full["complete"], json!(true));
        assert_eq!(full["written_cells"], json!(6));

        let lost = run_status_json(6, 4, "exited");
        assert_eq!(lost["complete"], json!(false));
        assert!(
            lost["reading"]
                .as_str()
                .unwrap()
                .contains("2 of 6 planned cells"),
            "{}",
            lost["reading"]
        );
        // Nothing ever ran: that must not read as "a batch of zero cells" either.
        assert_eq!(run_status_json(6, 0, "running")["complete"], json!(false));
        assert_eq!(run_status_json(0, 0, "running")["complete"], json!(true));
    }

    #[test]
    fn rows_sort_the_same_way_whichever_order_threads_finished_in() {
        let mut a = vec![
            row("rg+Read", "zeta"),
            row("cort", "alpha"),
            row("cort", "beta"),
        ];
        let mut b = vec![
            row("cort", "beta"),
            row("rg+Read", "zeta"),
            row("cort", "alpha"),
        ];
        sort_rows(&mut a);
        sort_rows(&mut b);
        assert_eq!(a, b);
        let order: Vec<String> = a
            .iter()
            .map(|r| {
                format!(
                    "{}/{}",
                    r["arm"].as_str().unwrap(),
                    r["task"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                "cort/alpha".to_string(),
                "cort/beta".to_string(),
                "rg+Read/zeta".to_string()
            ]
        );
    }
}

#[cfg(test)]
mod whitelist_coverage {
    use super::*;

    /// The earlier test only walked the whitelist, which proves nothing about coverage: a new
    /// `at(argv, "--x")` that nobody registered would still leave it green. Read the flags out of
    /// this file's own source instead, so the whitelist cannot fall behind the parser.
    #[test]
    fn every_option_the_parser_asks_for_is_whitelisted() {
        let src = include_str!("main.rs");
        let keys: Vec<String> = vec![format!("at(argv, {}", '"'), format!("has(argv, {}", '"')];
        let mut orphans = Vec::new();
        for (n, line) in src.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for key in &keys {
                if let Some(at) = code.find(key.as_str()) {
                    let rest = &code[at + key.len()..];
                    let name: String = rest.chars().take_while(|c| *c != '"').collect();
                    if !name.starts_with("--") {
                        continue;
                    }
                    let known = RUN_AGENTS_FLAGS
                        .iter()
                        .chain(VERIFY_IMPACT_FLAGS.iter())
                        .chain(SUMMARIZE_FLAGS.iter())
                        .any(|f| *f == name.as_str());
                    if !known {
                        orphans.push(format!("{name} (src/main.rs:{})", n + 1));
                    }
                }
            }
        }
        assert!(
            orphans.is_empty(),
            "options read by the parser but missing from every whitelist: {orphans:?}"
        );
    }

    #[test]
    fn the_scan_itself_finds_the_real_options() {
        // Guards against the test passing because it matched nothing at all.
        let src = include_str!("main.rs");
        let needle = format!("at(argv, {}--tasks{}", '"', '"');
        assert!(src.contains(&needle), "scanner found no options at all");
    }
}

#[cfg(test)]
mod batch_consumption {
    use super::*;

    /// `complete` is derived by run_status_json, so callers here only pick the counts.
    fn status(planned: usize, written: usize, state: &str) -> Option<Value> {
        Some(run_status_json(planned, written, state))
    }

    fn batch(rows_count: usize, st: Option<Value>) -> BatchRead {
        BatchRead {
            rows_path: "x/rows.json".to_string(),
            rows_count,
            status: st,
        }
    }

    #[test]
    fn a_whole_batch_raises_no_flag_and_reports_its_counts() {
        let b = batch(6, status(6, 6, "exited"));
        assert_eq!(b.problem(), None);
        let rep = b.report();
        assert_eq!(rep["cells"], json!(6));
        assert_eq!(rep["planned_cells"], json!(6));
        assert_eq!(rep["complete"], json!(true));

        // F-18's first cut wrote no `state`: say so instead of printing null.
        let legacy = batch(
            6,
            Some(json!({"planned_cells": 6, "written_cells": 6, "complete": true})),
        );
        assert_eq!(legacy.report()["state"], json!("unrecorded"));
        assert_eq!(legacy.problem(), None);
    }

    #[test]
    fn a_short_batch_is_named_with_exactly_how_many_cells_are_missing() {
        // This is the F-15 failure mode seen from the other end: 4 of 6 ran, and the aggregate
        // would otherwise read as "an experiment that planned 4".
        let b = batch(4, status(6, 4, "exited"));
        let p = b.problem().expect("a lost cell must be flagged");
        assert!(p.contains("2 of 6 planned cells"), "{p}");
        assert!(p.contains("(4 measured)"), "{p}");
    }

    #[test]
    fn a_killed_batch_is_caught_by_the_count_disagreement() {
        // SIGKILL never reaches Drop, so the sidecar still says "running / 0 written" while
        // rows.json already holds 4 cells. planned == written would look fine; the counts do not.
        let b = batch(4, status(6, 0, "running"));
        let p = b.problem().expect("interrupted batch must be flagged");
        assert!(p.contains("claims 0"), "{p}");
        assert!(p.contains("interrupted"), "{p}");
    }

    #[test]
    fn a_sidecar_with_no_rows_at_all_is_still_a_lost_batch() {
        let b = batch(0, status(6, 0, "running"));
        assert!(b.problem().unwrap().contains("6 of 6 planned cells"));
    }

    #[test]
    fn pre_f18_artefacts_are_not_called_suspicious() {
        // Every committed round-1/round-2 directory predates the counter. Distrusting them would
        // make historical reproductions look broken and train people to drop --strict.
        let b = batch(10, None);
        assert_eq!(b.problem(), None);
        assert!(b.report()["run_status"]
            .as_str()
            .unwrap()
            .contains("predates F-18"));
    }

    #[test]
    fn an_incomplete_flag_without_a_count_gap_is_still_a_problem() {
        let st =
            json!({"planned_cells": 6, "written_cells": 6, "complete": false, "state": "exited"});
        let b = batch(6, Some(st));
        let p = b.problem().expect("complete:false must be honoured");
        assert!(p.contains("did not complete"), "{p}");
    }

    #[test]
    fn load_reads_the_sidecar_beside_rows_json() {
        let dir = std::env::temp_dir().join(format!("cort-evals-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rows = dir.join("rows.json");
        std::fs::write(&rows, "[]\n").unwrap();
        std::fs::write(
            dir.join("run-status.json"),
            format!("{}\n", run_status_json(2, 2, "exited")),
        )
        .unwrap();
        let b = BatchRead::load(rows.to_str().unwrap(), 0);
        assert_eq!(b.field("planned_cells"), Some(2));
        assert_eq!(b.field("written_cells"), Some(2));
        // A rows.json of its own accord with 0 cells against a claimed 2 is a discrepancy.
        assert!(b.problem().unwrap().contains("interrupted"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
