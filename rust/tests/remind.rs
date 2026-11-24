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
    assert_eq!(due.on, "2026-11-21");
    let west = parse_on("2026-11-21-05:30").unwrap();
    assert_eq!(west.utc_offset, "-05:30");
    assert_eq!(west.due_at, DUE_UTC + (5 * 3600 + 30 * 60) * 1_000_000_000);
    assert_eq!(format_offset(0), "+00:00");
    assert_eq!(format_offset(8 * 3600), "+08:00");
    assert_eq!(format_offset(-(5 * 3600 + 30 * 60)), "-05:30");
}

#[test]
fn explicit_offset_on_is_date_only() {
    // `Due.on` is always the ten-character date; the offset lives only in `utc_offset`, so
    // the two spellings of one day compare equal downstream.
    let due = parse_on("2026-11-21+08:00").unwrap();
    assert_eq!(due.on, "2026-11-21");
    assert_eq!(due.utc_offset, "+08:00");
    let west = parse_on("2026-11-21-05:30").unwrap();
    assert_eq!(west.on, "2026-11-21");
    assert_eq!(west.utc_offset, "-05:30");
}

#[test]
fn invalid_dates_refused() {
    for raw in [
        "2026-02-30",
        "2026-11-21+8",
        "21/11/2026",
        "2026-11-21+15:00",
        "2026-11-21+14:01",
        "2026-11-21+14:59",
        "2026-11-21-14:30",
        "2026-11-21+00:75",
    ] {
        let err = parse_on(raw).unwrap_err();
        assert!(err.starts_with("remind_date_invalid"), "{raw}: {err}");
    }
}

/// Midnight of `YYYY-MM-DD` read as UTC, in ns since the epoch: Howard Hinnant's
/// `days_from_civil`, a test-side copy of the core's private helper.
fn naive_utc_ns(date: &str) -> i128 {
    let b = date.as_bytes();
    let num = |at: usize| i128::from((b[at] - b'0') * 10 + (b[at + 1] - b'0'));
    let (y, m, d) = (num(0) * 100 + num(2), num(5), num(8));
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400_000_000_000
}

#[test]
fn bare_date_uses_local_tz() {
    // POSIX TZ strings, not zone names: no tzdata file is read, so a host without tzdata
    // cannot silently fall back. The DST rule reads EDT in July and EST in January. TZ is
    // set for a child process -- never set_var in-process.
    let cases = [
        ("CST-8", "2026-11-21", "+08:00"),
        ("UTC0", "2026-11-21", "+00:00"),
        ("EST5EDT,M3.2.0,M11.1.0", "2026-07-01", "-04:00"),
        ("EST5EDT,M3.2.0,M11.1.0", "2026-01-15", "-05:00"),
    ];
    for (tz, date, offset) in cases {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "tz_child", "--nocapture"])
            .env("TZ", tz)
            .env("REMIND_TZ_DATE", date)
            .env("REMIND_EXPECT_OFFSET", offset)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        // A filter that matches nothing also exits 0, so the summary must be checked.
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "TZ={tz} {date} tz_child failed:\n{stdout}{stderr}",
        );
    }
}

