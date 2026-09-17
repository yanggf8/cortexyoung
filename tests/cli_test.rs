mod common;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudecat")
}

#[test]
fn scan_json_is_valid_and_complete() {
    let t = common::Tmp(common::temp_dir());
    common::write(
        &t.0,
        "package.json",
        r#"{"name":"webapp","scripts":{"start":"node server.js"},"dependencies":{"express":"^4"}}"#,
    );
    common::write(
        &t.0,
        "server.js",
        "const express = require('express');\napp.get('/', (req, res) => res.json({}));\n",
    );
    let out = Command::new(bin())
        .args([
            "scan",
            "--root",
            t.0.to_str().unwrap(),
            "--format",
            "json",
            "--top-files",
            "5",
        ])
        .output()
        .expect("run scan");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["meta"]["framework"], "Express.js");
    assert_eq!(v["meta"]["name"], "webapp");
    assert!(v["key_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"].as_str().unwrap() == "server.js"));
}

#[test]
fn update_dry_run_does_not_write() {
    let t = common::Tmp(common::temp_dir());
    let data = common::Tmp(common::temp_dir());
    common::write(&t.0, "Cargo.toml", "[package]\nname=\"demo\"\n");
    common::write(&t.0, "src/main.rs", "fn main() {}\n");
    let out = Command::new(bin())
        .args([
            "update",
            "--root",
            t.0.to_str().unwrap(),
            "--dry-run",
            "--top-files",
            "5",
        ])
        .env("CLAUDECAT_DATA_DIR", data.0.to_str().unwrap())
        .output()
        .expect("run update dry-run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !t.0.join("CLAUDE.md").exists(),
        "dry-run must not create CLAUDE.md"
    );
    assert!(
        !data.0.join("projects").exists(),
        "dry-run must not create the data dir"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("WOULD UPDATE"));
}

#[test]
fn update_writes_map_and_rules_only_claude_md() {
    let t = common::Tmp(common::temp_dir());
    // CLAUDECAT_DATA_DIR 必須指到掃描目標之外：指進專案內會讓掃描把地圖也算進去
    let data = common::Tmp(common::temp_dir());
    common::write(&t.0, "Cargo.toml", "[package]\nname=\"demo\"\n");
    common::write(&t.0, "src/main.rs", "fn main() {}\n");
    let run = |args: &[&str]| {
        Command::new(bin())
            .args(args)
            .env("CLAUDECAT_DATA_DIR", data.0.to_str().unwrap())
            .output()
            .expect("run claudecat")
    };
    let first = run(&[
        "update",
        "--root",
        t.0.to_str().unwrap(),
        "--top-files",
        "5",
    ]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let stdout = String::from_utf8_lossy(&first.stdout);
    let map_path = stdout
        .lines()
        .find_map(|l| l.strip_prefix("map -> "))
        .expect("update 應印出 map -> 路徑");
    let map = std::fs::read_to_string(map_path).unwrap();
    assert!(map.contains("# claudecat Project Map"));
    assert!(map.contains("Project Map"));
    assert!(map.contains("src/main.rs"));
    // CLAUDE.md 只剩規則＋種子
    let content = std::fs::read_to_string(t.0.join("CLAUDE.md")).unwrap();
    assert!(!content.contains("claudecat:auto"), "auto 區塊不得再寫入");
    assert!(content.contains("claudecat:map-pointer:begin"));
    assert!(content.contains("claudecat:guardrails:begin"));
    assert!(!content.contains("src/main.rs"), "地圖內容不得進 CLAUDE.md");
    // 二跑：CLAUDE.md no-op；地圖重寫（路徑不變）
    let second = run(&[
        "update",
        "--root",
        t.0.to_str().unwrap(),
        "--top-files",
        "5",
    ]);
    let stdout2 = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout2.contains("Up to date"),
        "expected no rewrite, got: {stdout2}"
    );
    assert!(stdout2.contains("map -> "));
}

#[test]
fn update_first_run_migrates_legacy_block() {
    let t = common::Tmp(common::temp_dir());
    let data = common::Tmp(common::temp_dir());
    common::write(&t.0, "Cargo.toml", "[package]\nname=\"demo\"\n");
    common::write(&t.0, "src/main.rs", "fn main() {}\n");
    common::write(
        &t.0,
        "CLAUDE.md",
        "# Rules\n\n- keep me\n\n`<!-- claudecat:auto:begin -->\n## Project Map (auto-maintained by claudecat)\n- **Root**: `/old/machine/path`\n<!-- claudecat:auto:end -->\n",
    );
    let out = Command::new(bin())
        .args([
            "update",
            "--root",
            t.0.to_str().unwrap(),
            "--top-files",
            "5",
        ])
        .env("CLAUDECAT_DATA_DIR", data.0.to_str().unwrap())
        .output()
        .expect("run update");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let content = std::fs::read_to_string(t.0.join("CLAUDE.md")).unwrap();
    assert!(!content.contains("claudecat:auto"), "舊區塊應被剝除");
    assert!(
        !content.contains("/old/machine/path"),
        "機器路徑應隨區塊消失"
    );
    assert!(
        !content.lines().any(|l| l.starts_with('`')),
        "行首殘留反引號（`` `<!-- claudecat:auto `` 的殘骸形狀）應隨整行消失"
    );
    assert!(content.contains("- keep me"), "手寫規則保留");
    assert!(
        content.contains("claudecat:map-pointer:begin"),
        "指標種子播下"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let map_path = stdout
        .lines()
        .find_map(|l| l.strip_prefix("map -> "))
        .expect("應印出 map -> 路徑");
    let map = std::fs::read_to_string(map_path).unwrap();
    assert!(map.contains("Project Map"), "地圖內容落在資料夾");
}
