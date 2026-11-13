//! Task 1 tests for the pure core (`cort::remind`), all against a pinned `now`.

use cort::question::Dedup;
use cort::remind::*;

const DAY: i128 = 86_400_000_000_000;
const HOUR: i128 = 3_600_000_000_000;
/// Midnight of 2026-11-21 at +08:00 = 2026-11-20T16:00Z.
const DUE: i128 = 1_795_190_400 * 1_000_000_000;
/// Midnight of 2026-11-21 at +00:00 = 2026-11-21T00:00Z.
const DUE_UTC: i128 = 1_795_219_200 * 1_000_000_000;

fn ev(id: &str, kind: EventKind, reminder: Option<&str>, at: i128) -> ReminderEvent {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "kind": kind,
        "reminder": reminder,
        "recorded_at": at.to_string(),
    }))
    .unwrap()
}

fn add(id: &str, text: &str, on: &str, due_at: i128, at: i128) -> ReminderEvent {
    let mut e = ev(id, EventKind::Add, None, at);
    e.text = Some(text.to_string());
    e.on = Some(on.to_string());
    e.utc_offset = Some("+08:00".to_string());
    e.due_at = Some(due_at.to_string());
    e.lead = Some(1);
    e
}

fn defer(id: &str, reminder: &str, on: &str, due_at: i128, at: i128) -> ReminderEvent {
    let mut e = ev(id, EventKind::Defer, Some(reminder), at);
    e.on = Some(on.to_string());
    e.utc_offset = Some("+08:00".to_string());
    e.due_at = Some(due_at.to_string());
    e
}

fn close(id: &str, reminder: &str, reason: &str, at: i128) -> ReminderEvent {
    let mut e = ev(id, EventKind::Close, Some(reminder), at);
    e.reason = Some(reason.to_string());
    e
}

fn one(events: Vec<ReminderEvent>, now: i128) -> Reminder {
    fold(events, now).reminders.remove(0)
}

#[test]
fn explicit_offset_due_at() {
    let due = parse_on("2026-11-21+08:00").unwrap();
    assert_eq!(due.due_at, DUE);
    assert_eq!(due.utc_offset, "+08:00");
    assert_eq!(due.on, "2026-11-21+08:00");
    let west = parse_on("2026-11-21-05:30").unwrap();
    assert_eq!(west.utc_offset, "-05:30");
    assert_eq!(west.due_at, DUE_UTC + (5 * 3600 + 30 * 60) * 1_000_000_000);
    assert_eq!(format_offset(0), "+00:00");
    assert_eq!(format_offset(8 * 3600), "+08:00");
    assert_eq!(format_offset(-(5 * 3600 + 30 * 60)), "-05:30");
}

#[test]
fn invalid_dates_refused() {
    for raw in [
        "2026-02-30",
        "2026-11-21+8",
        "21/11/2026",
        "2026-11-21+15:00",
    ] {
        let err = parse_on(raw).unwrap_err();
        assert!(err.starts_with("remind_date_invalid"), "{raw}: {err}");
    }
}

#[test]
fn bare_date_uses_local_tz() {
    // POSIX TZ strings, not zone names: no tzdata file is read, so a host without tzdata
    // cannot silently fall back. TZ is set for a child process -- never set_var in-process.
    for (tz, offset) in [("CST-8", "+08:00"), ("UTC0", "+00:00")] {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "tz_child", "--nocapture"])
            .env("TZ", tz)
            .env("REMIND_EXPECT_OFFSET", offset)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "TZ={tz} tz_child failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// Runs only as a subprocess of `bare_date_uses_local_tz`, which sets TZ and the expected
