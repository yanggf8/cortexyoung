use cort::measure::*;
use cort::measure_store::{CompactionRow, MeasureStore, SqliteStore, WindowCounts};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// 2026-10-19T00:00:00Z is a Monday, the first instant of ISO week 2026-W43.
const MON_W43_MS: i64 = 1_792_368_000_000;
const THU_W43_MS: i64 = 1_792_670_400_000; // 2026-10-22 12:00Z
const FRI_W43_MS: i64 = 1_792_756_800_000; // 2026-10-23 12:00Z

fn w(tool_calls: i64, rereads: i64, repeated: i64, lookups: i64, corrections: i64) -> WindowCounts {
    WindowCounts {
        tool_calls,
        rereads,
        repeated_commands: repeated,
        transcript_lookups: lookups,
        human_messages: 0,
        corrections,
    }
}

fn row(machine: &str, uuid: &str, t: i64, pre: WindowCounts, post: WindowCounts) -> CompactionRow {
    CompactionRow {
        machine_id: machine.into(),
        boundary_uuid: uuid.into(),
        session_id: format!("s-{uuid}"),
        time_ms: t,
        project_id: None,
        trigger: "auto".into(),
        pre_tokens: None,
        post_tokens: None,
        pre,
        post,
        post_complete: true,
    }
}

fn store() -> (tempfile::TempDir, SqliteStore) {
    let tmp = tempfile::tempdir().unwrap();
    let s = SqliteStore::open(&tmp.path().join("s.db")).unwrap();
    (tmp, s)
}

fn inputs(now_ms: i64) -> ReportInputs {
    ReportInputs {
        days: 7,
        compaction_days: 30,
        now_ms,
        offline: None,
    }
}

fn host_json<'a>(v: &'a Value, host: &str) -> &'a Value {
    v["compactions"]["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["host"] == host)
        .unwrap_or_else(|| panic!("no host {host} in {v}"))
}

