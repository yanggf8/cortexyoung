use cort_evals::uptake::scan;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::tempdir;

const HEADER: &str = "cort memory: local replica (may be behind the shared database; `cort memory recall \"<topic>\"` refreshes)";
const NOTES: &str = "unreviewed harness notes from other machines (not checked against source; read one with `cort memory notes show <id>`):";
fn row(kind: &str, second: usize, extra: Value) -> Value {
    let mut v =
        json!({"type":kind,"timestamp":format!("2026-10-27T00:00:{second:02}Z"),"sessionId":"s1"});
    for (k, value) in extra.as_object().unwrap() {
        v[k] = value.clone();
    }
    v
}
fn hook(second: usize, source: &str, stdout: &str) -> Value {
    row(
        "attachment",
        second,
        json!({"attachment":{"type":"hook_success","hookEvent":"SessionStart","hookName":format!("SessionStart:{source}"),"stdout":stdout,"command":"cort hook-recall"}}),
    )
}
fn attach(second: usize, text: &str, other: &str, cwd: &Path) -> Value {
    row(
        "attachment",
        second,
        json!({"cwd":cwd,"attachment":{"type":"hook_additional_context","hookEvent":"SessionStart","hookName":"SessionStart","content":[other,text]}}),
    )
}
fn tool(second: usize, name: &str, input: Value) -> Value {
    row(
        "assistant",
        second,
        json!({"message":{"content":[{"type":"tool_use","name":name,"input":input}]}}),
    )
}
fn assistant(second: usize, text: &str) -> Value {
    row(
        "assistant",
        second,
        json!({"message":{"content":[{"type":"text","text":text}]}}),
    )
}
fn user(second: usize, text: &str) -> Value {
    row(
        "user",
        second,
        json!({"promptSource":"typed","message":{"content":text}}),
    )
}
fn write(rows: &[Value], dir: &Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    let text = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join(name), format!("{text}\n")).unwrap();
}
fn injection() -> String {
    format!("{HEADER}\n- [secret_key] claim with `garnet_signal` and abcdef1234567 (src/alpha.rs:42, source_unchanged)\n- [cut_key] incomplete `violet_tag` ...\n... 2 entries left out\n{NOTES}\n- note-1 [host] private-note.md (2026-10-27)\n... 3 more notes")
}
fn memory_note(root: &Path) -> String {
    let path = root.join("projects/p/memory/private-note.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "note").unwrap();
    path.to_string_lossy().into_owned()
}
#[test]
fn attachment_windows_and_private_detail() {
    let tmp = tempdir().unwrap();
    let transcript = tmp.path().join("transcripts");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("AGENTS.md"), "instructions").unwrap();
    fs::write(project.join("CLAUDE.md"), "instructions").unwrap();
    let outside_note = tmp.path().join("outside/memory/private-note.md");
    fs::create_dir_all(outside_note.parent().unwrap()).unwrap();
    fs::write(&outside_note, "outside").unwrap();
    let rows = vec![
        hook(0, "compact", &injection()),
        hook(1, "compact", &injection()),
        attach(2, &injection(), "other hook", &project),
        tool(3, "Bash", json!({"command":"cort memory status"})),
        tool(4, "Bash", json!({"command":"cort memory recall topic"})),
        tool(4, "Bash", json!({"command":"cort memory record --key x"})),
        tool(5, "Read", json!({"file_path":"src/alpha.rs"})),
        tool(6, "Read", json!({"file_path":memory_note(&transcript)})),
        tool(6, "Read", json!({"file_path":outside_note})),
        user(7, "No, wrong.\n> pasted report: we decided to delete it"),
        row(
            "attachment",
            8,
            json!({"attachment":{"type":"hook_additional_context","hookEvent":"SessionStart","hookName":"SessionStart","content":["other only"]}}),
        ),
        assistant(9, "garnet_signal"),
    ];
    write(&rows, &transcript, "one.jsonl");
    let private = tmp.path().join("private");
    let report = scan(&transcript, None, Some(&private)).unwrap();
    assert_eq!(report["total"]["windows"], 1);
    assert_eq!(report["unconfirmed_hook_runs"], 0);
    assert_eq!(report["by_source"]["compact"]["windows"], 1);
    assert_eq!(report["total"]["u1"]["status"]["events"], 1);
    assert_eq!(report["total"]["u1"]["consumption"]["events"], 1);
    assert_eq!(report["total"]["u1"]["contribution"]["events"], 1);
    assert_eq!(report["total"]["u3"]["events"], 1);
    assert_eq!(report["total"]["m2_listed_note"]["events"], 1);
    assert_eq!(report["total"]["m2"]["events"], 1);
    assert_eq!(report["total"]["m3"]["events"], 1);
    assert_eq!(report["total"]["u2"]["events"], 0); // after the next SessionStart
    assert!(report["machine"]["id"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    assert!(report["machine"]["source"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    assert_eq!(report["total"]["partial_entries"], 1);
    assert_eq!(report["total"]["entries_left_out_windows"], 1);
    assert_eq!(report["total"]["notes_left_out_windows"], 1);
    let public = report.to_string();
    for secret in [
        "secret_key",
        "claim with",
        "private-note.md",
        "note-1",
        "alpha.rs",
        "project",
    ] {
        assert!(!public.contains(secret), "{secret}");
    }
    let detail = fs::read_to_string(private.join("windows.jsonl")).unwrap();
    assert!(detail.contains("secret_key"));
    assert!(detail.contains("private-note.md"));
    assert_eq!(
        fs::read_to_string(private.join("salt"))
            .unwrap()
            .trim()
            .len(),
        64
    );
}
#[test]
fn token_history_and_ordering() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("AGENTS.md"), "instruction_tag").unwrap();
    fs::write(project.join("CLAUDE.md"), "ordinary").unwrap();
    let text = format!("{HEADER}\n- [k] `prior_tag` `summary_tag` `other_tag` `instruction_tag` `fresh_tag` `lookup_tag` (src/a.rs:3, source_unchanged)");
    let mut rows = vec![
        assistant(0, "prior_tag"),
        user(1, "compaction summary summary_tag"),
        hook(2, "resume", &text),
        attach(3, &text, "other_tag", &project),
        assistant(
            4,
            "prior_tag summary_tag other_tag instruction_tag fresh_tag",
        ),
        tool(5, "Read", json!({"file_path":"lookup_tag"})),
        tool(6, "Read", json!({"file_path":"src/a.rs"})),
    ];
    write(&rows, &root, "one.jsonl");
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["u2"]["events"], 1);
    assert_eq!(report["total"]["u2_lookup"]["events"], 1);
    assert_eq!(report["total"]["u3"]["events"], 1);
    assert_eq!(report["total"]["m1"]["events"], 0); // reuse came first
    rows[4] = assistant(4, "ordinary");
    write(&rows, &root, "one.jsonl");
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["m1"]["events"], 1);
}
#[test]
fn exclusions_and_correction_paste() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let mut side = attach(1, &injection(), "", &project);
    side["isSidechain"] = json!(true);
    write(&[side], &root, "side.jsonl");
    write(
        &[attach(1, &injection(), "", &project)],
        &root.join("subagents"),
        "child.jsonl",
    );
    write(
        &[
            attach(1, &injection(), "", &project),
            user(2, "Review this\n> wrong we decided"),
        ],
        &root,
        "main.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["windows"], 1);
    assert_eq!(report["total"]["m3"]["events"], 0);
    assert_eq!(report["total"]["no_instruction_file_windows"], 1);
}
#[test]
fn private_rejects_git_worktree() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    fs::create_dir_all(&root).unwrap();
    let git = tmp.path().join("repo");
    fs::create_dir_all(&git).unwrap();
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&git)
        .arg("init")
        .output()
        .unwrap()
        .status
        .success());
    assert!(scan(&root, None, Some(&git.join("detail")))
        .unwrap_err()
        .contains("git worktree"));
    assert!(scan(&root, None, Some(&tmp.path().join("outside"))).is_ok());
}

