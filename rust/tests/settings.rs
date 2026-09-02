//! The settings merge, judged against the shape a real Claude Code settings.json has: other
//! people's hooks already in `PreToolUse`, and a file the installer must never make unreadable.

use cort::settings::{install_hook, installed_command, remove_hook, Change};
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
    let out = install_hook(&p, "/bin/cort hook-suggest").unwrap();
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
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
    let before = fs::read_to_string(&p).unwrap();
    let out = install_hook(&p, "/bin/cort hook-suggest").unwrap();
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
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
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
    install_hook(&p, "/home/u/.cargo/bin/cort hook-suggest").unwrap();
    let out = install_hook(&p, "/home/u/.local/bin/cort hook-suggest").unwrap();
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
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
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
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
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
    let err = install_hook(&p, "/bin/cort hook-suggest").unwrap_err();
    assert!(format!("{err}").contains("not valid JSON"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), "{ this is not json");
}

#[test]
fn check_can_report_the_wired_command_without_touching_the_file() {
    let (_d, p) = tmp();
    assert!(installed_command(&p).is_none());
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
    assert_eq!(installed_command(&p).unwrap(), "/bin/cort hook-suggest");
    remove_hook(&p).unwrap();
    assert!(installed_command(&p).is_none());
}