#[test]
fn report_compaction_section_shows_post_minus_pre() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "h1", 1).unwrap();
    // deltas: rereads 3 / 1, repeated 0 / 3, lookups 2 / 0, corrections 0 / 3
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(10, 2, 1, 0, 1),
        w(10, 5, 1, 2, 1),
    ))
    .unwrap();
    s.upsert_compaction(&row(
        "m1",
        "b",
        THU_W43_MS,
        w(10, 4, 2, 1, 0),
        w(10, 5, 5, 1, 3),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let h = host_json(&v, "h1");
    assert_eq!(h["compactions"], 2);
    assert_eq!(h["no_baseline"], 0);
    let m = &h["metrics"];
    assert_eq!(m["rereads"]["median"].as_f64(), Some(2.0));
    assert_eq!(m["rereads"]["sum"], 4);
    assert_eq!(m["repeated_commands"]["median"].as_f64(), Some(1.5));
    assert_eq!(m["repeated_commands"]["sum"], 3);
    assert_eq!(m["transcript_lookups"]["median"].as_f64(), Some(1.0));
    assert_eq!(m["transcript_lookups"]["sum"], 2);
    assert_eq!(m["corrections"]["median"].as_f64(), Some(1.5));
    assert_eq!(m["corrections"]["sum"], 3);
    let text = render_report(&v, false);
    assert!(text.contains("h1"), "{text}");
}

#[test]
fn zero_length_pre_window_is_counted_apart_not_compared() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "h1", 1).unwrap();
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(0, 0, 0, 0, 0),
        w(10, 9, 9, 9, 9),
    ))
    .unwrap();
    s.upsert_compaction(&row(
        "m1",
        "b",
        THU_W43_MS,
        w(10, 1, 0, 0, 0),
        w(10, 2, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let h = host_json(&v, "h1");
    assert_eq!(h["compactions"], 2);
    assert_eq!(h["no_baseline"], 1);
    assert_eq!(h["metrics"]["rereads"]["sum"], 1);
    assert!(render_report(&v, false).contains("no baseline"));
}

#[test]
fn report_weekly_trend_uses_utc_iso_weeks() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "h1", 1).unwrap();
    // Sunday 23:59:59.999Z is still W42; the next millisecond is W43.
    s.upsert_compaction(&row(
        "m1",
        "a",
        MON_W43_MS - 1,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    s.upsert_compaction(&row(
        "m1",
        "b",
        MON_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 2, 0, 0, 0),
    ))
    .unwrap();
    s.upsert_compaction(&row(
        "m1",
        "c",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 4, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let weeks = host_json(&v, "h1")["weeks"].as_array().unwrap().clone();
    assert_eq!(weeks.len(), 2, "{weeks:?}");
    assert_eq!(weeks[0]["week"], "2026-W42");
    assert_eq!(weeks[0]["compactions"], 1);
    assert_eq!(weeks[0]["rereads"], 1);
    assert_eq!(weeks[1]["week"], "2026-W43");
    assert_eq!(weeks[1]["compactions"], 2);
    assert_eq!(weeks[1]["rereads"], 6);
}

#[test]
fn iso_week_year_boundaries() {
    assert_eq!(iso_week(1_766_966_400_000 - 1), "2025-W52");
    assert_eq!(iso_week(1_766_966_400_000), "2026-W01");
    assert_eq!(iso_week(1_799_020_799_000), "2026-W53");
    assert_eq!(iso_week(1_799_020_800_000), "2027-W01");
    assert_eq!(iso_week(1_735_516_800_000), "2025-W01");
}

#[test]
fn report_without_compactions_says_so() {
    let (_t, mut s) = store();
    // Present but outside the window.
    s.upsert_compaction(&row(
        "m1",
        "old",
        1_788_264_000_000,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    assert_eq!(v["compactions"]["total"], 0);
    assert!(render_report(&v, false).contains("這段期間沒有壓縮"));
}

#[test]
fn unmapped_machine_shown_by_prefix() {
    let (_t, mut s) = store();
    s.upsert_compaction(&row(
        "abcdef1234567890",
        "a",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let h = host_json(&v, "machine abcdef12");
    assert_eq!(h["compactions"], 1);
}

#[test]
fn same_host_two_machines_warns() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "box", 1).unwrap();
    s.upsert_machine_host("m2", "box", 2).unwrap();
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    s.upsert_compaction(&row(
        "m2",
        "b",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let warns = v["warnings"].as_array().unwrap();
    assert!(
        warns.iter().any(|x| {
            let x = x.as_str().unwrap();
            x.contains("box") && x.contains("2 machine ids") && x.contains("m1") && x.contains("m2")
        }),
        "{warns:?}"
    );
    // Both machines are one host row, not two.
    assert_eq!(host_json(&v, "box")["compactions"], 2);
    assert!(render_report(&v, false).contains("2 machine ids"));
}

#[test]
fn renamed_machine_warns() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "old-name", 100).unwrap();
    s.upsert_machine_host("m1", "new-name", 200).unwrap();
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let warns = v["warnings"].as_array().unwrap();
    assert!(
        warns.iter().any(|x| {
            let x = x.as_str().unwrap();
            x.contains("m1")
                && x.contains("2 hosts")
                && x.contains("old-name")
                && x.contains("new-name")
        }),
        "{warns:?}"
    );
    // The latest-seen host wins the row.
    assert_eq!(host_json(&v, "new-name")["compactions"], 1);
}

fn line(v: Value) -> String {
    format!("{v}\n")
}

fn transcript_with_one_compaction() -> String {
    let mut t = String::new();
    for i in 0..3 {
        t.push_str(&line(json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t","name":"Bash","input":{"command":format!("pre {i}")}}]}})));
    }
    t.push_str(&line(
        json!({"type":"system","subtype":"compact_boundary","uuid":"b-1","sessionId":"sess-1",
        "timestamp":"2026-10-22T23:37:47.345Z","cwd":"/nonexistent/cwd",
        "compactMetadata":{"trigger":"auto","preTokens":100,"postTokens":10}}),
    ));
    t.push_str(&line(json!({"type":"assistant","message":{"content":[
        {"type":"tool_use","id":"t","name":"Bash","input":{"command":"post"}}]}})));
    t
}

struct Rec {
    _tmp: tempfile::TempDir,
    input: RecordInputs,
}

fn rec_env() -> Rec {
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("projects");
    fs::create_dir_all(projects.join("p1")).unwrap();
    fs::write(
        projects.join("p1/sess-1.jsonl"),
        transcript_with_one_compaction(),
    )
    .unwrap();
    let input = RecordInputs {
        projects_dir: projects,
        marks_path: tmp.path().join("marks.json"),
        machine_id: "m1".into(),
        host: "box".into(),
        now_ms: 5_000,
        events: None,
    };
    Rec { _tmp: tmp, input }
}

#[test]
fn record_order_writes_before_push() {
    let (_t, mut s) = store();
    s.fail_push(true);
    let r = rec_env();
    let out = record(&mut s, &r.input).unwrap();
    assert!(
        matches!(&out, RecordOutcome::SavedLocallyPushFailed(m) if !m.is_empty()),
        "{out:?}"
    );
    assert_eq!(s.compactions_since(0).unwrap().len(), 1);
    let hosts = s.machine_hosts().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].host, "box");
    assert_eq!(hosts[0].last_seen_ms, 5_000);
    // The local writes succeeded, so the watermark moved even though the push did not.
    assert!(r.input.marks_path.exists());
}

#[test]
fn record_pushes_when_store_accepts() {
    let (_t, mut s) = store();
    let r = rec_env();
    assert_eq!(record(&mut s, &r.input).unwrap(), RecordOutcome::Pushed);
    assert_eq!(s.compactions_since(0).unwrap().len(), 1);
    // A second run is idempotent.
    assert_eq!(record(&mut s, &r.input).unwrap(), RecordOutcome::Pushed);
    assert_eq!(s.compactions_since(0).unwrap().len(), 1);
}

#[test]
fn offline_report_first_line() {
    let (_t, mut s) = store();
    let mut inp = inputs(FRI_W43_MS);
    inp.offline = Some(format_utc(1_792_717_320_000));
    let v = report(&mut s, &inp).unwrap();
    let text = render_report(&v, false);
    assert_eq!(
        text.lines().next().unwrap(),
        "離線：讀本機副本，最後同步於 2026-10-23 01:02 UTC"
    );
    let mut online = inputs(FRI_W43_MS);
    online.offline = None;
    let v = report(&mut s, &online).unwrap();
    assert!(!render_report(&v, false).contains("離線"));
}

#[test]
fn color_only_when_asked() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "h1", 1).unwrap();
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    assert!(!render_report(&v, false).contains('\u{1b}'));
    assert!(render_report(&v, true).contains('\u{1b}'));
}

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

#[test]
fn record_without_config_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let home = tmp.path().join("home");
    let cache = tmp.path().join("cache");
    for d in [&data, &home, &cache] {
        fs::create_dir_all(d).unwrap();
    }
    let out = Command::new(cort_bin())
        .args(["measure", "record"])
        .env("CORT_MEMORY_DIR", &data)
        .env("HOME", &home)
        .env("CORT_CACHE_DIR", &cache)
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cort measure configure"), "{stdout}");
    let left: Vec<_> = fs::read_dir(&data)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "record left behind {left:?}");
    assert!(!Path::new(&data.join("measure-last-sync")).exists());
}

#[test]
fn same_host_warns_even_after_one_machine_was_renamed() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "A", 100).unwrap();
    s.upsert_machine_host("m1", "B", 200).unwrap();
    s.upsert_machine_host("m2", "A", 150).unwrap();
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(5, 0, 0, 0, 0),
        w(5, 1, 0, 0, 0),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    let warns: Vec<&str> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(
        warns.iter().any(|x| x.contains("host A")
            && x.contains("2 machine ids")
            && x.contains("m1")
            && x.contains("m2")),
        "{warns:?}"
    );
    assert!(
        warns
            .iter()
            .any(|x| x.contains("machine m1") && x.contains("2 hosts")),
        "{warns:?}"
    );
}

