//! Pure fold of the shared lifecycle table over the ledger (issue #11, phase 1a).
//!
//! Nothing here touches storage: events, the ledger and the clock go in, a `LifecycleView` comes
//! out. Style follows `remind.rs`: an unknown kind from a newer build decodes and changes nothing.

use crate::memory::{active_entries, MemoryEntry, MemoryLedger};
use crate::memory_tiers::{
    parse_recorded_ns, place, Placement, Tier, FORGETTING_NS, SHORT_TERM_NS,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    On,
    Off,
    Promote,
    Archive,
    Drop,
    Restore,
    /// A kind written by a newer cort: kept in its entry's history (or the orphans), changes
    /// nothing.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub id: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
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

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::On => "on",
        Kind::Off => "off",
        Kind::Promote => "promote",
        Kind::Archive => "archive",
        Kind::Drop => "drop",
        Kind::Restore => "restore",
        Kind::Unknown => "unknown",
    }
}

const KNOWN_KINDS: [&str; 6] = ["on", "off", "promote", "archive", "drop", "restore"];

enum Row {
    Event(Box<LifecycleEvent>),
    Unreadable,
    /// A newer build's row we cannot even place: it could change nothing, so it is not counted.
    Ignored,
}

fn classify(body: &str) -> Row {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let future_kind = value
        .as_ref()
        .and_then(|v| v.get("kind"))
        .and_then(|k| k.as_str())
        .is_some_and(|k| !KNOWN_KINDS.contains(&k));
    if let (true, Some(value)) = (future_kind, value.as_ref()) {
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
        return match (text("id"), text("recorded_at")) {
            (Some(id), Some(recorded_at)) if parse_recorded_ns(&recorded_at).is_some() => {
                Row::Event(Box::new(LifecycleEvent {
                    id,
                    kind: Kind::Unknown,
                    entry: text("entry"),
                    superseded_by: None,
                    quote: None,
                    note: None,
                    recorded_at,
                    recorded_machine: text("recorded_machine"),
                    recorded_machine_source: text("recorded_machine_source"),
                    harness: text("harness"),
                }))
            }
            _ => Row::Ignored,
        };
    }
    match serde_json::from_str::<LifecycleEvent>(body) {
        Ok(event) if time_of(&event).is_some() && has_required_entry(&event) => {
            Row::Event(Box::new(event))
        }
        _ => Row::Unreadable,
    }
}

/// Decode table bodies. A row whose JSON does not parse, whose `recorded_at` is not an in-range
/// integer, or whose known kind (all but `on`/`off`) lacks `entry` is skipped and counted. A kind
/// this build does not know decodes as `Unknown` when it carries a usable `id` and `recorded_at`,
/// and is otherwise skipped without counting.
pub fn decode_rows(bodies: &[String]) -> (Vec<LifecycleEvent>, usize) {
    let mut events = Vec::new();
    let mut unreadable = 0;
    for body in bodies {
        match classify(body) {
            Row::Event(event) => events.push(*event),
            Row::Unreadable => unreadable += 1,
            Row::Ignored => {}
        }
    }
    (events, unreadable)
}

fn has_required_entry(event: &LifecycleEvent) -> bool {
    matches!(event.kind, Kind::On | Kind::Off | Kind::Unknown) || event.entry.is_some()
}

fn time_of(event: &LifecycleEvent) -> Option<i128> {
    parse_recorded_ns(&event.recorded_at)
}

