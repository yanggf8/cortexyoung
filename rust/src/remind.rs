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
const NS_PER_HOUR: i128 = 3_600_000_000_000;
const NS_PER_MINUTE: i128 = 60_000_000_000;
/// Days early a date surfaces when no lead was given; a timed reminder surfaces at its instant.
const DEFAULT_DATE_LEAD: u32 = 1;
const DEFAULT_TIMED_LEAD: u32 = 0;
/// What `--on` accepts, for the refusal text.
const ON_FORMS: &str = "YYYY-MM-DD, YYYY-MM-DD±HH:MM, YYYY-MM-DDTHH:MM or YYYY-MM-DDTHH:MM±HH:MM";

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
    /// The time of day (`21:30`) of a timed reminder; absent on a date. A cort that does not
    /// know the field ignores it and still acts on `due_at`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "string_or_none"
    )]
    pub at: Option<String>,
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

/// A remote `at` of another type reads as absent, so one such row stays a date reminder instead
/// of failing the whole read.
fn string_or_none<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Some(s),
        _ => None,
    })
}

/// The kind as it is spelled in a stored body.
pub fn kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Add => "add",
        EventKind::Defer => "defer",
        EventKind::Close => "close",
        EventKind::Unknown => "unknown",
    }
}

/// An add or a defer carries its instant in `due_at`. A remote row of those kinds without a
/// decimal integer there is an error like any known kind whose fields do not parse: letting
/// it through would fold the reminder to the epoch (1970) and mark every read of it overdue.
fn check_due_at(event: &ReminderEvent) -> Result<(), String> {
    let decimal = event
        .due_at
        .as_deref()
        .is_some_and(|raw| raw.parse::<i128>().is_ok());
    if matches!(event.kind, EventKind::Add | EventKind::Defer) && !decimal {
        return Err(format!(
            "remind_due_at_invalid: {} {} needs a decimal due_at",
            kind_name(event.kind),
            event.id
        ));
    }
    Ok(())
}

