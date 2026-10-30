//! Sweeping Codex's sidecar hook file — `~/.codex/hooks.json`, plus `~/.codex/hooks/hooks.json`
//! should it ever exist — of copies of `cort`'s entries and of hook entries that provably came
//! from somewhere else.
//!
//! Why the file exists at all: since the 2026-09-23 desktop build (153.0.8010.53), Codex ships an
//! external-agent import-sync which, when `[desktop].external-agent-import-sync-enabled` is true,
//! copies `~/.claude/settings.json`'s whole hooks table into `~/.codex/hooks.json` and then *loads*
//! that file — it writes `[hooks.state]` trust rows keyed to it, which the 2026-09-02 codex-cli
//! 0.152.1 finding "hooks.json is silently ignored" did not see coming
//! (`docs/2026-09-29-codex-hooks-json-import-sync.md`). The consequence on a machine with this
//! crate wired into both harnesses: every Bash fires `hook-suggest` twice, once from `config.toml`
//! as `--harness codex` and once from the sidecar as `--harness claude-code`, and every edit fires
//! `hook-refresh` twice the same way.
//!
//! What this module may remove, and the line it will not cross:
//!
//! * Entries of ours (`is_ours_for`, the one token rule) go wherever they are found in a sidecar —
//!   they are ours, and in a file Codex loads they are at best a double fire and at worst
//!   mis-attributed. Note this catches a *deliberately* `--harness claude-code`-tagged entry too;
//!   the token rule is the whole of what "ours" means here, by design.
//! * Foreign entries go only when every foreign entry in the file can be found, structurally
//!   identical, in the live `~/.claude/settings.json` hooks table under the same event — i.e. the
//!   file is provably an import copy and nothing Codex-only is lost. One entry that matches
//!   nothing holds the whole batch back, and the report says so (`foreign_all_copies: false`).
//!   "Identical" is `serde_json::Value` equality, not byte equality: Codex re-serializes what it
//!   copies, so raw bytes could never match, and key order inside a JSON object is not meaning.
//!
//! The sweep never deletes a file. A sidecar emptied completely is left as `{}` — the file is
//! Codex's to own, and a deleted file the sync wants to recreate is a worse half-state than an
//! empty one.
//!
//! The whole mutating sweep is skipped while the Codex desktop app is running (`skipped_reason`):
//! it rewrites `config.toml` wholesale on its way out and its sync can repopulate the sidecar
//! under us, so a sweep mid-run buys a state nothing can reason about afterwards. `--status` and
//! `--remove` still run (read-only / ours-only respectively); only install mode is gated.

use crate::settings::{self, backup_as, is_ours_for, Change, HookEvent, SettingsError};
use crate::settings_toml;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The basename every Codex sidecar hook file shares. `[hooks.state]` trust rows are keyed by the
/// path Codex read them from, so the basename is what ties a trust row to this file — see
/// `settings_toml::sweep_trust_and_sync`.
pub const SIDECAR_BASENAME: &str = "hooks.json";

/// The sidecar locations, in the order they are swept. The second is where the 2026-09-02
/// probing put a hooks file; it is absent on every machine seen so far and costs one `exists()`.
pub fn sidecar_paths(codex_home: &Path) -> [PathBuf; 2] {
    [
        codex_home.join(SIDECAR_BASENAME),
        codex_home.join("hooks").join(SIDECAR_BASENAME),
    ]
}

/// What the caller is doing. `Install` sweeps ours plus verified copies and turns the sync off;
/// `Uninstall` sweeps ours only and never touches the sync; `Status` reads and writes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SidecarMode {
    Status,
    Uninstall,
    Install,
}

impl SidecarMode {
    fn as_str(self) -> &'static str {
        match self {
            SidecarMode::Status => "status",
            SidecarMode::Uninstall => "uninstall",
            SidecarMode::Install => "install",
        }
    }
}

/// One sidecar file's outcome. The counts are entries, not groups — the report says how many hook
/// commands are gone, because that is the number that shows up as double fires removed. In
/// `Status` mode nothing is written, and the same fields read as "what a sweep would remove".
#[derive(Debug)]
pub struct SidecarFile {
    pub path: PathBuf,
    pub existed: bool,
    pub ours_removed: usize,
    pub foreign_removed: usize,
    pub foreign_left: usize,
    pub foreign_all_copies: bool,
    pub change: Change,
    pub backup: Option<PathBuf>,
}

/// The whole sweep's outcome, as the installer reports it. Errors are collected, never fatal —
/// one unparsable sidecar must not cost the other one its sweep, the same rule the per-harness
/// install loop holds to.
#[derive(Debug)]
pub struct SidecarReport {
    pub mode: &'static str,
    pub codex_running: bool,
    pub skipped_reason: Option<&'static str>,
    pub files: Vec<SidecarFile>,
    /// `external-agent-import-sync-enabled` as it was read, before anything turned it off.
    pub import_sync_was: Option<bool>,
    /// What the sweep did to it, as `Change::as_str` — absent when the mode does not touch it.
    pub import_sync_change: Option<String>,
    pub trust_rows_dropped: usize,
    pub errors: Vec<String>,
}

