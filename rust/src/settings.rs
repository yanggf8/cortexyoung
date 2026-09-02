//! Wiring `cort hook-suggest` into a Claude Code `settings.json`, and taking it back out.
//!
//! This lives here rather than in `install.sh` for the reason the repo gives everywhere else: the
//! merge is logic, not plumbing. It has to preserve every hook the user already configured, be
//! idempotent across reinstalls, recognise its own entry after the binary moves, and refuse to
//! touch a file it cannot parse. A `jq` pipeline in bash would be a second implementation of that
//! with no tests attached, and `jq` is not a dependency the installer otherwise has.
//!
//! One entry is written, matched to `Bash`, because `hook-suggest` reads `tool_input.command` and
//! has nothing to say about any other tool. An unmatched entry would fire on every tool call in
//! the session to return `{}`.

use serde_json::{json, Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What a call actually did. The installer prints this, so "nothing to do" and "changed something"
/// are different words rather than the same silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// No entry was there; one was added.
    Installed,
    /// Our entry was already there with the same command — the file was not written.
    AlreadyPresent,
    /// Our entry was there with a different command (the binary moved) — rewritten in place.
    Updated,
    /// Our entry was there and was taken out.
    Removed,
    /// Nothing of ours to remove — the file was not written.
    NotPresent,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Installed => "installed",
            Change::AlreadyPresent => "already_present",
            Change::Updated => "updated",
            Change::Removed => "removed",
            Change::NotPresent => "not_present",
        }
    }

    /// Did this call write the file? Drives whether the installer reports a change.
    pub fn wrote(self) -> bool {
        matches!(self, Change::Installed | Change::Updated | Change::Removed)
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    /// The file exists but is not JSON, or its root is not an object. Both are refusals, not
    /// repairs: overwriting a settings file we failed to understand is the one unrecoverable
    /// thing this module could do.
    Unparsable(String),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Io(e) => write!(f, "{e}"),
            SettingsError::Unparsable(m) => write!(f, "{m}"),
        }
    }
}

impl From<io::Error> for SettingsError {
    fn from(e: io::Error) -> Self {
        SettingsError::Io(e)
    }
}

/// The tool whose payload `hook-suggest` can read.
const MATCHER: &str = "Bash";

/// Seconds. The rule is a string scan over one command line; the budget is here so a pathological
/// case can never hold up the agent's tool call.
const TIMEOUT_SECS: u64 = 5;

/// The default settings file, honouring the same override the installer uses for the skill
/// destination so a test (or a second agent home) never has to touch the real one.
pub fn default_settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("CLAUDE_SKILL_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))?;
    Some(home.join("settings.json"))
}

/// Is this configured hook ours? Matched on the verb rather than the whole string, so an install
/// that moved the binary (`~/.cargo/bin` to `~/.local/bin`, say) updates one entry instead of
/// leaving two that both fire.
fn is_ours(command: &str) -> bool {
    let c = command.trim();
    c == "hook-suggest" || c.ends_with(" hook-suggest") || c.ends_with("/hook-suggest")
}

fn hook_entry(command: &str) -> Value {
    json!({
        "matcher": MATCHER,
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": TIMEOUT_SECS,
        }],
    })
}

fn read_root(path: &Path) -> Result<Map<String, Value>, SettingsError> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e.into()),
    };
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_str(&raw).map_err(|e| {
        SettingsError::Unparsable(format!("{} is not valid JSON: {e}", path.display()))
    })?;
    match v {
        Value::Object(m) => Ok(m),
        _ => Err(SettingsError::Unparsable(format!(
            "{} does not hold a JSON object at its root",
            path.display()
        ))),
    }
}

