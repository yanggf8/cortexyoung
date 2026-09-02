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
/// Recognising our own entry cannot be anchored to the end of the command line. A hook command is
/// shell and people write shell: `.../cort hook-suggest 2>/dev/null || true` is ours, and ending in
/// `|| true` made it read as somebody else's. That is not cosmetic -- it is how a machine ends up
/// with `--status` answering `wired: false` while the hook fires hundreds of times, and with every
/// redeploy appending one more copy instead of updating the one that is there. Match the subcommand
/// as a token, and keep it ours only when the token in front of it is the binary.
///
/// It equally cannot be a suffix test in the other direction. `hook-suggest` is only ever a
/// *subcommand* here: `cort hook-install` writes `<exe> hook-suggest`, two tokens, always. A path
/// that merely ends in `/hook-suggest` is somebody else's binary that happens to share the name,
/// and claiming it is not a mislabelling -- install rewrites the whole entry and remove deletes it,
/// so a vendor's `/opt/vendor/bin/hook-suggest --daemon` is silently destroyed by an unrelated
/// `./install.sh`. This module's first promise is that every hook the user already has survives,
/// and that promise is only ever as good as this predicate.
fn is_ours(command: &str) -> bool {
    let tokens: Vec<&str> = command
        .split_whitespace()
        .map(|t| t.trim_matches(|c| c == '"' || c == '\''))
        .collect();
    tokens.iter().enumerate().any(|(i, t)| {
        if *t != "hook-suggest" {
            return false;
        }
        match i.checked_sub(1).map(|prev| tokens[prev]) {
            // A bare `hook-suggest` resolved through PATH -- the one shape with no binary token in
            // front of it. Never a path: `/opt/vendor/bin/hook-suggest` is not this token at all.
            None => true,
            Some(prev) => prev == "cort" || prev.ends_with("/cort"),
        }
    })
}

/// The one shape an entry of ours is allowed to have. Factored out because `install_hook` has to
/// be able to normalise an existing entry to it, not only to create one.
fn hook_command_entry(command: &str) -> Value {
    json!({
        "type": "command",
        "command": command,
        "timeout": TIMEOUT_SECS,
    })
}

fn hook_entry(command: &str) -> Value {
    json!({
        "matcher": MATCHER,
        "hooks": [hook_command_entry(command)],
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

/// The `PreToolUse` list, created when absent -- and refused, never replaced, when it is there with
/// the wrong shape.
///
/// Replacing it was the same unrecoverable act `SettingsError::Unparsable` exists to forbid, one
/// level in: a `hooks` that is a string is a file we do not understand, and overwriting it discards
/// data the user has no reason to go looking for. Claude Code's schema does not produce that shape,
/// which is precisely why nobody would notice.
fn pre_tool_use(root: &mut Map<String, Value>) -> Result<&mut Vec<Value>, SettingsError> {
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if !hooks.is_object() {
        return Err(SettingsError::Unparsable(
            "`hooks` is present but is not an object; refusing to overwrite it".into(),
        ));
    }
    let obj = hooks.as_object_mut().expect("hooks is an object");
    let entry = obj
        .entry("PreToolUse")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        return Err(SettingsError::Unparsable(
            "`hooks.PreToolUse` is present but is not an array; refusing to overwrite it".into(),
        ));
    }
    Ok(entry.as_array_mut().expect("PreToolUse is an array"))
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
    let list = pre_tool_use(&mut root)?;

    let canonical = hook_command_entry(command);
    let mut seen_ours = false;
    let mut found_same = false;
    let mut changed = false;
    let mut we_emptied: Vec<usize> = Vec::new();
    for (idx, group) in list.iter_mut().enumerate() {
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = hooks.len();
        hooks.retain_mut(|h| {
            let Some(cur) = h.get("command").and_then(Value::as_str) else {
                return true;
            };
            if !is_ours(cur) {
                return true;
            }
            // A file can already hold more than one of ours -- a hand-wired one did, which is what
            // firing the hook once per copy looks like. Keep the first, drop the rest, so that a
            // redeploy converges on one entry instead of adding to the pile.
            if seen_ours {
                changed = true;
                return false;
            }
            seen_ours = true;
            if *h == canonical {
                found_same = true;
            } else {
                // Normalise the whole entry, not just its command string. An entry of ours that
                // carries a hand-typed `if` condition covers less ground than the installer then
                // reports as wired, and a deployed state nobody can predict from the installer is
                // exactly what this module exists to rule out. Only entries `is_ours` are touched.
                *h = canonical.clone();
                changed = true;
            }
            true
        });
        if hooks.is_empty() && before > 0 {
            we_emptied.push(idx);
        }
    }
    // Drop only the groups this call emptied, addressed by index. A blanket "remove every empty
    // group" reached the user's own empty groups as well -- an empty `{"matcher":"Read"}` they keep
    // for their own reasons vanished on an unrelated install, which is what the comment here used
    // to claim could not happen.
    for idx in we_emptied.iter().rev() {
        list.remove(*idx);
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
    let list = pre_tool_use(&mut root)?;

    let mut removed = false;
    let mut we_emptied: Vec<usize> = Vec::new();
    for (idx, group) in list.iter_mut().enumerate() {
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
            if hooks.is_empty() {
                we_emptied.push(idx);
            }
        }
    }
    // A group *this call* emptied was ours and goes. The sweep used to be unconditional, which
    // took the user's own empty groups with it -- and, once `PreToolUse` was empty as a result,
    // the whole `hooks` key. Uninstalling our hook is not licence to tidy someone else's file.
    for idx in we_emptied.iter().rev() {
        list.remove(*idx);
    }

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
        // `continue`, not `?`: a group without a hooks array is somebody else's malformed entry,
        // and letting it end the scan would report `wired: false` for an entry sitting right
        // after it.
        let Some(hooks) = group.get("hooks").and_then(Value::as_array) else {
            continue;
        };
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
