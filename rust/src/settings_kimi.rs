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
//!   of the search traffic whose fires the row records. `"*"` is not a wildcard here; it throws, is
//!   caught, and silently disables the hook.
//! * **This file already has other owners.** The Kimi plugin writes its own managed block into it,
//!   one per host, and rewrites the file when it updates. `is_ours_for` is imported rather than
//!   reimplemented for the same reason as in `settings_toml`, and `toml_edit` is used so that
//!   everything we do not own -- including Kimi's `[hooks.state]`-equivalent bookkeeping and other
//!   people's `[[hooks]]` entries -- survives byte for byte.

use crate::settings::{events_for, is_ours_for, HookEvent, EDIT_MATCHER};
use crate::settings::{Change, Outcome};
use crate::settings_toml::{replace_entry, SettingsError};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, RawString, Table};

/// The tool surfaces whose fires the thin trigger records. Compiled as a regex against the tool
/// name, so one entry covers both.
const MATCHER: &str = "Bash|Grep";

fn matcher_for(event: HookEvent) -> Option<&'static str> {
    match event {
        HookEvent::Suggest => Some(MATCHER),
        HookEvent::Refresh => Some(EDIT_MATCHER),
        // Remind is never wired for Kimi: its `UserPromptSubmit` entry is already `hook-recall`,
        // and `events_for("kimi-code")` omits Remind, so `install` refuses it with `Unsupported`.
        HookEvent::Recall | HookEvent::Remind => None,
    }
}
fn event_name(event: HookEvent) -> &'static str {
    if event == HookEvent::Recall {
        "UserPromptSubmit"
    } else {
        event.name()
    }
}
/// Seconds, from the one table in `settings`: the work is the same subprocess on every harness.
/// Kimi's schema allows 1..=600.
fn timeout_secs(event: HookEvent) -> i64 {
    i64::from(event.timeout_secs())
}

/// `${KIMI_CODE_HOME:-~/.kimi-code}/config.toml`, the same override `resolveKimiHome` reads.
pub fn default_settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("KIMI_CODE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".kimi-code")))?;
    Some(home.join("config.toml"))
}

fn hook_table(command: &str, event: HookEvent) -> Table {
    let mut t = Table::new();
    t.insert("event", value(event_name(event)));
    if let Some(matcher) = matcher_for(event) {
        t.insert("matcher", value(matcher));
    }
    t.insert("command", value(command));
    t.insert("timeout", value(timeout_secs(event)));
    t
}

/// Is this entry exactly what `install_hook` would write right now? Field-by-field rather than a
/// serialized comparison, which would trip on formatting `toml_edit` introduces.
fn is_canonical(h: &Table, command: &str, event: HookEvent) -> bool {
    h.get("event").and_then(Item::as_str) == Some(event_name(event))
        && match matcher_for(event) {
            Some(matcher) => h.get("matcher").and_then(Item::as_str) == Some(matcher),
            None => h.get("matcher").is_none(),
        }
        && h.get("command").and_then(Item::as_str) == Some(command)
        && h.get("timeout").and_then(Item::as_integer) == Some(timeout_secs(event))
}

