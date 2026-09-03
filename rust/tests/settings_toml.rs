//! The Codex `config.toml` merge, judged against the same shapes `rust/tests/settings.rs` judges
//! the JSON side against: other things already in `hooks.PreToolUse`, and a file the installer must
//! never make unreadable. The predicate under test (`is_ours_for`) is the JSON module's, imported rather
//! than reimplemented -- these tests exist to pin the TOML-specific plumbing around it (the nested
//! array-of-tables navigation, refusal rules, and empty-scaffolding cleanup), not to re-litigate the
//! token-matching rule itself.

use cort::settings::{Change, HookEvent};
use cort::settings_toml::{install_hook, installed_command, installed_entry, remove_hook};
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
    fs::read_to_string(p)
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap()
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
    doc["hooks"]["PreToolUse"]
        .as_array_of_tables()
        .unwrap()
        .len()
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
    let out = install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    assert_eq!(out.change, Change::Installed);
    assert!(out.backup.is_none(), "nothing to back up");
    let v = read(&p);
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["matcher"].as_str(),
        Some("Bash")
    );
    assert_eq!(cmd_at(&v, 0, 0), "/bin/cort hook-suggest");
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["type"].as_str(),
        Some("command")
    );
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
    assert_eq!(group_count(&read(&p)), 1);
}

#[test]
fn every_hook_the_user_already_had_survives() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let v = read(&p);
    assert_eq!(v["approvals_reviewer"].as_str(), Some("user"));
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str(),
        Some("mos hook")
    );
    assert_eq!(
        group_count(&v),
        2,
        "ours is added beside theirs, not over it"
    );
    assert_eq!(cmd_at(&v, 0, 0), "mos hook");
    assert_eq!(cmd_at(&v, 1, 0), "/bin/cort hook-suggest");
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
    let v = read(&p);
    assert_eq!(group_count(&v), 1);
    assert_eq!(cmd_at(&v, 0, 0), "/home/u/.local/bin/cort hook-suggest");
}

#[test]
fn remove_takes_ours_out_and_leaves_theirs() {
    let (_d, p) = tmp();
    with_existing_hooks(&p);
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
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
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
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
    let err = install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap_err();
    assert!(format!("{err}").contains("not valid TOML"), "{err}");
    assert_eq!(
        fs::read_to_string(&p).unwrap(),
        "this = [ is not valid toml"
    );
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
    assert_eq!(group_hook_count(&v, 0), 1, "two copies fire the hook twice");
    assert_eq!(cmd_at(&v, 0, 0), "/home/u/.cargo/bin/cort hook-suggest");
    assert_eq!(group_count(&v), 1);
}

#[test]
fn the_surviving_entry_keeps_no_hand_typed_field() {
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
    assert!(
        entry.get("async").is_none(),
        "stale field survived: {entry}"
    );
    assert_eq!(entry["type"].as_str(), Some("command"));
    assert_eq!(entry["timeout"].as_integer(), Some(5));
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
    assert_eq!(
        installed_command(&p, HookEvent::Suggest).unwrap(),
        "/bin/cort hook-suggest"
    );
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
    assert!(installed_command(&p, HookEvent::Suggest).is_none());
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
    assert!(
        installed_command(&p, HookEvent::Suggest).is_none(),
        "claimed a vendor binary"
    );

    install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap();
    let v = read(&p);
    let all: Vec<String> = (0..group_count(&v))
        .flat_map(|g| (0..group_hook_count(&v, g)).map(move |h| (g, h)))
        .map(|(g, h)| cmd_at(&v, g, h))
        .collect();
    assert!(
        all.iter()
            .any(|c| c == "/opt/vendor/bin/hook-suggest --daemon"),
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
    let err = install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap_err();
    assert!(format!("{err}").contains("not a table"), "{err}");
    assert_eq!(fs::read_to_string(&p).unwrap(), before);
}

#[test]
fn a_pretooluse_of_the_wrong_type_is_refused_not_replaced() {
    let (_d, p) = tmp();
    let before = "[hooks]\nPreToolUse = \"not an array\"\n";
    fs::write(&p, before).unwrap();
    let err = install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap_err();
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
    install_hook(&p, "/x/cort hook-suggest", HookEvent::Suggest).unwrap();
    let v = read(&p);
    assert_eq!(
        group_count(&v),
        2,
        "expected the Read group plus one of ours"
    );
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["matcher"].as_str(),
        Some("Read")
    );
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
    assert_eq!(
        group_count(&v),
        1,
        "the user's empty group went with ours: {v}"
    );
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["matcher"].as_str(),
        Some("Read")
    );
}

// --- Codex's trust gate -------------------------------------------------------------------------
//
// Wiring is only half of it on this side: Codex will not run a hook it has not been shown once, and
// an unreviewed entry sits in config.toml firing nothing. `--check` called that state `wired` until
// 2026-09-02, on a machine where the hook had never run. These pin the reading of the
// `[hooks.state]` table Codex writes when a hook is reviewed; they do not pretend to validate the
// hash, which only Codex can recompute.