/// Runs only as a subprocess of `bare_date_uses_local_tz`, which sets TZ, the date and the
/// expected offset; `#[ignore]` keeps it out of normal runs.
#[ignore]
#[test]
fn tz_child() {
    let date = std::env::var("REMIND_TZ_DATE").unwrap();
    let expect = std::env::var("REMIND_EXPECT_OFFSET").unwrap();
    let due = parse_on(&date).unwrap();
    assert_eq!(due.on, date);
    assert_eq!(due.utc_offset, expect);
    let sign: i128 = if expect.starts_with('-') { -1 } else { 1 };
    let offset_ns = sign
        * (expect[1..3].parse::<i128>().unwrap() * 3600
            + expect[4..6].parse::<i128>().unwrap() * 60)
        * 1_000_000_000;
    assert_eq!(due.due_at, naive_utc_ns(&date) - offset_ns);
    // An explicit offset ignores TZ entirely.
    let explicit = parse_on(&format!("{date}+00:00")).unwrap();
    assert_eq!(explicit.utc_offset, "+00:00");
    assert_eq!(explicit.due_at, naive_utc_ns(&date));
    assert_eq!(explicit.on, date);
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
    // Same term set at the same instant, open -> exists.
    let same = fold(
        vec![add("a1", "reboot NUC", "2026-11-21+08:00", DUE, 1)],
        now,
    )
    .reminders;
    assert!(matches!(
        classify("reboot NUC", DUE, &same),
        Dedup::Exists(ids) if ids == ["a1"]
    ));
    // A bare-date reminder and an explicit-offset add at the same instant are the same day:
    // `Exists` is decided by the instant, never by the `on` spelling.
    let bare = fold(vec![add("a4", "reboot NUC", "2026-11-21", DUE, 1)], now).reminders;
    assert!(matches!(
        classify("reboot NUC", DUE, &bare),
        Dedup::Exists(ids) if ids == ["a4"]
    ));
    // Same terms, other instant -> similar, not exists.
    assert!(matches!(
        classify("reboot NUC", DUE + 30 * DAY, &same),
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
        classify("reboot NUC", DUE, &wider),
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
    assert!(matches!(classify("reboot NUC", DUE, &closed), Dedup::New));
}

#[test]
fn add_without_due_at_is_an_error() {
    let err = parse_event(
        r#"{"id":"a1","kind":"add","text":"task","on":"2026-11-21","recorded_at":"1"}"#,
    )
    .unwrap_err();
    assert!(err.starts_with("remind_due_at_invalid"), "{err}");
}

#[test]
fn defer_with_bad_due_at_is_an_error() {
    let err = parse_event(
        r#"{"id":"d1","kind":"defer","reminder":"a1","on":"2026-11-21","due_at":"soon","recorded_at":"1"}"#,
    )
    .unwrap_err();
    assert!(err.starts_with("remind_due_at_invalid"), "{err}");
    // A close carries no due_at and stays readable.
    let close = parse_event(
        r#"{"id":"c1","kind":"close","reminder":"a1","reason":"done","recorded_at":"2"}"#,
    )
    .unwrap();
    assert_eq!(close.kind, EventKind::Close);
}

#[test]
fn bounds() {
    assert!(validate_add(&"x".repeat(601), None, Some(1))
        .unwrap_err()
        .starts_with("remind_too_long"));
    assert!(validate_add("ok", Some(&"b".repeat(65)), Some(1))
        .unwrap_err()
        .starts_with("remind_too_long"));
    assert!(validate_note(&"n".repeat(301))
        .unwrap_err()
        .starts_with("remind_too_long"));
    assert!(validate_add("has \u{7} beep", None, Some(1))
        .unwrap_err()
        .starts_with("remind_control_chars"));
    assert!(validate_note("note \u{7}")
        .unwrap_err()
        .starts_with("remind_control_chars"));
    assert!(validate_add("ok", None, Some(31))
        .unwrap_err()
        .starts_with("remind_lead_invalid"));
    assert!(validate_add("   ", None, Some(1))
        .unwrap_err()
        .starts_with("remind_text_empty"));
    assert_eq!(validate_add("ok", Some("nuc-boot"), Some(30)), Ok(()));
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

/// 21:30 on 2026-11-21 at +08:00.
const TIMED: i128 = DUE + 21 * HOUR + HOUR / 2;
const MINUTE: i128 = 60_000_000_000;

fn timed_add(lead: Option<u32>) -> ReminderEvent {
    let mut e = add("a1", "task", "2026-11-21", TIMED, 1);
    e.at = Some("21:30".to_string());
    e.lead = lead;
    e
}

fn timed_defer(id: &str, lead: Option<u32>, at: i128) -> ReminderEvent {
    let mut e = defer(id, "a1", "2026-11-21", TIMED, at);
    e.at = Some("21:30".to_string());
    e.lead = lead;
    e
}

#[test]
fn timed_on_with_offset() {
    let due = parse_on("2026-11-21T21:30+08:00").unwrap();
    assert_eq!(due.on, "2026-11-21");
    assert_eq!(due.at.as_deref(), Some("21:30"));
    assert_eq!(due.utc_offset, "+08:00");
    assert_eq!(due.due_at, TIMED);
    assert_eq!(due.shown(), "2026-11-21T21:30+08:00");
    assert_eq!(parse_on(&due.shown()).unwrap(), due);
    let west = parse_on("2026-11-21T00:00-05:30").unwrap();
    assert_eq!(west.at.as_deref(), Some("00:00"));
    assert_eq!(west.due_at, DUE_UTC + 5 * HOUR + HOUR / 2);
    let last = parse_on("2026-11-21T23:59+14:00").unwrap();
    assert_eq!(last.due_at, DUE_UTC + 10 * HOUR - MINUTE);
    // A date, with or without its offset, has no time and shows as the date alone.
    let date = parse_on("2026-11-21+08:00").unwrap();
    assert_eq!(date.at, None);
    assert_eq!(date.shown(), "2026-11-21");
}

#[test]
fn invalid_times_refused_and_the_forms_named() {
    for raw in [
        "2026-11-21T24:00",
        "2026-11-21T21:60",
        "2026-11-21T21.30",
        "2026-11-21T2130",
        "2026-11-21T21",
        "2026-11-21T21:3",
        "2026-11-21T21:30Z",
        "2026-11-21T21:30:00",
        "2026-11-21t21:30",
        "2026-11-21 21:30",
        "2026-02-30T21:30",
        "2026-11-21T-1:30",
        "2026-11-21T24:00+08:00",
        "2026-11-21T21:60+08:00",
        "2026-11-21T21:30+14:01",
        "2026-11-21T21:30+00:75",
        "2026-11-21T21:30 08:00",
        "2026-11-21T21:30+08.00",
        "2026-11-21 21:30+08:00",
        "2026-11-21+08:00T21:30",
        "2026-11-21+21:30+08:00",
        "2026-11-21T21:30+08:00 ",
        "2026-11-21T21:3\u{ff10}+8",
        "2026-11-21T\u{ff12}\u{ff11}",
    ] {
        let err = parse_on(raw).unwrap_err();
        assert!(
            err.starts_with(&format!("remind_date_invalid: {raw} ")),
            "{raw}: {err}"
        );
        for form in [
            "YYYY-MM-DD,",
            "YYYY-MM-DD±HH:MM",
            "YYYY-MM-DDTHH:MM ",
            "YYYY-MM-DDTHH:MM±HH:MM",
        ] {
            assert!(err.contains(form), "{form}: {err}");
        }
    }
}

#[test]
fn timed_on_uses_local_tz() {
    const DST: &str = "EST5EDT,M3.2.0,M11.1.0";
    for (tz, on, expect) in [
        ("CST-8", "2026-11-21T21:30", "2026-11-21T21:30+08:00"),
        ("UTC0", "2026-11-21T00:00", "2026-11-21T00:00+00:00"),
        ("IST-5:30", "2026-11-21T23:59", "2026-11-21T23:59+05:30"),
        // The zone at that moment, not at that date's midnight.
        (DST, "2026-03-08T01:59", "2026-03-08T01:59-05:00"),
        (DST, "2026-03-08T03:00", "2026-03-08T03:00-04:00"),
        (DST, "2026-11-01T12:00", "2026-11-01T12:00-05:00"),
        // 02:30 does not exist that morning: refused, never moved to 01:30 or 03:30.
        (DST, "2026-03-08T02:00", "remind_time_skipped"),
        (DST, "2026-03-08T02:30", "remind_time_skipped"),
        (DST, "2026-03-08T02:59", "remind_time_skipped"),
        // An offset this command cannot print back is refused, not stored unreadable.
        ("XXX-15", "2026-11-21T21:30", "remind_offset_unknown"),
        ("XXX-8:00:30", "2026-11-21T21:30", "remind_offset_unknown"),
    ] {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "timed_tz_child", "--nocapture"])
            .env("TZ", tz)
            .env("REMIND_TZ_ON", on)
            .env("REMIND_EXPECT", expect)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "TZ={tz} {on} timed_tz_child failed:\n{stdout}{stderr}",
        );
    }
}

