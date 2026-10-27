//! `cort measure import-audit`: the parse of claudecat's retired `CORT-AUDIT.md`, the health event
//! shape, and the re-runnable import into a store.
//!
//! The fixture is the real table's shape with its roots rewritten to `/home/u/...` and
//! `/Users/u/...`: no developer's absolute path may live in the repo.

use cort::audit_import::{audit_health_event, import_audit, parse_audit_md};
use cort::measure_store::{MeasureStore, SqliteStore};
use cort::telemetry::{health_event_id, sorted_compact_json, SOURCE_HEALTH};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// 2026-09-07 00:00 UTC.
const SEP_07_MS: i64 = 1_788_739_200_000;

const HEADER: &str = "| 日期 | 專案 | host | fresh | chunks | relationships | 未chunk檔 | 真缺口 | FTS drift | 命令數/30d | core/30d | deep/30d | 命令數/7d | deep/7d | decline-top |\n|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|\n";

const SHORT_0906: &str = "| 2026-09-06 | `/home/u/a/claudecat` | fresh | 586 | 310 | 5 | ? | synced | 12051 | 473 | 3 | 12035 | 2 |\n";

const FULL_0907: &str = "| 2026-09-07 | `/home/u/a/claudecat` | NUC11i5 | fresh | 605 | 341 | 5 | ? | synced | 13413 | 473 | 3 | 13397 | 2 | not_a_search_tool=165 |\n";

const STALE_1020: &str = "| 2026-10-20 | `/home/u/a/claudecat` | NUC11i5 | STALE/refresh | 786 | 704 | 5 | 0 | synced | 115093 | 6 | 0 | 31964 | 0 | pattern_not_symbol=15520 |\n";

const MAC_1025: &str = "| 2026-10-25 | `/Users/u/a/claudecat` | MacBook-Pro.local | fresh | 40 | 12 | 2 | 3 | drift=4 | ? | ? | - | 7 | 1 | ? |\n";

const NO_INDEX: &str = "| 2026-10-25 | `/home/u/a/claudecat` | i51149R3050 | no-index | - | - | 5 | ? | ? | 859 | 16 | 2 | 828 | 0 | unparseable_command=28 |\n";

/// A whole file in the real table's shape: a title, the header, and the five rows used here.
fn audit_md() -> String {
    format!("# CORT-AUDIT\n\n{HEADER}{SHORT_0906}{FULL_0907}{STALE_1020}{MAC_1025}{NO_INDEX}")
}

fn store() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let s = SqliteStore::open(&dir.path().join("measure.db")).unwrap();
    (dir, s)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn payload_without_ts(payload: &str) -> String {
    let mut v: Value = serde_json::from_str(payload).unwrap();
    v.as_object_mut().unwrap().remove("ts");
    sorted_compact_json(&v)
}

#[test]
fn parses_full_rows() {
    let rows = parse_audit_md(&audit_md()).unwrap();
    assert_eq!(rows.len(), 5);

    let r = &rows[1];
    assert_eq!(r.line, 6);
    assert_eq!(r.day, "2026-09-07");
    assert_eq!(r.host, "NUC11i5");
    assert_eq!(r.root, "/home/u/a/claudecat");
    assert_eq!(r.status, "ok");
    assert_eq!(r.fresh.as_deref(), Some("fresh"));
    assert_eq!(r.chunks, Some(605));
    assert_eq!(r.relationships, Some(341));
    assert_eq!(r.not_chunked, Some(5));
    assert_eq!(r.real_gap, None);
    assert_eq!(r.fts.as_deref(), Some("synced"));
    assert_eq!(r.usage.day, "2026-09-07");
    assert_eq!(r.usage.host, "NUC11i5");
    assert_eq!(r.usage.commands_window, Some(13413));
    assert_eq!(r.usage.core_window, Some(473));
    assert_eq!(r.usage.deep_window, Some(3));
    assert_eq!(r.usage.commands_7d, Some(13397));
    assert_eq!(r.usage.deep_7d, Some(2));
    assert_eq!(
        r.usage.decline_top.as_deref(),
        Some("not_a_search_tool=165")
    );
    assert_eq!(r.usage.window_days, 30);

    // A second host spelling and a non-`synced` FTS cell survive as text.
    assert_eq!(rows[3].host, "MacBook-Pro.local");
    assert_eq!(rows[3].root, "/Users/u/a/claudecat");
    assert_eq!(rows[3].fts.as_deref(), Some("drift=4"));
    assert_eq!(rows[2].fresh.as_deref(), Some("STALE/refresh"));
    assert_eq!(rows[2].status, "ok");
    assert_eq!(rows[2].real_gap, Some(0));
}

