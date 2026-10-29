use serde_json::Value;
use std::fs;
use std::process::Command;

#[test]
fn cort_owns_the_memory_config_and_keeps_credentials_out_of_output() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old-config.json");
    let data = dir.path().join("cort-memory");
    let cache = dir.path().join("cort-cache");
    fs::write(
        &old,
        r#"{"url":"https://example.turso.io","token":"private-token"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cort"))
        .env("CORT_MEMORY_DIR", &data)
        .env("CORT_CACHE_DIR", &cache)
        .args(["memory", "import-config", "--from", old.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "configured");
    assert_eq!(
        report["path"],
        data.join("turso-memory.json").to_str().unwrap()
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-token"));
    let config: Value =
        serde_json::from_slice(&fs::read(data.join("turso-memory.json")).unwrap()).unwrap();
    assert_eq!(config["token"], "private-token");
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(data.join("turso-memory.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0,
    );
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn an_unconfigured_recall_logs_the_harness_without_the_query() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("cort-memory");
    let cache = dir.path().join("cort-cache");
    let output = Command::new(env!("CARGO_BIN_EXE_cort"))
        .env("CORT_MEMORY_DIR", &data)
        .env("CORT_CACHE_DIR", &cache)
        .env("CORT_MEMORY_PROJECT", "sample-project")
        .args([
            "memory",
            "recall",
            "private deployment",
            "--harness",
            "codex",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private deployment"));
    let db = fs::read(cache.join("usage.db")).unwrap();
    let bytes = String::from_utf8_lossy(&db);
    assert!(bytes.contains("harness_declared"));
    assert!(!bytes.contains("private deployment"));
}

#[test]
fn alias_write_requires_exactly_one_source() {
    let dir = tempfile::tempdir().unwrap();
    for action in ["add", "remove"] {
        for flags in [
            vec![],
            vec![
                "--from-remote",
                "https://example.test/x",
                "--from-project",
                "old",
            ],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_cort"))
                .env("CORT_MEMORY_DIR", dir.path().join("memory"))
                .env("CORT_CACHE_DIR", dir.path().join("cache"))
                .args(["memory", "alias", action])
                .args(flags)
                .output()
                .unwrap();
            assert!(!output.status.success());
            let error: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(error["error"], "unknown_command");
        }
    }
}

#[test]
fn alias_commands_report_unconfigured_without_exposing_remote_credentials() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["memory", "alias", "list"],
        vec![
            "memory",
            "alias",
            "add",
            "--from-remote",
            "https://user:secret@example.test/x.git",
        ],
        vec![
            "memory",
            "alias",
            "remove",
            "--from-remote",
            "https://user:secret@example.test/x.git",
        ],
        vec!["memory", "status"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cort"))
            .env("CORT_MEMORY_DIR", dir.path().join("memory"))
            .env("CORT_CACHE_DIR", dir.path().join("cache"))
            .env("CORT_MEMORY_PROJECT", "test-project")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("secret"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("secret"));
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"], "memory_error");
        assert!(error["detail"]["message"]
            .as_str()
            .unwrap()
            .contains("not configured"));
    }
}
