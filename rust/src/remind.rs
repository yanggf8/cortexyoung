//! Pure core of `cort remind`: dated-task event model, dates, fold and write bounds.
//! Nothing here touches the network or the disk.

use crate::question::{self, Dedup};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const NS_PER_SEC: i128 = 1_000_000_000;
const NS_PER_DAY: i128 = 86_400_000_000_000;
const MAX_TEXT: usize = 600;
const MAX_BLOCKS: usize = 64;
const MAX_NOTE: usize = 300;
const MAX_LEAD: u32 = 30;
const DEFAULT_LEAD: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Add,
    Defer,
    Close,
    /// A kind written by a newer cort. The fold keeps it in its reminder's history (or the
    /// orphans) and lets it change nothing: not the state, the effective date, nor the last
    /// activity.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderEvent {
    pub id: String,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utc_offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub recorded_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_machine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_machine_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
}

/// One stored event body. A row of a kind this version does not know is read leniently, so a
/// newer cort's fields cannot break an older one: whatever ids it carries (`reminder`, `of`)
/// are kept where they are strings. A body that is not JSON, or a known kind whose fields do
/// not parse, is still an error.
pub fn parse_event(body: &str) -> Result<ReminderEvent, String> {
    let error = match serde_json::from_str::<ReminderEvent>(body) {
        Ok(event) => return Ok(event),
        Err(error) => error.to_string(),
    };
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| error.clone())?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    match value.get("kind").and_then(|v| v.as_str()) {
        Some("add" | "defer" | "close") | None => return Err(error),
        Some(_) => {}
    }
    let id = text("id").ok_or_else(|| error.clone())?;
    Ok(ReminderEvent {
        id,
        kind: EventKind::Unknown,
        reminder: text("reminder"),
        text: None,
        blocks: None,
        lead: None,
        on: None,
        utc_offset: None,
        due_at: None,
        reason: None,
        of: text("of"),
        note: None,
        recorded_at: text("recorded_at").unwrap_or_else(|| "0".to_string()),
        recorded_machine: text("recorded_machine"),
        recorded_machine_source: text("recorded_machine_source"),
        harness: text("harness"),
    })
}

/// The date as given, the offset used and the instant of midnight at that offset (ns since
/// the epoch). Every reader compares `now` against `due_at`, so all machines flip at the
/// same instant; the day shown is always `on`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Due {
    pub on: String,
    pub utc_offset: String,
    pub due_at: i128,
}

#[cfg(unix)]
use std::os::raw::{c_char, c_int, c_long};

/// `struct tm` for the `mktime` FFI. glibc and macOS lay it out the same way:
/// nine `c_int` fields `tm_sec..tm_isdst`, then `tm_gmtoff: c_long` and
/// `tm_zone: *const c_char` (musl matches). Only the first nine are inputs here; the tail
/// is zeroed so the struct is the size the ABI expects. One function, no libc crate --
/// the same minimal-FFI style as `heal.rs`'s `setsid`.
#[cfg(unix)]
#[repr(C)]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

#[cfg(unix)]
extern "C" {
    fn mktime(tm: *mut Tm) -> c_long;
}

/// Midnight of a calendar date in this machine's local zone, as ns since the epoch.
/// `tm_isdst = -1` makes `mktime` resolve the zone (and DST) for that date, not today's
/// offset. Refused (`remind_offset_unknown`) off unix or when `mktime` cannot answer; the
/// explicit-offset form of `parse_on` works everywhere.
pub fn local_midnight(y: i32, m: u32, d: u32) -> Result<i128, String> {
    midnight_secs(y, m, d).map(|secs| i128::from(secs) * NS_PER_SEC)
}

/// `time_t` is `c_long` on every target this crate builds for (64-bit Linux and macOS),
/// so the `mktime` result needs no conversion here.
#[cfg(unix)]
fn midnight_secs(y: i32, m: u32, d: u32) -> Result<i64, String> {
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: d as c_int,
        tm_mon: m as c_int - 1,
        tm_year: y - 1900,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: -1,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: `tm` is a fully initialized `struct tm`; `mktime` reads it through the
    // pointer and normalizes it in place, touching no other memory.
    let secs = unsafe { mktime(&mut tm) };
    if secs == -1 {
        return Err("remind_offset_unknown".to_string());
    }
    Ok(secs)
}