impl SidecarReport {
    pub fn to_value(&self) -> Value {
        json!({
            "mode": self.mode,
            "codex_running": self.codex_running,
            "skipped_reason": self.skipped_reason,
            "files": self
                .files
                .iter()
                .map(|f| {
                    json!({
                        "path": f.path.to_string_lossy(),
                        "existed": f.existed,
                        "ours_removed": f.ours_removed,
                        "foreign_removed": f.foreign_removed,
                        "foreign_left": f.foreign_left,
                        "foreign_all_copies": f.foreign_all_copies,
                        "change": f.change.as_str(),
                        "backup": f.backup.as_ref().map(|b| b.to_string_lossy()),
                    })
                })
                .collect::<Vec<_>>(),
            "import_sync": {
                "was": self.import_sync_was,
                "change": self.import_sync_change,
            },
            "trust_rows_dropped": self.trust_rows_dropped,
            "errors": self.errors,
        })
    }
}

/// Which of our two events a sidecar event key maps to. Our hooks only ever live under these two
/// names; anything else in the file is somebody else's and can only ever leave as a verified copy.
fn event_of(key: &str) -> Option<HookEvent> {
    match key {
        "PreToolUse" => Some(HookEvent::Suggest),
        "PostToolUse" => Some(HookEvent::Refresh),
        _ => None,
    }
}

/// Does some group in the live Claude Code hooks under event `ev`, carrying the same `matcher`,
/// contain exactly `entry`? The whole of the copy signature: same event, same routing, same
/// command — compared structurally, because Codex re-serializes what it copies.
fn live_contains(
    live: Option<&Map<String, Value>>,
    ev: &str,
    matcher: Option<&str>,
    entry: &Value,
) -> bool {
    let Some(groups) = live.and_then(|l| l.get(ev)).and_then(Value::as_array) else {
        return false;
    };
    groups.iter().any(|g| {
        g.get("matcher").and_then(Value::as_str) == matcher
            && g.get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hs| hs.contains(entry))
    })
}

/// Sweep one sidecar file. `allow_foreign` lets verified-copy foreign entries go (install mode);
/// without it only ours leave (uninstall mode). `write` is what separates `Status` from the modes
/// that mutate: the walk is identical, only the backup-and-write is skipped.
fn sweep_json_hooks(
    path: &Path,
    live: Option<&Map<String, Value>>,
    allow_foreign: bool,
    write: bool,
) -> Result<SidecarFile, SettingsError> {
    let mut file = SidecarFile {
        path: path.to_path_buf(),
        existed: path.exists(),
        ours_removed: 0,
        foreign_removed: 0,
        foreign_left: 0,
        foreign_all_copies: false,
        change: Change::NotPresent,
        backup: None,
    };
    if !file.existed {
        return Ok(file);
    }
    let mut root = settings::read_root(path)?;
    if let Some(h) = root.get("hooks") {
        if !h.is_object() {
            // A malformed `hooks` is a refusal, not a repair — the same rule every settings
            // module in this crate holds to.
            return Err(SettingsError::Unparsable(format!(
                "{} has a `hooks` that is not an object; refusing to touch the file",
                path.display()
            )));
        }
    }
    // Absent entirely — including a file this sweep emptied on an earlier run — is a no-op.
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(file);
    };

    // Pass one, read-only: how much of this file is ours, how much is foreign, and is every
    // foreign entry a copy of something live. The verdict is per FILE, not per entry: a file that
    // is nine verified copies plus one Codex-only entry is a file this module does not fully
    // understand, and the safe move with such a file is to take only what is certainly ours.
    let mut ours_total = 0usize;
    let mut foreign_total = 0usize;
    let mut copies_total = 0usize;
    for (ev, list) in hooks.iter() {
        let event = event_of(ev);
        let Some(groups) = list.as_array() else {
            continue;
        };
        for g in groups {
            let matcher = g.get("matcher").and_then(Value::as_str);
            let Some(entries) = g.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for h in entries {
                if let Some(e) = event {
                    if h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|c| is_ours_for(c, e))
                    {
                        ours_total += 1;
                        continue;
                    }
                }
                foreign_total += 1;
                if live_contains(live, ev, matcher, h) {
                    copies_total += 1;
                }
            }
        }
    }
    file.ours_removed = ours_total;
    file.foreign_all_copies = foreign_total > 0 && copies_total == foreign_total;
    let take_foreign = allow_foreign && file.foreign_all_copies;

    // Pass two, mutating: ours always go; foreign go only under the file-level verdict above.
    let mut changed = false;
    let mut ours_gone = 0usize;
    let mut foreign_gone = 0usize;
    let mut emptied_events: Vec<String> = Vec::new();
    for (ev, list) in hooks.iter_mut() {
        let event = event_of(ev);
        let Some(groups) = list.as_array_mut() else {
            continue;
        };
        let before = groups.len();
        groups.retain_mut(|g| {
            let Some(entries) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            if take_foreign {
                // The verdict covers the whole file, so the group goes whole: ours out of it are
                // counted as ours, the rest as the verified copies they were proven to be.
                let ours_in_group = match event {
                    Some(e) => entries
                        .iter()
                        .filter(|h| {
                            h.get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|c| is_ours_for(c, e))
                        })
                        .count(),
                    None => 0,
                };
                ours_gone += ours_in_group;
                foreign_gone += entries.len() - ours_in_group;
                return false;
            }
            let b = entries.len();
            if let Some(e) = event {
                entries.retain(|h| {
                    !h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|c| is_ours_for(c, e))
                });
            }
            let gone = b - entries.len();
            ours_gone += gone;
            if gone > 0 {
                changed = true;
            }
            !entries.is_empty()
        });
        if groups.len() != before {
            changed = true;
            if groups.is_empty() && before > 0 {
                emptied_events.push(ev.clone());
            }
        }
    }
    for ev in &emptied_events {
        hooks.remove(ev);
    }
    // An event list this call emptied is scaffolding we made empty; `hooks` itself goes too once
    // nothing at all is left under it. Lists that were empty before we arrived are not ours.
    if changed
        && root
            .get("hooks")
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty)
    {
        root.remove("hooks");
    }

    file.ours_removed = ours_gone;
    if take_foreign {
        file.foreign_removed = foreign_gone;
    } else {
        file.foreign_left = foreign_total;
    }
    if changed && write {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // The `.sweep` tail keeps this backup from colliding with the one `install_hook` took
        // seconds earlier for the same run: two `.bak.<same stamp>` copies are one copy, and the
        // second write would erase the only snapshot of the pre-sweep state.
        file.backup = backup_as(path, &format!("json.bak.{stamp}.sweep"))?;
        settings::write_root(path, &root)?;
        file.change = Change::Removed;
    }
    Ok(file)
}

