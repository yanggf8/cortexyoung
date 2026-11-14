use cort::hook_recall::{
    render_prompt_reminders, render_reminders, render_stale_snapshot, MAX_REMINDERS,
    MAX_REMINDER_BYTES,
};
use cort::remind::{fold, parse_event, Reminder, ReminderEvent, State};
use cort::remind_snapshot::{
    append_shown, read_shown, read_snapshot, replace_shown, select_unshown, shown_key,
    snapshot_events, write_snapshot, ShownKey, Snapshot, STALE_AFTER_NS,
};
use std::collections::HashSet;

const DAY: i128 = 86_400_000_000_000;
const DUE: i128 = 1_800_000_000_000_000_000;

fn temp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cort-remind-snapshot-")
        .tempdir_in(std::env::temp_dir())
        .unwrap()
}

fn add(id: &str, due_at: i128, text: &str, lead: u32) -> ReminderEvent {
    parse_event(&format!(
        r#"{{"id":"{id}","kind":"add","text":"{text}","on":"2027-01-15","utc_offset":"+00:00","due_at":"{due_at}","lead":{lead},"recorded_at":"1"}}"#
    ))
    .unwrap()
}

fn defer(id: &str, of: &str, due_at: i128, at: u32) -> ReminderEvent {
    parse_event(&format!(
        r#"{{"id":"{id}","kind":"defer","reminder":"{of}","on":"2027-02-01","utc_offset":"+00:00","due_at":"{due_at}","recorded_at":"{at}"}}"#
    ))
    .unwrap()
}

fn close(id: &str, of: &str, at: u32) -> ReminderEvent {
    parse_event(&format!(
        r#"{{"id":"{id}","kind":"close","reminder":"{of}","reason":"done","recorded_at":"{at}"}}"#
    ))
    .unwrap()
}

fn snap(read_at: i128, events: Vec<ReminderEvent>) -> Snapshot {
    Snapshot::new("proj", read_at, events)
}

fn id(n: usize) -> String {
    format!("{n:08x}aaaa")
}

#[test]
fn snapshot_round_trip_and_version_guard() {
    let dir = temp();
    let s = snap(100, vec![add(&id(1), DUE, "Reboot the NUC", 1)]);
    assert!(write_snapshot(dir.path(), &s).unwrap());
    assert_eq!(read_snapshot(dir.path(), "proj"), Some(s.clone()));
    let path = dir.path().join("remind-snapshot/proj.json");
    let v2 = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"version\":1", "\"version\":2");
    std::fs::write(&path, v2).unwrap();
    assert_eq!(read_snapshot(dir.path(), "proj"), None);
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(read_snapshot(dir.path(), "proj"), None);
    assert_eq!(read_snapshot(dir.path(), "missing"), None);
}

#[test]
fn older_read_never_overwrites_newer() {
    let dir = temp();
    assert!(write_snapshot(dir.path(), &snap(200, vec![])).unwrap());
    let older = snap(100, vec![add(&id(1), DUE, "x", 1)]);
    assert!(!write_snapshot(dir.path(), &older).unwrap());
    let kept = read_snapshot(dir.path(), "proj").unwrap();
    assert_eq!(kept.read_at, "200");
    assert!(kept.events.is_empty());
    // An equal read_at is not newer, so it replaces.
    assert!(write_snapshot(dir.path(), &snap(200, vec![add(&id(1), DUE, "x", 1)])).unwrap());
}

#[test]
fn snapshot_events_drop_closed_reminders() {
    let (a, b) = (id(1), id(2));
    let events = vec![
        add(&a, DUE, "open", 1),
        defer("d1d1d1d1", &a, DUE + DAY, 5),
        add(&b, DUE, "closed", 1),
        close("c1c1c1c1", &b, 6),
    ];
    let folded = fold(events, DUE);
    let kept = snapshot_events(&folded.reminders);
    let ids: Vec<&str> = kept.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(kept.len(), 2, "{ids:?}");
    assert!(ids.contains(&a.as_str()) && ids.contains(&"d1d1d1d1"));
    assert!(!ids.contains(&b.as_str()));
}

fn four_states() -> Snapshot {
    snap(
        0,
        vec![
            add(&id(1), DUE + 10 * DAY, "scheduled", 1),
            add(&id(2), DUE + DAY / 2, "upcoming", 1),
            add(&id(3), DUE - DAY / 2, "due", 1),
            add(&id(4), DUE - 3 * DAY, "overdue", 1),
        ],
    )
}

#[test]
fn select_each_state_once() {
    let s = four_states();
    let picked = select_unshown(&s, &HashSet::new(), DUE);
    let got: Vec<(State, String)> = picked.iter().map(|r| (r.state, r.add.id.clone())).collect();
    assert_eq!(
        got,
        vec![
            (State::Overdue, id(4)),
            (State::Due, id(3)),
            (State::Upcoming, id(2)),
        ]
    );
    let shown: HashSet<ShownKey> = picked.iter().filter_map(shown_key).collect();
    assert_eq!(shown.len(), 3);
    assert!(select_unshown(&s, &shown, DUE).is_empty());
}