#[cfg(not(unix))]
fn midnight_secs(_y: i32, _m: u32, _d: u32) -> Result<i64, String> {
    Err("remind_offset_unknown".to_string())
}

/// `+08:00`, `-05:30`, `+00:00`: sign, two-digit hours, two-digit minutes.
pub fn format_offset(seconds: i64) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let abs = seconds.unsigned_abs();
    format!("{}{:02}:{:02}", sign, abs / 3600, abs % 3600 / 60)
}

/// Days from 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i32, m: u32, d: u32) -> i128 {
    let y = i128::from(if m <= 2 { y - 1 } else { y });
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (i128::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i128::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn naive_utc_seconds(y: i32, m: u32, d: u32) -> i128 {
    days_from_civil(y, m, d) * 86_400
}

fn leap(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in_month(y: i32, m: u32) -> Option<u32> {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if leap(y) => Some(29),
        2 => Some(28),
        _ => None,
    }
}

fn two(b: &[u8], at: usize) -> Option<u32> {
    match (b.get(at), b.get(at + 1)) {
        (Some(a), Some(c)) if a.is_ascii_digit() && c.is_ascii_digit() => {
            Some(u32::from(a - b'0') * 10 + u32::from(c - b'0'))
        }
        _ => None,
    }
}

/// Parse `--on`: `YYYY-MM-DD` (midnight in this machine's local zone, offset reported) or
/// `YYYY-MM-DD±HH:MM` (offsets up to ±14:00, `due_at` is pure arithmetic). The date must
/// be a real calendar date; anything else is `remind_date_invalid`.
pub fn parse_on(raw: &str) -> Result<Due, String> {
    let invalid = || format!("remind_date_invalid: {raw}");
    let b = raw.as_bytes();
    if b.len() != 10 && b.len() != 16 {
        return Err(invalid());
    }
    let year = two(b, 0)
        .zip(two(b, 2))
        .map(|(hi, lo)| (hi * 100 + lo) as i32); // two digits each, so at most 9999
    let (year, month, day) = match (year, two(b, 5), two(b, 8), b.get(4), b.get(7)) {
        (Some(y), Some(m), Some(d), Some(b'-'), Some(b'-')) => (y, m, d),
        _ => return Err(invalid()),
    };
    let month_days = days_in_month(year, month).ok_or_else(invalid)?;
    if day == 0 || day > month_days {
        return Err(invalid());
    }
    if b.len() == 10 {
        let due_at = local_midnight(year, month, day)?;
        // The offset is the difference between that instant and the same wall clock read
        // as UTC.
        let offset = (naive_utc_seconds(year, month, day) * NS_PER_SEC - due_at) / NS_PER_SEC;
        return Ok(Due {
            on: raw.to_string(),
            utc_offset: format_offset(i64::try_from(offset).unwrap_or(0)),
            due_at,
        });
    }
    let sign: i64 = match b[10] {
        b'+' => 1,
        b'-' => -1,
        _ => return Err(invalid()),
    };
    let (hours, minutes) = match (two(b, 11), two(b, 14), b.get(13)) {
        (Some(h), Some(m), Some(b':')) => (h, m),
        _ => return Err(invalid()),
    };
    if hours > 14 || minutes > 59 {
        return Err(invalid());
    }
    let offset = sign * (i64::from(hours) * 3600 + i64::from(minutes) * 60);
    let due_at = (naive_utc_seconds(year, month, day) - i128::from(offset)) * NS_PER_SEC;
    Ok(Due {
        on: raw.to_string(),
        utc_offset: format_offset(offset),
        due_at,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Scheduled,
    Upcoming,
    Due,
    Overdue,
    Closed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Reminder {
    pub add: ReminderEvent,
    pub events: Vec<ReminderEvent>,
    pub state: State,
    pub effective: Due,
    pub lead: u32,
    pub label: String,
    pub overdue_days: Option<i64>,
    pub close: Option<ReminderEvent>,
    pub last_activity: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Folded {
    pub reminders: Vec<Reminder>,
    pub orphan_events: Vec<ReminderEvent>,
}

fn ns(event: &ReminderEvent) -> i128 {
    event.recorded_at.parse().unwrap_or(0)
}

fn order(a: &ReminderEvent, b: &ReminderEvent) -> std::cmp::Ordering {
    ns(a).cmp(&ns(b)).then_with(|| a.id.cmp(&b.id))
}

fn due_from(event: &ReminderEvent) -> Due {
    Due {
        on: event.on.clone().unwrap_or_default(),
        utc_offset: event.utc_offset.clone().unwrap_or_default(),
        due_at: event
            .due_at
            .as_deref()
            .and_then(|raw| raw.parse::<i128>().ok())
            .unwrap_or(0),
    }
}

fn state_at(now_ns: i128, effective: &Due, lead: u32) -> State {
    let visible_at = effective
        .due_at
        .saturating_sub(i128::from(lead) * NS_PER_DAY);
    if now_ns < visible_at {
        State::Scheduled
    } else if now_ns < effective.due_at {
        State::Upcoming
    } else if now_ns < effective.due_at.saturating_add(NS_PER_DAY) {
        State::Due
    } else {
        State::Overdue
    }
}

/// Fold every event of a project into reminders. Non-closed reminders come first, by
/// effective `due_at` ascending then id; closed ones after. A defer or close naming an id
/// that is not in the component is an `orphan_event`, never dropped.
pub fn fold(events: Vec<ReminderEvent>, now_ns: i128) -> Folded {
    let mut adds: Vec<ReminderEvent> = Vec::new();
    let mut rest: Vec<ReminderEvent> = Vec::new();
    for event in events {
        if event.kind == EventKind::Add {
            if !adds.iter().any(|a| a.id == event.id) {
                adds.push(event);
            }
        } else {
            rest.push(event);
        }
    }
    let mut by_reminder: HashMap<String, Vec<ReminderEvent>> = HashMap::new();
    let mut orphan_events = Vec::new();
    for event in rest {
        match event.reminder.clone() {
            Some(r) if adds.iter().any(|a| a.id == r) => {
                by_reminder.entry(r).or_default().push(event)
            }
            _ => orphan_events.push(event),
        }
    }
    orphan_events.sort_by(order);
    let mut reminders: Vec<Reminder> = adds
        .into_iter()
        .map(|add| {
            let mut events = by_reminder.remove(&add.id).unwrap_or_default();
            events.sort_by(order);
            // The first close is final: a defer or a second close that lands after it is
            // kept in the history and changes nothing.
            let close_index = events.iter().position(|e| e.kind == EventKind::Close);
            let close = close_index.map(|i| events[i].clone());
            let effective = events[..close_index.unwrap_or(events.len())]
                .iter()
                .rev()
                .find(|e| e.kind == EventKind::Defer)
                .map(due_from)
                .unwrap_or_else(|| due_from(&add));
            let lead = add.lead.unwrap_or(DEFAULT_LEAD);
            let state = if close.is_some() {
                State::Closed
            } else {
                state_at(now_ns, &effective, lead)
            };
            let (label, overdue_days) = match state {
                State::Closed => (
                    close
                        .as_ref()
                        .and_then(|c| c.reason.clone())
                        .unwrap_or_else(|| "closed".to_string()),
                    None,
                ),
                State::Due => ("today".to_string(), None),
                State::Overdue => {
                    let days = i64::try_from(now_ns.saturating_sub(effective.due_at) / NS_PER_DAY)
                        .unwrap_or(i64::MAX);
                    (format!("overdue {days}d"), Some(days))
                }
                State::Scheduled | State::Upcoming => {
                    let remaining = effective.due_at.saturating_sub(now_ns);
                    let days = (remaining + NS_PER_DAY - 1) / NS_PER_DAY;
                    if days == 1 {
                        ("tomorrow".to_string(), None)
                    } else {
                        (format!("in {days}d"), None)
                    }
                }
            };
            let last = events
                .iter()
                .filter(|e| e.kind != EventKind::Unknown)
                .map(ns)
                .chain([ns(&add)])
                .max()
                .unwrap_or(0);
            Reminder {
                add,
                events,
                state,
                effective,
                lead,
                label,
                overdue_days,
                close,
                last_activity: last.to_string(),
            }
        })
        .collect();
    reminders.sort_by(|a, b| {
        (a.state == State::Closed)
            .cmp(&(b.state == State::Closed))
            .then_with(|| a.effective.due_at.cmp(&b.effective.due_at))
            .then_with(|| a.add.id.cmp(&b.add.id))
    });
    Folded {
        reminders,
        orphan_events,
    }
}

/// Classify a new reminder's text and `on` against the folded reminders. `Exists` needs
/// the same term set AND the same effective `on`, in a reminder that is not closed; a
/// half-or-more term overlap is `Similar`; closed reminders never block an add (the same
/// task next month is normal).
pub fn classify(text: &str, on: &str, reminders: &[Reminder]) -> Dedup {
    let wanted = question::terms(text);
    if wanted.is_empty() {
        return Dedup::New;
    }
    let exists: Vec<String> = reminders
        .iter()
        .filter(|r| r.state != State::Closed)
        .filter(|r| {
            r.effective.on == on && {
                let have = question::terms(r.add.text.as_deref().unwrap_or(""));
                have.len() == wanted.len() && wanted.iter().all(|t| have.contains(t))
            }
        })
        .map(|r| r.add.id.clone())
        .collect();
    if !exists.is_empty() {
        return Dedup::Exists(exists);
    }
    let similar: Vec<String> = reminders
        .iter()
        .filter(|r| r.state != State::Closed)
        .filter(|r| {
            let hay = r.add.text.clone().unwrap_or_default().to_lowercase();
            let hits = wanted.iter().filter(|t| question::occurs(&hay, t)).count();
            hits * 2 >= wanted.len()
        })
        .map(|r| r.add.id.clone())
        .collect();
    if similar.is_empty() {
        Dedup::New
    } else {
        Dedup::Similar(similar)
    }
}

fn check(what: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        return Err(format!("remind_control_chars: {what}"));
    }
    if value.chars().count() > max {
        return Err(format!("remind_too_long: {what} exceeds {max} characters"));
    }
    Ok(())
}

/// Bounds for `add`: text 600 characters, `blocks` 64, control characters refused, lead
/// 0-30.
pub fn validate_add(text: &str, blocks: Option<&str>, lead: u32) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("remind_text_empty".to_string());
    }
    check("text", text, MAX_TEXT)?;
    if let Some(b) = blocks {
        check("blocks", b, MAX_BLOCKS)?;
    }
    if lead > MAX_LEAD {
        return Err(format!("remind_lead_invalid: {lead} exceeds {MAX_LEAD}"));
    }
    Ok(())
}

/// Bounds for a defer or close note: 300 characters, control characters refused.
pub fn validate_note(note: &str) -> Result<(), String> {
    check("note", note, MAX_NOTE)
}

/// Refuse the past by instant, not by date: a day whose day has not ended at its own
/// offset is still accepted, anywhere in the world.
pub fn check_not_past(due: &Due, now_ns: i128) -> Result<(), String> {
    if due.due_at.saturating_add(NS_PER_DAY) <= now_ns {
        return Err("remind_date_past".to_string());
    }
    Ok(())
}

/// Resolve a lowercase hex prefix (at least 8 characters) to one reminder id, reusing
/// question's resolution and mapping its refusal words to remind's
/// (`remind_id_too_short`, `remind_not_found`, `remind_id_ambiguous`).
pub fn resolve_prefix<'a>(
    ids: impl Iterator<Item = &'a str>,
    prefix: &str,
) -> Result<String, String> {
    question::resolve_prefix(ids, prefix).map_err(|e| e.replacen("question_", "remind_", 1))
}
