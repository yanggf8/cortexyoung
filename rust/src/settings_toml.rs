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
//! moves, and refuse to touch a file it cannot parse. `is_ours_for` is imported rather than
//! reimplemented -- the token-matching rule it encodes cost two rounds of hardening
//! (`docs/2026-09-02-hook-wiring-correction.md` §7, §9), and a second copy of it here is a second
//! place for the same bug to reappear.

use crate::settings::{is_ours_for, HookEvent, EVENTS};
use crate::settings::{Change, Outcome};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

/// Codex's own tool names, which are not Claude Code's.
///
/// This file used to write `"Bash"` here and `settings::EDIT_MATCHER`
/// (`Edit|Write|MultiEdit|NotebookEdit`) for the other event, both inherited wholesale from the
/// JSON side. Codex has never had a tool by any of those names: its shell surface is
/// `exec_command` (1,272 calls in this machine's rollouts against zero of anything Claude-shaped)
/// with `shell` as the older spelling, and its edit surface is `apply_patch`. The 0.152.1 binary
/// contains the strings `exec_command`, `shell` and `apply_patch` and contains `Bash`, `Edit`,
/// `MultiEdit`, `Grep` and `Glob` exactly zero times, so it does not normalise into Claude Code's
/// vocabulary either -- the matcher is compared against the name Codex uses, and a `Bash` matcher
/// there cannot fire. Both entries were wired, trusted, and structurally incapable of running
/// (reproduced 2026-09-03: `usage.db` holds two `harness=codex` rows in 90 days and both were
/// hand-fed on stdin by a developer).
///
/// This is the defect `settings_kimi` was written to avoid -- its `"Bash|Grep"` exists precisely
/// because a Bash-only matcher misses the surface the rule is for -- never carried back here.
const MATCHER: &str = "exec_command|shell";

/// `apply_patch` is Codex's only edit tool. `write_stdin` feeds an already-running process and is
/// not a file edit, so it is deliberately absent: a post-hook that reindexes on it would run the
/// incremental pass against a tree nothing changed in.
const CODEX_EDIT_MATCHER: &str = "apply_patch";