#[test]
fn deferred_reminder_is_selected_again() {
    let a = id(1);
    let before = snap(0, vec![add(&a, DUE - DAY / 2, "due", 1)]);
    let shown: HashSet<ShownKey> = select_unshown(&before, &HashSet::new(), DUE)
        .iter()
        .filter_map(shown_key)
        .collect();
    assert_eq!(shown.len(), 1);
    // Same snapshot, same instant: suppressed.
    assert!(select_unshown(&before, &shown, DUE).is_empty());
    // Deferred to a date that is due now: a new due_at is a new key.
    let after = snap(
        0,
        vec![
            add(&a, DUE - DAY / 2, "due", 1),
            defer("d1d1d1d1", &a, DUE - DAY / 4, 5),
        ],
    );
    let again = select_unshown(&after, &shown, DUE);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].effective.due_at, DUE - DAY / 4);
}

#[test]
fn replace_then_append_shown() {
    let dir = temp();
    let a: ShownKey = (id(1), "due", DUE.to_string());
    let b: ShownKey = (id(2), "overdue", (DUE - DAY).to_string());
    assert!(read_shown(dir.path(), "sess").is_empty());
    replace_shown(dir.path(), "sess", std::slice::from_ref(&a)).unwrap();
    append_shown(dir.path(), "sess", std::slice::from_ref(&b)).unwrap();
    let got = read_shown(dir.path(), "sess");
    assert_eq!(got, [a.clone(), b.clone()].into_iter().collect());
    assert!(read_shown(dir.path(), "other").is_empty());
    replace_shown(dir.path(), "sess", &[]).unwrap();
    assert!(read_shown(dir.path(), "sess").is_empty());
}

fn due_reminders(n: usize, text: &str) -> Vec<Reminder> {
    let events = (1..=n)
        .map(|i| add(&id(i), DUE - DAY / 2 - i as i128, text, 1))
        .collect();
    fold(events, DUE).reminders
}

#[test]
fn render_reminders_returns_printed_keys_only() {
    let reminders = due_reminders(6, "Reboot the NUC");
    assert!(reminders.iter().all(|r| r.state == State::Due));
    let (text, keys) = render_reminders(&reminders).unwrap();
    assert_eq!(keys.len(), MAX_REMINDERS);
    for (key, line) in keys
        .iter()
        .zip(text.lines().filter(|l| l.starts_with("- ")))
    {
        assert!(line.starts_with(&format!("- {}", &key.0[..8])), "{line}");
        assert_eq!(key.1, "due");
    }
    assert!(text.contains("… 2 more due"), "{text}");
    let printed = text.lines().filter(|l| l.starts_with("- ")).count();
    assert_eq!(printed, keys.len());
}

#[test]
fn prompt_block_header_and_budget() {
    let reminders = due_reminders(6, &"x".repeat(100));
    let (text, keys) = render_prompt_reminders(&reminders, DUE, DUE).unwrap();
    let header = text.lines().next().unwrap();
    assert!(
        header.starts_with("Reminders not yet shown in this session (local snapshot read "),
        "{header}"
    );
    assert!(
        text.len() <= header.len() + MAX_REMINDER_BYTES,
        "{}",
        text.len()
    );
    assert_eq!(keys.len(), MAX_REMINDERS);
    assert!(text.ends_with("… 2 more due: cort remind list"), "{text}");
    assert!(render_prompt_reminders(&[], DUE, DUE).is_none());
    println!("{text}");
}

#[test]
fn stale_notice_and_threshold() {
    assert_eq!(STALE_AFTER_NS, 3 * DAY);
    let line = render_stale_snapshot(DUE);
    assert!(
        line.starts_with("cort reminders: the local snapshot is from "),
        "{line}"
    );
    assert!(line.ends_with("; run cort remind list"), "{line}");
}

fn wall_ns() -> i128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i128
}

#[test]
fn future_read_at_never_freezes_the_snapshot() {
    // A clock that jumped back leaves a snapshot "read" a day from now; a write stamped with the
    // real clock must still replace it, and a read must call it stale rather than replay it.
    let dir = temp();
    let now = wall_ns();
    assert!(write_snapshot(
        dir.path(),
        &snap(now + DAY, vec![add(&id(1), DUE, "old", 1)])
    )
    .unwrap());
    assert!(write_snapshot(dir.path(), &snap(now, vec![])).unwrap());
    let kept = read_snapshot(dir.path(), "proj").unwrap();
    assert!(kept.events.is_empty());
    // A newer read inside the slack still wins over an older one.
    assert!(write_snapshot(dir.path(), &snap(now + 60_000_000_000, vec![])).unwrap());
    assert!(!write_snapshot(dir.path(), &snap(now, vec![add(&id(2), DUE, "x", 1)])).unwrap());
}

#[test]
fn stale_in_both_directions() {
    use cort::remind_snapshot::{is_stale, FUTURE_SLACK_NS};
    let now = 1_000 * DAY;
    assert!(!is_stale(now, now));
    assert!(!is_stale(now - STALE_AFTER_NS, now));
    assert!(is_stale(now - STALE_AFTER_NS - 1, now));
    assert!(!is_stale(now + FUTURE_SLACK_NS, now));
    assert!(is_stale(now + FUTURE_SLACK_NS + 1, now));
}
