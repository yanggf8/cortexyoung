//! The arms themselves: what each one may touch, how its command line is built, and whether it
//! actually stayed inside its permitted set.
//!
//! The whitelist *is* the experiment, so this module is the experiment's control. `--allowedTools`
//! turned out not to bind Bash in headless mode (the first live cell ran `grep -rn` and `sed -n`
//! ten times with an empty `permission_denials`), which is why containment is enforced through the
//! arm's `PATH` instead, and every row carries `arm_held` so a leaked arm cannot be mistaken for a
//! comparison.

use crate::grade::{Task, ANSWER_CONTRACT};
use crate::stream::{estimate_tokens, Parsed};
use serde_json::{json, Value};
use std::path::Path;

/// cort is the Rust binary since the cutover; `bin/cort.js` is gone.
pub fn cort_bin() -> String {
    if let Ok(over) = std::env::var("CORT_BIN") {
        if !over.is_empty() {
            return over;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("rust").join("target").join("release").join("cort"))
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| "cort".to_string())
}

/// Shipped guidance, not a hint invented for the eval: this is what `skills/ast-grep/SKILL.md`
/// tells any agent that has cort. Withholding it would measure a tool nobody was told how to use —
/// but results must record that the cort arm received it.
pub fn cort_guidance() -> String {
    format!(
        "You have an offline code-intelligence CLI. Run it exactly like this, copying the path verbatim:\n\n\
{}\n\n\
It answers relationship questions — who reaches a symbol, and in how many hops — from a\n\
pre-built index, in one call per query. `-f lean` keeps the answer small. Its lean output\n\
reports stale=; if that is ever true, say so in your reply.",
        cort_bin()
    )
}

pub fn arm_binaries(arm: &str) -> Vec<String> {
    match arm {
        // Only `rg`. A jailed shell cannot fall back to grep/sed, which is what makes the
        // baseline a baseline rather than "whatever the agent reached for".
        "rg+Read" => vec!["rg".to_string()],
        // cort is allowed its own dependencies: it shells out to `ast-grep` (the only parser) and
        // `git` (staleness). A jail without either measures a broken tool — which is exactly what
        // happened on the first jailed live cell: cort answered `ast_grep_missing` for 20 turns.
        "cort" => vec![cort_bin(), "git".to_string(), "ast-grep".to_string()],
        _ => Vec::new(),
    }
}

pub fn allowed_tools(arm: &str) -> Vec<String> {
    match arm {
        "rg+Read" => vec!["Read".into(), "Bash(rg:*)".into()],
        "cort" => vec!["Read".into(), format!("Bash({}:*)", cort_bin())],
        other => panic!("unknown arm {other}"),
    }
}

pub fn guidance(arm: &str) -> Option<String> {
    match arm {
        "cort" => Some(cort_guidance()),
        "rg+Read" => None,
        other => panic!("unknown arm {other}"),
    }
}

pub const AGENT_ARMS: [&str; 2] = ["rg+Read", "cort"];

pub const REQUIRED_FIELDS: [&str; 18] = [
    "arm",
    "task",
    "success",
    "coverage",
    "precision",
    "answered_symbols",
    "total_tokens",
    "tool_return_tokens",
    "tool_return_bytes",
    "read_calls",
    "turns",
    "hit_turn_cap",
    "permission_denials",
    "estimator",
    "venue_head",
    "cort_calls",
    "arm_held",
    "shells_used",
];

