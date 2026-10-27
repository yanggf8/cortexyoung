use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::tempdir;

const QUOTE: &str = "the remembered fact matters";

fn packet(id: &str, stratum: &str, weight: f64) -> Value {
    json!({"window_id":id,"stratum":stratum,"weight":weight,"source":"secret source",
        "entries":[{"id":"entry","key":"secret key","claim":"secret claim"}],
        "turns":[{"turn":0,"speaker":"injection","text":"secret injection"},
            {"turn":1,"speaker":"user","text":"the remembered fact matters to me"},
            {"turn":2,"speaker":"assistant_text","text":"the remembered\n fact matters here"}],
        "elided_turns":0,"truncated":false})
}

fn row(id: &str, used: &str, reason: &str, speaker: &str, turn: u32, quote: &str) -> Value {
    json!({"window_id":id,"used":used,"opportunity":{"answer":"yes","turn":1},
        "reason":reason,"evidence":[{"turn":turn,"speaker":speaker,"quote":quote}]})
}

fn write_lines(path: &Path, values: &[Value]) {
    let body = values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(path, body).unwrap();
}

fn setup(packets: &[Value]) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().join("private");
    fs::create_dir(&dir).unwrap();
    write_lines(&dir.join("packets.jsonl"), packets);
    (tmp, dir)
}

fn judge(dir: &Path, rows: &[Value]) -> Value {
    let file = dir.join("review.jsonl");
    write_lines(&file, rows);
    cort_evals::uptake_judge::judge(dir, &file, Some(7)).unwrap()
}

#[test]
fn rejection_causes_and_unjudged_are_counted() {
    let packets = (0..11)
        .map(|i| packet(&format!("id{i}"), "candidate", 1.0))
        .collect::<Vec<_>>();
    let (_tmp, dir) = setup(&packets);
    let mut rows = vec![row("id0", "explicit", "asked_by_user", "user", 1, QUOTE)];
    rows.push(row("id0", "explicit", "asked_by_user", "user", 1, QUOTE));
    rows.push(row("absent", "explicit", "asked_by_user", "user", 1, QUOTE));
    rows.push(row("id1", "maybe", "asked_by_user", "user", 1, QUOTE));
    let mut bad = row("id2", "no", "not_relevant", "user", 1, QUOTE);
    bad["opportunity"]["answer"] = json!("perhaps");
    rows.push(bad);
    rows.push(row("id3", "no", "unknown", "user", 1, QUOTE));
    rows.push(row("id4", "no", "asked_by_user", "user", 1, QUOTE));
    let mut bad = row("id5", "no", "not_relevant", "user", 1, QUOTE);
    bad["opportunity"]["turn"] = json!(88);
    rows.push(bad);
    rows.push(row("id6", "no", "not_relevant", "assistant_text", 1, QUOTE));
    let mut bad = row("id7", "no", "not_relevant", "user", 1, QUOTE);
    bad["evidence"] = json!([]);
    rows.push(bad);
    let mut bad = row("id8", "no", "not_relevant", "user", 1, QUOTE);
    bad["opportunity"].as_object_mut().unwrap().remove("turn");
    rows.push(bad);
    let file = dir.join("review.jsonl");
    write_lines(&file, &rows);
    fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"not json\n")
        .unwrap();
    let report = cort_evals::uptake_judge::judge(&dir, &file, Some(7)).unwrap();
    for (cause, n) in [
        ("malformed", 3),
        ("unknown_window", 1),
        ("duplicate_window", 1),
        ("bad_used", 1),
        ("bad_opportunity", 1),
        ("bad_reason", 1),
        ("reason_used_mismatch", 1),
        ("bad_opportunity_turn", 1),
        ("bad_quote", 1),
    ] {
        assert_eq!(report["rejected"][cause], n, "{cause}");
    }
    assert_eq!(report["accepted"], 1);
    assert_eq!(report["unjudged"], 10);
    let rejected = fs::read_to_string(dir.join("judged-rejected.jsonl")).unwrap();
    assert!(rejected.contains("\"row\":\"not json\""));
}

use std::io::Write;