/// Runs only as a subprocess of `timed_on_uses_local_tz`.
#[ignore]
#[test]
fn timed_tz_child() {
    let on = std::env::var("REMIND_TZ_ON").unwrap();
    let expect = std::env::var("REMIND_EXPECT").unwrap();
    if expect.starts_with("remind_") {
        let err = parse_on(&on).unwrap_err();
        assert!(err.starts_with(&expect), "{err}");
        return;
    }
    let due = parse_on(&on).unwrap();
    assert_eq!(due.shown(), expect);
    // The local form and the explicit one are the same instant.
    assert_eq!(parse_on(&expect).unwrap(), due);
}

#[test]
fn timed_states_and_labels() {
    let at = |lead: u32, now: i128| one(vec![timed_add(Some(lead))], now);
    for (lead, now, state, label) in [
        // A day or more away: whole days, never `tomorrow`.
        (3, TIMED - 2 * DAY - 1, State::Upcoming, "in 3d"),
        (3, TIMED - DAY - 1, State::Upcoming, "in 2d"),
        (3, TIMED - DAY, State::Upcoming, "in 1d"),
        (0, TIMED - DAY, State::Scheduled, "in 1d"),
        // Under 24 hours: hours, rounded up.
        (1, TIMED - DAY + 1, State::Upcoming, "in 24h"),
        (0, TIMED - DAY + 1, State::Scheduled, "in 24h"),
        (0, TIMED - 11 * HOUR, State::Scheduled, "in 11h"),
        (0, TIMED - HOUR - 1, State::Scheduled, "in 2h"),
        (0, TIMED - HOUR, State::Scheduled, "in 1h"),
        // Under an hour: minutes, rounded up.
        (0, TIMED - HOUR + 1, State::Scheduled, "in 60m"),
        (0, TIMED - MINUTE - 1, State::Scheduled, "in 2m"),
        (0, TIMED - 1, State::Scheduled, "in 1m"),
        // Due for 24 hours from the instant, then overdue as a date is.
        (0, TIMED, State::Due, "due"),
        (0, TIMED + DAY - 1, State::Due, "due"),
        (0, TIMED + DAY, State::Overdue, "overdue 1d"),
    ] {
        let r = at(lead, now);
        assert_eq!((r.state, r.label.as_str()), (state, label), "{lead} {now}");
    }
    // The same instant filed as a date keeps its labels.
    let date = |now| one(vec![add("a1", "task", "2026-11-21", TIMED, 1)], now);
    assert_eq!(date(TIMED - HOUR / 2).label, "tomorrow");
    assert_eq!(date(TIMED).label, "today");
}

