//! `cort question` at the command line, with no network: the memory database is never
//! configured, so every verb that gets past its own argument checks stops at the config.

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
fn question_help_lists_four_verbs() {
    let f = Fixture::new();
    for args in [
        vec!["question", "--help"],
        vec!["question", "-h"],
        vec!["question", "help"],
    ] {
        let (output, value) = f.run(&args);
        assert!(output.status.success(), "{args:?}");
        assert_eq!(value["usage"], "cort question <command> [options]");
        let mut verbs: Vec<&str> = value["commands"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        verbs.sort();
        assert_eq!(verbs, ["add", "answer", "close", "list"], "{args:?}");
        let add = value["commands"]["add"].as_str().unwrap();
        for flag in [
            "--text",
            "--option",
            "--recommend",
            "--blocks",
            "--by",
            "--harness",
            "--force",
            "--root",
            "--format",
        ] {
            assert!(add.contains(flag), "{add} lacks {flag}");
        }
        let answer = value["commands"]["answer"].as_str().unwrap();
        for flag in ["--answer", "--quote", "--option", "--replace"] {
            assert!(answer.contains(flag), "{answer} lacks {flag}");
        }
        let close = value["commands"]["close"].as_str().unwrap();
        assert!(close.contains("stale|moot|duplicate"), "{close}");
        assert!(close.contains("--of"), "{close}");
        let list = value["commands"]["list"].as_str().unwrap();
        for flag in ["--open", "--answered", "--closed", "--all", "--match"] {
            assert!(list.contains(flag), "{list} lacks {flag}");
        }
    }
    let (_, value) = f.run(&["question", "answer", "--help"]);
    let keys: Vec<&String> = value["commands"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["answer"]);
    assert!(!f.cache.exists(), "--help must not touch the cache");
}

#[test]
fn top_level_usage_lists_question() {
    let f = Fixture::new();
    let (output, value) = f.run(&["--help"]);
    assert!(output.status.success());
    let line = value["commands"]["question"].as_str().unwrap();
    for verb in ["add", "list", "answer", "close"] {
        assert!(line.contains(verb), "{line}");
    }
    let (_, value) = f.run(&["nonsense"]);
    assert!(value["detail"]["known"]
        .as_array()
        .unwrap()
        .contains(&json!("question")));
}

#[test]
fn option_without_equals_is_refused_before_network() {
    let f = Fixture::new();
    for bad in ["A", "=text", "A="] {
        let (output, value) = f.run(&["question", "add", "--text", "which db?", "--option", bad]);
        assert!(!output.status.success());
        let message = value["detail"]["message"].as_str().unwrap();
        assert!(
            message.starts_with("question_option_invalid"),
            "{bad}: {value}"
        );
    }
    // Local validation also runs before the config is read.
    let (_, value) = f.run(&[
        "question",
        "add",
        "--text",
        "which db?",
        "--option",
        "A=turso",
        "--recommend",
        "B",
    ]);
    let message = value["detail"]["message"].as_str().unwrap();
    assert!(message.starts_with("question_recommend_unknown"), "{value}");
}

#[test]
fn unconfigured_add_reports_memory_config_error() {
    let f = Fixture::new();
    f.assert_unconfigured(&[
        "question",
        "add",
        "--text",
        "which db?",
        "--option",
        "A=turso: remote",
        "--option",
        "B=sqlite",
        "--recommend",
        "A",
        "--harness",
        "codex",
    ]);
    f.assert_unconfigured(&["question", "list"]);
    f.assert_unconfigured(&["question", "list", "--all", "-f", "lean"]);
    f.assert_unconfigured(&["question", "list", "--match", "which", "db"]);
    f.assert_unconfigured(&["question", "close", "deadbeef", "--reason", "moot"]);
    // An unknown harness or close reason is a usage error, not a config error.
    let (_, value) = f.run(&["question", "add", "--text", "q", "--harness", "vim"]);
    assert_eq!(value["error"], "unknown_command");
    let (_, value) = f.run(&["question", "close", "deadbeef", "--reason", "bored"]);
    assert_eq!(value["error"], "unknown_command");
    let (_, value) = f.run(&["question", "list", "--open", "--closed"]);
    assert_eq!(value["error"], "unknown_command");
}

#[test]
fn replace_accepts_zero_or_more_ids() {
    let f = Fixture::new();
    f.assert_unconfigured(&[
        "question",
        "answer",
        "deadbeef",
        "--answer",
        "x",
        "--replace",
    ]);
    f.assert_unconfigured(&[
        "question",
        "answer",
        "deadbeef",
        "--answer",
        "x",
        "--replace",
        "aaaaaaaa",
        "bbbbbbbb",
    ]);
    f.assert_unconfigured(&[
        "question",
        "answer",
        "deadbeef",
        "--answer",
        "x",
        "--quote",
        "use turso",
        "--option",
        "A",
    ]);
}

#[test]
fn usage_row_omits_question_text() {
    let f = Fixture::new();
    let secret = "zebra-quokka private words";
    for args in [
        vec![
            "question",
            "add",
            "--text",
            secret,
            "--option",
            "A=quokka-opt",
        ],
        vec![
            "question",
            "answer",
            "deadbeef",
            "--answer",
            "zebra-answer",
            "--quote",
            "zebra-quote",
            "--option",
            "zebra-key",
        ],
        vec!["question", "list", "--match", "zebra-match"],
        vec![
            "question",
            "add",
            "--text",
            "q",
            "--blocks",
            "zebra-blocks",
            "--by",
            "zebra-by",
        ],
    ] {
        let (output, _) = f.run(&args);
        assert!(!output.status.success());
    }
    let db = fs::read(f.cache.join("usage.db")).unwrap();
    let bytes = String::from_utf8_lossy(&db);
    assert!(bytes.contains("question"));
    assert!(bytes.contains(r#""verb":"add""#), "verb not recorded");
    assert!(bytes.contains(r#""verb":"answer""#), "verb not recorded");
    for word in ["zebra", "quokka"] {
        assert!(!bytes.contains(word), "{word} leaked into usage.db");
    }
}

#[test]
fn lean_list_has_no_empty_field() {
    let payload = json!({
        "project_id": "p",
        "stale_days": 14,
        "questions": [
            {"id": "0123456789abcdef", "state": "open", "stale": true, "age_days": 20,
             "text": "which\tdb?", "current_answers": []},
            {"id": "1111111122222222", "state": "answered", "stale": false, "age_days": 0,
             "text": "", "current_answers": [
                {"id": "a1", "answer": "turso", "quoted": false},
                {"id": "a2", "answer": "sqlite\nnow", "quoted": true, "quote": "sqlite"}
             ]},
            {"id": "3333333344444444", "state": "closed", "stale": false, "age_days": 3,
             "text": "old", "current_answers": [], "close": {"reason": "moot"}},
            {"id": "5555555566666666", "state": "answered", "stale": false, "age_days": 1,
             "text": "q", "current_answers": [{"id": "a3", "answer": "", "quoted": false}]}
        ]
    });
    let lean = cort::question_store::render_lean("list", &payload);
    let lines: Vec<&str> = lean.lines().collect();
    assert_eq!(lines.len(), 4, "{lean}");
    for line in &lines {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "{line:?}");
        assert!(fields.iter().all(|f| !f.is_empty()), "{line:?}");
    }
    assert_eq!(lines[0], "01234567\topen\tstale\t20d\twhich db?\t-");
    assert!(lines[1].starts_with("11111111\tanswered\t-\t0d\t-\t"));
    assert!(lines[1].contains("turso (no quote)"), "{}", lines[1]);
    assert!(lines[1].contains("sqlite now"), "{}", lines[1]);
    assert!(lines[3].ends_with("\t(no quote)"), "{}", lines[3]);

    let added = json!({"outcome": "added", "question": {"id": "0123456789abcdef"}, "similar": []});
    assert_eq!(
        cort::question_store::render_lean("add", &added),
        "added 0123456789abcdef\n"
    );
    let exists = json!({"outcome": "exists", "questions": [
        {"id": "0123456789abcdef", "state": "answered"},
        {"id": "fedcba9876543210", "state": "closed"}
    ]});
    assert_eq!(
        cort::question_store::render_lean("add", &exists),
        "exists 0123456789abcdef answered\nexists fedcba9876543210 closed\n"
    );
    let answered = json!({"outcome": "answered", "question": {"id": "0123456789abcdef"}});
    assert_eq!(
        cort::question_store::render_lean("answer", &answered),
        "answered 01234567\n"
    );
    let closed = json!({"outcome": "closed", "question": {"id": "0123456789abcdef"}});
    assert_eq!(
        cort::question_store::render_lean("close", &closed),
        "closed 01234567\n"
    );
}
