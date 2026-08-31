//! The demand screen's only defence against a fake conclusion: pasted agent output is not a user
//! wanting a caller set. Every case below is a shape that actually appears in these transcripts.

use cort_evals::demand::{
    classify, claude_user_line, codex_cwd_line, codex_user_line, matched_needles, own_words, scan,
    scrub, scrub_user,
};
use serde_json::json;
use std::path::Path;

fn write_jsonl(path: &Path, lines: &[serde_json::Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: String = lines
        .iter()
        .map(|l| format!("{}\n", serde_json::to_string(l).unwrap()))
        .collect();
    std::fs::write(path, body).unwrap();
}

fn claude_line(text: &str, cwd: &str, source: &str) -> serde_json::Value {
    json!({
        "type": "user", "cwd": cwd, "promptSource": source,
        "message": { "role": "user", "content": text },
        "timestamp": "2026-08-30T00:00:00.000Z",
    })
}

fn codex_line(text: &str) -> serde_json::Value {
    json!({
        "type": "response_item",
        "payload": { "type": "message", "role": "user",
                     "content": [ { "type": "input_text", "text": text } ] },
        "timestamp": "2026-08-30T00:00:00.000Z",
    })
}

#[test]
fn a_pasted_report_behind_review_is_not_a_user_asking_for_anything() {
    // The single most common shape in these logs, and the one that turned a 0.2% real rate into a
    // 7.9% artifact on the first pass.
    let pasted =
        "review\n\u{25cf} Refactoring Complete. I've renamed the DAL and fixed all callers";
    assert_eq!(own_words(pasted), "", "{pasted}");
    let bulleted = "review works\n\u{25cf} All changes complete. Summary of Changes below";
    assert_eq!(own_words(bulleted), "");
    // A voice no marker knows about is still caught, by the short-first-line rule.
    let zh = "review\n我已完成您要求的架構：\nFrontend (UI):\n移除輸入框，替換為 ZenTaskMenu";
    assert_eq!(own_words(zh), "", "{zh}");
    let en = "advise me\nWhy You're Getting the Warning\n\n  Your wrangler.toml has 2 environments";
    assert_eq!(own_words(en), "", "{en}");
}

#[test]
fn a_bare_directive_alone_contributes_nothing() {
    assert_eq!(own_words("review"), "");
    assert_eq!(own_words("corroborate"), "");
    assert_eq!(own_words("verify root cause"), "");
    // Too short to be an instruction at all.
    assert_eq!(own_words("ok go"), "");
}

#[test]
fn the_users_own_words_survive_a_leading_review() {
    // Dropping the paste must not silently delete the request that came with it.
    let kept = own_words("review who calls getCurrentTimeET before I rename it");
    assert!(kept.contains("who calls"), "{kept}");
    assert_eq!(classify(&kept).map(|c| c.0), Some("ask"));
    // Multi-line but the user's own first line is a whole sentence: it stays.
    let sentence =
        own_words("請把 handleReportsStatus 改名成 handleReportStatus\n所有呼叫點都要一起改，別漏");
    assert!(sentence.contains("改名"), "{sentence}");
    assert_eq!(classify(&sentence).map(|c| c.0), Some("task"));
}

#[test]
fn ask_and_task_are_separated_by_who_has_to_answer() {
    let ask = classify("這個改動的影響範圍有多大，誰呼叫了 logInfo").map(|c| c.0);
    assert_eq!(ask, Some("ask"));
    // Same question typed in simplified characters: under-counting demand is the failure this
    // screen cannot be allowed to have.
    assert_eq!(
        classify("这个改动的影响范围有多大").map(|c| c.0),
        Some("ask"),
        "simplified variants must normalize"
    );
    let task = classify("remove the dead code in market-drivers-cache-manager.ts").map(|c| c.0);
    assert_eq!(task, Some("task"));
    // An ask outranks a task: the blast-radius question subsumes the rename verb in the same turn.
    assert_eq!(
        classify("rename getCurrentDateET, and tell me the blast radius first").map(|c| c.0),
        Some("ask")
    );
    assert_eq!(
        classify("deploy the worker and tail the logs").map(|c| c.0),
        None
    );
}

#[test]
fn a_needle_barely_inside_a_longer_word_does_not_fire_but_a_suffix_may() {
    // Leading edge is bounded, trailing edge is not: an inflected verb is still the instruction.
    assert!(
        matched_needles("the noncallers list is empty", &["callers"]).is_empty(),
        "embedded inside a longer word is not a mention of callers"
    );
    assert!(matched_needles("please renamed the file", &["rename"]).contains(&"rename"));
    assert!(matched_needles("refactoring notes for later", &["refactor"]).contains(&"refactor"));
    assert!(
        matched_needles("blast radius", &["callers"]).is_empty(),
        "unrelated needle must not fire"
    );
}

#[test]
fn tool_results_and_injections_are_not_prompts() {
    let tool_result = json!({
        "type": "user", "cwd": "/tmp/x", "promptSource": "typed",
        "message": { "role": "user", "content": [ { "type": "tool_result", "content": "who calls X" } ] },
    });
    assert!(claude_user_line(&tool_result.to_string()).is_none());
    assert!(claude_user_line(
        &claude_line("who calls getCurrentTimeET", "/tmp/x", "sdk").to_string()
    )
    .is_none());
    assert!(claude_user_line(
        &json!({ "type": "user", "cwd": "/tmp/x", "promptSource": "typed", "isMeta": true,
                 "message": { "role": "user", "content": "who calls X" } })
        .to_string()
    )
    .is_none());
    // Sidechains are subagent turns: their "user" messages are prompts we wrote, not the human's.
    assert!(claude_user_line(
        &json!({ "type": "user", "cwd": "/tmp/x", "promptSource": "typed", "isSidechain": true,
                 "message": { "role": "user", "content": "who calls X" } })
        .to_string()
    )
    .is_none());
    for injected in [
        "<permissions instructions>who calls X",
        "/model opus",
        "# AGENTS.md instructions for /tmp/x",
    ] {
        assert!(
            claude_user_line(&claude_line(injected, "/tmp/x", "typed").to_string()).is_none(),
            "{injected}"
        );
        assert!(codex_user_line(&codex_line(injected).to_string()).is_none());
    }
    // A document, not an instruction.
    let huge = "x".repeat(2001);
    assert!(claude_user_line(&claude_line(&huge, "/tmp/x", "typed").to_string()).is_none());
}

#[test]
fn the_project_label_is_the_leaf_of_the_recorded_cwd() {
    let (project, _) = claude_user_line(
        &claude_line("who calls X please", "/home/dev/work/cct", "typed").to_string(),
    )
    .expect("a typed string prompt is an instruction");
    assert_eq!(project, "cct");
    let meta = json!({ "type": "session_meta", "payload": { "cwd": "/home/dev/work/dac" } });
    assert_eq!(codex_cwd_line(&meta.to_string()).as_deref(), Some("dac"));
    // Old rollouts carry no session_meta: the caller keeps "unknown" rather than inventing a name.
    assert_eq!(
        codex_cwd_line(&json!({ "id": "x", "timestamp": "t", "instructions": null }).to_string()),
        None
    );
}

#[test]
fn a_scan_reports_denominators_and_keeps_every_hit_auditable() {
    let root = std::env::temp_dir().join(format!("cort-demand-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let claude = root.join("claude");
    let codex = root.join("codex");

    write_jsonl(
        &claude.join("cct").join("s1.jsonl"),
        &[
            claude_line(
                "blast radius of getCurrentTimeET 是誰",
                "/home/dev/cct",
                "typed",
            ),
            claude_line(
                "review\n\u{25cf} I've refactored all callers",
                "/home/dev/cct",
                "typed",
            ),
            claude_line("ship it", "/home/dev/cct", "typed"),
        ],
    );
    write_jsonl(
        &codex.join("2026").join("08").join("30").join("r.jsonl"),
        &[
            json!({ "type": "session_meta", "payload": { "cwd": "/home/dev/dac" } }),
            codex_line("把 kv-client 改名成 kv-store，replace 所有用法"),
        ],
    );
    // A project the caller declares not-code is counted separately, never quietly dropped.
    write_jsonl(
        &claude.join("notes").join("s2.jsonl"),
        &[claude_line(
            "rename my habit tracker, who calls whom",
            "/home/dev/notes",
            "typed",
        )],
    );

    let report = scan(
        Some(Path::new(&claude)),
        Some(Path::new(&codex)),
        &["notes".to_string()],
    )
    .unwrap();
    // "ship it" is too short to be an instruction, and "review + pasted report" is not the user's
    // text: both are accounted for, neither is silently missing from the denominator.
    assert_eq!(report["usable_instructions"], json!(2), "{report}");
    assert_eq!(report["dropped_as_pure_paste"], json!(2));
    assert_eq!(report["ask"]["count"], json!(1));
    assert_eq!(report["task"]["count"], json!(1));
    assert_eq!(report["excluded_instructions"], json!(1));
    assert_eq!(report["files"]["read"], json!(3));
    let projects: Vec<String> = report["by_project"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["project"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(projects, vec!["cct".to_string(), "dac".to_string()]);
    assert_eq!(report["matches"][0]["needles"].as_array().unwrap().len(), 1);
    assert!(report["matches"][0]["instruction"]
        .as_str()
        .unwrap()
        .contains("blast radius"));
    // Shares are only meaningful next to the denominator they came from.
    assert_eq!(report["ask"]["share_of_instructions"], json!(0.5));
    assert_eq!(report["task_per_ask"], json!(1.0));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn no_data_is_an_error_rather_than_a_zero_rate() {
    assert!(scan(None, None, &[]).is_err());
    let root = std::env::temp_dir().join(format!("cort-demand-empty-{}", std::process::id()));
    let claude = root.join("claude");
    // Every message is paste: the correct output is an error, not "0% demand".
    write_jsonl(
        &claude.join("cct").join("s1.jsonl"),
        &[claude_line(
            "review\n\u{25cf} Summary of Changes: refactored callers",
            "/home/dev/cct",
            "typed",
        )],
    );
    let err = scan(Some(Path::new(&claude)), None, &[]).unwrap_err();
    assert!(err.contains("not a measurement"), "{err}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn an_excerpt_never_carries_a_developer_path_or_a_url() {
    let raw =
        "check /home/dev/work/repo/src/main.rs and https://worker.example.internal/api/v1 now, \
               plus ~/secret and (/tmp/x)";
    let clean = scrub(raw);
    assert!(
        !clean.contains("/home/") && !clean.contains("worker.example"),
        "{clean}"
    );
    assert_eq!(clean.matches("<path>").count(), 3, "{clean}");
    assert!(clean.contains("<url>"), "{clean}");
    // Over-redaction would destroy the audit trail the excerpt exists to provide: a slash that does
    // not open a token is ordinary prose or a repo-relative path.
    assert_eq!(
        scrub("rg/grep then evals/Cargo.toml and impact --depth 3"),
        "rg/grep then evals/Cargo.toml and impact --depth 3"
    );
    // The account name is redacted only as a whole token: a repo or symbol that merely contains it
    // must keep reading, or the audit trail becomes worthless.
    let account = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !account.is_empty() {
        assert_eq!(
            scrub_user(&format!("cd {account} && ls")),
            "cd <user> && ls"
        );
        assert_eq!(
            scrub_user(&format!("{account}-engine")),
            format!("{account}-engine")
        );
    }
}

#[test]
fn the_committed_demand_artefact_holds_no_machine_paths() {
    // AGENTS.md: no absolute paths from any developer's machine anywhere in the repo, including
    // fixtures. This artefact is generated from personal transcripts, so the promise is gated here
    // rather than trusted to whoever typed the command.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/runs/2026-08-31-demand/report.json"
    );
    let Ok(raw) = std::fs::read_to_string(path) else {
        panic!("{path} is missing: run `cort-evals demand --out` and commit it");
    };
    // Machine-independent markers only. Checking "the account name of whoever runs the test" would
    // pass here and fail on a CI runner whose account is literally `runner` the moment a transcript
    // mentions a test runner: an artefact gate has to assert invariants of the artefact, and the
    // username redaction is proved by `scrub_user` in the test above.
    for needle in ["/home/", "/mnt/", "/Users/", "~/", "AppData"] {
        assert!(
            !raw.contains(needle),
            "the demand artefact carries {needle:?}; scrub() is supposed to remove that"
        );
    }
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("report.json parses");
    assert!(doc["usable_instructions"].as_u64().unwrap_or(0) > 0);
    assert!(
        !doc["matches"].as_array().unwrap().is_empty(),
        "an empty match list is not an auditable artefact"
    );
}
