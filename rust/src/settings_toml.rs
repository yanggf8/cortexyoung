//! Wiring `cort hook-suggest` into Codex's `~/.codex/config.toml`, and taking it back out.
//!
//! Codex loads a `PreToolUse` hook only from `[[hooks.PreToolUse]]` in `config.toml` -- and,
//! contrary to what this file believed between 2026-09-02 and 2026-09-29, `~/.codex/hooks.json`
//! is *not* dead weight on every Codex. The claim that both `~/.codex/hooks/hooks.json` and
//! `~/.codex/hooks.json` are silently ignored was established on codex-cli 0.152.1
//! (`docs/2026-09-02-hook-wiring-correction.md` §12) and held for it; the 2026-09-23 desktop
//! build (153.0.8010.53) loads `hooks.json`, writes `[hooks.state]` trust rows keyed to it, and
//! -- when `[desktop].external-agent-import-sync-enabled` is true -- *generates* it by copying
//! `~/.claude/settings.json`'s whole hooks table (binary string table:
//! `codex.external_agent_config.import`, "failed to serialize hooks.json"). An import copy of
//! our Claude Code entries therefore double-fires every hook beside the entry here, mis-attributed
//! to `--harness claude-code`. `cort hook-install --all` sweeps that sidecar
//! (`settings_sidecar`) and turns the sync off; `trusted_at` below anchors trust recognition to
//! this file's basename so a `hooks.json` trust row can no longer vouch for an entry here.
//! `docs/2026-09-29-codex-hooks-json-import-sync.md`.
//!
//! The group shape below -- a `matcher` for tool events, no matcher for SessionStart, and a nested `hooks` array of
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

use crate::settings::{backup_as, Change, Outcome};
use crate::settings::{events_for, is_ours_for, HookEvent, EDIT_MATCHER};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table, Value};

/// The tool this hook is matched to on Codex.
///
/// `Bash` is not a tool Codex has -- its rollouts name `exec_command`, `exec` and `apply_patch` --
/// and on 2026-09-03 that looked like the reason a wired, trusted, `Active`-reported entry never
/// fired on this machine across 323 tool calls. It is not. A second machine on the *same* codex-cli
/// 0.152.1, with the *same* tool vocabulary and byte-identical `matcher = "Bash"` and command, fires
/// 417 times. Later the same day a live payload settled why: Codex normalises the tool name in the
/// *hook payload* to Claude Code's vocabulary (`tool_name: "Bash"` while the rollout calls the same
/// tool `exec_command`), so the matcher is compared against `Bash` and always was. Both machines
/// agree; the 0-vs-417 gap was an entry created that morning plus a matcher edited into something
/// that could not match. Changing this constant was a change with no evidence behind it: reverted.
/// `docs/2026-09-03-installer-dedup-and-attribution.md` §7.
///
/// That payload did not establish whether the matcher is a regex -- `"Bash"` was the only value ever
/// observed firing, and the one alternative tried (`exec_command|shell`) could not have matched on
/// either reading. A live run on 2026-09-03 16:48 settled it: `matcher_for(Refresh)`'s alternation
/// fired on a `Bash` call, so Codex compiles it. The same run showed the refresh entry firing on a
/// file edit with *no* `hook-suggest` beside it -- and since `Suggest`'s matcher is the bare `Bash`
/// above, the tool was not Bash, which means Codex normalises its edit tool into Claude Code's
/// vocabulary too, exactly as it does `exec_command`. Which of the four edit names it lands on is
/// still unread; the gate is the whole alternation, so it does not change what to write here. §13.
const MATCHER: &str = "Bash";

fn matcher_for(event: HookEvent) -> Option<&'static str> {
    match event {
        HookEvent::Suggest => Some(MATCHER),
        HookEvent::Refresh => Some(EDIT_MATCHER),
        // The 2026-10-29 Codex SessionStart probe fired without a matcher key.
        HookEvent::Recall => None,
    }
}

fn matcher_matches(group: &Table, event: HookEvent) -> bool {
    match matcher_for(event) {
        Some(matcher) => group.get("matcher").and_then(Item::as_str) == Some(matcher),
        None => group.get("matcher").is_none(),
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
    /// An event this harness does not ship (`settings::events_for`), named by its hook event.
    Unsupported(&'static str),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Io(e) => write!(f, "{e}"),
            SettingsError::Unparsable(m) => write!(f, "{m}"),
            SettingsError::Unsupported(event) => write!(f, "{event} is not wired for this harness"),
        }
    }
}

impl From<io::Error> for SettingsError {
    fn from(e: io::Error) -> Self {
        SettingsError::Io(e)
    }
}

