use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    other: PathBuf,
    claude: PathBuf,
    memory: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        // macOS tempdirs live under /var, a symlink to /private/var; the decode walk refuses a
        // symlink, as Claude Code never records one in a real cwd. Encode the real path.
        let root = temp.path().canonicalize().unwrap();
        let repo = root.join("demo");
        let other = root.join("other");
        let claude = root.join("claude");
        let memory = root.join("memory-data");
        let home = root.join("home");
        for path in [&repo, &other, &claude, &memory, &home] {
            fs::create_dir_all(path).unwrap();
        }
        for (path, remote) in [(&repo, "demo"), (&other, "other")] {
            assert!(Command::new("git")
                .arg("init")
                .arg(path)
                .output()
                .unwrap()
                .status
                .success());
            assert!(Command::new("git")
                .current_dir(path)
                .args([
                    "remote",
                    "add",
                    "origin",
                    &format!("https://example.com/{remote}.git")
                ])
                .output()
                .unwrap()
                .status
                .success());
        }
        Self {
            _temp: temp,
            repo,
            other,
            claude,
            memory,
            home,
        }
    }

    fn run(&self, args: &[&str]) -> (Output, Value) {
        let output = Command::new(env!("CARGO_BIN_EXE_cort"))
            .current_dir(&self.repo)
            .env("CORT_MEMORY_DIR", &self.memory)
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude)
            .env("CORT_CACHE_DIR", self.home.join("cache"))
            .env_remove("CORT_MEMORY_PROJECT")
            .env_remove("CLAUDECAT_MEMORY_PROJECT")
            .args(["memory", "notes"])
            .args(args)
            .output()
            .unwrap();
        let stream = if output.stdout.is_empty() {
            &output.stderr
        } else {
            &output.stdout
        };
        let value = serde_json::from_slice(stream)
            .unwrap_or_else(|_| panic!("output: {}", String::from_utf8_lossy(stream)));
        (output, value)
    }

    fn note(&self, repo: &Path, name: &str, body: &str) {
        let encoded: String = repo
            .to_str()
            .unwrap()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self.claude.join("projects").join(encoded).join("memory");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), body).unwrap();
    }
}

#[test]
fn opt_in_out_and_status() {
    let f = Fixture::new();
    let (out, opted) = f.run(&["opt-in"]);
    assert!(out.status.success());
    assert_eq!(opted["status"], "opted_in");
    let project = opted["project"].clone();
    let (_, status) = f.run(&["status"]);
    assert_eq!(status["opted_in"][0], project);
    assert_eq!(status["memory_configured"], false);
    assert_eq!(status["tombstones_unknown"], true);
    assert_eq!(status["uploaded_notes"], 0);
    assert_eq!(status["claude_home"], f.claude.to_str().unwrap());
    let (out, opted) = f.run(&["opt-out"]);
    assert!(out.status.success());
    assert_eq!(opted["status"], "opted_out");
    assert_eq!(opted["remote_rows"], "kept");
    let (_, status) = f.run(&["status"]);
    assert!(status["opted_in"].as_array().unwrap().is_empty());
}

#[test]
fn dry_run_screens_notes_without_configuration() {
    let f = Fixture::new();
    f.note(&f.repo, "safe.md", "A project note.\n");
    f.note(&f.repo, "secret.md", "password: Abcdef12345!\n");
    f.note(&f.other, "other.md", "Another project note.\n");
    assert!(f.run(&["opt-in"]).0.status.success());
    let (out, report) = f.run(&["upload", "--dry-run"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(report["planned_from"], "none");
    assert_eq!(report["uploaded"], false);
    assert_eq!(report["tombstones_unknown"], true);
    assert_eq!(report["tombstone_skipped"], 0);
    assert_eq!(report["host_written"], false);
    let listing = report["listing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        listing
            .iter()
            .any(|line| line.starts_with("upload\t") && line.contains("safe.md")),
        "{listing:?}"
    );
    assert!(
        listing
            .iter()
            .any(|line| line.starts_with("not_opted_in\t") && line.contains("other.md")),
        "{listing:?}"
    );
    assert!(
        listing
            .iter()
            .any(|line| line.starts_with("skipped_secret\t")
                && line.contains("secret.md")
                && line.contains("rule=credential_assignment")),
        "{listing:?}"
    );
}

#[test]
fn unconfigured_network_and_reads_fail_as_memory_error() {
    let f = Fixture::new();
    for args in [&["upload"][..], &["list"][..], &["show", "abcdef"][..]] {
        let (out, error) = f.run(args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(error["error"], "memory_error", "{error}");
    }
}