/// The `[hooks.state]` key Codex writes: the config file's own path, then the event, then the
/// position of the entry inside `hooks.PreToolUse`.
fn trust_block(path: &str, group: usize, hook: usize) -> String {
    format!(
        "\n[hooks.state.\"{path}:pre_tool_use:{group}:{hook}\"]\ntrusted_hash = \"sha256:deadbeef\"\n"
    )
}

fn append(p: &PathBuf, s: &str) {
    let mut cur = fs::read_to_string(p).unwrap();
    cur.push_str(s);
    fs::write(p, cur).unwrap();
}

#[test]
fn a_wired_entry_with_no_state_table_reads_as_untrusted() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let (command, trusted) = installed_entry(&p, HookEvent::Suggest).unwrap();
    assert_eq!(command, "/bin/cort hook-suggest");
    assert!(!trusted, "no [hooks.state] at all cannot read as trusted");
}

#[test]
fn a_trusted_hash_at_our_position_reads_as_trusted() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    append(&p, &trust_block(&p.to_string_lossy(), 0, 0));
    assert!(installed_entry(&p, HookEvent::Suggest).unwrap().1);
}

/// The path half of the key is Codex's spelling of the same file, so it is deliberately not
/// compared -- a trust entry written against a symlinked or `CODEX_HOME`-relative spelling still
/// counts. Anchoring recognition to two processes agreeing on how to write a path is the mistake
/// §7/§9 already paid for once on the command line.
#[test]
fn the_path_half_of_the_key_is_not_compared() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    append(&p, &trust_block("/some/other/spelling/config.toml", 0, 0));
    assert!(installed_entry(&p, HookEvent::Suggest).unwrap().1);
}

/// Position is compared, though: trust granted to somebody else's entry is not ours. `0:0` must
/// also not read as a suffix of `10:0`.
#[test]
fn a_trusted_hash_at_a_different_position_does_not_count() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let path = p.to_string_lossy().to_string();
    append(&p, &trust_block(&path, 1, 0));
    append(&p, &trust_block(&path, 10, 0));
    assert!(!installed_entry(&p, HookEvent::Suggest).unwrap().1);
}

/// An empty `trusted_hash` is not a trust decision, and neither is a state entry without one.
#[test]
fn a_state_entry_without_a_hash_does_not_count() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    let path = p.to_string_lossy().to_string();
    append(
        &p,
        &format!("\n[hooks.state.\"{path}:pre_tool_use:0:0\"]\ntrusted_hash = \"\"\n"),
    );
    assert!(!installed_entry(&p, HookEvent::Suggest).unwrap().1);
}

/// The trust table is Codex's, and a reinstall must leave it exactly where it was -- otherwise
/// every redeploy silently un-trusts the hook it just wired. Both paths matter: the no-op reinstall
/// and the one that actually rewrites the command.
#[test]
fn install_preserves_a_trust_table_it_did_not_write() {
    let (_d, p) = tmp();
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    append(&p, &trust_block(&p.to_string_lossy(), 0, 0));

    assert_eq!(
        install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest)
            .unwrap()
            .change,
        Change::AlreadyPresent
    );
    assert!(
        installed_entry(&p, HookEvent::Suggest).unwrap().1,
        "no-op reinstall dropped it"
    );

    assert_eq!(
        install_hook(&p, "/other/cort hook-suggest", HookEvent::Suggest)
            .unwrap()
            .change,
        Change::Updated
    );
    assert!(
        installed_entry(&p, HookEvent::Suggest).unwrap().1,
        "rewriting the command dropped it"
    );
}

/// `installed_command` is the same scan with the trust half discarded; it must not start
/// disagreeing with `installed_entry` about what is wired.
#[test]
fn installed_command_still_agrees_with_installed_entry() {
    let (_d, p) = tmp();
    assert_eq!(installed_command(&p, HookEvent::Suggest), None);
    assert!(installed_entry(&p, HookEvent::Suggest).is_none());
    install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    assert_eq!(
        installed_command(&p, HookEvent::Suggest).unwrap(),
        installed_entry(&p, HookEvent::Suggest).unwrap().0
    );
}

// --- Two events in one flat array ---------------------------------------------------------------
//
// Kimi keeps every hook in a single top-level `[[hooks]]` list, so both of ours are neighbours
// there and only the command line says which is which. Everything below exists because installing
// one must not disturb the other -- on the JSON and Codex sides the event key separates them for
// free, and here nothing does.

fn kimi_tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::Builder::new()
        .prefix("cort-settings-kimi-")
        .tempdir()
        .unwrap();
    let p = d.path().join("config.toml");
    (d, p)
}

