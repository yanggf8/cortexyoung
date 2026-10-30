//! The Codex sidecar sweep (`~/.codex/hooks.json`), judged against the machine that produced it:
//! an import-sync copy of `~/.claude/settings.json`'s hooks table sitting beside a `config.toml`
//! that carries both our entries and Codex's `[hooks.state]` trust rows for several files at once.
//! The predicate under test (`is_ours_for`) and the JSON plumbing (`read_root`/`write_root`) are
//! `settings.rs`'s, imported rather than reimplemented; these tests pin the sweep's own decisions —
//! what counts as a verified copy, what a mixed file keeps, what gets skipped and why.

use cort::settings::Change;
use cort::settings_sidecar::{sweep, SidecarMode};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::Builder::new()
        .prefix("cort-sidecar-")
        .tempdir()
        .unwrap();
    let home = d.path().to_path_buf();
    (d, home)
}

fn claude_settings_path(home: &Path) -> PathBuf {
    let p = home.join(".claude").join("settings.json");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

/// The live Claude Code hooks table, shaped like the real `~/.claude/settings.json` on the
/// machine this sweep was written for: one of ours per event, iterm2 groups around them, and an
/// event (`Stop`) that carries no cort entry at all.
fn claude_hooks() -> Value {
    json!({
        "PreToolUse": [
            {"matcher": "Bash", "hooks": [
                {"type": "command", "command": "/bin/cort hook-suggest --harness claude-code", "timeout": 5}
            ]},
            {"hooks": [{"type": "command", "command": "/Users/x/.config/iterm2/cc-status"}]}
        ],
        "PostToolUse": [
            {"matcher": "Bash|Edit|Write|MultiEdit|NotebookEdit", "hooks": [
                {"type": "command", "command": "/bin/cort hook-refresh --harness claude-code", "timeout": 5}
            ]},
            {"hooks": [{"type": "command", "command": "/Users/x/.config/iterm2/cc-status"}]}
        ],
        "Stop": [
            {"hooks": [{"type": "command", "command": "/Users/x/.config/iterm2/cc-status"}]}
        ]
    })
}

fn write_json(p: &Path, root: Value) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, serde_json::to_string_pretty(&root).unwrap()).unwrap();
}

/// The sidecar as the import-sync leaves it: the same table, plus a non-hooks key the sweep has
/// no business removing. The cort entries carry `--harness claude-code` because that is what a
/// copy of the Claude Code entries looks like — and what makes them double-fire in Codex.
fn write_sidecar(codex_home: &Path, hooks: Value) -> PathBuf {
    let p = codex_home.join("hooks.json");
    fs::create_dir_all(codex_home).unwrap();
    write_json(&p, json!({"other_setting": true, "hooks": hooks}));
    p
}

/// A plausible real `config.toml`: our two entries, the sync flag, and Codex's trust rows for
/// three different files at once — the sidecar, this config itself, and a plugin hook.
fn write_config(codex_home: &Path) -> PathBuf {
    let p = codex_home.join("config.toml");
    let sidecar = codex_home.join("hooks.json").display().to_string();
    let self_path = p.to_string_lossy();
    fs::write(
        &p,
        format!(
            r#"[desktop]
external-agent-import-sync-enabled = true
preventSleepWhileRunning = true

[[hooks.PreToolUse]]
matcher = "Bash"
[[hooks.PreToolUse.hooks]]
type = "command"
command = "/bin/cort hook-suggest --harness codex"
timeout = 5

[hooks.state."{sidecar}:pre_tool_use:0:0"]
trusted_hash = "sha256:sidecar-pre"

[hooks.state."{sidecar}:stop:0:0"]
trusted_hash = "sha256:sidecar-stop"

[hooks.state."{self_path}:pre_tool_use:0:0"]
trusted_hash = "sha256:ours-pre"

[hooks.state."{self_path}:post_tool_use:0:0"]
trusted_hash = "sha256:ours-post"

[hooks.state."browser@openai-bundled:plugin.json#hooks[0]:stop:0:0"]
trusted_hash = "sha256:plugin"
"#
        ),
    )
    .unwrap();
    p
}