#[test]
fn recorded_host_without_compactions_still_gets_a_section() {
    let (_t, mut s) = store();
    s.upsert_machine_host("m1", "busy", 1).unwrap();
    s.upsert_machine_host("m2", "quiet", 2).unwrap();
    s.upsert_compaction(&row(
        "m1",
        "a",
        THU_W43_MS,
        w(10, 2, 1, 0, 1),
        w(10, 5, 1, 2, 1),
    ))
    .unwrap();
    let v = report(&mut s, &inputs(FRI_W43_MS)).unwrap();
    assert_eq!(v["compactions"]["total"], 1);
    let quiet = host_json(&v, "quiet");
    assert_eq!(quiet["compactions"], 0);
    assert_eq!(quiet["compared"], 0);
    assert_eq!(quiet["machine_ids"], json!(["m2"]));
    assert!(quiet["weeks"].as_array().unwrap().is_empty());
    assert_eq!(host_json(&v, "busy")["compactions"], 1);
    let text = render_report(&v, false);
    let quiet_at = text.find("quiet").unwrap();
    assert!(
        text[quiet_at..].contains("這段期間沒有壓縮"),
        "empty host must say so under its own section: {text}"
    );
    assert!(
        !text[..text.find("quiet").unwrap()].contains("這段期間沒有壓縮"),
        "the busy host must not claim it: {text}"
    );
}

#[test]
fn record_surfaces_scan_summary_in_both_outcomes() {
    for fail in [false, true] {
        let (_t, mut s) = store();
        s.fail_push(fail);
        let r = rec_env();
        let mut body = fs::read_to_string(r.input.projects_dir.join("p1/sess-1.jsonl")).unwrap();
        body.push_str("not json\n{\"type\":\"brand-new-thing\"}\n");
        fs::write(r.input.projects_dir.join("p1/sess-1.jsonl"), body).unwrap();
        let (scan, outcome) = record_with_scan(&mut s, &r.input).unwrap();
        assert_eq!(matches!(outcome, RecordOutcome::Pushed), !fail);
        assert_eq!(
            scan.to_json(),
            json!({"files_seen": 1, "files_scanned": 1, "compactions": 1,
                "skipped_lines": 1, "unknown_type_lines": 1})
        );
    }
}