#[test]
fn compact_resets_prior_but_keeps_summary() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text =
        format!("{HEADER}\n- [key] `garnet_signal` `summary_tag` (src/a.rs:3, source_unchanged)");
    let mut summary = user(4, "summary_tag");
    summary["isCompactSummary"] = json!(true);
    write(
        &[
            hook(0, "startup", &text),
            attach(1, &text, "", tmp.path()),
            assistant(2, "garnet_signal"),
            summary,
            row("system", 5, json!({"subtype":"compact_boundary"})),
            hook(6, "compact", &text),
            attach(7, &text, "", tmp.path()),
            assistant(8, "garnet_signal summary_tag"),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["windows"], 2);
    // One use before the compaction, one after it; the summary still blocks summary_tag.
    assert_eq!(report["total"]["u2"]["events"], 2);
    assert_eq!(report["total"]["u2"]["windows"], 2);
    assert_eq!(report["total"]["u2_distinct_session_entries"], 1);
}

#[test]
fn resume_keeps_prior_agent_words() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] `garnet_signal` (src/a.rs:3, source_unchanged)");
    write(
        &[
            hook(0, "startup", &text),
            attach(1, &text, "", tmp.path()),
            assistant(2, "garnet_signal"),
            hook(3, "resume", &text),
            attach(4, &text, "", tmp.path()),
            assistant(5, "garnet_signal"),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["u2"]["events"], 1);
    assert_eq!(report["total"]["u2"]["windows"], 1);
}

