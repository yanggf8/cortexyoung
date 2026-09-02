//! The Codex `config.toml` merge, judged against the same shapes `rust/tests/settings.rs` judges
//! the JSON side against: other things already in `hooks.PreToolUse`, and a file the installer must
//! never make unreadable. The predicate under test (`is_ours`) is the JSON module's, imported rather
//! than reimplemented -- these tests exist to pin the TOML-specific plumbing around it (the nested
//! array-of-tables navigation, refusal rules, and empty-scaffolding cleanup), not to re-litigate the
//! token-matching rule itself.

use cort::settings::Change;
use cort::settings_toml::{install_hook, installed_command, remove_hook};
use std::fs;
use std::path::PathBuf;
use toml_edit::DocumentMut;

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::Builder::new()
        .prefix("cort-settings-toml-")
        .tempdir()
        .unwrap();
    let p = d.path().join("config.toml");
    (d, p)
}

fn read(p: &PathBuf) -> DocumentMut {
    fs::read_to_string(p).unwrap().parse::<DocumentMut>().unwrap()
}

fn cmd_at(doc: &DocumentMut, group: usize, hook: usize) -> String {
    doc["hooks"]["PreToolUse"][group]["hooks"][hook]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Handles both shapes an empty `hooks` can legally have on disk: `[]` (the only way TOML can
/// literally spell "zero of them" -- an array-of-tables with no elements produces no header at all,
/// so it is indistinguishable from absent) and a real array of tables with entries in it.
fn group_hook_count(doc: &DocumentMut, group: usize) -> usize {
    let item = &doc["hooks"]["PreToolUse"][group]["hooks"];
    if let Some(a) = item.as_array_of_tables() {
        a.len()
    } else if let Some(a) = item.as_array() {
        a.len()
    } else {
        0
    }
}

fn group_count(doc: &DocumentMut) -> usize {
    doc["hooks"]["PreToolUse"].as_array_of_tables().unwrap().len()
}

/// A plausible real `config.toml`: an ordinary top-level key, an unrelated hook event
/// (`SessionStart`), and a `PreToolUse` group running something that is not ours.
fn with_existing_hooks(p: &PathBuf) {
    fs::write(
        p,
        r#"
approvals_reviewer = "user"

[[hooks.SessionStart]]
[[hooks.SessionStart.hooks]]
type = "command"
command = "mos hook"

[[hooks.PreToolUse]]
[[hooks.PreToolUse.hooks]]
type = "command"
command = "mos hook"
timeout = 5
"#,
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
    assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"].as_str(), Some("Bash"));
    assert_eq!(cmd_at(&v, 0, 0), "/bin/cort hook-suggest");
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["type"].as_str(),
        Some("command")
    );
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
    assert_eq!(group_count(&read(&p)), 1);
}

#[test]
fn every_hook_the_user_already_had_survives() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
    let v = read(&p);
    assert_eq!(v["approvals_reviewer"].as_str(), Some("user"));
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str(),
        Some("mos hook")
    );
    assert_eq!(group_count(&v), 2, "ours is added beside theirs, not over it");
    assert_eq!(cmd_at(&v, 0, 0), "mos hook");
    assert_eq!(cmd_at(&v, 1, 0), "/bin/cort hook-suggest");
}

#[test]
fn a_moved_binary_updates_the_entry_instead_of_adding_a_second_one() {
    let (_d, p) = tmp();
    install_hook(&p, "/home/u/.cargo/bin/cort hook-suggest").unwrap();
    let out = install_hook(&p, "/home/u/.local/bin/cort hook-suggest").unwrap();
    assert_eq!(out.change, Change::Updated);
    assert!(out.backup.is_some(), "a rewrite of an existing file is backed up");
    let v = read(&p);
    assert_eq!(group_count(&v), 1);
    assert_eq!(cmd_at(&v, 0, 0), "/home/u/.local/bin/cort hook-suggest");
}