#[test]
fn record_pull_failure_still_returns_scan() {
    let (_t, mut s) = store();
    s.fail_pull(true);
    let r = rec_env();
    let (scan, outcome) = record_with_scan(&mut s, &r.input).unwrap();
    assert!(
        matches!(&outcome, RecordOutcome::PushedPullFailed(m) if !m.is_empty()),
        "{outcome:?}"
    );
    assert_eq!(scan.compactions, 1);
    assert_eq!(scan.files_scanned, 1);
    // The plain wrapper keeps its old contract: a pull failure is an error.
    assert!(record(&mut s, &r.input)
        .unwrap_err()
        .contains("pull failed"));
}

// ---------------------------------------------------------------------------------------------
// record + events: the event step may refuse or fail without costing the compaction record
// ---------------------------------------------------------------------------------------------

fn usage_db_with_rows(dir: &Path) -> PathBuf {
    let path = dir.join("usage.db");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(include_str!("../src/usage_schema.sql"))
        .unwrap();
    c.execute_batch(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,bytes_out)
         VALUES (1,1760000000000,NULL,'m1','env','impact','a','ok',1)",
    )
    .unwrap();
    path
}

fn with_events(r: &mut Rec, usage_db: PathBuf) {
    r.input.events = Some(cort::telemetry::UploadInputs {
        usage_db,
        cost_dir: r._tmp.path().join("no-cost"),
        marker_path: r._tmp.path().join("verified.json"),
        machine_id: "m1".into(),
    });
}

#[test]
fn record_keeps_compactions_when_events_refuse() {
    let (_t, mut s) = store();
    let mut r = rec_env();
    let db = usage_db_with_rows(r._tmp.path());
    with_events(&mut r, db);
    let (scan, outcome) = record_with_scan(&mut s, &r.input).unwrap();
    assert_eq!(outcome, RecordOutcome::Pushed);
    assert_eq!(scan.compactions, 1);
    assert_eq!(s.compactions_since(0).unwrap().len(), 1);
    let events = scan.to_json()["events"].clone();
    assert_eq!(events["status"], "refused", "{events}");
    assert!(events["reason"].as_str().unwrap().contains("--dry-run"));
    // Nothing reached the store: no events, no watermark.
    assert!(s.remote_ids("m1", "cort_command").unwrap().is_empty());
    assert_eq!(s.event_watermark("m1", "cort_command").unwrap(), None);
}

#[test]
fn record_keeps_compactions_when_events_fail() {
    let (_t, mut s) = store();
    let mut r = rec_env();
    let bad = r._tmp.path().join("usage.db");
    fs::write(&bad, "this is not a sqlite file").unwrap();
    s.set_event_watermark("m1", "cort_command", 0, "stale")
        .unwrap(); // past the gate, so the read fails
    with_events(&mut r, bad);
    let (scan, outcome) = record_with_scan(&mut s, &r.input).unwrap();
    assert_eq!(outcome, RecordOutcome::Pushed);
    assert_eq!(s.compactions_since(0).unwrap().len(), 1);
    assert_eq!(scan.to_json()["events"]["status"], "failed");
}

#[test]
fn record_uploads_events_before_the_push() {
    let (_t, mut s) = store();
    let mut r = rec_env();
    let db = usage_db_with_rows(r._tmp.path());
    s.set_event_watermark("m1", "cort_command", 0, "stale")
        .unwrap();
    with_events(&mut r, db);
    s.fail_push(true);
    let (scan, outcome) = record_with_scan(&mut s, &r.input).unwrap();
    // Push failed, yet the event is already in the local replica and the watermark moved.
    assert!(matches!(outcome, RecordOutcome::SavedLocallyPushFailed(_)));
    let events = scan.to_json()["events"].clone();
    assert_eq!(events["status"], "uploaded");
    assert_eq!(events["inserted"], 1);
    assert_eq!(
        s.event_watermark("m1", "cort_command")
            .unwrap()
            .map(|w| w.0),
        Some(1)
    );
}

#[test]
fn record_without_events_input_has_no_events_key() {
    let (_t, mut s) = store();
    let r = rec_env();
    let (scan, _) = record_with_scan(&mut s, &r.input).unwrap();
    assert!(scan.to_json().get("events").is_none());
}
