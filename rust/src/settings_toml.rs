//! Wiring `cort hook-suggest` into Codex's `~/.codex/config.toml`, and taking it back out.
//!
//! Codex loads a `PreToolUse` hook only from `[[hooks.PreToolUse]]` in `config.toml` --
//! `~/.codex/hooks/hooks.json` and `~/.codex/hooks.json` are both silently ignored, and Codex's
//! payload is byte-for-byte Claude Code's shape (established 2026-09-02,
//! `docs/2026-09-02-hook-wiring-correction.md` §12). Until this module, the working Codex hook was
//! deliberately left unwired: hand-added outside `install.sh`, absent from the manifest, invisible
//! to `--check` -- the same "a route that has to be wired by hand is a route that is not wired"
//! failure the rest of that document is about, one harness later.
//!
//! The group shape below -- a `matcher` per group, and a nested `hooks` array of
//! `{type, command, timeout}` entries -- mirrors `settings.rs`'s JSON shape field for field. That is
//! not a guess: the shipped Codex binary's own string table names a `ConfiguredHookMatcherGroup`
//! struct with exactly two fields (`matcher`, `hooks`), and separately lists `type`/`command`/
//! `timeout` as sibling fields of a hook entry. `codex --strict-config doctor` accepts this shape
//! without complaint, but `doctor` does not deeply validate the hooks substructure (it accepted a
//! deliberately bogus field the same way), so that check is corroborating, not proof. A live
//! `codex exec` firing this hook end to end -- the same bisection method §12 used -- has not been
//! run against this module's output; do that before trusting this in the field on a machine where
//! Codex is the only harness in use.
//!
//! Everything past the shape mirrors `settings.rs`'s guarantees line for line, because the failure
//! mode is the same regardless of which parser is doing the reading: preserve every hook the user
//! already configured, be idempotent across reinstalls, recognise its own entry after the binary
//! moves, and refuse to touch a file it cannot parse. `is_ours` is imported rather than
//! reimplemented -- the token-matching rule it encodes cost two rounds of hardening
//! (`docs/2026-09-02-hook-wiring-correction.md` §7, §9), and a second copy of it here is a second
//! place for the same bug to reappear.

use crate::settings::{is_ours, Change, Outcome};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

/// The tool whose payload `hook-suggest` can read -- same rule as the JSON side.
const MATCHER: &str = "Bash";

/// Seconds, matching `settings::TIMEOUT_SECS`. Kept as a separate constant rather than a shared one:
/// the two modules do not share a value type (`i64` here, `u64` there), and coupling them on a
/// number that happens to agree today is a future drift bug waiting for whichever module changes
/// it first.
const TIMEOUT_SECS: i64 = 5;

#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    /// The file exists but is not TOML, or a `hooks` / `hooks.PreToolUse` it already holds is not
    /// the shape this module expects. Both are refusals, not repairs: overwriting a config file we
    /// failed to understand is the one unrecoverable thing this module could do.
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

/// The default config file, honouring the same override Codex itself reads for its home directory
/// (mirrored by `install.sh`'s `CODEX_SKILL_DEST` for the skill), so a test never has to touch the
/// real one.
pub fn default_settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))?;
    Some(home.join("config.toml"))
}

fn hook_command_table(command: &str) -> Table {
    let mut t = Table::new();
    t.insert("type", value("command"));
    t.insert("command", value(command));
    t.insert("timeout", value(TIMEOUT_SECS));
    t
}

/// Is this hook entry exactly the one `install_hook` would write for `command` right now? Used to
/// tell "nothing to do" apart from "ours, but stale" without string-comparing serialized TOML, which
/// would trip on formatting toml_edit itself introduces.
fn is_canonical(h: &Table, command: &str) -> bool {
    h.get("command").and_then(Item::as_str) == Some(command)
        && h.get("type").and_then(Item::as_str) == Some("command")
        && h.get("timeout").and_then(Item::as_integer) == Some(TIMEOUT_SECS)
}

fn read_doc(path: &Path) -> Result<DocumentMut, SettingsError> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DocumentMut::new()),
        Err(e) => return Err(e.into()),
    };
    raw.parse::<DocumentMut>()
        .map_err(|e| SettingsError::Unparsable(format!("{} is not valid TOML: {e}", path.display())))
}

/// Write next to the target and rename, so an interrupted install cannot leave a half-written
/// config file -- Codex reads this one on every session start.
fn write_doc(path: &Path, doc: &DocumentMut) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.cort-tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// One copy of the file as it was before the first change we make to it, same convention as the
/// JSON side's `.json.bak.<epoch-seconds>`.
fn backup(path: &Path) -> Result<Option<PathBuf>, SettingsError> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = path.with_extension(format!("toml.bak.{stamp}"));
    fs::copy(path, &dest)?;
    Ok(Some(dest))
}