fn config_doc(p: &Path) -> toml_edit::DocumentMut {
    fs::read_to_string(p)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap()
}

fn state_keys(doc: &toml_edit::DocumentMut) -> Vec<String> {
    doc["hooks"]["state"]
        .as_table()
        .map(|t| t.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn install_mode_sweeps_verified_copies_and_flips_the_sync() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"model": "x", "hooks": claude_hooks()}));
    let sidecar = write_sidecar(&codex_home, claude_hooks());
    let config = write_config(&codex_home);

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Install);

    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    assert_eq!(rep.skipped_reason, None);
    let f = &rep.files[0];
    assert_eq!(f.change, Change::Removed);
    assert_eq!(f.ours_removed, 2, "one cort entry per event");
    assert_eq!(f.foreign_removed, 3, "iterm2 x2 plus the Stop event");
    assert_eq!(f.foreign_left, 0);
    assert!(f.foreign_all_copies);
    assert!(f.backup.is_some(), "a swept file gets a .sweep backup");

    // The emptied sidecar keeps the keys that are not `hooks`, and loses the table itself.
    let after: Value = serde_json::from_str(&fs::read_to_string(&sidecar).unwrap()).unwrap();
    assert!(after.get("hooks").is_none());
    assert_eq!(after["other_setting"].as_bool(), Some(true));

    // config.toml: the sidecar's trust rows are gone, everyone else's are exactly where they were.
    assert_eq!(rep.trust_rows_dropped, 2);
    let doc = config_doc(&config);
    let keys = state_keys(&doc);
    assert_eq!(keys.len(), 3, "config.toml's own two plus the plugin row");
    assert!(
        keys.iter().all(|k| !k.contains("hooks.json")),
        "no sidecar row survives: {keys:?}"
    );
    assert!(keys.iter().any(|k| k.contains("plugin.json#hooks[0]")));
    assert_eq!(
        doc["desktop"]["external-agent-import-sync-enabled"].as_bool(),
        Some(false),
        "the sync is off, so the copies stay gone"
    );
    assert_eq!(
        doc["desktop"]["preventSleepWhileRunning"].as_bool(),
        Some(true),
        "sibling desktop keys survive"
    );
    assert_eq!(rep.import_sync_was, Some(true));
    assert_eq!(rep.import_sync_change.as_deref(), Some("updated"));
    // Our wired entry in config.toml is untouched by all of this.
    assert_eq!(
        doc["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
        Some("/bin/cort hook-suggest --harness codex")
    );
}

#[test]
fn a_codex_only_foreign_entry_holds_the_whole_foreign_batch_back() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    let sidecar = write_sidecar(
        &codex_home,
        json!({
            "PreToolUse": [
                {"matcher": "Bash", "hooks": [
                    {"type": "command", "command": "/bin/cort hook-suggest --harness claude-code", "timeout": 5}
                ]},
                {"hooks": [{"type": "command", "command": "/Users/x/.config/iterm2/cc-status"}]},
                {"matcher": "Read", "hooks": [
                    {"type": "command", "command": "codex-only-tool --watch"}
                ]}
            ]
        }),
    );
    write_config(&codex_home);

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Install);

    let f = &rep.files[0];
    assert!(
        !f.foreign_all_copies,
        "one unmatched entry vetoes the batch"
    );
    assert_eq!(f.ours_removed, 1);
    assert_eq!(
        f.foreign_left, 2,
        "the copy and the Codex-only entry both stay"
    );
    assert_eq!(f.foreign_removed, 0);
    assert!(f.change.wrote(), "ours still left, so the file was written");
    let after: Value = serde_json::from_str(&fs::read_to_string(&sidecar).unwrap()).unwrap();
    let groups = after["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "ours' group is gone, both foreign stay");
    // Ours left the file, so the sidecar's trust rows would point at shifted positions: they go.
    assert_eq!(rep.trust_rows_dropped, 2);
    assert_eq!(
        config_doc(&codex_home.join("config.toml"))["desktop"]
            ["external-agent-import-sync-enabled"]
            .as_bool(),
        Some(false),
        "install mode flips the sync regardless of how much the sweep removed"
    );
}