/// offset; `#[ignore]` keeps it out of normal runs.
#[ignore]
#[test]
fn tz_child() {
    let expect = std::env::var("REMIND_EXPECT_OFFSET").unwrap();
    let due = parse_on("2026-11-21").unwrap();
    assert_eq!(due.utc_offset, expect);
    let expected: i128 = match expect.as_str() {
        "+08:00" => DUE,
        "+00:00" => DUE_UTC,
        other => panic!("unexpected REMIND_EXPECT_OFFSET: {other}"),
    };
    assert_eq!(due.due_at, expected);
    assert_eq!(due.on, "2026-11-21");
    // An explicit offset ignores TZ entirely.
    let explicit = parse_on("2026-11-21+00:00").unwrap();
    assert_eq!(explicit.utc_offset, "+00:00");
    assert_eq!(explicit.due_at, DUE_UTC);
}

#[test]
fn states_and_labels() {
    fn with_lead(lead: u32, now: i128) -> Reminder {
        let mut a = add("a1", "task", "2026-11-21+08:00", DUE, 1);
        a.lead = Some(lead);
        one(vec![a], now)
    }
    // 11-19 12:00+08, lead 1: a day and a half out -> scheduled, in 2d.
    let r = with_lead(1, DUE - DAY - DAY / 2);
    assert_eq!(r.state, State::Scheduled);
    assert_eq!(r.label, "in 2d");
    assert_eq!(r.overdue_days, None);
    // 11-20 12:00+08, lead 1: inside the lead window -> upcoming, tomorrow.
    let r = with_lead(1, DUE - DAY / 2);
    assert_eq!(r.state, State::Upcoming);
    assert_eq!(r.label, "tomorrow");
    // 11-21 09:00+08: on the day -> due, today.
    let r = with_lead(1, DUE + 9 * HOUR);
    assert_eq!(r.state, State::Due);
    assert_eq!(r.label, "today");
    // 11-23 09:00+08: two whole days past -> overdue 2d.
    let r = with_lead(1, DUE + 2 * DAY + 9 * HOUR);
    assert_eq!(r.state, State::Overdue);
    assert_eq!(r.label, "overdue 2d");
    assert_eq!(r.overdue_days, Some(2));
    // Lead 0 at 11-20 12:00+08: not yet visible -> scheduled.
    let r = with_lead(0, DUE - DAY / 2);
    assert_eq!(r.state, State::Scheduled);
}

#[test]
fn newest_defer_wins_and_close_is_final() {
    let december = DUE + 14 * DAY;
    let january = DUE + 50 * DAY;
    let february = DUE + 100 * DAY;
    // Two defers, no close: the newest defer is the effective date.
    let r = one(
        vec![
            add("a1", "task", "2026-11-21+08:00", DUE, 1),
            defer("d1", "a1", "2026-12-05+08:00", december, 10),
            defer("d2", "a1", "2027-01-10+08:00", january, 20),
        ],
        DUE - 2 * DAY,
    );
    assert_eq!(r.effective.due_at, january);
    assert_eq!(r.effective.on, "2027-01-10+08:00");
    assert_eq!(r.state, State::Scheduled);
    // A defer that lands after a close is kept in the history and changes nothing.
    let r = one(
        vec![
            add("a1", "task", "2026-11-21+08:00", DUE, 1),
            defer("d1", "a1", "2026-12-05+08:00", december, 10),
            defer("d2", "a1", "2027-01-10+08:00", january, 20),
            close("c1", "a1", "done", 25),
            defer("d3", "a1", "2027-02-01+08:00", february, 30),
        ],
        DUE - 2 * DAY,
    );
    assert_eq!(r.state, State::Closed);
    assert_eq!(r.label, "done");
    assert_eq!(r.effective.due_at, january);
    assert!(r.close.is_some());
    assert!(r.events.iter().any(|e| e.id == "d3"));
}

#[test]
fn orphan_and_unknown_events() {
    let unknown =
        parse_event(r#"{"id":"u1","kind":"snooze","reminder":"a1","recorded_at":"50"}"#).unwrap();
    assert_eq!(unknown.kind, EventKind::Unknown);
    let folded = fold(
        vec![
            add("a1", "task", "2026-11-21+08:00", DUE, 1),
            defer("d9", "nope", "2026-12-01+08:00", DUE + 10 * DAY, 5),
            unknown,
        ],
        DUE - 2 * DAY,
    );
    let orphan_ids: Vec<&str> = folded.orphan_events.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(orphan_ids, ["d9"]);
    let r = &folded.reminders[0];
    assert_eq!(r.state, State::Scheduled);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].kind, EventKind::Unknown);
    assert_eq!(r.last_activity, "1"); // the unknown event moved nothing
}

