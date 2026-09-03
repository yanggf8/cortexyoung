//! The settings merge, judged against the shape a real Claude Code settings.json has: other
//! people's hooks already in `PreToolUse`, and a file the installer must never make unreadable.

use cort::settings::{install_hook, installed_command, remove_hook, Change, HookEvent};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::Builder::new()
        .prefix("cort-settings-")
        .tempdir()
        .unwrap();
    let p = d.path().join("settings.json");
    (d, p)
}

fn read(p: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

/// The observed shape on this machine: an unmatched group running `mos hook`.
fn with_existing_hooks(p: &PathBuf) {
    fs::write(
        p,
        serde_json::to_string_pretty(&json!({
            "permissions": { "allow": ["Bash(git status)"] },
            "hooks": {
                "SessionStart": [{"hooks": [{"type": "command", "command": "mos hook"}]}],
                "PreToolUse": [{"hooks": [{"type": "command", "command": "mos hook", "timeout": 5}]}],
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn installs_into_a_file_that_does_not_exist_yet() {
    let (_d, p) = tmp();
    let out = install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    assert_eq!(out.change, Change::Installed);
    assert!(out.backup.is_none(), "nothing to back up");
    let v = read(&p);
    let group = &v["hooks"]["PreToolUse"][0];
    assert_eq!(group["matcher"], "Bash");
    assert_eq!(group["hooks"][0]["command"], "/bin/cort hook-suggest");
    assert_eq!(group["hooks"][0]["type"], "command");
}

#[test]
fn a_second_install_changes_nothing_and_does_not_rewrite() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let before = fs::read_to_string(&p).unwrap();
    let out = install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    assert_eq!(out.change, Change::AlreadyPresent);
    assert!(!out.change.wrote());
    assert_eq!(fs::read_to_string(&p).unwrap(), before);
    // and exactly one entry, not two
    assert_eq!(read(&p)["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
}

#[test]
fn every_hook_the_user_already_had_survives() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let v = read(&p);
    assert_eq!(v["permissions"]["allow"][0], "Bash(git status)");
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "mos hook"
    );
    let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2, "ours is added beside theirs, not over it");
    assert_eq!(pre[0]["hooks"][0]["command"], "mos hook");
    assert_eq!(pre[1]["hooks"][0]["command"], "/bin/cort hook-suggest");
}

#[test]
fn a_moved_binary_updates_the_entry_instead_of_adding_a_second_one() {
    let (_d, p) = tmp();
    install_hook(
        &p,
        "/home/u/.cargo/bin/cort hook-suggest",
        HookEvent::Suggest,
    )
    .unwrap();
    let out = install_hook(
        &p,
        "/home/u/.local/bin/cort hook-suggest",
        HookEvent::Suggest,
    )
    .unwrap();
    assert_eq!(out.change, Change::Updated);
    assert!(
        out.backup.is_some(),
        "a rewrite of an existing file is backed up"
    );
    let pre = read(&p)["hooks"]["PreToolUse"].as_array().unwrap().clone();
    assert_eq!(pre.len(), 1);
    assert_eq!(
        pre[0]["hooks"][0]["command"],
        "/home/u/.local/bin/cort hook-suggest"
    );
}

#[test]
fn remove_takes_ours_out_and_leaves_theirs() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let out = remove_hook(&p).unwrap();
    assert_eq!(out.change, Change::Removed);
    let v = read(&p);
    let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 1);
    assert_eq!(pre[0]["hooks"][0]["command"], "mos hook");
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "mos hook"
    );
}

#[test]
fn uninstalling_a_hook_we_never_installed_writes_nothing() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    let before = fs::read_to_string(&p).unwrap();
    let out = remove_hook(&p).unwrap();
    assert_eq!(out.change, Change::NotPresent);
    assert_eq!(fs::read_to_string(&p).unwrap(), before);
}

#[test]
fn remove_leaves_no_empty_scaffolding_in_a_file_that_had_no_hooks() {
    let (_d, p) = tmp();
    fs::write(&p, r#"{"permissions":{"allow":[]}}"#).unwrap();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    remove_hook(&p).unwrap();
    let v = read(&p);
    assert!(
        v.get("hooks").is_none(),
        "the key we added is the key we remove: {v}"
    );
    assert!(v.get("permissions").is_some());
}

#[test]
fn a_settings_file_we_cannot_parse_is_refused_not_overwritten() {
    let (_d, p) = tmp();
    fs::write(&p, "{ this is not json").unwrap();
    let err = install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap_err();
    assert!(format!("{err}").contains("not valid JSON"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), "{ this is not json");
}

#[test]
fn check_can_report_the_wired_command_without_touching_the_file() {
    let (_d, p) = tmp();
    assert!(installed_command(&p, HookEvent::Suggest).is_none());
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    assert_eq!(
        installed_command(&p, HookEvent::Suggest).unwrap(),
        "/bin/cort hook-suggest"
    );
    remove_hook(&p).unwrap();
    assert!(installed_command(&p, HookEvent::Suggest).is_none());
}

/// The real `~/.claude/settings.json` on 2026-09-02: two hand-wired entries in one Bash group, each
/// ending in `2>/dev/null || true` and each narrowed by its own `if`. Anchoring recognition to the
/// end of the command line made all of this invisible.
fn with_hand_wired_duplicates(p: &PathBuf) {
    fs::write(
        p,
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [
                        {"type": "command", "command": "$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true", "if": "Bash(grep:*)", "timeout": 10},
                        {"type": "command", "command": "$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true", "if": "Bash(rg:*)", "timeout": 10},
                    ],
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn a_command_with_a_redirection_suffix_is_still_ours() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    // The bug this pins: `--status` answered `wired: false` on a machine whose hook was firing.
    assert_eq!(
        installed_command(&p, HookEvent::Suggest).unwrap(),
        "$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true"
    );
}

#[test]
fn a_redeploy_collapses_hand_wired_duplicates_to_one_entry() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    let out = install_hook(
        &p,
        "/home/u/.cargo/bin/cort hook-suggest",
        HookEvent::Suggest,
    )
    .unwrap();
    assert_eq!(out.change, Change::Updated);
    let v = read(&p);
    let hooks = v["hooks"]["PreToolUse"][0]["hooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 1, "two copies fire the hook twice: {v}");
    assert_eq!(hooks[0]["command"], "/home/u/.cargo/bin/cort hook-suggest");
    assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
}

#[test]
fn the_surviving_entry_keeps_no_hand_typed_condition() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    install_hook(
        &p,
        "/home/u/.cargo/bin/cort hook-suggest",
        HookEvent::Suggest,
    )
    .unwrap();
    let v = read(&p);
    let entry = &v["hooks"]["PreToolUse"][0]["hooks"][0];
    // An `if: Bash(grep:*)` left in place means the hook covers grep alone while the installer
    // reports it wired for Bash -- less coverage than anyone reading the installer would predict.
    assert!(
        entry.get("if").is_none(),
        "stale condition survived: {entry}"
    );
    assert_eq!(entry["type"], "command");
    assert_eq!(entry["timeout"], 5);
}

#[test]
fn collapsing_duplicates_is_idempotent() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    install_hook(
        &p,
        "/home/u/.cargo/bin/cort hook-suggest",
        HookEvent::Suggest,
    )
    .unwrap();
    let out = install_hook(
        &p,
        "/home/u/.cargo/bin/cort hook-suggest",
        HookEvent::Suggest,
    )
    .unwrap();
    assert_eq!(out.change, Change::AlreadyPresent);
    assert_eq!(out.backup, None);
}

#[test]
fn remove_takes_out_a_hand_wired_entry_too() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    let out = remove_hook(&p).unwrap();
    assert_eq!(out.change, Change::Removed);
    assert!(installed_command(&p, HookEvent::Suggest).is_none());
    let v = read(&p);
    assert!(
        v.get("hooks").is_none(),
        "empty scaffolding left behind: {v}"
    );
}