#[test]
fn parses_short_0906_row_as_unknown_host() {
    let rows = parse_audit_md(&audit_md()).unwrap();
    let r = &rows[0];
    assert_eq!(r.line, 5);
    assert_eq!(r.day, "2026-09-06");
    assert_eq!(r.host, "unknown");
    assert_eq!(r.usage.host, "unknown");
    assert_eq!(r.usage.decline_top, None);
    assert_eq!(r.chunks, Some(586));
    assert_eq!(r.relationships, Some(310));
    assert_eq!(r.not_chunked, Some(5));
    assert_eq!(r.fts.as_deref(), Some("synced"));
    assert_eq!(r.usage.commands_window, Some(12051));
    assert_eq!(r.usage.core_window, Some(473));
    assert_eq!(r.usage.deep_window, Some(3));
    assert_eq!(r.usage.commands_7d, Some(12035));
    assert_eq!(r.usage.deep_7d, Some(2));
    assert_eq!(r.usage.window_days, 30);
}

#[test]
fn question_mark_is_null_not_zero() {
    let rows = parse_audit_md(&audit_md()).unwrap();
    // `?` on a health cell is "could not read", never 0.
    assert_eq!(rows[0].real_gap, None);
    assert_eq!(rows[1].real_gap, None);
    // `?` and `-` on usage cells and the decline cell are both unreadable.
    let mac = &rows[3].usage;
    assert_eq!(mac.commands_window, None);
    assert_eq!(mac.core_window, None);
    assert_eq!(mac.deep_window, None);
    assert_eq!(mac.decline_top, None);
    // The cells that do hold numbers keep them.
    assert_eq!(rows[3].chunks, Some(40));
    assert_eq!(rows[3].not_chunked, Some(2));
    assert_eq!(rows[3].real_gap, Some(3));
    assert_eq!(mac.commands_7d, Some(7));
    assert_eq!(mac.deep_7d, Some(1));
}

#[test]
fn no_index_label_maps_to_status() {
    let rows = parse_audit_md(&audit_md()).unwrap();
    let r = &rows[4];
    assert_eq!(r.status, "no_index");
    assert_eq!(r.fresh, None);
    assert_eq!(r.chunks, None);
    assert_eq!(r.relationships, None);
    assert_eq!(r.not_chunked, Some(5));
    assert_eq!(r.real_gap, None);
    assert_eq!(r.fts, None);
    assert_eq!(r.usage.commands_window, Some(859));
}

#[test]
fn malformed_row_errors_with_line_number() {
    // Fourteen columns: the short shape has 13 and the full shape 15.
    let wrong_width = "| 2026-09-07 | `/home/u/a/claudecat` | NUC11i5 | fresh | 605 | 341 | 5 | ? | synced | 13413 | 473 | 3 | 13397 | 2 |\n";
    let e = parse_audit_md(wrong_width).unwrap_err();
    assert!(e.contains("line 1"), "{e}");

    let not_an_integer = "| 2026-09-07 | `/home/u/a/claudecat` | NUC11i5 | fresh | abc | 341 | 5 | ? | synced | 13413 | 473 | 3 | 13397 | 2 | x |\n";
    let e = parse_audit_md(not_an_integer).unwrap_err();
    assert!(e.contains("line 1"), "{e}");
    assert!(e.contains("chunks"), "{e}");

    let unknown_label = "| 2026-09-07 | `/home/u/a/claudecat` | NUC11i5 | weird | 605 | 341 | 5 | ? | synced | 13413 | 473 | 3 | 13397 | 2 | x |\n";
    let e = parse_audit_md(unknown_label).unwrap_err();
    assert!(e.contains("line 1"), "{e}");
    assert!(e.contains("fresh"), "{e}");
}