#[test]
fn hook_events_group_and_stop_windows() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] `garnet_signal` (src/a.rs:3, source_unchanged)");
    write(
        &[
            hook(0, "startup", &text),
            hook(1, "startup", &text),
            attach(2, &text, "", tmp.path()),
            hook(3, "resume", &text),
            hook(4, "resume", &text),
            assistant(5, "garnet_signal"),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["unconfirmed_hook_runs"], 1);
    assert_eq!(report["total"]["windows"], 1);
    assert_eq!(report["total"]["u2"]["events"], 0);
}

#[test]
fn event_rates_are_bounded_and_hex_needs_a_letter() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] 123456789 (src/a.rs:3, source_unchanged)");
    let mut rows = Vec::new();
    for i in 0..5 {
        let second = i * 4;
        rows.push(hook(second, "clear", &text));
        rows.push(attach(second + 1, &text, "", tmp.path()));
        rows.push(tool(
            second + 2,
            "Bash",
            json!({"command":"cort memory status"}),
        ));
        rows.push(tool(
            second + 3,
            "Bash",
            json!({"command":"cort memory status"}),
        ));
        rows.push(assistant(second + 3, "123456789"));
    }
    write(&rows, &root, "one.jsonl");
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["u1"]["status"]["events"], 10);
    assert_eq!(report["total"]["u1"]["status"]["windows"], 5);
    assert_eq!(report["total"]["u1"]["status"]["rate"], 1.0);
    assert_eq!(report["total"]["u2"]["events"], 0);
}

#[test]
fn note_id_is_a_token_and_other_hook_stdout_is_not_visible() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = injection();
    let other_hook = hook(0, "startup", "note-1");
    write(
        &[
            other_hook,
            hook(1, "startup", &text),
            attach(2, &text, "", tmp.path()),
            assistant(3, "note-1"),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["u2"]["events"], 1);
}

#[test]
fn bookkeeping_rows_are_ignored_and_visible_rows_block_reuse() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!(
        "{HEADER}\n- [key] `garnet_signal` `file_tag` `paste_tag` (src/a.rs:3, source_unchanged)"
    );
    let mut blob = row("api-request-blob", 2, json!({"message":{"content":text}}));
    blob["hash"] = json!("h");
    write(
        &[
            hook(0, "startup", &text),
            attach(1, &text, "", tmp.path()),
            blob,
            row(
                "attachment",
                3,
                json!({"attachment":{"type":"file","content":"file_tag"}}),
            ),
            user(4, "look at this\n> paste_tag"),
            assistant(5, "garnet_signal file_tag paste_tag"),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["u2"]["events"], 1);
}

#[test]
fn only_events_that_ran_hook_recall_can_be_unconfirmed() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let mut other = hook(0, "startup", "");
    other["attachment"]["command"] =
        json!("\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" session-start");
    let mut ours = hook(9, "resume", "");
    ours["attachment"]["command"] = json!("/opt/bin/cort hook-recall --harness claude-code");
    write(&[other, ours], &root, "one.jsonl");
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["hook_recall_runs"], 1);
    assert_eq!(report["unconfirmed_hook_runs"], 1);
}

#[test]
fn a_deleted_memory_note_read_still_counts() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    fs::create_dir_all(&root).unwrap();
    let gone = fs::canonicalize(&root)
        .unwrap()
        .join("projects/p/memory/gone.md");
    write(
        &[
            hook(0, "startup", &injection()),
            attach(1, &injection(), "", tmp.path()),
            tool(2, "Read", json!({"file_path":gone})),
            tool(
                3,
                "Read",
                json!({"file_path":"/elsewhere/projects/p/memory/x.md"}),
            ),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["m2"]["events"], 1);
}

#[test]
fn large_transcript_scan_finishes_in_debug_budget() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] `garnet_signal` (src/a.rs:3, source_unchanged)");
    let mut rows = Vec::with_capacity(20_000);
    for i in 0..20_000 {
        // One startup, then resumes: every window reaches back to the first row.
        if i % 650 == 0 && i / 650 < 30 {
            rows.push(hook(0, if i == 0 { "startup" } else { "resume" }, &text));
        } else if i % 650 == 1 && i / 650 < 30 {
            rows.push(attach(1, &text, "", tmp.path()));
        } else {
            rows.push(assistant(2, "ordinary text"));
        }
    }
    write(&rows, &root, "large.jsonl");
    let started = Instant::now();
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["windows"], 30);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "scan took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_hook_event_spans_interleaved_rows_and_slow_hooks() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] `garnet_signal` (src/a.rs:3, source_unchanged)");
    let mut other = hook(9, "startup", "");
    other["attachment"]["command"] = json!("run-hook.cmd session-start");
    write(
        &[
            hook(0, "startup", &text),
            row(
                "attachment",
                1,
                json!({"attachment":{"type":"file","content":"unrelated"}}),
            ),
            other,
            attach(10, &text, "", tmp.path()),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["hook_recall_runs"], 1);
    assert_eq!(report["unconfirmed_hook_runs"], 0);
    assert_eq!(report["by_source"]["startup"]["windows"], 1);
}