#[test]
fn a_malformed_group_does_not_hide_the_entry_behind_it() {
    let (_d, p) = tmp();
    fs::write(
        p.as_path(),
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash"},
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "/bin/cort hook-suggest"}]},
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    // A `?` here used to end the whole scan on the first group, reporting not-wired.
    assert_eq!(
        installed_command(&p, HookEvent::Suggest).unwrap(),
        "/bin/cort hook-suggest"
    );
}

#[test]
fn a_command_merely_mentioning_the_word_is_not_ours() {
    let (_d, p) = tmp();
    fs::write(
        p.as_path(),
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [{"hooks": [{"type": "command", "command": "echo hook-suggest >> /tmp/log"}]}]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(installed_command(&p, HookEvent::Suggest).is_none());
    remove_hook(&p).unwrap();
    let v = read(&p);
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "echo hook-suggest >> /tmp/log",
        "somebody else's hook was removed"
    );
}

/// The predicate is the whole of the "every hook you already have survives" promise. A vendor's
/// binary that happens to be named `hook-suggest` is not our subcommand, and claiming it is not a
/// label -- install rewrites the entry it claims and remove deletes it. The `echo hook-suggest`
/// case above guards a different seam: this one is a real path in the command position.
#[test]
fn a_third_party_binary_named_hook_suggest_is_not_ours() {
    let (_d, p) = tmp();
    let theirs = json!({"command": "/opt/vendor/bin/hook-suggest --daemon", "timeout": 30});
    fs::write(
        p.as_path(),
        serde_json::to_string_pretty(&json!({
            "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [theirs]}]}
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        installed_command(&p, HookEvent::Suggest).is_none(),
        "claimed a vendor binary"
    );

    install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap();
    let v = read(&p);
    let hooks = v["hooks"]["PreToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap().iter())
        .collect::<Vec<_>>();
    assert!(
        hooks
            .iter()
            .any(|h| h["command"] == "/opt/vendor/bin/hook-suggest --daemon" && h["timeout"] == 30),
        "the vendor's hook was rewritten: {v}"
    );
    assert!(
        hooks.iter().any(|h| h["command"] == "/x/cort hook-suggest"),
        "ours was not added: {v}"
    );

    remove_hook(&p).unwrap();
    let v = read(&p);
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "/opt/vendor/bin/hook-suggest --daemon",
        "the vendor's hook was deleted by our uninstall: {v}"
    );
}

/// `SettingsError::Unparsable` exists because overwriting a file we failed to understand is the one
/// unrecoverable thing this module can do. A `hooks` of the wrong type is that same file, one level
/// in -- it used to be replaced wholesale, and the user had no reason to go looking.
#[test]
fn a_hooks_key_of_the_wrong_type_is_refused_not_replaced() {
    let (_d, p) = tmp();
    let before = serde_json::to_string_pretty(&json!({
        "hooks": "oops-user-data",
        "permissions": {"allow": ["Bash(git status)"]}
    }))
    .unwrap();
    fs::write(p.as_path(), &before).unwrap();
    let err = install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap_err();
    assert!(format!("{err}").contains("not an object"), "{err}");
    assert_eq!(
        fs::read_to_string(&p).unwrap(),
        before,
        "the file was written"
    );
}

#[test]
fn a_pretooluse_of_the_wrong_type_is_refused_not_replaced() {
    let (_d, p) = tmp();
    let before = serde_json::to_string_pretty(&json!({
        "hooks": {"PreToolUse": {"not": "an array"}}
    }))
    .unwrap();
    fs::write(p.as_path(), &before).unwrap();
    let err = install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap_err();
    assert!(format!("{err}").contains("not an array"), "{err}");
    assert_eq!(
        fs::read_to_string(&p).unwrap(),
        before,
        "the file was written"
    );
}

/// Collapsing our own duplicates is not licence to tidy the file. An empty group the user keeps for
/// their own reasons used to vanish, because the sweep was "remove every empty group" gated on a
/// single flag rather than on which groups this call actually emptied.
#[test]
fn the_users_own_empty_group_survives_a_collapse() {
    let (_d, p) = tmp();
    fs::write(
        p.as_path(),
        serde_json::to_string_pretty(&json!({
            "hooks": {"PreToolUse": [
                {"matcher": "Read", "hooks": []},
                {"matcher": "Bash", "hooks": [{"command": "/a/cort hook-suggest || true"}]},
                {"matcher": "Bash", "hooks": [{"command": "/a/cort hook-suggest || true"}]},
            ]}
        }))
        .unwrap(),
    )
    .unwrap();
    install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap();
    let pre = read(&p)["hooks"]["PreToolUse"].as_array().unwrap().clone();
    assert_eq!(
        pre.len(),
        2,
        "expected the Read group plus one of ours: {pre:?}"
    );
    assert_eq!(pre[0]["matcher"], "Read");
    assert!(pre[0]["hooks"].as_array().unwrap().is_empty());
}

#[test]
fn the_users_own_empty_group_survives_a_remove() {
    let (_d, p) = tmp();
    fs::write(
        p.as_path(),
        serde_json::to_string_pretty(&json!({
            "hooks": {"PreToolUse": [
                {"matcher": "Read", "hooks": []},
                {"matcher": "Bash", "hooks": [{"command": "/a/cort hook-suggest"}]},
            ]}
        }))
        .unwrap(),
    )
    .unwrap();
    remove_hook(&p).unwrap();
    let v = read(&p);
    let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 1, "the user's empty group went with ours: {v}");
    assert_eq!(pre[0]["matcher"], "Read");
}

/// The refresh hook watches the shell too, not only the edit tools.
///
/// An agent does not only edit through `Edit`/`Write`: a `python3 - <<PY` heredoc, a `sed -i`, a
/// `git checkout` all change the tree and none of them is an edit tool. Measured on this repo on
/// 2026-09-03 -- a session that wrote most of its changes through shell went two hours and three
/// commits without firing this hook once, and the index sat at the head it held when the last
/// `Write` happened, reporting `stale` on answers that were in fact correct and would have reported
/// fresh on answers that were not.
///
/// The negative half matters as much: the *pre* event must stay narrow. `hook-suggest` reads
/// `tool_input.command` and has nothing to say about an `Edit`, so widening both would put a hook on
/// every tool call in the session to return `{}`.
#[test]
fn the_refresh_event_watches_the_shell_and_the_suggest_event_does_not() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh).unwrap();
    let v = read(&p);

    let post = v["hooks"]["PostToolUse"][0]["matcher"].as_str().unwrap();
    for tool in ["Bash", "Edit", "Write", "MultiEdit", "NotebookEdit"] {
        assert!(
            post.contains(tool),
            "the refresh matcher must cover {tool}: {post:?}"
        );
    }

    let pre = v["hooks"]["PreToolUse"][0]["matcher"].as_str().unwrap();
    assert_eq!(
        pre, "Bash",
        "the suggest event reads a command line and must not fire on the edit tools"
    );
}