#[test]
fn health_event_shape() {
    let rows = parse_audit_md(&audit_md()).unwrap();
    let ev = audit_health_event(&rows[1], "proj123");
    assert_eq!(ev.source, SOURCE_HEALTH);
    assert_eq!(ev.machine_id, "imported:NUC11i5");
    assert_eq!(ev.machine_source, "cort-audit-md");
    assert_eq!(ev.project_id.as_deref(), Some("proj123"));
    assert_eq!(ev.time_ms, Some(SEP_07_MS));
    assert_eq!(ev.rowid, None);
    assert_eq!(
        ev.event_id,
        health_event_id(
            &ev.machine_id,
            "2026-09-07",
            &rows[1].root,
            &payload_without_ts(&ev.payload)
        )
    );

    // Exactly the `index_health_log` columns minus `id`, plus `origin`.
    let v: Value = serde_json::from_str(&ev.payload).unwrap();
    assert_eq!(v.as_object().unwrap().len(), 15);
    let keys = [
        "chunks",
        "error",
        "fresh",
        "fts",
        "local_project_id",
        "machine_id",
        "machine_source",
        "not_chunked",
        "origin",
        "project_id",
        "real_gap",
        "relationships",
        "root",
        "status",
        "ts",
    ];
    // Alphabetical order on the raw bytes: a map's iteration order cannot show this, the string can.
    let mut at = 0usize;
    for key in keys {
        let needle = format!("\"{key}\":");
        let found = ev.payload[at..]
            .find(&needle)
            .unwrap_or_else(|| panic!("{key} missing or out of order in {}", ev.payload));
        at += found + needle.len();
    }
    // Compact, and the same serializer's output round-trips byte for byte.
    assert_eq!(ev.payload, sorted_compact_json(&v));
    assert!(!ev.payload.contains(": "), "{}", ev.payload);

    assert_eq!(v["chunks"], 605);
    assert_eq!(v["relationships"], 341);
    assert_eq!(v["not_chunked"], 5);
    assert!(v["real_gap"].is_null());
    assert_eq!(v["fresh"], "fresh");
    assert_eq!(v["fts"], "synced");
    assert_eq!(v["status"], "ok");
    assert_eq!(v["root"], "/home/u/a/claudecat");
    assert_eq!(v["machine_id"], "imported:NUC11i5");
    assert_eq!(v["machine_source"], "cort-audit-md");
    assert_eq!(v["origin"], "cort-audit-md");
    assert_eq!(v["project_id"], "proj123");
    assert!(v["local_project_id"].is_null());
    assert!(v["error"].is_null());
    assert_eq!(v["ts"], SEP_07_MS);
}

#[test]
fn health_event_id_is_content_keyed() {
    let rows = parse_audit_md(&audit_md()).unwrap();
    let row = &rows[1];
    let a = audit_health_event(row, "p");
    let without_ts = payload_without_ts(&a.payload);

    // The documented digest, computed independently here.
    let expected = hex(&Sha256::digest(
        format!(
            "cort_index_health\0{}\0{}\0{}\0{}",
            a.machine_id, row.day, row.root, without_ts
        )
        .as_bytes(),
    ));
    assert_eq!(a.event_id, expected);

    // A different `ts` alone leaves the id alone: `ts` is not in the hashed payload.
    let mut later_payload: Value = serde_json::from_str(&a.payload).unwrap();
    later_payload["ts"] = json!(a.time_ms.unwrap() + 60_000);
    let later = sorted_compact_json(&later_payload);
    assert_eq!(without_ts, payload_without_ts(&later));
    assert_eq!(
        a.event_id,
        health_event_id(
            &a.machine_id,
            &row.day,
            &row.root,
            &payload_without_ts(&later)
        )
    );

    // Any metric changes it.
    let mut other = row.clone();
    other.chunks = Some(606);
    assert_ne!(a.event_id, audit_health_event(&other, "p").event_id);

    // So does the day.
    let mut next_day = row.clone();
    next_day.day = "2026-09-08".to_string();
    next_day.usage.day = "2026-09-08".to_string();
    let b = audit_health_event(&next_day, "p");
    assert_ne!(a.event_id, b.event_id);
    assert_eq!(b.time_ms, Some(SEP_07_MS + 86_400_000));
}

#[test]
fn reimport_is_idempotent() {
    let (_t, mut s) = store();
    let rows = parse_audit_md(&audit_md()).unwrap();

    let first = import_audit(&mut s, &rows, "p").unwrap();
    assert_eq!(first["rows"], 5);
    assert_eq!(first["health_inserted"], 5);
    assert_eq!(first["health_already_present"], 0);
    assert_eq!(first["usage_inserted"], 5);
    assert_eq!(first["usage_unchanged"], 0);
    assert!(first["usage_conflicts"].as_array().unwrap().is_empty());
    assert_eq!(first["hosts"]["NUC11i5"], 2);
    assert_eq!(first["hosts"]["unknown"], 1);
    assert_eq!(first["hosts"]["MacBook-Pro.local"], 1);
    assert_eq!(first["hosts"]["i51149R3050"], 1);

    let second = import_audit(&mut s, &rows, "p").unwrap();
    assert_eq!(second["health_inserted"], 0);
    assert_eq!(second["health_already_present"], 5);
    assert_eq!(second["usage_inserted"], 0);
    assert_eq!(second["usage_unchanged"], 5);
    assert!(second["usage_conflicts"].as_array().unwrap().is_empty());
    assert_eq!(s.imported_usage().unwrap().len(), 5);
}