/// One stored event body. A row of a kind this version does not know is read leniently, so a
/// newer cort's fields cannot break an older one: whatever ids it carries (`reminder`, `of`)
/// are kept where they are strings. A body that is not JSON, or a known kind whose fields do
/// not parse, is still an error.
pub fn parse_event(body: &str) -> Result<ReminderEvent, String> {
    let error = match serde_json::from_str::<ReminderEvent>(body) {
        Ok(event) => return check_due_at(&event).map(|()| event),
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
        at: None,
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

/// The date (always the first ten characters, `YYYY-MM-DD`), the time of day of a timed
/// reminder (`HH:MM`), the offset used and the instant (ns since the epoch) of that time, or of
/// midnight for a date, at that offset. An explicit offset lives only in `utc_offset` and a
/// time only in `at`, never in `on`, so the two spellings of one day compare equal
/// downstream. Every reader compares `now` against `due_at`, so all machines flip at the
/// same instant; the day shown is always `on`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Due {
    pub on: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub utc_offset: String,
    pub due_at: i128,
}

impl Due {
    /// What a human or agent reads: [`on_text`].
    pub fn shown(&self) -> String {
        on_text(&self.on, self.at.as_deref(), &self.utc_offset)
    }

    /// The lead of a reminder of this kind when none was given.
    pub fn default_lead(&self) -> u32 {
        match self.at {
            Some(_) => DEFAULT_TIMED_LEAD,
            None => DEFAULT_DATE_LEAD,
        }
    }
}

/// The one way a reminder's date is printed: a date as it is, a timed one as
/// `YYYY-MM-DDTHH:MM±HH:MM`, which `--on` accepts back.
pub fn on_text(on: &str, at: Option<&str>, utc_offset: &str) -> String {
    match at {
        Some(at) => format!("{on}T{at}{utc_offset}"),
        None => on.to_string(),
    }
}

#[cfg(unix)]
use std::os::raw::{c_char, c_int, c_long};

/// `struct tm` for the `mktime` FFI. glibc and macOS lay it out the same way:
/// nine `c_int` fields `tm_sec..tm_isdst`, then `tm_gmtoff: c_long` and
/// `tm_zone: *const c_char` (musl matches). Only the first nine are inputs here; the tail
/// is zeroed so the struct is the size the ABI expects. One function, no libc crate --
/// the same minimal-FFI style as `heal.rs`'s `setsid`.
/// `remind_snapshot` reads local time through the same struct and `localtime_r`.
#[cfg(unix)]
#[repr(C)]
pub(crate) struct Tm {
    pub(crate) tm_sec: c_int,
    pub(crate) tm_min: c_int,
    pub(crate) tm_hour: c_int,
    pub(crate) tm_mday: c_int,
    pub(crate) tm_mon: c_int,
    pub(crate) tm_year: c_int,
    pub(crate) tm_wday: c_int,
    pub(crate) tm_yday: c_int,
    pub(crate) tm_isdst: c_int,
    pub(crate) tm_gmtoff: c_long,
    pub(crate) tm_zone: *const c_char,
}

#[cfg(unix)]
extern "C" {
    fn mktime(tm: *mut Tm) -> c_long;
    pub(crate) fn localtime_r(time: *const c_long, out: *mut Tm) -> *mut Tm;
}

/// Midnight of a calendar date in this machine's local zone, as ns since the epoch.
/// `tm_isdst = -1` makes `mktime` resolve the zone (and DST) for that date, not today's
/// offset. Refused (`remind_offset_unknown`) off unix or when `mktime` cannot answer; the
/// explicit-offset form of `parse_on` works everywhere.
pub fn local_midnight(y: i32, m: u32, d: u32) -> Result<i128, String> {
    local_secs(y, m, d, 0, 0).map(|(secs, _)| i128::from(secs) * NS_PER_SEC)
}

/// Seconds since the epoch of a local wall-clock time, and whether that time exists: in the hour
/// a clock change skips, `mktime` answers another wall clock.
///
/// `time_t` is `c_long` on every target this crate builds for (64-bit Linux and macOS),
/// so the `mktime` result needs no conversion here.
#[cfg(unix)]
fn local_secs(y: i32, m: u32, d: u32, hour: u32, min: u32) -> Result<(i64, bool), String> {
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: min as c_int,
        tm_hour: hour as c_int,
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
    let exists = (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour, tm.tm_min)
        == (
            y - 1900,
            m as c_int - 1,
            d as c_int,
            hour as c_int,
            min as c_int,
        );
    Ok((secs, exists))
}

#[cfg(not(unix))]
fn local_secs(_y: i32, _m: u32, _d: u32, _hour: u32, _min: u32) -> Result<(i64, bool), String> {
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

/// `±HH:MM` at the start of `b` as seconds, up to ±14:00, the whole zone range: +14:01..+14:59
/// is refused, and so is any minute past 59.
fn parse_offset(b: &[u8]) -> Option<i64> {
    let sign: i64 = match b.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes) = match (two(b, 1), two(b, 4), b.get(3)) {
        (Some(h), Some(m), Some(b':')) => (h, m),
        _ => return None,
    };
    if minutes > 59 || hours * 60 + minutes > 14 * 60 {
        return None;
    }
    Some(sign * (i64::from(hours) * 3600 + i64::from(minutes) * 60))
}

/// `HH:MM` at the start of `b`, 00:00 to 23:59.
fn parse_time(b: &[u8]) -> Option<(u32, u32)> {
    match (two(b, 0), two(b, 3), b.get(2)) {
        (Some(h), Some(m), Some(b':')) if h < 24 && m < 60 => Some((h, m)),
        _ => None,
    }
}

/// Parse `--on`: `YYYY-MM-DD` (midnight in this machine's local zone, offset reported),
/// `YYYY-MM-DD±HH:MM` (offsets up to ±14:00, `due_at` is pure arithmetic), or either with a
/// time of day, `YYYY-MM-DDTHH:MM` and `YYYY-MM-DDTHH:MM±HH:MM`. In every form `Due.on` is the
/// ten-character date; the offset lives only in `utc_offset` and the time only in `at`. The date
/// must be a real calendar date; anything else is `remind_date_invalid`. A local time a clock
/// change skips is `remind_time_skipped`.
pub fn parse_on(raw: &str) -> Result<Due, String> {
    let invalid = || format!("remind_date_invalid: {raw} (accepted: {ON_FORMS})");
    let b = raw.as_bytes();
    // Length 16 is a date with an offset or a date with a time; byte 10 says which.
    let timed = b.get(10) == Some(&b'T');
    if !matches!((b.len(), timed), (10, false) | (16, _) | (22, true)) {
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
    let (hour, minute) = if timed {
        parse_time(&b[11..16]).ok_or_else(invalid)?
    } else {
        (0, 0)
    };
    let naive =
        naive_utc_seconds(year, month, day) + i128::from(hour) * 3600 + i128::from(minute) * 60;
    let on = raw[..10].to_string();
    let at = timed.then(|| raw[11..16].to_string());
    // What follows the date and time: an offset, or nothing for the local zone.
    let rest = &b[if timed { 16 } else { 10 }..];
    if !rest.is_empty() {
        let offset = parse_offset(rest).ok_or_else(invalid)?;
        return Ok(Due {
            on,
            at,
            utc_offset: format_offset(offset),
            due_at: (naive - i128::from(offset)) * NS_PER_SEC,
        });
    }
    let (secs, exists) = local_secs(year, month, day, hour, minute)?;
    // The offset is the difference between that instant and the same wall clock read as UTC.
    let offset = i64::try_from(naive - i128::from(secs)).unwrap_or(0);
    if timed {
        if !exists {
            return Err(format!(
                "remind_time_skipped: {raw} does not exist in this machine's time zone (a clock change skips it)"
            ));
        }
        // `shown` must give `due_at` back, so a zone whose offset has seconds or lies outside
        // ±14:00 needs an explicit offset.
        if parse_offset(format_offset(offset).as_bytes()) != Some(offset) {
            return Err("remind_offset_unknown".to_string());
        }
    }
    Ok(Due {
        on,
        at,
        utc_offset: format_offset(offset),
        due_at: i128::from(secs) * NS_PER_SEC,
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

/// The due of a stored add or defer. `at` is kept only when date, time and offset are spelled
/// exactly as `parse_on` writes them and name the instant in `due_at`; a row from another
/// machine where they do not (malformed or contradictory) is a date reminder at its instant, so
/// neither field is ever printed unchecked.
fn due_from(event: &ReminderEvent) -> Due {
    let due = Due {
        at: event.at.clone(),
        ..date_from(event)
    };
    match &due.at {
        Some(_) if parse_on(&due.shown()).as_ref() != Ok(&due) => date_from(event),
        _ => due,
    }
}

fn date_from(event: &ReminderEvent) -> Due {
    Due {
        on: event.on.clone().unwrap_or_default(),
        at: None,
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

/// How far away a due instant is: whole days for a date (`tomorrow` for one); for a timed
/// reminder hours under a day and minutes under an hour, all rounded up, and never `tomorrow`.
fn remaining_label(effective: &Due, now_ns: i128) -> String {
    let remaining = effective.due_at.saturating_sub(now_ns);
    let ceil = |unit: i128| remaining.saturating_add(unit - 1) / unit;
    let days = ceil(NS_PER_DAY);
    match effective.at {
        Some(_) if remaining < NS_PER_HOUR => format!("in {}m", ceil(NS_PER_MINUTE)),
        Some(_) if remaining < NS_PER_DAY => format!("in {}h", ceil(NS_PER_HOUR)),
        None if days == 1 => "tomorrow".to_string(),
        _ => format!("in {days}d"),
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
            let newest = events[..close_index.unwrap_or(events.len())]
                .iter()
                .rev()
                .find(|e| e.kind == EventKind::Defer)
                .unwrap_or(&add);
            let effective = due_from(newest);
            // A defer written before defers carried a lead leaves the add's.
            let lead = newest
                .lead
                .or(add.lead)
                .unwrap_or_else(|| effective.default_lead());
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
                // A timed reminder is due for 24 hours from its instant, into the next day.
                State::Due if effective.at.is_some() => ("due".to_string(), None),
                State::Due => ("today".to_string(), None),
                State::Overdue => {
                    let days = i64::try_from(now_ns.saturating_sub(effective.due_at) / NS_PER_DAY)
                        .unwrap_or(i64::MAX);
                    (format!("overdue {days}d"), Some(days))
                }
                State::Scheduled | State::Upcoming => (remaining_label(&effective, now_ns), None),
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

/// Classify a new reminder's text and instant against the folded reminders. `Exists` needs
/// the same term set AND the same effective `due_at` (the instant), in a reminder that is
/// not closed; a half-or-more term overlap is `Similar`; closed reminders never block an
/// add (the same task next month is normal).
pub fn classify(text: &str, due_at: i128, reminders: &[Reminder]) -> Dedup {
    let wanted = question::terms(text);
    if wanted.is_empty() {
        return Dedup::New;
    }
    let exists: Vec<String> = reminders
        .iter()
        .filter(|r| r.state != State::Closed)
        .filter(|r| {
            r.effective.due_at == due_at && {
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
/// 0-30 when given.
pub fn validate_add(text: &str, blocks: Option<&str>, lead: Option<u32>) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("remind_text_empty".to_string());
    }
    check("text", text, MAX_TEXT)?;
    if let Some(b) = blocks {
        check("blocks", b, MAX_BLOCKS)?;
    }
    if let Some(lead) = lead.filter(|lead| *lead > MAX_LEAD) {
        return Err(format!("remind_lead_invalid: {lead} exceeds {MAX_LEAD}"));
    }
    Ok(())
}

/// Bounds for a defer or close note: 300 characters, control characters refused.
pub fn validate_note(note: &str) -> Result<(), String> {
    check("note", note, MAX_NOTE)
}

/// Refuse the past by instant, not by date: a day whose day has not ended at its own
/// offset is still accepted, anywhere in the world. A timed due is past at its instant.
pub fn check_not_past(due: &Due, now_ns: i128) -> Result<(), String> {
    let ends_at = match due.at {
        Some(_) => due.due_at,
        None => due.due_at.saturating_add(NS_PER_DAY),
    };
    if ends_at <= now_ns {
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