fn read_doc(path: &Path) -> Result<DocumentMut, SettingsError> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DocumentMut::new()),
        Err(e) => return Err(SettingsError::Io(e)),
    };
    raw.parse::<DocumentMut>().map_err(|e| {
        SettingsError::Unparsable(format!("{} is not valid TOML: {e}", path.display()))
    })
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
///
/// That holds only for an entry that is last in the file, and `toml_edit` places one without a
/// position after the last `[[hooks]]`, above any table that follows: the trailer then moved up
/// with it, and our entry stood in front of a fence that a following table carried as its prefix.
fn push_after_trailing(doc: &mut DocumentMut, entry: Table) -> Result<(), SettingsError> {
    let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
    let mut entry = entry;
    let mut last = 0;
    each_table(doc.as_table(), &mut |t| {
        last = last.max(t.position().unwrap_or(0));
    });
    entry.set_position(Some(last + 1));
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
pub fn install_hook(
    path: &Path,
    command: &str,
    event: HookEvent,
) -> Result<Outcome, SettingsError> {
    // The per-harness event set lives in settings::events_for; refuse what it does not ship.
    if !crate::settings::events_for("kimi-code").contains(&event) {
        return Err(SettingsError::Unsupported(event.name()));
    }
    let mut doc = read_doc(path)?;
    let list = hooks_array(&mut doc)?;

    let mut seen_ours = false;
    let mut found_same = false;
    let mut changed = false;
    let mut drop_idx: Vec<usize> = Vec::new();

    for i in 0..list.len() {
        let entry = list.get_mut(i).expect("i in range");
        let Some(cur) = entry
            .get("command")
            .and_then(Item::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if !is_ours_for(&cur, event) {
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
        if is_canonical(entry, command, event) {
            found_same = true;
        } else {
            replace_entry(entry, hook_table(command, event));
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
        push_after_trailing(&mut doc, hook_table(command, event))?;
        Change::Installed
    };
    let bak = backup(path)?;
    write_doc(path, &doc)?;
    Ok(Outcome {
        change,
        backup: bak,
    })
}

/// Everything around and inside one of our entries that we did not write: the text above its header
/// and any comment somebody put in it, each on a line of its own.
fn left_behind(entry: &Table) -> String {
    let above = entry.decor().prefix().and_then(RawString::as_str);
    let above = above.unwrap_or_default().trim_end_matches([' ', '\t']);
    // The blank line directly above the header is the one the install added. Only that one goes:
    // keeping it would grow the file on every cycle, and the rest were there before.
    let mut text = match above.strip_suffix('\n') {
        Some(rest) if rest.is_empty() || rest.ends_with('\n') => rest,
        _ => above,
    }
    .to_string();
    let mut keep_comments = |raw: Option<&RawString>| {
        for line in raw.and_then(RawString::as_str).unwrap_or_default().lines() {
            if line.trim_start().starts_with('#') {
                text.push_str(line.trim());
                text.push('\n');
            }
        }
    };
    keep_comments(entry.decor().suffix());
    for (name, item) in entry.iter() {
        keep_comments(entry.key(name).and_then(|k| k.leaf_decor().prefix()));
        keep_comments(item.as_value().and_then(|v| v.decor().suffix()));
    }
    text
}

/// Every table printed below `table`, in no particular order.
fn each_table<'t>(table: &'t Table, visit: &mut dyn FnMut(&'t Table)) {
    for (_, item) in table.iter() {
        if let Some(t) = item.as_table() {
            visit(t);
            each_table(t, visit);
        }
        for t in item.as_array_of_tables().into_iter().flatten() {
            visit(t);
            each_table(t, visit);
        }
    }
}

/// The position of the first header after `position` in the file, whichever table it opens.
fn position_after(root: &Table, position: isize) -> Option<isize> {
    let mut next: Option<isize> = None;
    each_table(root, &mut |t| {
        if let Some(p) = t.position().filter(|&p| p > position) {
            next = Some(next.map_or(p, |n| n.min(p)));
        }
    });
    next
}

/// Put `text` above the header at `position`. True once it is done.
fn prepend_to_prefix(table: &mut Table, position: isize, text: &str) -> bool {
    let mut on = |t: &mut Table| {
        if t.position() != Some(position) {
            return prepend_to_prefix(t, position, text);
        }
        let prefix = t.decor().prefix().and_then(RawString::as_str);
        let prefix = format!("{text}{}", prefix.unwrap_or_default());
        t.decor_mut().set_prefix(prefix);
        true
    };
    for (_, item) in table.iter_mut() {
        let done = match item {
            Item::Table(t) => on(t),
            Item::ArrayOfTables(list) => list.iter_mut().any(&mut on),
            _ => false,
        };
        if done {
            return true;
        }
    }
    false
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
            .is_some_and(|c| events_for("kimi-code").iter().any(|&e| is_ours_for(c, e)))
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
    // What decorates one of our entries is somebody else's: cort writes no comment here, and
    // `push_after_trailing` parks the Kimi plugin's own `# === END … ===` line in front of our
    // entry. Their uninstaller looks for that line, so it stays where it stands -- in front of
    // whatever follows, or at the end of the file when nothing does. It went to the end
    // unconditionally until 2026-11-25, which carried the fence past every entry added after ours.
    let mut left: Vec<(isize, String)> = Vec::new();
    for i in drop_idx.into_iter().rev() {
        let entry = list.get(i).expect("i in range");
        // A parsed table always has a position; without one, it was the last.
        left.push((entry.position().unwrap_or(isize::MAX), left_behind(entry)));
        list.remove(i);
    }
    let now_empty = list.is_empty();
    // Last entry first, so that neighbours of ours end up in front of the same table in order.
    left.sort_by_key(|&(position, _)| std::cmp::Reverse(position));
    for (position, text) in left {
        match position_after(doc.as_table(), position) {
            Some(next) => {
                prepend_to_prefix(doc.as_table_mut(), next, &text);
            }
            None => {
                let trailing = doc.trailing().as_str().unwrap_or_default();
                let trailing = format!("{text}{trailing}");
                doc.set_trailing(trailing);
            }
        }
    }
    // Leave no empty scaffolding: a `hooks` array we created and then emptied should not outlive
    // the entry it was created for.
    if now_empty {
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
pub fn installed_command(path: &Path, event: HookEvent) -> Option<String> {
    let doc = read_doc(path).ok()?;
    let list = doc.as_table().get("hooks")?.as_array_of_tables()?;
    for entry in list.iter() {
        if let Some(c) = entry.get("command").and_then(Item::as_str) {
            if is_ours_for(c, event) {
                return Some(c.to_string());
            }
        }
    }
    None
}

/// Does an entry exactly in today's shape exist for `expected_command`? Kimi's entry is flat
/// and self-contained, so this is `is_canonical` (event, matcher, command, timeout — every
/// field `install_hook` writes) over the whole `[[hooks]]` list: a correct command with a
/// stale matcher finds no canonical entry and reads not-in-shape.
pub fn entry_shape_ok(path: &Path, event: HookEvent, expected_command: &str) -> bool {
    let Ok(doc) = read_doc(path) else {
        return false;
    };
    let Some(list) = doc
        .as_table()
        .get("hooks")
        .and_then(Item::as_array_of_tables)
    else {
        return false;
    };
    // One predicate carries the whole check: a canonical entry is command+event+matcher+
    // timeout, all four exactly what `install_hook` writes today. A rewritten matcher means
    // no entry is canonical, and the answer is false — the same verdict as a missing entry.
    let found = list
        .iter()
        .any(|e| is_canonical(e, expected_command, event));
    let _ = list; // the borrow must outlive the `any` temporary (NLL quirk, measured)
    found
}