/// The sidecar sweep, end to end: classify and (except in `Status`) rewrite every sidecar under
/// `codex_home`, then hand `config.toml` to `sweep_trust_and_sync` for the orphaned trust rows
/// and the sync flag. Never fatal: every failure lands in `errors` and the rest of the sweep
/// carries on.
///
/// Trust rows are dropped only when a sidecar was actually written: a row for an entry that no
/// longer exists is an orphan, and one for an entry that *moved* (any removal shifts the indices
/// below it) is worse — it vouches for whatever slid into its slot. A sidecar this call did not
/// change keeps its rows, because its entries sit exactly where Codex reviewed them.
pub fn sweep(
    codex_home: &Path,
    claude_settings: Option<&Path>,
    codex_running: bool,
    mode: SidecarMode,
) -> SidecarReport {
    let config = codex_home.join("config.toml");
    let mut rep = SidecarReport {
        mode: mode.as_str(),
        codex_running,
        skipped_reason: None,
        files: Vec::new(),
        import_sync_was: None,
        import_sync_change: None,
        trust_rows_dropped: 0,
        errors: Vec::new(),
    };
    match settings_toml::import_sync_flag(&config) {
        Ok(v) => rep.import_sync_was = v,
        Err(e) => rep.errors.push(e.to_string()),
    }
    let live: Option<Map<String, Value>> = match claude_settings {
        None => None,
        Some(p) => match settings::read_root(p) {
            Ok(root) => Some(
                root.get("hooks")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            ),
            Err(e) => {
                rep.errors.push(format!(
                    "{} unreadable, foreign entries unverified: {e}",
                    p.display()
                ));
                None
            }
        },
    };

    if mode == SidecarMode::Install && codex_running {
        rep.skipped_reason = Some("codex_running");
        return rep;
    }

    let allow_foreign = mode != SidecarMode::Uninstall;
    let write = mode != SidecarMode::Status;
    for p in sidecar_paths(codex_home) {
        match sweep_json_hooks(&p, live.as_ref(), allow_foreign, write) {
            Ok(f) => rep.files.push(f),
            Err(e) => rep.errors.push(e.to_string()),
        }
    }

    if write {
        let any_changed = rep.files.iter().any(|f| f.change.wrote());
        let basenames: &[&str] = if any_changed {
            &[SIDECAR_BASENAME]
        } else {
            &[]
        };
        let flip = mode == SidecarMode::Install;
        match settings_toml::sweep_trust_and_sync(&config, basenames, flip) {
            Ok((dropped, sync_change, _bak)) => {
                rep.trust_rows_dropped = dropped;
                if flip {
                    rep.import_sync_change = Some(sync_change.as_str().to_string());
                }
            }
            Err(e) => rep.errors.push(e.to_string()),
        }
    }
    rep
}