/// The transition table: which events a state accepts. One table, used by `apply`.
pub fn allowed(state: State) -> &'static [Kind] {
    match state {
        State::ShortTerm | State::ShortTermExpired => &[Kind::Promote, Kind::Drop, Kind::Archive],
        State::LongTerm | State::ForgettingCandidate => &[Kind::Archive],
        State::Archived | State::Dropped => &[Kind::Restore],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    ShortTerm,
    ShortTermExpired,
    LongTerm,
    ForgettingCandidate,
    Archived,
    Dropped,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShortTerm => "short_term",
            Self::ShortTermExpired => "short_term_expired",
            Self::LongTerm => "long_term",
            Self::ForgettingCandidate => "forgetting_candidate",
            Self::Archived => "archived",
            Self::Dropped => "dropped",
        }
    }

    /// The inverse of `as_str`.
    pub fn parse(s: &str) -> Option<State> {
        Some(match s {
            "short_term" => Self::ShortTerm,
            "short_term_expired" => Self::ShortTermExpired,
            "long_term" => Self::LongTerm,
            "forgetting_candidate" => Self::ForgettingCandidate,
            "archived" => Self::Archived,
            "dropped" => Self::Dropped,
            _ => return None,
        })
    }

    pub fn is_hidden(self) -> bool {
        matches!(self, Self::Archived | Self::Dropped)
    }

    pub fn is_due(self) -> bool {
        matches!(self, Self::ShortTermExpired | Self::ForgettingCandidate)
    }
}

#[derive(Clone, Debug)]
pub struct EntryState {
    pub state: State,
    pub due_at_ns: Option<i128>,
    pub history: Vec<LifecycleEvent>,
}

#[derive(Clone, Debug)]
pub struct LifecycleView {
    pub enabled: bool,
    pub epoch_ns: Option<i128>,
    pub entries: HashMap<String, EntryState>,
    pub superseded: Vec<(String, Vec<LifecycleEvent>)>,
    pub orphans: Vec<LifecycleEvent>,
    pub unreadable: usize,
}

impl LifecycleView {
    /// `None` when the project is not enabled (even if `entries` was filled by an epoch override).
    pub fn state_of(&self, id: &str) -> Option<State> {
        if !self.enabled {
            return None;
        }
        self.entries.get(id).map(|e| e.state)
    }
}

#[derive(Clone, Copy)]
enum Base {
    Short {
        expires: i128,
    },
    /// `None` when `recorded_at` did not parse: long-term, never due.
    Long {
        last: Option<i128>,
    },
}

#[derive(Clone, Copy)]
struct Running {
    base: Base,
    hidden: Option<State>,
}

fn initial(entry: &MemoryEntry, epoch_ns: i128, now_ns: i128) -> Running {
    let base = match parse_recorded_ns(&entry.recorded_at) {
        None => Base::Long { last: None },
        Some(recorded) => {
            let Placement {
                tier,
                expires_at_ns,
                ..
            } = place(recorded, epoch_ns, now_ns);
            match (tier, expires_at_ns) {
                (Tier::Short, Some(expires)) => Base::Short { expires },
                _ => Base::Long {
                    last: Some(recorded),
                },
            }
        }
    };
    Running { base, hidden: None }
}

fn apply(run: &mut Running, kind: Kind, at: i128) {
    let (state, _) = derive(*run, at);
    if !allowed(state).contains(&kind) {
        return;
    }
    match kind {
        Kind::Promote => run.base = Base::Long { last: Some(at) },
        Kind::Drop => run.hidden = Some(State::Dropped),
        Kind::Archive => run.hidden = Some(State::Archived),
        Kind::Restore => {
            run.hidden = None;
            run.base = match run.base {
                Base::Short { .. } => Base::Short {
                    expires: at.saturating_add(SHORT_TERM_NS),
                },
                Base::Long { .. } => Base::Long { last: Some(at) },
            };
        }
        _ => {}
    }
}

fn derive(run: Running, now_ns: i128) -> (State, Option<i128>) {
    if let Some(hidden) = run.hidden {
        return (hidden, None);
    }
    match run.base {
        Base::Short { expires } => {
            let state = if now_ns >= expires {
                State::ShortTermExpired
            } else {
                State::ShortTerm
            };
            (state, Some(expires))
        }
        Base::Long { last: None } => (State::LongTerm, None),
        Base::Long { last: Some(last) } => {
            let due = last.saturating_add(FORGETTING_NS);
            let state = if due <= now_ns {
                State::ForgettingCandidate
            } else {
                State::LongTerm
            };
            (state, Some(due))
        }
    }
}