#[test]
fn remove_takes_ours_out_and_leaves_theirs() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
    let out = remove_hook(&p).unwrap();
    assert_eq!(out.change, Change::Removed);
    let v = read(&p);
    assert_eq!(group_count(&v), 1);
    assert_eq!(cmd_at(&v, 0, 0), "mos hook");
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str(),
        Some("mos hook")
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
    fs::write(&p, "approvals_reviewer = \"user\"\n").unwrap();
    install_hook(&p, "/bin/cort hook-suggest").unwrap();
    remove_hook(&p).unwrap();
    let v = read(&p);
    assert!(
        v.get("hooks").is_none(),
        "the key we added is the key we remove: {v}"
    );
    assert_eq!(v["approvals_reviewer"].as_str(), Some("user"));
}

#[test]
fn a_config_file_we_cannot_parse_is_refused_not_overwritten() {
    let (_d, p) = tmp();
    fs::write(&p, "this = [ is not valid toml").unwrap();
    let err = install_hook(&p, "/bin/cort hook-suggest").unwrap_err();
    assert!(format!("{err}").contains("not valid TOML"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), "this = [ is not valid toml");
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

/// The JSON side's real failure mode, reproduced on config.toml: two hand-wired entries in one
/// group, each carrying a field the installer's own shape does not write.
fn with_hand_wired_duplicates(p: &PathBuf) {
    fs::write(
        p,
        r#"
[[hooks.PreToolUse]]
matcher = "Bash"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true"
timeout = 10
async = true

[[hooks.PreToolUse.hooks]]
type = "command"
command = "$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true"
timeout = 10
async = true
"#,
    )
    .unwrap();
}

#[test]
fn a_command_with_a_redirection_suffix_is_still_ours() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    assert_eq!(
        installed_command(&p).unwrap(),
        "$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true"
    );
}

#[test]
fn a_redeploy_collapses_hand_wired_duplicates_to_one_entry() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    let out = install_hook(&p, "/home/u/.cargo/bin/cort hook-suggest").unwrap();
    assert_eq!(out.change, Change::Updated);
    let v = read(&p);
    assert_eq!(group_hook_count(&v, 0), 1, "two copies fire the hook twice");
    assert_eq!(cmd_at(&v, 0, 0), "/home/u/.cargo/bin/cort hook-suggest");
    assert_eq!(group_count(&v), 1);
}

#[test]
fn the_surviving_entry_keeps_no_hand_typed_field() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    install_hook(&p, "/home/u/.cargo/bin/cort hook-suggest").unwrap();
    let v = read(&p);
    let entry = &v["hooks"]["PreToolUse"][0]["hooks"][0];
    assert!(entry.get("async").is_none(), "stale field survived: {entry}");
    assert_eq!(entry["type"].as_str(), Some("command"));
    assert_eq!(entry["timeout"].as_integer(), Some(5));
}

#[test]
fn collapsing_duplicates_is_idempotent() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    install_hook(&p, "/home/u/.cargo/bin/cort hook-suggest").unwrap();
    let out = install_hook(&p, "/home/u/.cargo/bin/cort hook-suggest").unwrap();
    assert_eq!(out.change, Change::AlreadyPresent);
    assert_eq!(out.backup, None);
}

#[test]
fn remove_takes_out_a_hand_wired_entry_too() {
    let (_d, p) = tmp();
    with_hand_wired_duplicates(&p);
    let out = remove_hook(&p).unwrap();
    assert_eq!(out.change, Change::Removed);
    assert!(installed_command(&p).is_none());
    let v = read(&p);
    assert!(v.get("hooks").is_none(), "empty scaffolding left behind: {v}");
}

#[test]
fn a_malformed_group_does_not_hide_the_entry_behind_it() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        r#"
[[hooks.PreToolUse]]
matcher = "Bash"

[[hooks.PreToolUse]]
matcher = "Bash"
[[hooks.PreToolUse.hooks]]
type = "command"
command = "/bin/cort hook-suggest"
"#,
    )
    .unwrap();
    // A `continue`-worthy group (no `hooks` array) must not end the scan before the real entry.
    assert_eq!(installed_command(&p).unwrap(), "/bin/cort hook-suggest");
}

