//! The measure-health CLI writes one local health observation and reports its outcome.

use cort::db::project_id_for;
use cort::usage::{record_command_at, CommandRecord};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    _project: tempfile::TempDir,
    cache: tempfile::TempDir,
    data: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let project = tempfile::tempdir().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical root");
        fs::create_dir_all(root.join("src")).expect("src");
        fs::write(
            root.join("src/main.ts"),
            "export function alpha() { return 1; }\n",
        )
        .expect("source");
        let cache = tempfile::tempdir().expect("cache");
        let data = tempfile::tempdir().expect("data");
        let fixture = Self {
            _project: project,
            cache,
            data,
            root,
        };
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "t@e.com"],
            &["config", "user.name", "t"],
            &["add", "-A"],
            &["commit", "-qm", "init"],
        ] {
            let out = Command::new("git")
                .arg("-C")
                .arg(&fixture.root)
                .args(args)
                .output()
                .expect("git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let out = fixture.run(&["index"]);
        assert!(
            out.status.success(),
            "index: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        record_command_at(
            &fixture.cache.path().join("usage.db"),
            &CommandRecord {
                now_ms: cort::usage::now_ms(),
                project_id: Some(project_id_for(fixture.root.to_str().expect("utf8 root"))),
                command: "impact".into(),
                args_summary: "{}".into(),
                status: "ok".into(),
                error_code: None,
                read_source: None,
                requested_content_mode: None,
                effective_content_mode: None,
                receipt_hit: None,
                index_stale: None,
                bytes_out: 0,
                saved_bytes: 0,
            },
        );
        fixture
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cort"))
            .args(args)
            .current_dir(&self.root)
            .env("CORT_CACHE_DIR", self.cache.path())
            .env("CORT_MEMORY_DIR", self.data.path())
            .env("CORT_CRONTAB_FILE", self.data.path().join("crontab"))
            .output()
            .expect("cort")
    }

    fn measure_rows(&self) -> i64 {
        Connection::open(self.cache.path().join("usage.db"))
            .expect("usage.db")
            .query_row(
                "SELECT COUNT(*) FROM command_log WHERE command = 'measure'",
                [],
                |row| row.get(0),
            )
            .expect("command_log")
    }

    fn log(&self) -> PathBuf {
        self.data.path().join("measure.log")
    }

    fn logged_summary(&self) -> Value {
        let log = fs::read_to_string(self.log()).expect("measure.log");
        assert_eq!(log.lines().count(), 1, "one log line: {log}");
        serde_json::from_str(log.trim_end()).expect("JSON log line")
    }
}

fn assert_summary(value: &Value) {
    assert_eq!(value["lane"], "health");
    assert!(value["ts_ms"].is_i64());
    assert_eq!(value["observed"], 1);
    assert_eq!(value["written"], 1);
    assert_eq!(value["unmapped"], 0);
    assert_eq!(value["skipped_busy"], false);
}

#[test]
fn background_writes_log_not_stdout() {
    let f = Fixture::new();
    let out = f.run(&["measure", "health", "--background"]);
    assert!(out.status.success(), "status: {:?}", out.status);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    assert_summary(&f.logged_summary());
    // Quiet is not unrecorded: the kicked child still writes its own `measure` row (design §5).
    assert_eq!(f.measure_rows(), 1);
}

#[test]
fn background_with_a_value_is_still_quiet() {
    let f = Fixture::new();
    let out = f.run(&["measure", "health", "--background=true"]);
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn background_help_and_version_are_quiet() {
    let f = Fixture::new();
    for extra in ["--help", "--version"] {
        let out = f.run(&["measure", "health", "--background", extra]);
        assert!(
            out.stdout.is_empty(),
            "{extra}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            out.stderr.is_empty(),
            "{extra}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Two collectors finishing together must not glue their lines: each line is one `write`.
#[test]
fn concurrent_appends_never_split_a_line() {
    let data = tempfile::tempdir().expect("data");
    let line = |writer: usize, n: usize| format!("{{\"writer\":{writer},\"n\":{n}}}");
    std::thread::scope(|scope| {
        for writer in 0..4 {
            let dir = data.path();
            scope.spawn(move || {
                for n in 0..500 {
                    cort::collect::append_measure_log(dir, &line(writer, n)).expect("append");
                }
            });
        }
    });
    let log = fs::read_to_string(data.path().join("measure.log")).expect("measure.log");
    let mut lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2_000, "every append is its own line");
    for text in &lines {
        serde_json::from_str::<Value>(text).unwrap_or_else(|e| panic!("{e}: {text:?}"));
    }
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(lines.len(), 2_000);
}

#[test]
fn foreground_prints_the_summary() {
    let f = Fixture::new();
    let out = f.run(&["measure", "health"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary: Value = serde_json::from_slice(&out.stdout).expect("JSON stdout");
    assert_summary(&summary);
    assert!(!f.log().exists());
}

#[test]
fn background_failure_is_logged_and_nonzero() {
    let f = Fixture::new();
    let db = Connection::open(f.cache.path().join("usage.db")).expect("usage.db");
    db.execute_batch(
        "CREATE TRIGGER block_health BEFORE INSERT ON index_health_log
         BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
    )
    .expect("trigger");
    let out = f.run(&["measure", "health", "--background"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    let summary = f.logged_summary();
    assert_eq!(summary["lane"], "health");
    assert!(summary["error"]
        .as_str()
        .expect("error")
        .contains("index_health_log_write_failed"));
}

#[test]
fn the_run_records_its_own_measure_row() {
    let f = Fixture::new();
    let out = f.run(&["measure", "health"]);
    assert!(out.status.success());
    assert_eq!(f.measure_rows(), 1);
}