/// The `hooks.PreToolUse` array of tables, created when absent -- and refused, never replaced, when
/// it is there with the wrong shape. Mirrors `settings::pre_tool_use`'s refusal rule: a `hooks` that
/// already holds something other than a table, or a `PreToolUse` that already holds something other
/// than an array of tables, is a file this module does not understand.
fn pre_tool_use(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, SettingsError> {
    let root = doc.as_table_mut();
    let hooks_item = root.entry("hooks").or_insert(Item::Table(Table::new()));
    if !hooks_item.is_table() {
        return Err(SettingsError::Unparsable(
            "`hooks` is present but is not a table; refusing to overwrite it".into(),
        ));
    }
    let hooks_table = hooks_item.as_table_mut().expect("hooks is a table");
    let pre = hooks_table
        .entry("PreToolUse")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    if !pre.is_array_of_tables() {
        return Err(SettingsError::Unparsable(
            "`hooks.PreToolUse` is present but is not an array of tables; refusing to overwrite it"
                .into(),
        ));
    }
    Ok(pre
        .as_array_of_tables_mut()
        .expect("PreToolUse is an array of tables"))
}

/// Add (or refresh) the `PreToolUse` group that runs `command`.
pub fn install_hook(path: &Path, command: &str) -> Result<Outcome, SettingsError> {
    let mut doc = read_doc(path)?;
    let list = pre_tool_use(&mut doc)?;

    let mut seen_ours = false;
    let mut found_same = false;
    let mut changed = false;
    let mut empty_groups: Vec<usize> = Vec::new();

    for gi in 0..list.len() {
        let group = list.get_mut(gi).expect("gi in range");
        let Some(hooks) = group.get_mut("hooks").and_then(Item::as_array_of_tables_mut) else {
            continue;
        };
        let before = hooks.len();
        let mut drop_idx: Vec<usize> = Vec::new();
        for hi in 0..hooks.len() {
            let h = hooks.get_mut(hi).expect("hi in range");
            let Some(cur) = h.get("command").and_then(Item::as_str).map(str::to_string) else {
                continue;
            };
            if !is_ours(&cur) {
                continue;
            }
            // A file can already hold more than one of ours. Keep the first, drop the rest, so a
            // redeploy converges on one entry instead of adding to the pile -- same rule as the
            // JSON side, same reason (§7 of the wiring doc).
            if seen_ours {
                changed = true;
                drop_idx.push(hi);
                continue;
            }
            seen_ours = true;
            if is_canonical(h, command) {
                found_same = true;
            } else {
                *h = hook_command_table(command);
                changed = true;
            }
        }
        for hi in drop_idx.into_iter().rev() {
            hooks.remove(hi);
        }
        if hooks.len() == 0 && before > 0 {
            empty_groups.push(gi);
        }
    }
    // Drop only the groups this call emptied, addressed by index -- not every empty group, which
    // would take the user's own with it (§9.3 of the wiring doc, same trap on the JSON side).
    for gi in empty_groups.into_iter().rev() {
        list.remove(gi);
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
        let mut group = Table::new();
        group.insert("matcher", value(MATCHER));
        let mut hooks = ArrayOfTables::new();
        hooks.push(hook_command_table(command));
        group.insert("hooks", Item::ArrayOfTables(hooks));
        list.push(group);
        Change::Installed
    };
    let bak = backup(path)?;
    write_doc(path, &doc)?;
    Ok(Outcome {
        change,
        backup: bak,
    })
}

/// Take our entry back out, leaving every other hook -- and any group we shared -- intact.
pub fn remove_hook(path: &Path) -> Result<Outcome, SettingsError> {
    if !path.exists() {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    let mut doc = read_doc(path)?;
    let list = pre_tool_use(&mut doc)?;

    let mut removed = false;
    let mut empty_groups: Vec<usize> = Vec::new();
    for gi in 0..list.len() {
        let group = list.get_mut(gi).expect("gi in range");
        let Some(hooks) = group.get_mut("hooks").and_then(Item::as_array_of_tables_mut) else {
            continue;
        };
        let before = hooks.len();
        let mut drop_idx: Vec<usize> = Vec::new();
        for hi in 0..hooks.len() {
            let h = hooks.get_mut(hi).expect("hi in range");
            if h.get("command")
                .and_then(Item::as_str)
                .is_some_and(is_ours)
            {
                drop_idx.push(hi);
            }
        }
        for hi in drop_idx.into_iter().rev() {
            hooks.remove(hi);
        }
        if hooks.len() != before {
            removed = true;
            if hooks.len() == 0 {
                empty_groups.push(gi);
            }
        }
    }
    for gi in empty_groups.into_iter().rev() {
        list.remove(gi);
    }

    if !removed {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    // Leave no empty scaffolding behind: an install that added `hooks.PreToolUse` to a file that
    // had neither should not leave them once it is uninstalled.
    let drop_pre = doc
        .as_table()
        .get("hooks")
        .and_then(Item::as_table)
        .and_then(|h| h.get("PreToolUse"))
        .and_then(Item::as_array_of_tables)
        .is_some_and(|a| a.len() == 0);
    if drop_pre {
        if let Some(h) = doc.as_table_mut().get_mut("hooks").and_then(Item::as_table_mut) {
            h.remove("PreToolUse");
            if h.is_empty() {
                doc.as_table_mut().remove("hooks");
            }
        }
    }

    let bak = backup(path)?;
    write_doc(path, &doc)?;
    Ok(Outcome {
        change: Change::Removed,
        backup: bak,
    })
}

/// Is our entry currently configured, and with which command? Used by `install.sh --check`, which
/// must be able to say "the hook is wired" without mutating anything.
pub fn installed_command(path: &Path) -> Option<String> {
    let doc = read_doc(path).ok()?;
    let list = doc
        .as_table()
        .get("hooks")?
        .as_table()?
        .get("PreToolUse")?
        .as_array_of_tables()?;
    for group in list.iter() {
        // `continue`, not `?`: a group without a hooks array is somebody else's malformed entry,
        // and letting it end the scan would report `wired: false` for an entry sitting right after
        // it -- the same bug §7 of the wiring doc found on the JSON side.
        let Some(hooks) = group.get("hooks").and_then(Item::as_array_of_tables) else {
            continue;
        };
        for h in hooks.iter() {
            if let Some(c) = h.get("command").and_then(Item::as_str) {
                if is_ours(c) {
                    return Some(c.to_string());
                }
            }
        }
    }
    None
}