#[test]
fn a_command_merely_mentioning_the_word_is_not_ours() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        r#"
[[hooks.PreToolUse]]
[[hooks.PreToolUse.hooks]]
type = "command"
command = "echo hook-suggest >> /tmp/log"
"#,
    )
    .unwrap();
    assert!(installed_command(&p).is_none());
    remove_hook(&p).unwrap();
    let v = read(&p);
    assert_eq!(
        cmd_at(&v, 0, 0),
        "echo hook-suggest >> /tmp/log",
        "somebody else's hook was removed"
    );
}

#[test]
fn a_third_party_binary_named_hook_suggest_is_not_ours() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        r#"
[[hooks.PreToolUse]]
matcher = "Bash"
[[hooks.PreToolUse.hooks]]
command = "/opt/vendor/bin/hook-suggest --daemon"
timeout = 30
"#,
    )
    .unwrap();
    assert!(installed_command(&p).is_none(), "claimed a vendor binary");

    install_hook(&p, "/x/cort hook-suggest").unwrap();
    let v = read(&p);
    let all: Vec<String> = (0..group_count(&v))
        .flat_map(|g| (0..group_hook_count(&v, g)).map(move |h| (g, h)))
        .map(|(g, h)| cmd_at(&v, g, h))
        .collect();
    assert!(
        all.iter().any(|c| c == "/opt/vendor/bin/hook-suggest --daemon"),
        "the vendor's hook was rewritten: {v}"
    );
    assert!(
        all.iter().any(|c| c == "/x/cort hook-suggest"),
        "ours was not added: {v}"
    );

    remove_hook(&p).unwrap();
    let v = read(&p);
    assert_eq!(
        cmd_at(&v, 0, 0),
        "/opt/vendor/bin/hook-suggest --daemon",
        "the vendor's hook was deleted by our uninstall: {v}"
    );
}

#[test]
fn a_hooks_key_of_the_wrong_type_is_refused_not_replaced() {
    let (_d, p) = tmp();
    let before = "hooks = \"oops-user-data\"\napprovals_reviewer = \"user\"\n";
    fs::write(&p, before).unwrap();
    let err = install_hook(&p, "/x/cort hook-suggest").unwrap_err();
    assert!(format!("{err}").contains("not a table"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), before);
}

#[test]
fn a_pretooluse_of_the_wrong_type_is_refused_not_replaced() {
    let (_d, p) = tmp();
    let before = "[hooks]\nPreToolUse = \"not an array\"\n";
    fs::write(&p, before).unwrap();
    let err = install_hook(&p, "/x/cort hook-suggest").unwrap_err();
    assert!(format!("{err}").contains("not an array of tables"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), before);
}

#[test]
fn the_users_own_empty_group_survives_a_collapse() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        r#"
[[hooks.PreToolUse]]
matcher = "Read"
hooks = []

[[hooks.PreToolUse]]
matcher = "Bash"
[[hooks.PreToolUse.hooks]]
command = "/a/cort hook-suggest || true"

[[hooks.PreToolUse]]
matcher = "Bash"
[[hooks.PreToolUse.hooks]]
command = "/a/cort hook-suggest || true"
"#,
    )
    .unwrap();
    install_hook(&p, "/x/cort hook-suggest").unwrap();
    let v = read(&p);
    assert_eq!(group_count(&v), 2, "expected the Read group plus one of ours");
    assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"].as_str(), Some("Read"));
    assert_eq!(group_hook_count(&v, 0), 0);
}

#[test]
fn the_users_own_empty_group_survives_a_remove() {
    let (_d, p) = tmp();
    fs::write(
        &p,
        r#"
[[hooks.PreToolUse]]
matcher = "Read"
hooks = []

[[hooks.PreToolUse]]
matcher = "Bash"
[[hooks.PreToolUse.hooks]]
command = "/a/cort hook-suggest"
"#,
    )
    .unwrap();
    remove_hook(&p).unwrap();
    let v = read(&p);
    assert_eq!(group_count(&v), 1, "the user's empty group went with ours: {v}");
    assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"].as_str(), Some("Read"));
}