#[test]
fn shell_purpose_and_source_reads_follow_the_program_that_runs() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] claim (src/alpha.rs:3, source_unchanged)");
    write(
        &[
            hook(0, "startup", &text),
            attach(1, &text, "", tmp.path()),
            tool(
                2,
                "Bash",
                json!({"command":"echo \"cort memory recall x\""}),
            ),
            tool(
                3,
                "Bash",
                json!({"command":"cort memory record --key k --source src/alpha.rs --claim grep"}),
            ),
            tool(
                4,
                "Bash",
                json!({"command":"CI=1 nice -n 15 /opt/bin/cort memory notes show abc && grep -n x src/alpha.rs"}),
            ),
            tool(
                5,
                "Bash",
                json!({"command":"echo \"done; cort memory recall topic\""}),
            ),
            tool(
                6,
                "Bash",
                json!({"command":"grep -E 'alpha|beta' src/alpha.rs"}),
            ),
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, None, None).unwrap();
    assert_eq!(report["total"]["u1"]["consumption"]["events"], 1);
    assert_eq!(report["total"]["u1"]["contribution"]["events"], 1);
    assert_eq!(report["total"]["u3"]["events"], 2);
}

#[test]
fn only_a_recall_that_returned_the_entry_suppresses_m1() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [entry_key] claim (src/alpha.rs:3, source_unchanged)");
    let call = |second: usize, id: &str| {
        row(
            "assistant",
            second,
            json!({"message":{"content":[{"type":"tool_use","id":id,"name":"Bash","input":{"command":"cort memory recall topic"}}]}}),
        )
    };
    let result = |second: usize, id: &str, body: &str| {
        row(
            "user",
            second,
            json!({"message":{"content":[{"type":"tool_result","tool_use_id":id,"content":body}]}}),
        )
    };
    let read = tool(9, "Read", json!({"file_path":"src/alpha.rs"}));
    let scan_with = |body: &str| {
        write(
            &[
                hook(0, "startup", &text),
                attach(1, &text, "", tmp.path()),
                call(2, "t1"),
                result(3, "t1", body),
                read.clone(),
            ],
            &root,
            "one.jsonl",
        );
        scan(&root, None, None).unwrap()["total"]["m1"]["events"].clone()
    };
    assert_eq!(scan_with("{\"hits\":[{\"key\":\"other\"}]}"), 1);
    assert_eq!(scan_with("{\"hits\":[{\"key\":\"entry_key\"}]}"), 0);
}

#[test]
fn since_counts_undated_rows_instead_of_guessing() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    let text = format!("{HEADER}\n- [key] claim (src/a.rs:3, source_unchanged)");
    let mut undated = attach(30, &text, "", tmp.path());
    undated.as_object_mut().unwrap().remove("timestamp");
    write(
        &[
            attach(1, &text, "", tmp.path()),
            hook(20, "resume", &text),
            attach(21, &text, "", tmp.path()),
            undated,
        ],
        &root,
        "one.jsonl",
    );
    let report = scan(&root, Some("2026-10-27T00:00:10Z"), None).unwrap();
    assert_eq!(report["total"]["windows"], 1);
    assert_eq!(report["by_source"]["resume"]["windows"], 1);
    assert_eq!(report["undated_skipped"]["windows"], 1);
    assert_eq!(report["undated_skipped"]["hook_runs"], 0);
    assert_eq!(report["hook_recall_runs"], 1);
}

#[test]
fn a_refused_private_directory_is_never_created_and_the_salt_is_reused() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("transcripts");
    fs::create_dir_all(&root).unwrap();
    let git = tmp.path().join("repo");
    fs::create_dir_all(&git).unwrap();
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&git)
        .arg("init")
        .output()
        .unwrap()
        .status
        .success());
    assert!(scan(&root, None, Some(&git.join("a/b"))).is_err());
    assert!(!git.join("a").exists());
    let private = tmp.path().join("private");
    scan(&root, None, Some(&private)).unwrap();
    let first = fs::read_to_string(private.join("salt")).unwrap();
    scan(&root, None, Some(&private)).unwrap();
    assert_eq!(fs::read_to_string(private.join("salt")).unwrap(), first);
    use std::os::unix::fs::PermissionsExt;
    for name in ["salt", "windows.jsonl"] {
        fs::set_permissions(private.join(name), fs::Permissions::from_mode(0o644)).unwrap();
    }
    scan(&root, None, Some(&private)).unwrap();
    assert_eq!(
        fs::metadata(private.join("salt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
    let mode = fs::metadata(private.join("windows.jsonl"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0);
}
