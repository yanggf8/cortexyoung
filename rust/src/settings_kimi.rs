//! Wiring `cort hook-suggest` into Kimi's `${KIMI_CODE_HOME:-~/.kimi-code}/config.toml`, and taking
//! it back out.
//!
//! A third module rather than a parameter on the second one, because Kimi's shape is not Codex's
//! shape wearing different names. Codex nests: an array of `[[hooks.PreToolUse]]` groups, each with
//! a `matcher` and its own inner `hooks` array. Kimi is flat -- one top-level `[[hooks]]` array whose
//! entries are self-contained, with the event as a *field* rather than as the array's name:
//!
//! ```toml
//! [[hooks]]
//! event = "PreToolUse"
//! matcher = "Bash|Grep"
//! command = "<cort> hook-suggest --harness kimi-code"
//! timeout = 5
//! ```
//!
//! Read off the shipped `@moonshot-ai/kimi-code` bundle's own `hookDefSchema` (`event` required,
//! `matcher` optional string, `command` required non-empty, `timeout` an optional 1..=600 integer)
//! and confirmed against a live run, 2026-09-02.
//!
//! Two things this file has to get right that the other two never faced:
//!
//! * **The matcher is a regex, compiled with `new RegExp()`.** `"Bash|Grep"` is deliberate: Kimi's
//!   search surface is mostly its structured `Grep` tool -- 834 calls against 32 shell greps in the
//!   local corpus, the opposite of Claude Code's split -- so a `Bash`-only matcher would miss most
//!   of the traffic the rule exists for. `"*"` is not a wildcard here; it throws, is caught, and
//!   silently disables the hook.
//! * **This file already has other owners.** The Kimi plugin writes its own managed block into it,
//!   one per host, and rewrites the file when it updates. `is_ours` is imported rather than
//!   reimplemented for the same reason as in `settings_toml`, and `toml_edit` is used so that
//!   everything we do not own -- including Kimi's `[hooks.state]`-equivalent bookkeeping and other
//!   people's `[[hooks]]` entries -- survives byte for byte.

use crate::settings::{is_ours, Change, Outcome};
use crate::settings_toml::SettingsError;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

/// Both surfaces a search can arrive on. Compiled as a regex against the tool name.
const MATCHER: &str = "Bash|Grep";
const EVENT: &str = "PreToolUse";
/// Seconds. Kimi's schema allows 1..=600; the other two modules use 5 and there is no reason for
/// this one to differ, since the work is the same subprocess doing the same lookup.
const TIMEOUT_SECS: i64 = 5;

/// `${KIMI_CODE_HOME:-~/.kimi-code}/config.toml`, the same override `resolveKimiHome` reads.
pub fn default_settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("KIMI_CODE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".kimi-code")))?;
    Some(home.join("config.toml"))
}

fn hook_table(command: &str) -> Table {
    let mut t = Table::new();
    t.insert("event", value(EVENT));
    t.insert("matcher", value(MATCHER));
    t.insert("command", value(command));
    t.insert("timeout", value(TIMEOUT_SECS));
    t
}

/// Is this entry exactly what `install_hook` would write right now? Field-by-field rather than a
/// serialized comparison, which would trip on formatting `toml_edit` introduces.
fn is_canonical(h: &Table, command: &str) -> bool {
    h.get("event").and_then(Item::as_str) == Some(EVENT)
        && h.get("matcher").and_then(Item::as_str) == Some(MATCHER)
        && h.get("command").and_then(Item::as_str) == Some(command)
        && h.get("timeout").and_then(Item::as_integer) == Some(TIMEOUT_SECS)
}

fn read_doc(path: &Path) -> Result<DocumentMut, SettingsError> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DocumentMut::new()),
        Err(e) => return Err(SettingsError::Io(e)),
    };
    raw.parse::<DocumentMut>()
        .map_err(|e| SettingsError::Unparsable(format!("{} is not valid TOML: {e}", path.display())))
}

/// Write beside the target and rename: Kimi reads this file on every session start, and another
/// process (its own plugin installer) may be reading it at the same moment.
fn write_doc(path: &Path, doc: &DocumentMut) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(SettingsError::Io)?;
    }
    let tmp = path.with_extension("toml.cort-tmp");
    fs::write(&tmp, doc.to_string()).map_err(SettingsError::Io)?;
    fs::rename(&tmp, path).map_err(SettingsError::Io)?;
    Ok(())
}

fn backup(path: &Path) -> Result<Option<PathBuf>, SettingsError> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = path.with_extension(format!("toml.bak.{stamp}"));
    fs::copy(path, &dest).map_err(SettingsError::Io)?;
    Ok(Some(dest))
}