/// Fold `events` over the active entries of `ledger` at `now_ns`. With `epoch_override` the states
/// are computed from that epoch even when the project is disabled; `enabled` still reports the
/// real switch.
pub fn fold(
    ledger: &MemoryLedger,
    events: Vec<LifecycleEvent>,
    unreadable: usize,
    now_ns: i128,
    epoch_override: Option<i128>,
) -> LifecycleView {
    let mut ordered: Vec<(i128, LifecycleEvent)> = events
        .into_iter()
        .filter_map(|e| time_of(&e).map(|t| (t, e)))
        .collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));

    let mut enabled = false;
    let mut run_epoch: Option<i128> = None;
    for (at, event) in &ordered {
        match event.kind {
            Kind::On if !enabled => {
                enabled = true;
                run_epoch = Some(*at);
            }
            Kind::Off => {
                enabled = false;
                run_epoch = None;
            }
            _ => {}
        }
    }
    let epoch_ns = epoch_override.or(run_epoch);

    let active = active_entries(ledger);
    let active_ids: HashSet<&str> = active.iter().map(|e| e.id.as_str()).collect();
    let ledger_ids: HashSet<&str> = ledger.entries.iter().map(|e| e.id.as_str()).collect();

    let mut per_entry: HashMap<String, Vec<(i128, LifecycleEvent)>> = HashMap::new();
    let mut superseded: HashMap<String, Vec<LifecycleEvent>> = HashMap::new();
    let mut orphans = Vec::new();
    for (at, event) in ordered {
        if matches!(event.kind, Kind::On | Kind::Off) {
            continue;
        }
        match event.entry.clone() {
            Some(id) if active_ids.contains(id.as_str()) => {
                per_entry.entry(id).or_default().push((at, event))
            }
            Some(id) if ledger_ids.contains(id.as_str()) => {
                superseded.entry(id).or_default().push(event)
            }
            _ => orphans.push(event),
        }
    }
    let mut superseded: Vec<(String, Vec<LifecycleEvent>)> = superseded.into_iter().collect();
    superseded.sort_by(|a, b| a.0.cmp(&b.0));

    let mut entries = HashMap::new();
    if let Some(epoch) = epoch_ns {
        for entry in active {
            let mut run = initial(entry, epoch, now_ns);
            let mut history = Vec::new();
            for (at, event) in per_entry.remove(&entry.id).unwrap_or_default() {
                apply(&mut run, event.kind, at);
                history.push(event);
            }
            let (state, due_at_ns) = derive(run, now_ns);
            entries.insert(
                entry.id.clone(),
                EntryState {
                    state,
                    due_at_ns,
                    history,
                },
            );
        }
    }
    LifecycleView {
        enabled,
        epoch_ns,
        entries,
        superseded,
        orphans,
        unreadable,
    }
}