/// A faithful lift of the JSON side's error into this module's own: `sweep_trust_and_sync` takes
/// `backup_as` from `settings`, and its failures are the same two kinds this module names.
impl From<crate::settings::SettingsError> for SettingsError {
    fn from(e: crate::settings::SettingsError) -> Self {
        match e {
            crate::settings::SettingsError::Io(io) => SettingsError::Io(io),
            crate::settings::SettingsError::Unparsable(m) => SettingsError::Unparsable(m),
        }
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
    // The per-harness event set lives in settings::events_for; refuse what it does not ship.
    if !crate::settings::events_for("codex").contains(&event) {
        return Err(SettingsError::Unsupported(event.name()));
    }
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
        if ours_in_this_group && remaining == 1 && !matcher_matches(group, event) {
            if let Some(matcher) = matcher_for(event) {
                group.insert("matcher", value(matcher));
            } else {
                group.remove("matcher");
            }
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
        if let Some(matcher) = matcher_for(event) {
            group.insert("matcher", value(matcher));
        }
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

    for &event in events_for("codex") {
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
                    return Some((c.to_string(), trusted_at(path, hooks, gi, hi, event)));
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

/// Does `hooks.state` carry a `trusted_hash` for the entry at `gi`/`hi` of *this* settings file?
///
/// Codex's key shape is `<absolute path>:<event>:<gi>:<hi>`. Only the tail from the settings
/// file's own basename onward is matched -- `<basename>:<event>:<gi>:<hi>` -- never the full path.
/// That is deliberate in both directions. Comparing the full path would make recognition of our
/// own entry depend on two processes agreeing on how to write a path, the mistake §7/§9 already
/// paid for once on the command line; but matching the bare `:<event>:<gi>:<hi>` tail was its
/// own bug, found on 2026-09-29: Codex also keys trust rows for `~/.codex/hooks.json` (the
/// external-agent import-sync copy, see the module head), and `hooks.json:pre_tool_use:0:0`
/// ends with `pre_tool_use:0:0` exactly as `config.toml:pre_tool_use:0:0` does -- so a trust
/// decision Codex gave to a *different file's* entry vouched for ours, and `--status` reported
/// trusted. The basename sits between the two failure modes: path spellings in front of it
/// still don't matter, while a row written for another hook-bearing file can no longer be
/// mistaken for ours.
///
/// Presence is the whole of what this can report, and the doc comment on `installed_entry` is
/// where that limit is stated rather than hidden: the hash covers something only Codex can
/// recompute, so an entry trusted under an *older* command reads here exactly like a current
/// one. That is why `install.sh` says trust has to be renewed at the moment it rewrites the
/// command, instead of trying to detect it afterwards.
fn trusted_at(path: &Path, hooks: &Table, gi: usize, hi: usize, event: HookEvent) -> bool {
    let Some(state) = hooks.get("state").and_then(Item::as_table) else {
        return false;
    };
    let key = match event {
        HookEvent::Suggest => "pre_tool_use",
        HookEvent::Refresh => "post_tool_use",
        HookEvent::Recall => "session_start",
    };
    let basename = path
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_default();
    let suffix = format!("{basename}:{key}:{gi}:{hi}");
    state.iter().any(|(key, entry)| {
        // The basename must be a complete path segment of the key: whatever precedes it is a
        // separator, or there is nothing before it at all. Matching the bare tail alone would
        // let a trust row for a file named `myhooks.json` vouch for `hooks.json` -- a prefix,
        // not a basename.
        ((key.ends_with(&suffix) && key[..key.len() - suffix.len()].ends_with('/'))
            || key == suffix)
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
/// uninstaller must undo: scanning all events on the way out would otherwise leave behind an empty
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
    for &event in events_for("codex") {
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

/// Does the entry carrying exactly `expected_command` still have the shape this module writes
/// today? The matcher lives on the GROUP, which `installed_entry` never inspects — that gap
/// shipped a wired, trusted, green-in-`--check` hook aimed at `Bash`, a tool Codex does not
/// have (see the §12-13 comment in `install_hook`). Command-equality alone cannot see it.
pub fn entry_shape_ok(path: &Path, event: HookEvent, expected_command: &str) -> bool {
    let Ok(doc) = read_doc(path) else {
        return false;
    };
    let Some(list) = doc
        .as_table()
        .get("hooks")
        .and_then(Item::as_table)
        .and_then(|t| t.get(event.name()))
        .and_then(Item::as_array_of_tables)
    else {
        return false;
    };
    for group in list.iter() {
        let Some(entries) = group.get("hooks").and_then(Item::as_array_of_tables) else {
            continue;
        };
        for h in entries.iter() {
            if h.get("command").and_then(Item::as_str) == Some(expected_command) {
                return matcher_matches(group, event)
                    && h.get("type").and_then(Item::as_str) == Some("command")
                    && h.get("timeout").and_then(Item::as_integer) == Some(TIMEOUT_SECS);
            }
        }
    }
    false
}

/// What is `[desktop].external-agent-import-sync-enabled` set to, if the key is there at all?
/// Read-only; `--status` reports it because the sync is what re-creates the sidecar this crate
/// sweeps (see `settings_sidecar`).
pub fn import_sync_flag(path: &Path) -> Result<Option<bool>, SettingsError> {
    if !path.exists() {
        return Ok(None);
    }
    let doc = read_doc(path)?;
    Ok(doc
        .as_table()
        .get("desktop")
        .and_then(Item::as_table)
        .and_then(|t| t.get("external-agent-import-sync-enabled"))
        .and_then(Item::as_value)
        .and_then(Value::as_bool))
}

/// Is this `[hooks.state]` key one of Codex's trust rows for a file with one of `basenames`?
///
/// The key shape is `<absolute path>:<event>:<gi>:<hi>`, so the tail after the last three colons
/// is what identifies a trust row at all; the last two segments must be numeric, which is what
/// keeps plugin rows (`browser@openai-bundled:plugin.json#hooks[0]:stop:0:0`) -- whose "event"
/// is `stop` and whose path part names no hook file of ours -- from ever matching a basename.
fn state_key_names_basename(key: &str, basenames: &[&str]) -> bool {
    let mut parts = key.rsplitn(4, ':');
    let hi = parts.next();
    let gi = parts.next();
    let event = parts.next();
    let path_part = parts.next();
    let numeric = |s: Option<&str>| s.is_some_and(|v| v.parse::<usize>().is_ok());
    if !numeric(hi) || !numeric(gi) {
        return false;
    }
    match (event, path_part) {
        (Some(ev), Some(p)) if !ev.is_empty() => Path::new(p)
            .file_name()
            .map(|b| basenames.iter().any(|n| b == *n))
            .unwrap_or(false),
        _ => false,
    }
}

/// One read-modify-write pass over `config.toml` on behalf of the sidecar sweep
/// (`settings_sidecar`): drop every `[hooks.state]` trust row keyed to one of `sidecar_basenames`,
/// and — when `flip_sync_off` — turn `[desktop].external-agent-import-sync-enabled` to `false`.
///
/// Both edits share one backup and one write, not because either is expensive but because they
/// happen in the same second as `install_hook`'s own backup: two `.bak.<same stamp>` files are
/// one file, and the second copy silently erases the first. That is also why the backup tail is
/// `.sweep`-suffixed rather than `backup`'s plain stamp.
///
/// Dropping rows is keyed to what the sweep *did to the sidecar*, not to what it found here:
/// once entries are removed from `hooks.json`, every index below them shifts, and Codex's own
/// `hooks.json:<event>:<gi>:<hi>` rows silently re-aim at whatever moved into that slot. A row
/// for an entry that no longer exists is an orphan; a row for an entry that *moved* is worse --
/// it vouches for somebody else's position. Both go. Rows for this file, for plugins, and for
/// any sidecar that was not changed are not ours to drop.
pub fn sweep_trust_and_sync(
    path: &Path,
    sidecar_basenames: &[&str],
    flip_sync_off: bool,
) -> Result<(usize, Change, Option<PathBuf>), SettingsError> {
    if !path.exists() {
        return Ok((0, Change::NotPresent, None));
    }
    let mut doc = read_doc(path)?;
    let mut dropped = 0usize;

    if let Some(state) = doc
        .as_table_mut()
        .get_mut("hooks")
        .and_then(Item::as_table_mut)
        .and_then(|t| t.get_mut("state"))
        .and_then(Item::as_table_mut)
    {
        let dead: Vec<String> = state
            .iter()
            .map(|(k, _)| k.to_owned())
            .filter(|k| state_key_names_basename(k, sidecar_basenames))
            .collect();
        for k in &dead {
            state.remove(k);
            dropped += 1;
        }
    }
    // A `state` table this call emptied still prints its `[hooks.state]` header -- toml_edit's
    // `is_empty` counts the table itself, so `prune_empty_scaffolding` would leave the shell
    // behind and `hooks` would never quite vanish. Only a table we emptied goes.
    if dropped > 0 {
        let state_emptied = doc
            .as_table_mut()
            .get_mut("hooks")
            .and_then(Item::as_table_mut)
            .and_then(|t| t.get_mut("state"))
            .and_then(Item::as_table_mut)
            .map(|t| t.is_empty());
        if state_emptied == Some(true) {
            if let Some(h) = doc
                .as_table_mut()
                .get_mut("hooks")
                .and_then(Item::as_table_mut)
            {
                h.remove("state");
            }
        }
    }

    let mut sync = Change::NotPresent;
    if flip_sync_off {
        let current = doc
            .as_table()
            .get("desktop")
            .and_then(Item::as_table)
            .and_then(|d| d.get("external-agent-import-sync-enabled"))
            .and_then(Item::as_value)
            .and_then(Value::as_bool);
        sync = match current {
            Some(true) => {
                if let Some(d) = doc
                    .as_table_mut()
                    .get_mut("desktop")
                    .and_then(Item::as_table_mut)
                {
                    d.insert("external-agent-import-sync-enabled", value(false));
                }
                Change::Updated
            }
            Some(false) => Change::AlreadyPresent,
            None => Change::NotPresent,
        };
    }

    if dropped == 0 && !sync.wrote() {
        return Ok((0, sync, None));
    }
    // A `hooks` table that held only the dropped `state` is scaffolding this call emptied.
    prune_empty_scaffolding(&mut doc);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bak = backup_as(path, &format!("toml.bak.{stamp}.sweep"))?;
    write_doc(path, &doc)?;
    Ok((dropped, sync, bak))
}