fn matcher_for(event: HookEvent) -> &'static str {
    match event {
        HookEvent::Suggest => MATCHER,
        HookEvent::Refresh => CODEX_EDIT_MATCHER,
    }
}

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
fn is_canonical(h: &Table, command: &str, _event: HookEvent) -> bool {
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
    raw.parse::<DocumentMut>().map_err(|e| {
        SettingsError::Unparsable(format!("{} is not valid TOML: {e}", path.display()))
    })
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
fn event_list(
    doc: &mut DocumentMut,
    event: HookEvent,
) -> Result<&mut ArrayOfTables, SettingsError> {
    let root = doc.as_table_mut();
    let hooks_item = root.entry("hooks").or_insert(Item::Table(Table::new()));
    if !hooks_item.is_table() {
        return Err(SettingsError::Unparsable(
            "`hooks` is present but is not a table; refusing to overwrite it".into(),
        ));
    }
    let hooks_table = hooks_item.as_table_mut().expect("hooks is a table");
    let pre = hooks_table
        .entry(event.name())
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    if !pre.is_array_of_tables() {
        return Err(SettingsError::Unparsable(format!(
            "`hooks.{}` is present but is not an array of tables; refusing to overwrite it",
            event.name()
        )));
    }
    Ok(pre
        .as_array_of_tables_mut()
        .expect("the event list is an array of tables"))
}

/// Add (or refresh) the `PreToolUse` group that runs `command`.
pub fn install_hook(
    path: &Path,
    command: &str,
    event: HookEvent,
) -> Result<Outcome, SettingsError> {
    let mut doc = read_doc(path)?;
    let list = event_list(&mut doc, event)?;

    let mut seen_ours = false;
    let mut found_same = false;
    let mut changed = false;
    let mut empty_groups: Vec<usize> = Vec::new();

    for gi in 0..list.len() {
        let group = list.get_mut(gi).expect("gi in range");
        let mut ours_in_this_group = false;
        let Some((before, remaining)) = ({
            match group
                .get_mut("hooks")
                .and_then(Item::as_array_of_tables_mut)
            {
                None => None,
                Some(hooks) => {
                    let before = hooks.len();
                    let mut drop_idx: Vec<usize> = Vec::new();
                    for hi in 0..hooks.len() {
                        let h = hooks.get_mut(hi).expect("hi in range");
                        let Some(cur) = h.get("command").and_then(Item::as_str).map(str::to_string)
                        else {
                            continue;
                        };
                        if !is_ours_for(&cur, event) {
                            continue;
                        }
                        // A file can already hold more than one of ours. Keep the first, drop the
                        // rest, so a redeploy converges on one entry instead of adding to the pile
                        // -- same rule as the JSON side, same reason (§7 of the wiring doc).
                        if seen_ours {
                            changed = true;
                            drop_idx.push(hi);
                            continue;
                        }
                        seen_ours = true;
                        ours_in_this_group = true;
                        if is_canonical(h, command, event) {
                            found_same = true;
                        } else {
                            *h = hook_command_table(command);
                            changed = true;
                        }
                    }
                    for hi in drop_idx.into_iter().rev() {
                        hooks.remove(hi);
                    }
                    Some((before, hooks.len()))
                }
            }
        }) else {
            continue;
        };
        // The matcher lives on the group, not on the entry, so `is_canonical` -- which only ever
        // sees the entry -- cannot notice a stale one. That gap shipped a Codex hook that was
        // wired, trusted, green in `--check` and matched against `Bash`, a tool Codex does not
        // have: every redeploy read the command as unchanged and reported `already_present` while
        // the matcher stayed dead. Same lesson as the skill's "a hash match must not excuse the
        // shape" (`tests/install-smoke.sh` Test 17), which this file never learned.
        //
        // Only when the group holds our entry and nothing else. A matcher on a shared group is
        // also somebody else's routing, and rewriting it would silently re-aim their hook.
        if ours_in_this_group
            && remaining == 1
            && group.get("matcher").and_then(Item::as_str) != Some(matcher_for(event))
        {
            group.insert("matcher", value(matcher_for(event)));
            changed = true;
            found_same = false;
        }
        if remaining == 0 && before > 0 {
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
        group.insert("matcher", value(matcher_for(event)));
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
    let mut removed = false;

    for event in EVENTS {
        let list = event_list(&mut doc, event)?;
        let mut empty_groups: Vec<usize> = Vec::new();
        for gi in 0..list.len() {
            let group = list.get_mut(gi).expect("gi in range");
            let Some(hooks) = group
                .get_mut("hooks")
                .and_then(Item::as_array_of_tables_mut)
            else {
                continue;
            };
            let before = hooks.len();
            let mut drop_idx: Vec<usize> = Vec::new();
            for hi in 0..hooks.len() {
                let h = hooks.get_mut(hi).expect("hi in range");
                if h.get("command")
                    .and_then(Item::as_str)
                    .is_some_and(|c| is_ours_for(c, event))
                {
                    drop_idx.push(hi);
                }
            }
            for hi in drop_idx.into_iter().rev() {
                hooks.remove(hi);
            }
            if hooks.len() != before {
                removed = true;
                if hooks.is_empty() {
                    empty_groups.push(gi);
                }
            }
        }
        for gi in empty_groups.into_iter().rev() {
            list.remove(gi);
        }
    }

    if !removed {
        return Ok(Outcome {
            change: Change::NotPresent,
            backup: None,
        });
    }
    // Leave no empty scaffolding behind: an install that added `hooks.PreToolUse` to a file that
    // had neither should not leave them once it is uninstalled.
    prune_empty_scaffolding(&mut doc);

    let bak = backup(path)?;
    write_doc(path, &doc)?;
    Ok(Outcome {
        change: Change::Removed,
        backup: bak,
    })
}

/// Is our entry currently configured, with which command, and has Codex been told to run it?
///
/// Wiring is only half of it here, which is the one way this file differs in kind from the JSON
/// side. Codex will not execute a hook it has not been shown: a wired-but-unreviewed entry sits in
/// `config.toml` and never fires, and `codex exec --dangerously-bypass-hook-trust` is the flag that
/// skips that gate. Reviewing it once in an interactive session persists, beside our entry:
///
/// ```toml
/// [hooks.state."<absolute config path>:pre_tool_use:<group>:<hook>"]
/// trusted_hash = "sha256:..."
/// ```
///
/// (observed on a real `~/.codex/config.toml`, 2026-09-02, Codex 0.152.1). Until this function,
/// `--check` answered `wired` for an entry in exactly that state, and the hook had never once run
/// on a normally-invoked Codex -- the same class of lie as `wired: false` while it fired hundreds
/// of times (`docs/2026-09-02-hook-wiring-correction.md` §7, §9), pointing the other way.
pub fn installed_entry(path: &Path, event: HookEvent) -> Option<(String, bool)> {
    let doc = read_doc(path).ok()?;
    let hooks = doc.as_table().get("hooks")?.as_table()?;
    let list = hooks.get(event.name())?.as_array_of_tables()?;
    for (gi, group) in list.iter().enumerate() {
        // `continue`, not `?`: a group without a hooks array is somebody else's malformed entry,
        // and letting it end the scan would report `wired: false` for an entry sitting right after
        // it -- the same bug §7 of the wiring doc found on the JSON side.
        let Some(entries) = group.get("hooks").and_then(Item::as_array_of_tables) else {
            continue;
        };
        for (hi, h) in entries.iter().enumerate() {
            if let Some(c) = h.get("command").and_then(Item::as_str) {
                if is_ours_for(c, event) {
                    return Some((c.to_string(), trusted_at(hooks, gi, hi, event)));
                }
            }
        }
    }
    None
}

/// Our entry's command alone, for callers that have no use for the trust half.
pub fn installed_command(path: &Path, event: HookEvent) -> Option<String> {
    installed_entry(path, event).map(|(command, _)| command)
}

/// Does `hooks.state` carry a `trusted_hash` for the entry at `gi`/`hi`?
///
/// Only the `:pre_tool_use:<gi>:<hi>` tail of the key is matched. The half in front of it is
/// Codex's spelling of the same file we were handed, and making recognition of our own entry
/// depend on two processes agreeing on how to write a path is the mistake §7/§9 already paid for
/// once on the command line. The tail cannot collide: `:pre_tool_use:0:0` is not a suffix of
/// `:pre_tool_use:10:0`.
///
/// Presence is the whole of what this can report, and the doc comment on `installed_entry` is
/// where that limit is stated rather than hidden: the hash covers something only Codex can
/// recompute, so an entry trusted under an *older* command reads here exactly like a current one.
/// That is why `install.sh` says trust has to be renewed at the moment it rewrites the command,
/// instead of trying to detect it afterwards.
fn trusted_at(hooks: &Table, gi: usize, hi: usize, event: HookEvent) -> bool {
    let Some(state) = hooks.get("state").and_then(Item::as_table) else {
        return false;
    };
    let key = match event {
        HookEvent::Suggest => "pre_tool_use",
        HookEvent::Refresh => "post_tool_use",
    };
    let suffix = format!(":{key}:{gi}:{hi}");
    state.iter().any(|(key, entry)| {
        key.ends_with(&suffix)
            && entry
                .as_table_like()
                .and_then(|t| t.get("trusted_hash"))
                .and_then(Item::as_str)
                .is_some_and(|h| !h.is_empty())
    })
}

/// Drop any event list this module created and then left empty, and `hooks` itself once nothing of
/// ours or anyone else's is left in it.
///
/// `event_list` creates the key it is asked for, which is what an installer wants and what an
/// uninstaller must undo: scanning both events on the way out would otherwise leave behind an empty
/// `hooks.PostToolUse` in a file that never had one. Only empty lists go, and only after the scan
/// -- somebody else's populated `PostToolUse` is not ours to touch.
fn prune_empty_scaffolding(doc: &mut DocumentMut) {
    let Some(h) = doc
        .as_table_mut()
        .get_mut("hooks")
        .and_then(Item::as_table_mut)
    else {
        return;
    };
    for event in EVENTS {
        if h.get(event.name())
            .and_then(Item::as_array_of_tables)
            .is_some_and(|a| a.is_empty())
        {
            h.remove(event.name());
        }
    }
    if h.is_empty() {
        doc.as_table_mut().remove("hooks");
    }
}