/// Resolve a bare binary name against PATH without spawning anything: a lookup that needed `which`
/// fails under a sandboxed runner, and the jail has to be buildable before any cell costs money.
pub fn resolve_binary(bin: &str) -> Option<String> {
    if bin.contains('/') {
        return if Path::new(bin).exists() {
            Some(bin.to_string())
        } else {
            None
        };
    }
    for dir in std::env::split_paths(std::env::var("PATH").as_deref().unwrap_or("")) {
        let cand = dir.join(bin);
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

/// A directory of symlinks to exactly the arm's permitted binaries; returns the PATH to run with.
pub fn make_jail(dir: &str, binaries: &[String]) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
    for bin in binaries {
        let abs =
            resolve_binary(bin).ok_or_else(|| format!("permitted binary not found: {bin}"))?;
        let link = Path::new(dir).join(Path::new(&abs).file_name().unwrap());
        if !link.exists() {
            std::os::unix::fs::symlink(&abs, &link)
                .map_err(|e| format!("{}: {e}", link.display()))?;
        }
    }
    Ok(dir.to_string())
}

fn first_token(command: &str) -> String {
    command.split_whitespace().next().unwrap_or("").to_string()
}

/// What the arm actually executed: native tool names, plus the first token of every Bash command.
pub fn shell_binaries(calls: &[crate::stream::ToolCall]) -> Vec<String> {
    calls
        .iter()
        .map(|c| {
            if c.name == "Bash" {
                first_token(&c.command)
            } else {
                c.name.clone()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// True when the arm stayed inside its permitted set. A false value means the cell measured
/// something other than the arm it is named after.
pub fn arm_held(arm: &str, calls: &[crate::stream::ToolCall]) -> bool {
    let permitted = arm_binaries(arm);
    shell_binaries(calls).iter().all(|used| {
        if used == "Read" {
            return true;
        }
        permitted.iter().any(|allowed| {
            // A bare name can only come from the jail; an absolute path has to be the very
            // binary the arm was configured with, so a copy or a symlink elsewhere is not "held".
            if used.contains('/') {
                used == allowed
            } else {
                Path::new(allowed)
                    .file_name()
                    .map(|f| f == used.as_str())
                    .unwrap_or(false)
            }
        })
    })
}

/// Did this command invoke cort? Matching the deleted `cort.js` filename reported 0 calls for an
/// arm that called the tool on every turn — the metric that proves the whitelist was exercised.
pub fn is_cort_command(command: &str) -> bool {
    let raw = command.trim();
    if raw.is_empty() {
        return false;
    }
    let first = first_token(raw);
    let bin = cort_bin();
    first == bin
        || Path::new(&first)
            .file_name()
            .map(|f| f == "cort")
            .unwrap_or(false)
        || raw.contains(&bin)
}

pub fn build_prompt(task: &Task, arm: &str) -> String {
    [
        Some(task.prompt.clone()),
        guidance(arm),
        Some(ANSWER_CONTRACT.to_string()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
}

pub fn build_args(task: &Task, arm: &str, max_turns: i64) -> Invocation {
    Invocation {
        program: "claude".to_string(),
        // cwd is load-bearing: cort derives projectId from it, and from the wrong directory the
        // same query returns seeds=0 and stale=true — measuring a missing symbol, not the tool.
        args: vec![
            "-p".into(),
            build_prompt(task, arm),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--strict-mcp-config".into(),
            "--max-turns".into(),
            max_turns.to_string(),
            "--allowedTools".into(),
            allowed_tools(arm).join(","),
        ],
        cwd: task.venue.clone(),
    }
}

pub fn build_env(
    config_dir: &str,
    cache_dir: &str,
    jail_dir: Option<&str>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::env::vars().collect();
    set(&mut out, "CLAUDE_CONFIG_DIR", config_dir.to_string());
    // CORT_CACHE_DIR has to come from the parent process: a prefix-matched allowance means an
    // agent that wrote the variable itself gets denied and quietly falls back to Read.
    set(&mut out, "CORT_CACHE_DIR", cache_dir.to_string());
    // Claude Code normalises the Bash tool's PATH down to something like
    // /usr/local/bin:/usr/bin:/bin:~/.local/bin, which drops wherever ast-grep actually lives on
    // this host (here, an nvm bin dir). cort then answers `{"error":"ast_grep_missing"}` and the arm
    // measures a broken tool instead of the product. Pinning the parent-resolved path keeps the
    // cort under test the same cort a user runs. Product-side fix for this is audit F-13.
    if std::env::var_os("CORT_AST_GREP_BIN").is_none() {
        if let Some(ag) = resolve_binary("ast-grep") {
            set(&mut out, "CORT_AST_GREP_BIN", ag);
        }
    }
    if let Some(jail) = jail_dir {
        set(&mut out, "PATH", jail.to_string());
    }
    out
}

fn set(env: &mut Vec<(String, String)>, key: &str, value: String) {
    match env.iter_mut().find(|(k, _)| k == key) {
        Some(existing) => existing.1 = value,
        None => env.push((key.to_string(), value)),
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// One cell's row. Refuses to build a row from an unmeasured metric rather than writing null.
pub fn build_row(
    arm: &str,
    task: &Task,
    parsed: &Parsed,
    venue_head: &str,
    jailed: Option<bool>,
) -> Result<Value, String> {
    let graded = crate::grade::grade_answer(&parsed.answer_text, task);
    let calls = &parsed.tool_calls;
    let cort_calls = calls.iter().filter(|c| is_cort_command(&c.command)).count() as i64;
    let rg_calls = calls
        .iter()
        .filter(|c| c.name == "Bash" && first_token(&c.command) == "rg")
        .count() as i64;

    let row = json!({
        "arm": arm,
        "task": task.id,
        "success": graded.success,
        "coverage": graded.coverage,
        "precision": graded.precision,
        "hop_accuracy": graded.hop_accuracy,
        "answer_block": graded.answer_block,
        "answered_symbols": graded.answered_symbols,
        "covered_symbols": graded.covered_symbols,
        "spurious_symbols": graded.spurious_symbols,
        "wrong_hop": graded.wrong_hop.iter().map(|w| json!({
            "symbol": w.symbol, "said": w.said, "actual": w.actual
        })).collect::<Vec<_>>(),
        "expected_symbols": task.expected_symbols,
        "total_tokens": parsed.total_tokens,
        "input_tokens": parsed.input_tokens,
        "cache_creation": parsed.cache_creation,
        "cache_read": parsed.cache_read,
        "output_tokens": parsed.output_tokens,
        "tool_return_tokens": parsed.tool_return_tokens,
        "tool_return_bytes": parsed.tool_return_bytes,
        "read_calls": parsed.read_calls,
        "cort_calls": cort_calls,
        "rg_calls": rg_calls,
        "shells_used": shell_binaries(calls),
        "arm_held": arm_held(arm, calls),
        "jailed": jailed,
        "turns": parsed.turns,
        "hit_turn_cap": parsed.hit_turn_cap,
        "permission_denials": parsed.permission_denials.len(),
        "guidance_given": guidance(arm).is_some(),
        "cost_usd": parsed.cost_usd,
        "session_id": parsed.session_id,
        "estimator": crate::ESTIMATOR,
        "venue_head": venue_head,
    });

    for key in [
        "turns",
        "tool_return_tokens",
        "tool_return_bytes",
        "read_calls",
        "total_tokens",
    ] {
        if num(row.get(key).unwrap_or(&Value::Null)).is_none() {
            return Err(format!(
                "{}/{}: {key} is not a number; refusing to write a null metric",
                arm, task.id
            ));
        }
    }
    for key in REQUIRED_FIELDS {
        if row.get(key).is_none() {
            return Err(format!("{}/{}: missing required field {key}", arm, task.id));
        }
    }
    Ok(row)
}

/// Re-exported so callers that only have raw text can price it the same way both arms are priced.
pub fn estimate(text: &str) -> usize {
    estimate_tokens(text)
}