#[test]
fn quotes_basis_weights_and_report_privacy() {
    let (_tmp, dir) = setup(&[
        packet("private-window-one", "rest", 2.5),
        packet("private-window-two", "candidate", 1.0),
        packet("private-window-three", "rest", 2.5),
        packet("private-window-four", "rest", 2.5),
    ]);
    let rows = vec![
        row(
            "private-window-one",
            "explicit",
            "task_needed_fact",
            "assistant_text",
            2,
            QUOTE,
        ),
        row(
            "private-window-two",
            "explicit",
            "task_needed_fact",
            "user",
            1,
            QUOTE,
        ),
        row(
            "private-window-three",
            "explicit",
            "asked_by_user",
            "user",
            1,
            QUOTE,
        ),
        row("private-window-four", "no", "not_noticed", "user", 1, QUOTE),
    ];
    let report = judge(&dir, &rows);
    assert_eq!(report["reasons"]["task_needed_fact"]["observed"], 1);
    assert_eq!(report["reasons"]["task_needed_fact"]["inferred"], 1);
    assert_eq!(report["reasons"]["task_needed_fact"]["weighted"], 3.5);
    assert_eq!(report["reasons"]["asked_by_user"]["observed"], 1);
    assert_eq!(report["reasons"]["not_noticed"]["inferred"], 1);
    assert_eq!(report["by_stratum"]["rest"]["accepted"], 3);
    let body = report.to_string();
    for secret in [
        "private-window-one",
        "private-window-two",
        "private-window-three",
        "private-window-four",
        QUOTE,
        "secret key",
        "secret claim",
        "secret source",
        "secret injection",
    ] {
        assert!(!body.contains(secret), "{secret}");
    }
    assert_eq!(report["audit"]["status"], "not_drawn");
    assert!(
        !dir.join("audit.jsonl").exists(),
        "four accepted rows draw no audit"
    );
    for name in ["judged-accepted.jsonl", "judged-rejected.jsonl"] {
        assert_eq!(
            fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let accepted = fs::read_to_string(dir.join("judged-accepted.jsonl")).unwrap();
    assert!(accepted.contains("\"basis\":\"observed\""));
}

#[test]
fn quote_difference_and_turn_zero_are_rejected() {
    let (_tmp, dir) = setup(&[packet("a", "rest", 1.0), packet("b", "rest", 1.0)]);
    let report = judge(
        &dir,
        &[
            row(
                "a",
                "explicit",
                "task_needed_fact",
                "assistant_text",
                2,
                "the remembered fact matterx",
            ),
            row(
                "b",
                "no",
                "not_relevant",
                "injection",
                0,
                "secret injection",
            ),
        ],
    );
    assert_eq!(report["rejected"]["bad_quote"], 2);
}

#[test]
fn audit_is_deterministic_persistent_and_reads_agreement() {
    let packets = (0..11)
        .map(|i| packet(&format!("id{i}"), "rest", 2.0))
        .collect::<Vec<_>>();
    let (_tmp, dir) = setup(&packets);
    let rows = (0..11)
        .map(|i| {
            row(
                &format!("id{i}"),
                "explicit",
                "asked_by_user",
                "user",
                1,
                QUOTE,
            )
        })
        .collect::<Vec<_>>();
    let report = judge(&dir, &rows);
    assert_eq!(report["audit"]["n"], 10);
    assert_eq!(report["audit"]["status"], "pending");
    let path = dir.join("audit.jsonl");
    let original = fs::read_to_string(&path).unwrap();
    let ids = original
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["window_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 10);
    let (_other_tmp, other_dir) = setup(&packets);
    judge(&other_dir, &rows);
    assert_eq!(
        fs::read_to_string(other_dir.join("audit.jsonl")).unwrap(),
        original
    );
    judge(&dir, &rows);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let filled = original
        .lines()
        .map(|line| {
            let mut row: Value = serde_json::from_str(line).unwrap();
            row["agree"] = json!(true);
            row.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &filled).unwrap();
    let report = judge(&dir, &rows);
    assert_eq!(report["audit"]["status"], "done");
    assert_eq!(report["audit"]["agreement"], 1.0);
    assert_eq!(report["findings_publishable"], true);
    assert_eq!(fs::read_to_string(&path).unwrap(), filled);
    let invalid = filled.replacen("\"agree\":true", "\"agree\":\"yes\"", 1);
    fs::write(&path, invalid).unwrap();
    assert!(cort_evals::uptake_judge::judge(&dir, &dir.join("review.jsonl"), Some(7)).is_err());
}

#[test]
fn cli_requires_private_and_judged_file_inside_it() {
    let (_tmp, dir) = setup(&[packet("id", "rest", 1.0)]);
    let external = dir.parent().unwrap().join("external.jsonl");
    fs::write(&external, "").unwrap();
    let binary = env!("CARGO_BIN_EXE_cort-evals");
    let no_private = std::process::Command::new(binary)
        .args(["memory-uptake", "--judged", external.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!no_private.status.success());
    assert!(String::from_utf8_lossy(&no_private.stderr).contains("requires --private"));
    let outside = std::process::Command::new(binary)
        .args([
            "memory-uptake",
            "--private",
            dir.to_str().unwrap(),
            "--judged",
            external.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!outside.status.success());
    assert!(String::from_utf8_lossy(&outside.stderr).contains("inside --private"));
}

#[test]
fn opportunity_turns_are_bound_to_the_answer_and_never_the_injection() {
    let packets = (0..3)
        .map(|i| packet(&format!("id{i}"), "rest", 1.0))
        .collect::<Vec<_>>();
    let (_tmp, dir) = setup(&packets);
    let mut zero = row("id0", "explicit", "asked_by_user", "user", 1, QUOTE);
    zero["opportunity"]["turn"] = json!(0);
    let mut turn_without_yes = row("id1", "no", "not_relevant", "user", 1, QUOTE);
    turn_without_yes["opportunity"]["answer"] = json!("no");
    let mut yes_without_turn = row("id2", "explicit", "asked_by_user", "user", 1, QUOTE);
    yes_without_turn["opportunity"]["turn"] = Value::Null;
    let file = dir.join("review.jsonl");
    write_lines(&file, &[zero, turn_without_yes, yes_without_turn]);
    fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"\n   \n")
        .unwrap();
    let report = cort_evals::uptake_judge::judge(&dir, &file, Some(7)).unwrap();
    assert_eq!(report["rejected"]["bad_opportunity_turn"], 3);
    assert_eq!(report["rows"], 3);
}

#[test]
fn an_audit_from_another_sample_is_refused_not_scored() {
    let (_tmp, dir) = setup(&[packet("new", "rest", 1.0)]);
    fs::write(
        dir.join("audit.jsonl"),
        "{\"window_id\":\"old\",\"used\":\"no\",\"reason\":\"not_relevant\",\"basis\":\"inferred\",\"agree\":true,\"note\":\"\"}\n",
    )
    .unwrap();
    let file = dir.join("review.jsonl");
    write_lines(
        &file,
        &[row("new", "explicit", "asked_by_user", "user", 1, QUOTE)],
    );
    let err = cort_evals::uptake_judge::judge(&dir, &file, Some(7)).unwrap_err();
    assert!(err.contains("another"), "{err}");
}

#[test]
fn scan_flags_are_refused_with_judged() {
    let (_tmp, dir) = setup(&[packet("id", "rest", 1.0)]);
    let file = dir.join("review.jsonl");
    fs::write(&file, "").unwrap();
    for flag in ["--claude-dir", "--since", "--sample", "--seed"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_cort-evals"))
            .args([
                "memory-uptake",
                "--private",
                dir.to_str().unwrap(),
                "--judged",
                file.to_str().unwrap(),
                flag,
                "1",
            ])
            .output()
            .unwrap();
        assert!(!out.status.success(), "{flag}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("does not apply"));
    }
}

#[test]
fn a_rejected_first_row_still_claims_its_window() {
    let (_tmp, dir) = setup(&[packet("id0", "rest", 1.0)]);
    let mut empty = row("id0", "explicit", "asked_by_user", "user", 1, QUOTE);
    empty["evidence"] = json!([]);
    let report = judge(
        &dir,
        &[
            empty,
            row("id0", "explicit", "asked_by_user", "user", 1, QUOTE),
        ],
    );
    assert_eq!(report["rejected"]["malformed"], 1);
    assert_eq!(report["rejected"]["duplicate_window"], 1);
    assert_eq!(report["accepted"], 0);
    assert_eq!(report["used"]["explicit"], 0);
    assert_eq!(report["opportunity"]["unclear"], 0);
}

#[test]
fn quote_length_inferred_evidence_and_basis() {
    let mut p = packet("a", "rest", 1.0);
    p["turns"][2]["text"] = json!("abcdefghijkl and more");
    let (_tmp, dir) = setup(&[p, packet("b", "rest", 1.0), packet("c", "rest", 1.0)]);
    let mut inferred = row("c", "explicit", "task_needed_fact", "user", 1, QUOTE);
    inferred["evidence"] = json!("inferred");
    let report = judge(
        &dir,
        &[
            row(
                "a",
                "explicit",
                "task_needed_fact",
                "assistant_text",
                2,
                "abcdefghijkl",
            ),
            row(
                "b",
                "explicit",
                "task_needed_fact",
                "assistant_text",
                2,
                "the remembe",
            ),
            inferred,
        ],
    );
    assert_eq!(report["rejected"]["bad_quote"], 1);
    assert_eq!(report["reasons"]["task_needed_fact"]["observed"], 1);
    assert_eq!(report["reasons"]["task_needed_fact"]["inferred"], 1);
}

#[test]
fn a_symlink_out_of_the_private_directory_is_refused() {
    let (tmp, dir) = setup(&[packet("id", "rest", 1.0)]);
    let outside = tmp.path().join("outside.jsonl");
    fs::write(&outside, "").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("link.jsonl")).unwrap();
    let err = cort_evals::uptake_judge::judge(&dir, &dir.join("link.jsonl"), Some(7)).unwrap_err();
    assert!(err.contains("inside"), "{err}");
    let dotted = dir.join("..").join("outside.jsonl");
    assert!(cort_evals::uptake_judge::judge(&dir, &dotted, Some(7)).is_err());
}

fn audit_rows(dir: &Path) -> Vec<Value> {
    fs::read_to_string(dir.join("audit.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn write_audit(dir: &Path, rows: &[Value]) {
    write_lines(&dir.join("audit.jsonl"), rows);
}

#[test]
fn the_audit_gate_counts_distinct_current_judgments_under_one_seed() {
    let packets = (0..11)
        .map(|i| packet(&format!("id{i}"), "rest", 1.0))
        .collect::<Vec<_>>();
    let (_tmp, dir) = setup(&packets);
    let rows = (0..11)
        .map(|i| {
            row(
                &format!("id{i}"),
                "explicit",
                "asked_by_user",
                "user",
                1,
                QUOTE,
            )
        })
        .collect::<Vec<_>>();
    let report = judge(&dir, &rows[..9]);
    assert_eq!(report["audit"]["status"], "not_drawn");
    assert!(!dir.join("audit.jsonl").exists());
    let report = judge(&dir, &rows);
    assert_eq!(report["audit"]["status"], "pending");
    assert_eq!(report["audit"]["n"], 10);
    assert_eq!(report["audit"]["seed"], 7);
    assert_eq!(report["findings_publishable"], false);
    let drawn = audit_rows(&dir);
    assert!(drawn.iter().all(|r| r["seed"] == 7));

    let mut partial = drawn.clone();
    partial[0]["agree"] = json!(false);
    write_audit(&dir, &partial);
    let report = judge(&dir, &rows);
    assert_eq!(report["audit"]["status"], "partial");
    assert_eq!(report["audit"]["agreement"], 0.0);

    let mut done = drawn.clone();
    for r in &mut done {
        r["agree"] = json!(true);
    }
    write_audit(&dir, &done);
    let report = judge(&dir, &rows);
    assert_eq!(report["audit"]["status"], "done");
    assert_eq!(report["findings_publishable"], true);
    // A hand-trimmed audit can be done without being publishable.
    write_audit(&dir, &done[..9]);
    let report = judge(&dir, &rows);
    assert_eq!(report["audit"]["status"], "done");
    assert_eq!(report["audit"]["n"], 9);
    assert_eq!(report["findings_publishable"], false);
    write_audit(&dir, &done);

    // Without --audit-seed the stored seed is reported; a different one is refused.
    let file = dir.join("review.jsonl");
    let report = cort_evals::uptake_judge::judge(&dir, &file, None).unwrap();
    assert_eq!(report["audit"]["seed"], 7);
    let err = cort_evals::uptake_judge::judge(&dir, &file, Some(8)).unwrap_err();
    assert!(err.contains("seed 7"), "{err}");

    let mut twice = done.clone();
    twice[1] = twice[0].clone();
    write_audit(&dir, &twice);
    assert!(judge_err(&dir, &rows).contains("twice"));

    let mut eleven = done.clone();
    let spare = rows
        .iter()
        .map(|r| r["window_id"].as_str().unwrap())
        .find(|id| done.iter().all(|d| d["window_id"] != *id))
        .unwrap();
    let mut extra = done[0].clone();
    extra["window_id"] = json!(spare);
    eleven.push(extra);
    write_audit(&dir, &eleven);
    assert!(judge_err(&dir, &rows).contains("more than 10"));

    let mut unseeded = done.clone();
    unseeded[0].as_object_mut().unwrap().remove("seed");
    write_audit(&dir, &unseeded);
    assert!(judge_err(&dir, &rows).contains("move it aside"));

    write_audit(&dir, &done);
    let before = fs::read_to_string(dir.join("judged-accepted.jsonl")).unwrap();
    let mut changed = rows.clone();
    let audited = done[0]["window_id"].as_str().unwrap();
    for r in &mut changed {
        if r["window_id"] == audited {
            r["reason"] = json!("verifying_a_claim");
        }
    }
    assert!(judge_err(&dir, &changed).contains("changed"));
    assert_eq!(
        fs::read_to_string(dir.join("judged-accepted.jsonl")).unwrap(),
        before,
        "a refused run leaves the previous outputs alone"
    );
}

fn judge_err(dir: &Path, rows: &[Value]) -> String {
    let file = dir.join("review.jsonl");
    write_lines(&file, rows);
    cort_evals::uptake_judge::judge(dir, &file, Some(7)).unwrap_err()
}

#[test]
fn a_flag_without_its_value_is_refused() {
    let (_tmp, dir) = setup(&[packet("id", "rest", 1.0)]);
    for args in [
        vec!["--private", dir.to_str().unwrap(), "--judged"],
        vec!["--judged", "--private", dir.to_str().unwrap()],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_cort-evals"))
            .arg("memory-uptake")
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("needs a value"));
    }
}