/// `recorded_at` for a new event: `max(now, latest relevant event + 1 ns)`. Relevant means the
/// events on `entry`, or the switch (`on`/`off`) events when `entry` is `None`.
pub fn stamp(now_ns: i128, events: &[LifecycleEvent], entry: Option<&str>) -> i128 {
    events
        .iter()
        .filter(|e| match entry {
            Some(id) => e.entry.as_deref() == Some(id),
            None => matches!(e.kind, Kind::On | Kind::Off),
        })
        .filter_map(time_of)
        .max()
        .map_or(now_ns, |latest| now_ns.max(latest.saturating_add(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_parse_round_trips_as_str() {
        for s in [
            State::ShortTerm,
            State::ShortTermExpired,
            State::LongTerm,
            State::ForgettingCandidate,
            State::Archived,
            State::Dropped,
        ] {
            assert_eq!(State::parse(s.as_str()), Some(s));
        }
        assert_eq!(State::parse("nope"), None);
    }

    const DAY: i128 = crate::memory_tiers::DAY_NS;
    const EPOCH: i128 = 1_000_000 * 1_000_000_000;

    fn entry(id: &str, recorded_at: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            key: format!("key-{id}"),
            claim: "claim".to_string(),
            source: String::new(),
            source_line: 0,
            source_sha256: String::new(),
            unsourced: true,
            source_line_sha256: None,
            recorded_at: recorded_at.to_string(),
            supersedes: None,
            resolves: Vec::new(),
            recorded_machine: None,
            recorded_machine_source: None,
        }
    }

    fn ledger(entries: Vec<MemoryEntry>) -> MemoryLedger {
        MemoryLedger {
            entries,
            ..MemoryLedger::default()
        }
    }

    fn ev(id: &str, kind: Kind, entry: Option<&str>, at: i128) -> LifecycleEvent {
        LifecycleEvent {
            id: id.to_string(),
            kind,
            entry: entry.map(str::to_string),
            superseded_by: None,
            quote: None,
            note: None,
            recorded_at: at.to_string(),
            recorded_machine: None,
            recorded_machine_source: None,
            harness: None,
        }
    }

    fn on(id: &str, at: i128) -> LifecycleEvent {
        ev(id, Kind::On, None, at)
    }

    fn state(view: &LifecycleView, id: &str) -> State {
        view.entries[id].state
    }

    #[test]
    fn no_switch_is_disabled() {
        let l = ledger(vec![entry("x", &EPOCH.to_string())]);
        let view = fold(&l, vec![], 0, EPOCH, None);
        assert!(!view.enabled);
        assert_eq!(view.epoch_ns, None);
        assert_eq!(view.state_of("x"), None);
        assert!(view.entries.is_empty());
    }

    #[test]
    fn off_after_on_disables() {
        let l = ledger(vec![entry("x", &EPOCH.to_string())]);
        let events = vec![on("o1", EPOCH), ev("o2", Kind::Off, None, EPOCH + DAY)];
        let view = fold(&l, events, 0, EPOCH + 2 * DAY, None);
        assert!(!view.enabled);
        assert_eq!(view.state_of("x"), None);
        assert_eq!(view.epoch_ns, None);
        assert!(view.entries.is_empty());
    }

    #[test]
    fn epoch_is_on_that_began_current_run() {
        let l = ledger(vec![]);
        let v = fold(&l, vec![on("a", 10), on("b", 20)], 0, 100, None);
        assert!(v.enabled);
        assert_eq!(v.epoch_ns, Some(10));
        let events = vec![on("a", 10), ev("b", Kind::Off, None, 20), on("c", 30)];
        let v = fold(&l, events, 0, 100, None);
        assert_eq!(v.epoch_ns, Some(30));
    }

    #[test]
    fn earlier_run_events_still_apply() {
        let l = ledger(vec![entry("x", &EPOCH.to_string())]);
        let events = vec![
            on("o1", EPOCH),
            ev("a1", Kind::Archive, Some("x"), EPOCH + DAY),
            ev("o2", Kind::Off, None, EPOCH + 2 * DAY),
            on("o3", EPOCH + 3 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 4 * DAY, None);
        assert!(view.enabled);
        assert_eq!(state(&view, "x"), State::Archived);
    }

    #[test]
    fn promote_short_to_long() {
        let l = ledger(vec![entry("x", &(EPOCH + DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("p", Kind::Promote, Some("x"), EPOCH + 2 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 3 * DAY, None);
        let s = &view.entries["x"];
        assert_eq!(s.state, State::LongTerm);
        assert_eq!(s.due_at_ns, Some(EPOCH + 2 * DAY + 60 * DAY));
        assert_eq!(s.history.len(), 1);
    }

    #[test]
    fn drop_short_hides() {
        let l = ledger(vec![entry("x", &(EPOCH + DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("d", Kind::Drop, Some("x"), EPOCH + 2 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 3 * DAY, None);
        assert_eq!(state(&view, "x"), State::Dropped);
        assert!(state(&view, "x").is_hidden());
        assert_eq!(view.entries["x"].due_at_ns, None);
    }

    #[test]
    fn archive_long_hides() {
        let l = ledger(vec![entry("x", &(EPOCH - 30 * DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("x"), EPOCH + DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 3 * DAY, None);
        assert_eq!(state(&view, "x"), State::Archived);
    }

    #[test]
    fn restore_archived_long_refreshes_activity() {
        let l = ledger(vec![entry("x", &(EPOCH - 30 * DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("x"), EPOCH + DAY),
            ev("r", Kind::Restore, Some("x"), EPOCH + 2 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 3 * DAY, None);
        let s = &view.entries["x"];
        assert_eq!(s.state, State::LongTerm);
        assert_eq!(s.due_at_ns, Some(EPOCH + 2 * DAY + 60 * DAY));
    }

    #[test]
    fn restore_dropped_short_expires_seven_days_later() {
        let l = ledger(vec![entry("x", &(EPOCH + DAY / 2).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("d", Kind::Drop, Some("x"), EPOCH + DAY),
            ev("r", Kind::Restore, Some("x"), EPOCH + 2 * DAY),
        ];
        let expiry = EPOCH + 9 * DAY;
        let before = fold(&l, events.clone(), 0, expiry - 1, None);
        assert_eq!(state(&before, "x"), State::ShortTerm);
        assert_eq!(before.entries["x"].due_at_ns, Some(expiry));
        let at = fold(&l, events, 0, expiry, None);
        assert_eq!(state(&at, "x"), State::ShortTermExpired);
        assert!(state(&at, "x").is_due());
    }

    #[test]
    fn not_allowed_event_is_inert() {
        let l = ledger(vec![entry("x", &(EPOCH - 30 * DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("p", Kind::Promote, Some("x"), EPOCH + DAY),
            ev("d", Kind::Drop, Some("x"), EPOCH + 2 * DAY),
            ev("r", Kind::Restore, Some("x"), EPOCH + 3 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 4 * DAY, None);
        let s = &view.entries["x"];
        assert_eq!(s.state, State::LongTerm);
        assert_eq!(s.due_at_ns, Some(EPOCH - 30 * DAY + 60 * DAY));
        assert_eq!(s.history.len(), 3);
    }

    #[test]
    fn equal_recorded_at_ordered_by_id() {
        let l = ledger(vec![entry("x", &(EPOCH - 30 * DAY).to_string())]);
        let t = EPOCH + DAY;
        let archive_first = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("x"), t),
            ev("b", Kind::Restore, Some("x"), t),
        ];
        let view = fold(&l, archive_first, 0, t + DAY, None);
        assert_eq!(state(&view, "x"), State::LongTerm);
        let restore_first = vec![
            on("o", EPOCH),
            ev("b", Kind::Archive, Some("x"), t),
            ev("a", Kind::Restore, Some("x"), t),
        ];
        let view = fold(&l, restore_first, 0, t + DAY, None);
        assert_eq!(state(&view, "x"), State::Archived);
    }

    #[test]
    fn unknown_kind_changes_nothing() {
        let l = ledger(vec![entry("x", &(EPOCH - 30 * DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("u", Kind::Unknown, Some("x"), EPOCH + DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 2 * DAY, None);
        let s = &view.entries["x"];
        assert_eq!(s.state, State::LongTerm);
        assert_eq!(s.due_at_ns, Some(EPOCH - 30 * DAY + 60 * DAY));
        assert_eq!(s.history.len(), 1);
    }

    #[test]
    fn orphan_event_listed() {
        let l = ledger(vec![entry("x", &EPOCH.to_string())]);
        let events = vec![on("o", EPOCH), ev("a", Kind::Archive, Some("nope"), EPOCH)];
        let view = fold(&l, events, 0, EPOCH + DAY, None);
        assert_eq!(view.orphans.len(), 1);
        assert_eq!(view.orphans[0].id, "a");
        assert!(view.superseded.is_empty());
    }

    #[test]
    fn event_on_superseded_entry_listed_as_superseded() {
        let mut new = entry("new", &(EPOCH + DAY).to_string());
        new.supersedes = Some("old".to_string());
        let l = ledger(vec![entry("old", &EPOCH.to_string()), new]);
        let events = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("old"), EPOCH + 2 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 3 * DAY, None);
        assert!(!view.entries.contains_key("old"));
        assert_eq!(state(&view, "new"), State::ShortTerm);
        assert_eq!(view.superseded.len(), 1);
        assert_eq!(view.superseded[0].0, "old");
        assert_eq!(view.superseded[0].1[0].id, "a");
        assert!(view.orphans.is_empty());
    }

    #[test]
    fn decode_skips_and_counts_bad_rows() {
        let bodies: Vec<String> = [
            r#"{"id":"e1","kind":"on","recorded_at":"100"}"#,
            "not json",
            r#"{"id":"e2","kind":"archive","recorded_at":"200"}"#,
            r#"{"id":"e3","kind":"archive","entry":"x","recorded_at":"300"}"#,
            r#"{"id":"e4","kind":"merge","entry":"x","recorded_at":"400"}"#,
            r#"{"id":"e5","kind":"on","recorded_at":"later"}"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (events, unreadable) = decode_rows(&bodies);
        assert_eq!(unreadable, 3);
        let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["e1", "e3", "e4"]);
        assert_eq!(events[2].kind, Kind::Unknown);
        assert_eq!(kind_name(Kind::Promote), "promote");
        assert_eq!(kind_name(Kind::Unknown), "unknown");
        let view = fold(&ledger(vec![]), events, unreadable, 1000, None);
        assert_eq!(view.unreadable, 3);
        assert!(view.enabled);
    }

    #[test]
    fn unparsed_recorded_at_is_long_term_never_due() {
        let l = ledger(vec![entry("x", "garbage")]);
        let events = vec![on("o", EPOCH)];
        let view = fold(&l, events.clone(), 0, EPOCH + 1000 * DAY, None);
        let s = &view.entries["x"];
        assert_eq!(s.state, State::LongTerm);
        assert_eq!(s.due_at_ns, None);
        assert!(!s.state.is_due());
        let mut with_archive = events;
        with_archive.push(ev("a", Kind::Archive, Some("x"), EPOCH + DAY));
        let view = fold(&l, with_archive, 0, EPOCH + 2 * DAY, None);
        assert_eq!(state(&view, "x"), State::Archived);
    }

    #[test]
    fn placement_boundaries() {
        let l = ledger(vec![
            entry("edge", &(EPOCH - 7 * DAY).to_string()),
            entry("older", &(EPOCH - 7 * DAY - 1).to_string()),
        ]);
        let events = vec![on("o", EPOCH)];
        let expiry = EPOCH + 7 * DAY;
        let before = fold(&l, events.clone(), 0, expiry - 1, None);
        assert_eq!(state(&before, "edge"), State::ShortTerm);
        assert_eq!(before.entries["edge"].due_at_ns, Some(expiry));
        assert_eq!(state(&before, "older"), State::LongTerm);
        let at = fold(&l, events, 0, expiry, None);
        assert_eq!(state(&at, "edge"), State::ShortTermExpired);
    }

    #[test]
    fn forgetting_candidate_at_sixty_days() {
        let rec = EPOCH - 30 * DAY;
        let l = ledger(vec![entry("x", &rec.to_string())]);
        let events = vec![on("o", EPOCH)];
        let due = rec + 60 * DAY;
        let before = fold(&l, events.clone(), 0, due - 1, None);
        assert_eq!(state(&before, "x"), State::LongTerm);
        let at = fold(&l, events, 0, due, None);
        assert_eq!(state(&at, "x"), State::ForgettingCandidate);
        assert!(state(&at, "x").is_due());
        assert_eq!(State::ForgettingCandidate.as_str(), "forgetting_candidate");
    }

    #[test]
    fn epoch_override_computes_states_when_disabled() {
        let l = ledger(vec![entry("x", &EPOCH.to_string())]);
        let view = fold(&l, vec![], 0, EPOCH + 8 * DAY, Some(EPOCH));
        assert!(!view.enabled);
        assert_eq!(view.state_of("x"), None);
        assert_eq!(state(&view, "x"), State::ShortTermExpired);
    }

    #[test]
    fn future_kind_with_odd_fields_is_unknown_not_unreadable() {
        let bodies: Vec<String> = [
            r#"{"id":"e","kind":"merge","entry":["a","b"],"recorded_at":"1"}"#,
            r#"{"kind":"merge","recorded_at":"1"}"#,
            r#"{"id":"f","kind":"merge","recorded_at":"later"}"#,
            r#"{"id":"g","kind":"archive","recorded_at":"1"}"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (events, unreadable) = decode_rows(&bodies);
        assert_eq!(unreadable, 1, "only the known-kind row counts");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "e");
        assert_eq!(events[0].kind, Kind::Unknown);
        assert_eq!(events[0].entry, None);
    }

    #[test]
    fn far_future_event_time_is_unreadable() {
        let far = format!("{}", 3_000_000i128 * DAY);
        let bodies = vec![
            format!(r#"{{"id":"a","kind":"on","recorded_at":"{far}"}}"#),
            format!(r#"{{"id":"b","kind":"archive","entry":"x","recorded_at":"{far}"}}"#),
        ];
        let (events, unreadable) = decode_rows(&bodies);
        assert!(events.is_empty());
        assert_eq!(unreadable, 2);
    }

    #[test]
    fn allowed_matches_spec_table() {
        let short = [Kind::Promote, Kind::Drop, Kind::Archive];
        assert_eq!(allowed(State::ShortTerm), short);
        assert_eq!(allowed(State::ShortTermExpired), short);
        assert_eq!(allowed(State::LongTerm), [Kind::Archive]);
        assert_eq!(allowed(State::ForgettingCandidate), [Kind::Archive]);
        assert_eq!(allowed(State::Archived), [Kind::Restore]);
        assert_eq!(allowed(State::Dropped), [Kind::Restore]);
    }

    #[test]
    fn events_ordered_by_numeric_time_then_id() {
        let l = ledger(vec![entry("x", &(EPOCH - 30 * DAY).to_string())]);
        // Numeric order: archive@999 then restore@1000 (string order would reverse them).
        // The epoch is set so these tiny times are in range.
        let events = vec![
            on("o", 1),
            ev("r", Kind::Restore, Some("x"), 1000),
            ev("a", Kind::Archive, Some("x"), 999),
        ];
        let view = fold(&l, events, 0, 2000, None);
        let s = &view.entries["x"];
        assert_eq!(s.state, State::LongTerm);
        let order: Vec<&str> = s.history.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(order, ["a", "r"]);
        // Same time: id decides, whatever the Vec order.
        let events = vec![
            on("o", 1),
            ev("z", Kind::Restore, Some("x"), 5),
            ev("m", Kind::Archive, Some("x"), 5),
        ];
        let view = fold(&l, events, 0, 2000, None);
        assert_eq!(state(&view, "x"), State::LongTerm);
    }

    #[test]
    fn epoch_override_wins_when_enabled() {
        let rec = EPOCH + 10 * DAY;
        let l = ledger(vec![entry("x", &rec.to_string())]);
        let events = vec![on("o", EPOCH)];
        // Real epoch EPOCH: recorded after it, short-term.
        let real = fold(&l, events.clone(), 0, rec + DAY, None);
        assert_eq!(state(&real, "x"), State::ShortTerm);
        // Override far later: recorded before override - 7d, long-term.
        let over = EPOCH + 30 * DAY;
        let view = fold(&l, events, 0, rec + DAY, Some(over));
        assert!(view.enabled);
        assert_eq!(view.epoch_ns, Some(over));
        assert_eq!(state(&view, "x"), State::LongTerm);
    }

    #[test]
    fn promote_and_drop_allowed_when_expired() {
        let rec = EPOCH + DAY;
        let l = ledger(vec![
            entry("p", &rec.to_string()),
            entry("d", &rec.to_string()),
        ]);
        let now = EPOCH + 30 * DAY;
        let events = vec![
            on("o", EPOCH),
            ev("e1", Kind::Promote, Some("p"), now - DAY),
            ev("e2", Kind::Drop, Some("d"), now - DAY),
        ];
        let view = fold(&l, events, 0, now, None);
        assert_eq!(state(&view, "p"), State::LongTerm);
        assert_eq!(state(&view, "d"), State::Dropped);
    }

    #[test]
    fn archive_on_dropped_is_inert() {
        let l = ledger(vec![entry("x", &(EPOCH + DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("d", Kind::Drop, Some("x"), EPOCH + 2 * DAY),
            ev("a", Kind::Archive, Some("x"), EPOCH + 3 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 4 * DAY, None);
        assert_eq!(state(&view, "x"), State::Dropped);
        assert_eq!(view.entries["x"].history.len(), 2);
    }

    #[test]
    fn restore_archived_short_term() {
        let l = ledger(vec![entry("x", &(EPOCH + DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("x"), EPOCH + 2 * DAY),
            ev("r", Kind::Restore, Some("x"), EPOCH + 3 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 4 * DAY, None);
        assert_eq!(state(&view, "x"), State::ShortTerm);
        assert_eq!(view.entries["x"].due_at_ns, Some(EPOCH + 10 * DAY));
    }

    #[test]
    fn promote_archive_restore_returns_long() {
        let l = ledger(vec![entry("x", &(EPOCH + DAY).to_string())]);
        let events = vec![
            on("o", EPOCH),
            ev("p", Kind::Promote, Some("x"), EPOCH + 2 * DAY),
            ev("a", Kind::Archive, Some("x"), EPOCH + 3 * DAY),
            ev("r", Kind::Restore, Some("x"), EPOCH + 4 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 5 * DAY, None);
        assert_eq!(state(&view, "x"), State::LongTerm);
        assert_eq!(view.entries["x"].due_at_ns, Some(EPOCH + 64 * DAY));
    }

    #[test]
    fn resolves_marks_superseded() {
        let mut new = entry("new", &(EPOCH + DAY).to_string());
        new.resolves = vec!["old".to_string()];
        let l = ledger(vec![entry("old", &EPOCH.to_string()), new]);
        let events = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("old"), EPOCH + 2 * DAY),
        ];
        let view = fold(&l, events, 0, EPOCH + 3 * DAY, None);
        assert!(!view.entries.contains_key("old"));
        assert_eq!(view.superseded.len(), 1);
        assert_eq!(view.superseded[0].0, "old");
    }

    #[test]
    fn unparsed_entry_restore_counts_as_activity() {
        let l = ledger(vec![entry("x", "garbage")]);
        let events = vec![
            on("o", EPOCH),
            ev("a", Kind::Archive, Some("x"), EPOCH + DAY),
            ev("r", Kind::Restore, Some("x"), EPOCH + 2 * DAY),
        ];
        let due = EPOCH + 2 * DAY + 60 * DAY;
        let before = fold(&l, events.clone(), 0, due - 1, None);
        assert_eq!(state(&before, "x"), State::LongTerm);
        assert_eq!(before.entries["x"].due_at_ns, Some(due));
        let at = fold(&l, events, 0, due, None);
        assert_eq!(state(&at, "x"), State::ForgettingCandidate);
    }

    #[test]
    fn stamp_lands_after_existing_later_event() {
        let events = vec![
            ev("a", Kind::Archive, Some("x"), 500),
            ev("b", Kind::Archive, Some("y"), 900),
            on("c", 700),
        ];
        assert_eq!(stamp(100, &events, Some("x")), 501);
        assert_eq!(stamp(1000, &events, Some("x")), 1000);
        assert_eq!(stamp(100, &events, Some("z")), 100);
        assert_eq!(stamp(100, &events, None), 701);
    }
}
