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
    let tasks_path = at(argv, "--tasks", "evals/tasks-graph.json");
    let doc = load_tasks(&tasks_path)?;
    let only = at(argv, "--only", "");
    let tasks: Vec<Task> = doc
        .into_iter()
        .filter(|t| only.is_empty() || t.id == only)
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

    let head = venue_head(&tasks[0].venue)?;
    let work: Vec<(Task, String)> = tasks
        .iter()
        .cloned()
        .flat_map(|t| arms.iter().map(move |a| (t.clone(), a.clone())))
        .collect();

    let queue = Arc::new(Mutex::new(work.into_iter()));
    let mut threads = Vec::new();
    for _ in 0..concurrency {
        let queue = Arc::clone(&queue);
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
                rows.push(row);
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
    rows.sort_by_key(|r| {
        format!(
            "{} {}",
            r["arm"].as_str().unwrap_or(""),
            r["task"].as_str().unwrap_or("")
        )
    });
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
    for path in &paths {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let parsed: Vec<Value> = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
        rows.extend(parsed);
    }
    let out = cort_evals::summary::summarize(&rows, has(argv, "--strict"))?;
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
            eprintln!(
                "usage: cort-evals <run-agents|verify-impact|summarize> [options]\n(got {:?})",
                other.unwrap_or("")
            );
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