#[test]
fn no_live_claude_settings_means_no_foreign_entry_can_be_proven_a_copy() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let sidecar = write_sidecar(&codex_home, claude_hooks());

    let rep = sweep(&codex_home, None, false, SidecarMode::Install);

    let f = &rep.files[0];
    assert!(!f.foreign_all_copies);
    assert_eq!(f.ours_removed, 2);
    assert_eq!(f.foreign_left, 3);
    let after: Value = serde_json::from_str(&fs::read_to_string(&sidecar).unwrap()).unwrap();
    assert_eq!(
        after["hooks"]["Stop"][0]["hooks"][0]["command"],
        json!("/Users/x/.config/iterm2/cc-status"),
        "unprovable foreign entries are not the sweep's to take"
    );
}

#[test]
fn install_mode_skips_everything_while_codex_is_running() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    let sidecar = write_sidecar(&codex_home, claude_hooks());
    let config = write_config(&codex_home);
    let sidecar_before = fs::read_to_string(&sidecar).unwrap();
    let config_before = fs::read_to_string(&config).unwrap();

    let rep = sweep(&codex_home, Some(&claude), true, SidecarMode::Install);

    assert_eq!(rep.skipped_reason, Some("codex_running"));
    assert!(rep.files.is_empty(), "no file was even classified");
    assert_eq!(
        rep.import_sync_was,
        Some(true),
        "the flag is still reported, read-only"
    );
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), sidecar_before);
    assert_eq!(fs::read_to_string(&config).unwrap(), config_before);
}

#[test]
fn uninstall_mode_takes_ours_and_never_touches_the_sync() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    let sidecar = write_sidecar(&codex_home, claude_hooks());
    let config = write_config(&codex_home);

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Uninstall);

    let f = &rep.files[0];
    assert_eq!(f.ours_removed, 2);
    assert_eq!(f.foreign_removed, 0, "uninstall has no foreign mandate");
    assert_eq!(f.foreign_left, 3);
    let after: Value = serde_json::from_str(&fs::read_to_string(&sidecar).unwrap()).unwrap();
    assert_eq!(
        after["hooks"]["Stop"][0]["hooks"][0]["command"],
        json!("/Users/x/.config/iterm2/cc-status")
    );
    assert_eq!(
        rep.import_sync_change, None,
        "uninstall does not flip the flag"
    );
    assert_eq!(
        config_doc(&config)["desktop"]["external-agent-import-sync-enabled"].as_bool(),
        Some(true)
    );
    // Ours left a mixed file, so the sidecar's own trust rows go with it — but nobody else's.
    assert_eq!(rep.trust_rows_dropped, 2);
}

#[test]
fn status_mode_reports_what_a_sweep_would_do_and_writes_nothing() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    let sidecar = write_sidecar(&codex_home, claude_hooks());
    let config = write_config(&codex_home);
    let sidecar_before = fs::read_to_string(&sidecar).unwrap();
    let config_before = fs::read_to_string(&config).unwrap();

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Status);

    let f = &rep.files[0];
    assert_eq!(f.change, Change::NotPresent);
    assert_eq!(f.ours_removed, 2, "what a sweep would remove");
    assert_eq!(f.foreign_removed, 3);
    assert!(f.foreign_all_copies);
    assert!(f.backup.is_none());
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), sidecar_before);
    assert_eq!(fs::read_to_string(&config).unwrap(), config_before);
    assert_eq!(rep.trust_rows_dropped, 0);
    assert_eq!(rep.import_sync_change, None);
}