/// The top-level `hooks` array of tables, created when absent -- and refused, never replaced, when
/// it is there with a shape this module does not understand.
fn hooks_array(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, SettingsError> {
    let root = doc.as_table_mut();
    let item = root
        .entry("hooks")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    if !item.is_array_of_tables() {
        return Err(SettingsError::Unparsable(
            "`hooks` is present but is not an array of tables; refusing to overwrite it".into(),
        ));
    }
    Ok(item
        .as_array_of_tables_mut()
        .expect("hooks is an array of tables"))
}

/// Append our entry *after* everything already in the file, comments included.
///
/// `toml_edit` puts a pushed array-of-tables element before whatever trails the document, and this
/// particular file ends inside somebody else's fence: the Kimi plugin writes
/// `# === BEGIN kimi-plugin-cc-managed:<host> ===` … `# === END … ===` around its own `[[hooks]]`
/// entry, and that END line is the document trailer. Pushed naively, our entry lands *inside* their
/// block -- where their own uninstall would take it out with theirs, silently, and the next time
/// they rewrite the block it would be gone.
///
/// So the trailer is moved to the front of what we add, which puts it back immediately after the
/// entries it was closing. Their bytes are preserved exactly; only our entry moves.
fn push_after_trailing(doc: &mut DocumentMut, entry: Table) -> Result<(), SettingsError> {
    let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
    let mut entry = entry;
    if !trailing.trim().is_empty() {
        let mut prefix = trailing.clone();
        if !prefix.ends_with('\n') {
            prefix.push('\n');
        }
        prefix.push('\n');
        entry.decor_mut().set_prefix(prefix);
        doc.set_trailing("");
    }
    hooks_array(doc)?.push(entry);
    Ok(())
}
/// Add, or bring up to date, the one entry that runs `command`.
pub fn install_hook(path: &Path, command: &str) -> Result<Outcome, SettingsError> {
    let mut doc = read_doc(path)?;
    let list = hooks_array(&mut doc)?;

    let mut seen_ours = false;
    let mut found_same = false;
    let mut changed = false;
    let mut drop_idx: Vec<usize> = Vec::new();

    for i in 0..list.len() {
        let entry = list.get_mut(i).expect("i in range");
        let Some(cur) = entry.get("command").and_then(Item::as_str).map(str::to_string) else {
            continue;
        };
        if !is_ours(&cur) {
            continue;
        }
        // A file can already hold more than one of ours -- hand-wiring, or an older layout. Keep
        // the first and drop the rest so a redeploy converges instead of adding to the pile.
        if seen_ours {
            changed = true;
            drop_idx.push(i);
            continue;
        }
        seen_ours = true;
        if is_canonical(entry, command) {
            found_same = true;
        } else {
            *entry = hook_table(command);
            changed = true;
        }
    }
    for i in drop_idx.into_iter().rev() {
        list.remove(i);
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
        push_after_trailing(&mut doc, hook_table(command))?;
        Change::Installed
    };
    let bak = backup(path)?;
    write_doc(path, &doc)?;
    Ok(Outcome {
        change,
        backup: bak,
    })
}

/// Take our entry out, leaving every other `[[hooks]]` entry -- notably the Kimi plugin's own
/// managed block, which gates that plugin's whole safety contract -- untouched.
pub fn remove_hook(path: &Path) -> Result<Outcome, SettingsError> {
    if !path.exists() {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    let mut doc = read_doc(path)?;
    let list = hooks_array(&mut doc)?;
    let mut drop_idx: Vec<usize> = Vec::new();
    for i in 0..list.len() {
        let entry = list.get(i).expect("i in range");
        if entry
            .get("command")
            .and_then(Item::as_str)
            .is_some_and(is_ours)
        {
            drop_idx.push(i);
        }
    }
    if drop_idx.is_empty() {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    for i in drop_idx.into_iter().rev() {
        list.remove(i);
    }
    // Leave no empty scaffolding: a `hooks` array we created and then emptied should not outlive
    // the entry it was created for.
    if list.len() == 0 {
        doc.as_table_mut().remove("hooks");
    }
    let bak = backup(path)?;
    write_doc(path, &doc)?;
    Ok(Outcome {
        change: Change::Removed,
        backup: bak,
    })
}

/// Our entry's command, if it is configured. Kimi has no trust gate of Codex's kind, so wired is
/// the whole question here.
pub fn installed_command(path: &Path) -> Option<String> {
    let doc = read_doc(path).ok()?;
    let list = doc.as_table().get("hooks")?.as_array_of_tables()?;
    for entry in list.iter() {
        if let Some(c) = entry.get("command").and_then(Item::as_str) {
            if is_ours(c) {
                return Some(c.to_string());
            }
        }
    }
    None
}