fn kimi_entries(p: &PathBuf) -> Vec<(String, String)> {
    let doc = fs::read_to_string(p)
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap();
    let Some(list) = doc
        .as_table()
        .get("hooks")
        .and_then(|h| h.as_array_of_tables())
    else {
        return Vec::new();
    };
    list.iter()
        .map(|t| {
            (
                t.get("event")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                t.get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn installing_the_refresh_hook_does_not_disturb_the_suggest_hook() {
    let (_d, p) = kimi_tmp();
    cort::settings_kimi::install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    cort::settings_kimi::install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh).unwrap();

    let entries = kimi_entries(&p);
    assert_eq!(entries.len(), 2, "one entry per event, got {entries:?}");
    assert!(entries.contains(&(
        "PreToolUse".to_string(),
        "/bin/cort hook-suggest".to_string()
    )));
    assert!(entries.contains(&(
        "PostToolUse".to_string(),
        "/bin/cort hook-refresh".to_string()
    )));

    // And a redeploy of either converges rather than piling up or rewriting the other.
    assert_eq!(
        cort::settings_kimi::install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest)
            .unwrap()
            .change,
        Change::AlreadyPresent
    );
    assert_eq!(kimi_entries(&p).len(), 2);
}

#[test]
fn a_moved_binary_updates_each_event_in_place() {
    let (_d, p) = kimi_tmp();
    cort::settings_kimi::install_hook(&p, "/old/cort hook-suggest", HookEvent::Suggest).unwrap();
    cort::settings_kimi::install_hook(&p, "/old/cort hook-refresh", HookEvent::Refresh).unwrap();
    cort::settings_kimi::install_hook(&p, "/new/cort hook-refresh", HookEvent::Refresh).unwrap();

    let entries = kimi_entries(&p);
    assert_eq!(
        entries.len(),
        2,
        "updating must not add a third: {entries:?}"
    );
    assert!(entries.contains(&(
        "PreToolUse".to_string(),
        "/old/cort hook-suggest".to_string()
    )));
    assert!(entries.contains(&(
        "PostToolUse".to_string(),
        "/new/cort hook-refresh".to_string()
    )));
}

/// Uninstalling is one act: `--remove` takes both out, and leaves everyone else's entries alone.
#[test]
fn remove_takes_both_events_and_nothing_else() {
    let (_d, p) = kimi_tmp();
    fs::write(
        &p,
        "[[hooks]]\nevent = \"PreToolUse\"\ncommand = \"/opt/theirs/gate.js\"\ntimeout = 15\n",
    )
    .unwrap();
    cort::settings_kimi::install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    cort::settings_kimi::install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh).unwrap();
    assert_eq!(kimi_entries(&p).len(), 3);

    assert_eq!(
        cort::settings_kimi::remove_hook(&p).unwrap().change,
        Change::Removed
    );
    let left = kimi_entries(&p);
    assert_eq!(left.len(), 1, "only theirs survives: {left:?}");
    assert_eq!(left[0].1, "/opt/theirs/gate.js");
}

/// The two commands are told apart by their subcommand token, not by position or by the event
/// field an entry happens to carry -- an entry with the wrong event but our refresh command is
/// still ours to update.
#[test]
fn the_subcommand_is_what_identifies_which_of_ours_an_entry_is() {
    let (_d, p) = kimi_tmp();
    cort::settings_kimi::install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    assert_eq!(
        cort::settings_kimi::installed_command(&p, HookEvent::Refresh),
        None,
        "the suggest entry must not answer for the refresh one"
    );
    assert_eq!(
        cort::settings_kimi::installed_command(&p, HookEvent::Suggest).unwrap(),
        "/bin/cort hook-suggest"
    );
}

/// The file we write into is fenced by somebody else's comments, and one of them ends up decorating
/// our entry. Removing the entry must give it back, not take it along.
///
/// Reproduced on 2026-09-03 on this machine's real `~/.kimi-code/config.toml`: install kept both
/// `kimi-plugin-cc-managed` markers, remove left one -- and their uninstaller looks for the pair.
/// The whole round trip is asserted byte for byte, because "the marker is still somewhere" is not
/// the promise; the promise is that a file we touched and then let go of is the file we found.
#[test]
fn an_install_and_remove_cycle_returns_the_file_exactly_as_it_was() {
    let (_d, p) = kimi_tmp();
    let original = "model = \"k3\"\n\n\
                    # === BEGIN kimi-plugin-cc-managed:claude-code (v1.9.13) ===\n\
                    # DO NOT EDIT — managed by /kimi:setup.\n\
                    [[hooks]]\n\
                    event = \"PreToolUse\"\n\
                    command = \"node /plugin/approval-hook.js\"\n\
                    timeout = 15\n\
                    # === END kimi-plugin-cc-managed:claude-code ===\n";
    fs::write(&p, original).unwrap();

    cort::settings_kimi::install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest).unwrap();
    cort::settings_kimi::install_hook(&p, "/bin/cort hook-refresh", HookEvent::Refresh).unwrap();
    assert_eq!(
        fs::read_to_string(&p)
            .unwrap()
            .matches("cc-managed")
            .count(),
        2,
        "installing must not disturb the fence either"
    );

    cort::settings_kimi::remove_hook(&p).unwrap();
    assert_eq!(fs::read_to_string(&p).unwrap(), original);

    // And repeatedly, because a cycle that grows the file by a line is the same bug slowed down.
    for _ in 0..3 {
        cort::settings_kimi::install_hook(&p, "/bin/cort hook-suggest", HookEvent::Suggest)
            .unwrap();
        cort::settings_kimi::remove_hook(&p).unwrap();
    }
    assert_eq!(fs::read_to_string(&p).unwrap(), original);
}