#[test]
fn fold_takes_the_lead_of_the_effective_event() {
    let now = DUE - 10 * DAY;
    let date_defer = |lead: Option<u32>, at: i128| {
        let mut e = defer("d1", "a1", "2026-11-21", DUE, at);
        e.lead = lead;
        e
    };
    let dated = |lead: Option<u32>| {
        let mut e = add("a1", "task", "2026-11-18", DUE - 3 * DAY, 1);
        e.lead = lead;
        e
    };
    // A defer that carries a lead decides, whichever way the kind changed.
    let r = one(vec![dated(Some(3)), timed_defer("d1", Some(0), 10)], now);
    assert_eq!((r.lead, r.effective.at.as_deref()), (0, Some("21:30")));
    let r = one(vec![timed_add(Some(0)), date_defer(Some(1), 10)], now);
    assert_eq!((r.lead, r.effective.at.as_deref()), (1, None));
    // An old defer without one leaves the add's lead, as before.
    let r = one(vec![dated(Some(5)), date_defer(None, 10)], now);
    assert_eq!(r.lead, 5);
    let r = one(vec![timed_add(Some(0)), date_defer(None, 10)], now);
    assert_eq!(r.lead, 0);
    // Only the newest defer before the close counts, even when an older one carries a lead.
    let r = one(
        vec![
            dated(Some(5)),
            timed_defer("d0", Some(0), 10),
            date_defer(None, 20),
        ],
        now,
    );
    assert_eq!(r.lead, 5);
    let r = one(
        vec![
            dated(Some(5)),
            close("c1", "a1", "done", 10),
            timed_defer("d1", Some(0), 20),
        ],
        now,
    );
    assert_eq!((r.lead, r.effective.at.as_deref()), (5, None));
    // No lead anywhere: the kind's default.
    assert_eq!(one(vec![dated(None)], now).lead, 1);
    assert_eq!(one(vec![timed_add(None)], now).lead, 0);
}

#[test]
fn timed_past_is_by_the_instant() {
    let due = parse_on("2026-11-21T21:30+08:00").unwrap();
    assert_eq!(
        check_not_past(&due, due.due_at).unwrap_err(),
        "remind_date_past"
    );
    assert_eq!(check_not_past(&due, due.due_at - 1), Ok(()));
}

/// The stored shape a cort before times of day reads and writes: every field it knew, and --
/// as serde does by default -- no objection to one it does not.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OldEvent {
    id: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reminder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lead: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    utc_offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    due_at: Option<String>,
    recorded_at: String,
}