/// Write next to the target and rename, so an interrupted install cannot leave a half-written
/// settings file — that file is read by the harness on every session start.
fn write_root(path: &Path, root: &Map<String, Value>) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.cort-tmp");
    let mut body = serde_json::to_string_pretty(&Value::Object(root.clone()))
        .map_err(|e| SettingsError::Unparsable(e.to_string()))?;
    body.push('\n');
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// One copy of the file as it was before the first change we make to it. Named with a timestamp
/// for the same reason the skill deploy log is: "what did the installer do, and when".
fn backup(path: &Path) -> Result<Option<PathBuf>, SettingsError> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = path.with_extension(format!("json.bak.{stamp}"));
    fs::copy(path, &dest)?;
    Ok(Some(dest))
}

fn pre_tool_use(root: &mut Map<String, Value>) -> &mut Vec<Value> {
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if !hooks.is_object() {
        *hooks = Value::Object(Map::new());
    }
    let obj = hooks.as_object_mut().expect("hooks is an object");
    let entry = obj
        .entry("PreToolUse")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    entry.as_array_mut().expect("PreToolUse is an array")
}

/// The outcome of a call, with the backup path when one was taken.
#[derive(Debug)]
pub struct Outcome {
    pub change: Change,
    pub backup: Option<PathBuf>,
}

/// Add (or refresh) the `PreToolUse` entry that runs `command`.
pub fn install_hook(path: &Path, command: &str) -> Result<Outcome, SettingsError> {
    let mut root = read_root(path)?;
    let list = pre_tool_use(&mut root);

    let mut found_same = false;
    let mut changed = false;
    for group in list.iter_mut() {
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        for h in hooks.iter_mut() {
            let Some(cur) = h.get("command").and_then(Value::as_str) else {
                continue;
            };
            if !is_ours(cur) {
                continue;
            }
            if cur == command {
                found_same = true;
            } else {
                h["command"] = json!(command);
                changed = true;
            }
        }
    }

    if found_same && !changed {
        return Ok(Outcome {
            change: Change::AlreadyPresent,
            backup: None,
        });
    }

    let change = if changed {
        Change::Updated
    } else {
        list.push(hook_entry(command));
        Change::Installed
    };
    let bak = backup(path)?;
    write_root(path, &root)?;
    Ok(Outcome {
        change,
        backup: bak,
    })
}

/// Take our entry back out, leaving every other hook — and any group we shared — intact.
pub fn remove_hook(path: &Path) -> Result<Outcome, SettingsError> {
    if !path.exists() {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    let mut root = read_root(path)?;
    let list = pre_tool_use(&mut root);

    let mut removed = false;
    for group in list.iter_mut() {
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = hooks.len();
        hooks.retain(|h| {
            !h.get("command")
                .and_then(Value::as_str)
                .is_some_and(is_ours)
        });
        if hooks.len() != before {
            removed = true;
        }
    }
    // A group we emptied is ours and goes; a group that still has hooks belongs to someone else.
    list.retain(|g| {
        g.get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|h| !h.is_empty())
    });

    if !removed {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    // Leave no empty scaffolding behind: an install that added `hooks.PreToolUse` to a file that
    // had neither should not leave them once it is uninstalled.
    let drop_pre = root
        .get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty);
    if drop_pre {
        if let Some(h) = root.get_mut("hooks").and_then(Value::as_object_mut) {
            h.remove("PreToolUse");
            if h.is_empty() {
                root.remove("hooks");
            }
        }
    }

    let bak = backup(path)?;
    write_root(path, &root)?;
    Ok(Outcome {
        change: Change::Removed,
        backup: bak,
    })
}

/// Is our entry currently configured, and with which command? Used by `install.sh --check`, which
/// must be able to say "the hook is wired" without mutating anything.
pub fn installed_command(path: &Path) -> Option<String> {
    let root = read_root(path).ok()?;
    let list = root.get("hooks")?.get("PreToolUse")?.as_array()?;
    for group in list {
        let hooks = group.get("hooks").and_then(Value::as_array)?;
        for h in hooks {
            if let Some(c) = h.get("command").and_then(Value::as_str) {
                if is_ours(c) {
                    return Some(c.to_string());
                }
            }
        }
    }
    None
}