#[test]
fn a_second_sweep_of_a_swept_machine_is_a_clean_no_op() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    write_sidecar(&codex_home, claude_hooks());
    write_config(&codex_home);

    sweep(&codex_home, Some(&claude), false, SidecarMode::Install);
    let config = codex_home.join("config.toml");
    let after_first = fs::read_to_string(&config).unwrap();
    let backups = fs::read_dir(&codex_home)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
        .count();

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Install);

    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    let f = &rep.files[0];
    assert_eq!(
        f.change,
        Change::NotPresent,
        "an emptied sidecar is a no-op"
    );
    assert_eq!(rep.trust_rows_dropped, 0, "nothing changed, no rows move");
    assert_eq!(
        rep.import_sync_change.as_deref(),
        Some("already_present"),
        "the flag is already off, and the file is not rewritten for it"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), after_first);
    let backups_after = fs::read_dir(&codex_home)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
        .count();
    assert_eq!(backups_after, backups, "a no-op writes no backup");
}

#[test]
fn an_empty_state_table_leaves_no_header_behind() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    write_sidecar(&codex_home, claude_hooks());
    // A config whose ONLY trust rows are the sidecar's: after the sweep the state table is
    // empty, and a header with nothing under it is scaffolding, not state.
    let config = codex_home.join("config.toml");
    let sidecar_key = codex_home.join("hooks.json").display().to_string();
    fs::write(
        &config,
        format!(
            "[desktop]\nexternal-agent-import-sync-enabled = true\n\n[hooks.state.\"{sidecar_key}:pre_tool_use:0:0\"]\ntrusted_hash = \"sha256:x\"\n"
        ),
    )
    .unwrap();

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Install);

    assert_eq!(rep.trust_rows_dropped, 1);
    let rendered = fs::read_to_string(&config).unwrap();
    assert!(!rendered.contains("[hooks.state"), "{rendered}");
    assert!(!rendered.contains("[hooks]"), "{rendered}");
    assert!(
        rendered.contains("[desktop]"),
        "everything else stays exactly where it was"
    );
}

#[test]
fn a_sidecar_without_a_hooks_table_is_a_no_op_not_an_error() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let sidecar = codex_home.join("hooks.json");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(&sidecar, "{}\n").unwrap();

    let rep = sweep(&codex_home, None, false, SidecarMode::Install);

    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    assert_eq!(rep.files[0].change, Change::NotPresent);
    assert!(rep.files[0].existed);
}

#[test]
fn a_malformed_hooks_value_refuses_instead_of_being_repaired() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let sidecar = codex_home.join("hooks.json");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(&sidecar, r#"{"hooks": "not a table"}"#).unwrap();

    let rep = sweep(&codex_home, None, false, SidecarMode::Install);

    assert_eq!(rep.errors.len(), 1, "collected, not fatal");
    assert!(fs::read_to_string(&sidecar)
        .unwrap()
        .contains("not a table"));
}

/// Our entries are recognised under the RIGHT event only: a `hook-refresh` copy in PreToolUse is
/// not `is_ours_for(_, Suggest)`, and the sweep must not take it as ours on the strength of the
/// verb alone — it is foreign here, and foreign needs a copy signature it does not have.
#[test]
fn ours_are_matched_per_event() {
    let (_d, home) = tmp();
    let codex_home = home.join(".codex");
    let claude = claude_settings_path(&home);
    write_json(&claude, json!({"hooks": claude_hooks()}));
    write_sidecar(
        &codex_home,
        json!({
            "PreToolUse": [
                {"matcher": "Bash", "hooks": [
                    {"type": "command", "command": "/bin/cort hook-refresh --harness codex", "timeout": 5}
                ]}
            ]
        }),
    );

    let rep = sweep(&codex_home, Some(&claude), false, SidecarMode::Install);

    let f = &rep.files[0];
    assert_eq!(
        f.ours_removed, 0,
        "hook-refresh is not ours under PreToolUse"
    );
    assert_eq!(f.foreign_left, 1);
    assert!(!f.change.wrote(), "nothing recognisable was removed");
}