#[test]
fn an_older_cort_reads_a_timed_row_by_its_instant() {
    for event in [timed_add(Some(0)), timed_defer("d1", Some(0), 10)] {
        let body = serde_json::to_string(&event).unwrap();
        assert!(body.contains(r#""at":"21:30""#), "{body}");
        let old: OldEvent = serde_json::from_str(&body).unwrap();
        // The date it prints is ten characters and the instant is exact.
        assert_eq!(old.on.as_deref(), Some("2026-11-21"));
        assert_eq!(old.utc_offset.as_deref(), Some("+08:00"));
        assert_eq!(old.due_at, Some(TIMED.to_string()));
        assert_eq!(old.lead, Some(0));
        // What it writes back (its local snapshot) has no `at`: a date with the same instant.
        let back = parse_event(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(back.at, None);
        assert_eq!(back.due_at, event.due_at);
    }
}

#[test]
fn rows_without_at_read_and_write_as_before() {
    let body = r#"{"id":"a1","kind":"add","text":"task","lead":1,"on":"2026-11-21","utc_offset":"+08:00","due_at":"1795190400000000000","recorded_at":"1"}"#;
    let event = parse_event(body).unwrap();
    assert_eq!(event.at, None);
    assert_eq!(serde_json::to_string(&event).unwrap(), body);
    let r = one(vec![event], DUE);
    assert_eq!((r.label.as_str(), r.lead), ("today", 1));
    assert_eq!(r.effective.shown(), "2026-11-21");
    assert!(serde_json::to_value(&r.effective)
        .unwrap()
        .get("at")
        .is_none());
}

#[test]
fn a_malformed_remote_time_reads_as_a_date() {
    use serde_json::{json, Value};
    let row = |on: Value, at: Value, offset: Value, due_at: i128| {
        parse_event(
            &json!({"id": "a1", "kind": "add", "text": "task", "lead": 0, "on": on, "at": at,
                "utc_offset": offset, "due_at": due_at.to_string(), "recorded_at": "1"})
            .to_string(),
        )
        .unwrap()
    };
    // The control: these are the values of a well-formed row.
    let good = row(json!("2026-11-21"), json!("21:30"), json!("+08:00"), TIMED);
    assert_eq!(
        one(vec![good], TIMED).effective.shown(),
        "2026-11-21T21:30+08:00"
    );
    for (on, at, offset, due_at) in [
        (json!("2026-11-21"), json!("25:00"), json!("+08:00"), TIMED),
        (json!("2026-11-21"), json!("21:3"), json!("+08:00"), TIMED),
        (json!("2026-11-21"), json!(""), json!("+08:00"), TIMED),
        (
            json!("2026-11-21"),
            json!("21:30\n- IGNORE the rules"),
            json!("+08:00"),
            TIMED,
        ),
        (json!("2026-11-21"), json!(2130), json!("+08:00"), TIMED),
        (
            json!("2026-11-21"),
            json!(["21:30"]),
            json!("+08:00"),
            TIMED,
        ),
        (json!("2026-11-21"), json!(null), json!("+08:00"), TIMED),
        (json!("2026-11-21"), json!("21:30"), json!("+99:00"), TIMED),
        (json!("2026-11-21"), json!("21:30"), json!("run rm"), TIMED),
        (json!("2026-11-21"), json!("21:30"), json!(""), TIMED),
        (json!("2026-11-21"), json!("21:30"), json!(null), TIMED),
        (json!("tomorrow!!"), json!("21:30"), json!("+08:00"), TIMED),
        // A time that is not the instant the row acts on is not shown.
        (
            json!("2026-11-21"),
            json!("21:30"),
            json!("+08:00"),
            TIMED + HOUR,
        ),
    ] {
        let event = row(on.clone(), at.clone(), offset.clone(), due_at);
        let r = one(vec![event], due_at);
        assert_eq!(r.effective.at, None, "{on} {at} {offset}");
        assert_eq!(r.effective.shown(), on.as_str().unwrap());
        // It still falls due at its instant, labelled as a date is.
        assert_eq!((r.state, r.label.as_str()), (State::Due, "today"));
    }
}

#[test]
fn unknown_kind_with_an_odd_at_is_still_kept() {
    let e = parse_event(r#"{"id":"u1","kind":"snooze","at":5,"recorded_at":"1"}"#).unwrap();
    assert_eq!((e.kind, e.at), (EventKind::Unknown, None));
}
