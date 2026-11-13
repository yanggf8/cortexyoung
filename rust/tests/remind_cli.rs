//! `cort remind` at the command line, with no network: the memory database is never
//! configured, so every verb that gets past its own argument checks stops at the config --
//! and the checks that need no network (a malformed `--on`, the `--of` rule) refuse before
//! the config is read at all.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    memory: PathBuf,
    cache: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let repo = root.join("demo");
        let memory = root.join("memory-data");
        let cache = root.join("cache");
        let home = root.join("home");
        for path in [&repo, &home] {
            fs::create_dir_all(path).unwrap();
        }
        assert!(Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(["remote", "add", "origin", "https://example.com/demo.git"])
            .output()
            .unwrap()
            .status
            .success());
        Self {
            _temp: temp,
            repo,
            memory,
            cache,
            home,
        }
    }

    fn run(&self, args: &[&str]) -> (Output, Value) {
        let output = Command::new(env!("CARGO_BIN_EXE_cort"))
            .current_dir(&self.repo)
            .env("CORT_MEMORY_DIR", &self.memory)
            .env("CORT_CACHE_DIR", &self.cache)
            .env("HOME", &self.home)
            .env_remove("CORT_MEMORY_PROJECT")
            .env_remove("CLAUDECAT_MEMORY_PROJECT")
            .args(args)
            .output()
            .unwrap();
        let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "stdout: {} stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output, value)
    }

    fn assert_unconfigured(&self, args: &[&str]) {
        let (output, value) = self.run(args);
        assert!(!output.status.success(), "{args:?}");
        assert_eq!(value["error"], "memory_error", "{args:?}: {value}");
        let message = value["detail"]["message"].as_str().unwrap();
        assert!(message.contains("not configured"), "{args:?}: {message}");
    }
}