#[test]
fn dedup() {
    let now = DUE - 2 * DAY;
    // Same term set on the same on, open -> exists.
    let same = fold(
        vec![add("a1", "reboot NUC", "2026-11-21+08:00", DUE, 1)],
        now,
    )
    .reminders;
    assert!(matches!(
        classify("reboot NUC", "2026-11-21+08:00", &same),
        Dedup::Exists(ids) if ids == ["a1"]
    ));
    // Same terms, other on -> similar, not exists.
    assert!(matches!(
        classify("reboot NUC", "2026-12-21+08:00", &same),
        Dedup::Similar(_)
    ));
    // A term subset of an open reminder -> similar, not exists.
    let wider = fold(
        vec![add(
            "a2",
            "reboot NUC and upgrade cort",
            "2026-11-21+08:00",
            DUE,
            1,
        )],
        now,
    )
    .reminders;
    assert!(matches!(
        classify("reboot NUC", "2026-11-21+08:00", &wider),
        Dedup::Similar(_)
    ));
    // A match only in a closed reminder never blocks: the same task next month is normal.
    let closed = fold(
        vec![
            add("a3", "reboot NUC", "2026-11-21+08:00", DUE, 1),
            close("c3", "a3", "done", 2),
        ],
        now,
    )
    .reminders;
    assert!(matches!(
        classify("reboot NUC", "2026-11-21+08:00", &closed),
        Dedup::New
    ));
}

#[test]
fn bounds() {
    assert!(validate_add(&"x".repeat(601), None, 1)
        .unwrap_err()
        .starts_with("remind_too_long"));
    assert!(validate_add("ok", Some(&"b".repeat(65)), 1)
        .unwrap_err()
        .starts_with("remind_too_long"));
    assert!(validate_note(&"n".repeat(301))
        .unwrap_err()
        .starts_with("remind_too_long"));
    assert!(validate_add("has \u{7} beep", None, 1)
        .unwrap_err()
        .starts_with("remind_control_chars"));
    assert!(validate_note("note \u{7}")
        .unwrap_err()
        .starts_with("remind_control_chars"));
    assert!(validate_add("ok", None, 31)
        .unwrap_err()
        .starts_with("remind_lead_invalid"));
    assert!(validate_add("   ", None, 1)
        .unwrap_err()
        .starts_with("remind_text_empty"));
    assert_eq!(validate_add("ok", Some("nuc-boot"), 30), Ok(()));
    assert_eq!(validate_note("fine"), Ok(()));
}

#[test]
fn past_by_instant() {
    let due = parse_on("2026-11-21+08:00").unwrap();
    // The day has fully ended at its own offset -> past.
    assert_eq!(
        check_not_past(&due, due.due_at + DAY).unwrap_err(),
        "remind_date_past"
    );
    // One ns before that instant the day has not ended -> accepted.
    assert_eq!(check_not_past(&due, due.due_at + DAY - 1), Ok(()));
}

#[test]
fn prefix_resolution_maps_question_codes() {
    let ids = ["aabbccdd00000001", "aabbccdd00000002"];
    let resolve = |prefix: &str| resolve_prefix(ids.iter().copied(), prefix);
    assert_eq!(resolve("aabbccdd00000001").unwrap(), "aabbccdd00000001");
    assert_eq!(resolve("short").unwrap_err(), "remind_id_too_short");
    assert_eq!(resolve("aabbccdd").unwrap_err(), "remind_id_ambiguous");
    assert_eq!(resolve("ffffffff").unwrap_err(), "remind_not_found");
}