#[test]
fn usage_conflict_is_reported_not_overwritten() {
    let (_t, mut s) = store();
    let rows = parse_audit_md(&audit_md()).unwrap();
    import_audit(&mut s, &rows, "p").unwrap();

    // Same (day, host), one different counter.
    let changed = FULL_0907.replace("13413", "999");
    let incoming = parse_audit_md(&changed).unwrap();
    let report = import_audit(&mut s, &incoming, "p").unwrap();
    assert_eq!(report["usage_inserted"], 0);
    assert_eq!(report["usage_unchanged"], 0);
    // The health payload carries no usage column, so this row is the same health event.
    assert_eq!(report["health_inserted"], 0);
    let conflicts = report["usage_conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["day"], "2026-09-07");
    assert_eq!(conflicts[0]["host"], "NUC11i5");
    assert_eq!(conflicts[0]["existing"]["commands_window"], 13413);
    assert_eq!(conflicts[0]["incoming"]["commands_window"], 999);

    // The stored value is the one that got there first.
    let stored = s.imported_usage().unwrap();
    let row = stored
        .iter()
        .find(|u| u.day == "2026-09-07" && u.host == "NUC11i5")
        .unwrap();
    assert_eq!(row.commands_window, Some(13413));
}

#[test]
fn two_files_union() {
    let (_t, mut s) = store();
    let file_a = format!("# CORT-AUDIT\n\n{HEADER}{SHORT_0906}{FULL_0907}");
    let file_b = format!("# CORT-AUDIT\n\n{HEADER}{FULL_0907}{STALE_1020}");
    let mut rows = parse_audit_md(&file_a).unwrap();
    rows.extend(parse_audit_md(&file_b).unwrap());
    assert_eq!(rows.len(), 4);

    let report = import_audit(&mut s, &rows, "p").unwrap();
    assert_eq!(report["rows"], 4);
    assert_eq!(report["usage_inserted"], 3);
    assert_eq!(report["usage_unchanged"], 1);
    assert_eq!(report["health_inserted"], 3);
    assert_eq!(report["health_already_present"], 1);
    assert!(report["usage_conflicts"].as_array().unwrap().is_empty());

    let stored = s.imported_usage().unwrap();
    let keys: Vec<(String, String)> = stored
        .iter()
        .map(|u| (u.day.clone(), u.host.clone()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("2026-09-06".to_string(), "unknown".to_string()),
            ("2026-09-07".to_string(), "NUC11i5".to_string()),
            ("2026-10-20".to_string(), "NUC11i5".to_string()),
        ]
    );

    let mut counts = s.event_counts().unwrap();
    counts.sort();
    assert_eq!(
        counts,
        vec![
            (SOURCE_HEALTH.to_string(), "imported:NUC11i5".to_string(), 2),
            (SOURCE_HEALTH.to_string(), "imported:unknown".to_string(), 1),
        ]
    );
}

#[test]
fn a_non_constraint_error_is_returned_as_is() {
    // Only a key collision is read back and classified; any other failure surfaces unchanged and
    // leaves the table empty.
    let (_t, mut s) = store();
    let rows = parse_audit_md(&audit_md()).unwrap();
    s.fail_write(true);
    let err = import_audit(&mut s, &rows, "p").unwrap_err();
    assert!(err.contains("write failed"), "{err}");
    assert!(s.imported_usage().unwrap().is_empty());
}

#[test]
fn same_batch_disagreement_is_a_conflict() {
    // Two files that disagree about one (day, host): the first one read wins, the second is named.
    let (_t, mut s) = store();
    let other = FULL_0907.replace("| 13413 |", "| 13414 |");
    let md = format!("{HEADER}{FULL_0907}{other}");
    let rows = parse_audit_md(&md).unwrap();
    let report = import_audit(&mut s, &rows, "p").unwrap();
    assert_eq!(report["usage_inserted"], 1);
    let conflicts = report["usage_conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["existing"]["commands_window"], 13413);
    assert_eq!(conflicts[0]["incoming"]["commands_window"], 13414);
    assert_eq!(s.imported_usage().unwrap()[0].commands_window, Some(13413));
}