#[test]
fn remind_help_lists_four_verbs() {
    let f = Fixture::new();
    for args in [
        vec!["remind", "--help"],
        vec!["remind", "-h"],
        vec!["remind", "help"],
    ] {
        let (output, value) = f.run(&args);
        assert!(output.status.success(), "{args:?}");
        assert_eq!(value["usage"], "cort remind <command> [options]");
        let mut verbs: Vec<&str> = value["commands"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        verbs.sort();
        assert_eq!(verbs, ["add", "close", "defer", "list"], "{args:?}");
        let add = value["commands"]["add"].as_str().unwrap();
        for flag in [
            "--text",
            "--on",
            "--lead",
            "--blocks",
            "--force",
            "--harness",
            "--root",
            "--format",
        ] {
            assert!(add.contains(flag), "{add} lacks {flag}");
        }
        let defer = value["commands"]["defer"].as_str().unwrap();
        for flag in ["--on", "--note"] {
            assert!(defer.contains(flag), "{defer} lacks {flag}");
        }
        let close = value["commands"]["close"].as_str().unwrap();
        assert!(close.contains("done|moot|duplicate"), "{close}");
        assert!(close.contains("--of"), "{close}");
        let list = value["commands"]["list"].as_str().unwrap();
        for flag in ["--open", "--closed", "--all", "--match"] {
            assert!(list.contains(flag), "{list} lacks {flag}");
        }
    }
    let (_, value) = f.run(&["remind", "defer", "--help"]);
    let keys: Vec<&String> = value["commands"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["defer"]);
    assert!(!f.cache.exists(), "--help must not touch the cache");
}

#[test]
fn top_level_usage_lists_remind() {
    let f = Fixture::new();
    let (output, value) = f.run(&["--help"]);
    assert!(output.status.success());
    let line = value["commands"]["remind"].as_str().unwrap();
    for verb in ["add", "list", "defer", "close"] {
        assert!(line.contains(verb), "{line}");
    }
    assert!(line.contains("cort remind --help"), "{line}");
    let (_, value) = f.run(&["nonsense"]);
    assert!(value["detail"]["known"]
        .as_array()
        .unwrap()
        .contains(&json!("remind")));
}

#[test]
fn malformed_on_refused_before_network() {
    let f = Fixture::new();
    for bad in [
        "2026-13-01",
        "2026-02-30",
        "21/11/2026",
        "2026-11-21+8",
        "2026-11-21+15:00",
    ] {
        for args in [
            vec!["remind", "add", "--on", bad, "--text", "task"],
            vec!["remind", "defer", "0011223344556677", "--on", bad],
        ] {
            let (output, value) = f.run(&args);
            assert!(!output.status.success(), "{bad}: {args:?}");
            assert_eq!(value["error"], "memory_error", "{bad}: {args:?}: {value}");
            let message = value["detail"]["message"].as_str().unwrap();
            assert!(message.starts_with("remind_date_invalid"), "{bad}: {value}");
        }
    }
    // The refusal fires before the ledger is opened: with a date that parses, the same
    // command gets as far as the (missing) config.
    let (_, value) = f.run(&["remind", "add", "--on", "2026-12-01", "--text", "task"]);
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(message.contains("not configured"), "{value}");
}

#[test]
fn unconfigured_add_reports_memory_config_error() {
    let f = Fixture::new();
    f.assert_unconfigured(&[
        "remind",
        "add",
        "--on",
        "2026-12-01",
        "--text",
        "NUC boot auto-recovery",
        "--lead",
        "2",
        "--blocks",
        "nuc-boot",
        "--harness",
        "codex",
    ]);
    f.assert_unconfigured(&[
        "remind",
        "add",
        "--on",
        "2026-12-01+08:00",
        "--text",
        "task",
    ]);
    f.assert_unconfigured(&["remind", "list"]);
    f.assert_unconfigured(&["remind", "list", "--all", "-f", "lean"]);
    f.assert_unconfigured(&["remind", "list", "--closed"]);
    f.assert_unconfigured(&["remind", "list", "--match", "nuc", "boot"]);
    f.assert_unconfigured(&["remind", "list", "0011223344556677"]);
    f.assert_unconfigured(&[
        "remind",
        "defer",
        "0011223344556677",
        "--on",
        "2026-12-08",
        "--note",
        "next one",
    ]);
    f.assert_unconfigured(&["remind", "close", "0011223344556677", "--reason", "done"]);
    f.assert_unconfigured(&[
        "remind",
        "close",
        "0011223344556677",
        "--reason",
        "duplicate",
        "--of",
        "9988776655443322",
    ]);
    // An unknown harness or close reason is a usage error, and a bad lead a bounds
    // refusal; neither is a config error.
    let (_, value) = f.run(&[
        "remind",
        "add",
        "--on",
        "2026-12-01",
        "--text",
        "t",
        "--harness",
        "vim",
    ]);
    assert_eq!(value["error"], "unknown_command", "{value}");
    let (_, value) = f.run(&["remind", "close", "0011223344556677", "--reason", "bored"]);
    assert_eq!(value["error"], "unknown_command", "{value}");
    let (_, value) = f.run(&["remind", "list", "--open", "--closed"]);
    assert_eq!(value["error"], "unknown_command", "{value}");
    let (_, value) = f.run(&[
        "remind",
        "add",
        "--on",
        "2026-12-01",
        "--text",
        "t",
        "--lead",
        "31",
    ]);
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(message.starts_with("remind_lead_invalid"), "{value}");
}

#[test]
fn usage_row_omits_reminder_text() {
    let f = Fixture::new();
    let secret = "zebra-quokka private words";
    for args in [
        vec![
            "remind",
            "add",
            "--on",
            "2026-12-01",
            "--text",
            secret,
            "--blocks",
            "zebra-blocks",
        ],
        vec![
            "remind",
            "defer",
            "0011223344556677",
            "--on",
            "2026-12-08",
            "--note",
            "zebra-note",
        ],
        vec![
            "remind",
            "close",
            "0011223344556677",
            "--reason",
            "moot",
            "--note",
            "zebra-note",
        ],
        vec!["remind", "list", "--match", "zebra-match"],
    ] {
        let (output, _) = f.run(&args);
        assert!(!output.status.success());
    }
    let db = fs::read(f.cache.join("usage.db")).unwrap();
    let bytes = String::from_utf8_lossy(&db);
    assert!(bytes.contains("remind"));
    for verb in ["add", "defer", "close", "list"] {
        assert!(
            bytes.contains(&format!("\"verb\":\"{verb}\"")),
            "{verb} not recorded"
        );
    }
    for word in ["zebra", "quokka"] {
        assert!(!bytes.contains(word), "{word} leaked into usage.db");
    }
}

#[test]
fn duplicate_without_of_refused_before_network() {
    let f = Fixture::new();
    let (output, value) = f.run(&[
        "remind",
        "close",
        "0011223344556677",
        "--reason",
        "duplicate",
    ]);
    assert!(!output.status.success());
    assert_eq!(value["error"], "memory_error", "{value}");
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(message.starts_with("remind_of_required"), "{value}");
    // `--of` belongs to duplicate alone, and that too needs no network.
    let (_, value) = f.run(&[
        "remind",
        "close",
        "0011223344556677",
        "--reason",
        "done",
        "--of",
        "9988776655443322",
    ]);
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(message.starts_with("remind_of_unexpected"), "{value}");
    // With `--of` the command gets as far as the config, which is the point: the refusal
    // above fired before any ledger was opened.
    let (_, value) = f.run(&[
        "remind",
        "close",
        "0011223344556677",
        "--reason",
        "duplicate",
        "--of",
        "9988776655443322",
    ]);
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(message.contains("not configured"), "{value}");
}