/// A redeploy repairs a stale matcher on the JSON side too.
///
/// The matcher lives on the group and `canonical` only ever describes the entry, so an entry whose
/// command was already current reported `already_present` with the matcher never examined -- on
/// every redeploy, forever. Reproduced on 2026-09-03 by widening `EDIT_MATCHER`: Codex and Kimi
/// picked it up and Claude Code silently did not. `settings_toml` had been given this repair hours
/// earlier and this module was not; `settings_kimi` never needed it, because its shape is flat and
/// `matcher` is a field of the entry its `is_canonical` already compares.
#[test]
fn a_redeploy_repairs_a_stale_matcher_even_when_the_command_is_unchanged() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        serde_json::to_string_pretty(&serde_json::json!({
            "hooks": { "PostToolUse": [ {
                "matcher": "Edit|Write",
                "hooks": [ { "type": "command", "command": "/bin/cort hook-refresh", "timeout": 5 } ]
            } ] }
        }))
        .unwrap(),
    )
    .unwrap();

    let out = install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh).unwrap();
    assert_eq!(
        out.change,
        Change::Updated,
        "an unchanged command must not report already_present while the matcher is stale"
    );
    let v = read(&p);
    assert_eq!(
        v["hooks"]["PostToolUse"][0]["matcher"].as_str(),
        Some("Bash|Edit|Write|MultiEdit|NotebookEdit")
    );
    assert_eq!(
        v["hooks"]["PostToolUse"].as_array().unwrap().len(),
        1,
        "repaired in place, not added beside"
    );

    // Converged: the next run has nothing left to do.
    assert_eq!(
        install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh)
            .unwrap()
            .change,
        Change::AlreadyPresent
    );
}

/// A matcher on a group we share is somebody else's routing, and we do not re-aim it.
#[test]
fn a_shared_groups_matcher_is_left_alone_on_the_json_side() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        serde_json::to_string_pretty(&serde_json::json!({
            "hooks": { "PostToolUse": [ {
                "matcher": "SomebodyElsesMatcher",
                "hooks": [
                    { "type": "command", "command": "mos hook" },
                    { "type": "command", "command": "/bin/cort hook-refresh", "timeout": 5 }
                ]
            } ] }
        }))
        .unwrap(),
    )
    .unwrap();

    install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh).unwrap();
    let v = read(&p);
    assert_eq!(
        v["hooks"]["PostToolUse"][0]["matcher"].as_str(),
        Some("SomebodyElsesMatcher"),
        "rewriting a shared group's matcher would silently re-aim the other owner's hook"
    );
    assert_eq!(
        v["hooks"]["PostToolUse"][0]["hooks"][0]["command"].as_str(),
        Some("mos hook"),
        "their entry survives untouched"
    );
}
