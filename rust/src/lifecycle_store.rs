//! Write verbs of the memory lifecycle (issue #11, phase 1a): `promote`, `archive`, `drop`,
//! `restore` and the per-project switch. Each reads the ledger and the lifecycle rows in one
//! pipeline, folds them at the current time, checks the transition, and writes one event under
//! the current project id with the ledger's checked-write rule (insert by id, read the id back
//! when the response is lost). Every refusal is a `LedgerError::Message` that starts with its
//! code word and names the next step; nothing is written on any of them.

use crate::memory::{active_entries, MemoryEntry, MemoryLedger};
use crate::memory_lifecycle::{
    allowed, fold, kind_name, stamp, Kind, LifecycleEvent, LifecycleView, State,
};
use crate::memory_store::{outcome_unknown, project, resolve_one, Ledger, LedgerError};
use crate::memory_tiers::mentions_user;
use crate::remind_store::{lean_field, not_recorded, now_ns};
use crate::remote::Cell;
use serde_json::{json, Value};
use std::path::Path;

const INSERT: &str =
    "INSERT OR IGNORE INTO project_memory_lifecycle (project, id, body) VALUES (?, ?, ?)";
const FIND: &str = "SELECT 1 FROM project_memory_lifecycle WHERE project = ? AND id = ?";
const MAX_QUOTE: usize = 600;
const MAX_NOTE: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Promote,
    Archive,
    Drop,
    Restore,
}

impl Verb {
    fn kind(self) -> Kind {
        match self {
            Self::Promote => Kind::Promote,
            Self::Archive => Kind::Archive,
            Self::Drop => Kind::Drop,
            Self::Restore => Kind::Restore,
        }
    }

    fn outcome(self) -> &'static str {
        match self {
            Self::Promote => "promoted",
            Self::Archive => "archived",
            Self::Drop => "dropped",
            Self::Restore => "restored",
        }
    }

    fn releases(self) -> bool {
        matches!(self, Self::Archive | Self::Drop)
    }
}

/// What lets `archive`/`drop` retire an entry that mentions the user: a newer entry that covers
/// it, or the user's own words. At most one of the two.
#[derive(Clone, Debug, Default)]
pub struct Release {
    pub superseded_by: Option<String>,
    pub quote: Option<String>,
}

struct Loaded {
    data: MemoryLedger,
    events: Vec<LifecycleEvent>,
    unreadable: usize,
    now: i128,
    view: LifecycleView,
}

fn load(ledger: &Ledger, project: &str) -> Result<Loaded, LedgerError> {
    let (data, rows) = ledger.read_with_lifecycle(project).map_err(not_recorded)?;
    let now = now_ns()?;
    let view = fold(&data, rows.events.clone(), rows.unreadable, now, None);
    Ok(Loaded {
        data,
        events: rows.events,
        unreadable: rows.unreadable,
        now,
        view,
    })
}

fn refuse(code: &'static str, message: impl Into<String>, detail: Value) -> LedgerError {
    LedgerError::Refused {
        code,
        message: message.into(),
        detail,
    }
}

fn check_note(note: Option<&str>) -> Result<(), LedgerError> {
    match note {
        Some(n) if n.len() > MAX_NOTE => Err(refuse(
            "memory_note_invalid",
            format!("--note is {} bytes; the limit is {MAX_NOTE}", n.len()),
            json!({"bytes": n.len(), "limit": MAX_NOTE}),
        )),
        _ => Ok(()),
    }
}

/// A quote needs at least one letter, digit or CJK character: a run of punctuation, spaces or
/// zero-width characters is blank.
fn check_quote(quote: Option<&str>) -> Result<(), LedgerError> {
    match quote {
        Some(q) if !q.chars().any(char::is_alphanumeric) => Err(refuse(
            "memory_quote_invalid",
            "--quote is empty or has no word in it; pass the user's own words",
            json!({"bytes": q.len()}),
        )),
        Some(q) if q.len() > MAX_QUOTE => Err(refuse(
            "memory_quote_invalid",
            format!(
                "--quote is {} bytes; the limit is {MAX_QUOTE}; quote the part that releases the entry",
                q.len()
            ),
            json!({"bytes": q.len(), "limit": MAX_QUOTE}),
        )),
        _ => Ok(()),
    }
}

fn off_error() -> LedgerError {
    refuse(
        "memory_lifecycle_off",
        "this project's lifecycle is not switched on; nothing was written; run `cort memory lifecycle on` first",
        json!({}),
    )
}

fn new_event(
    kind: Kind,
    entry: Option<&str>,
    recorded_at: i128,
    harness: Option<String>,
) -> LifecycleEvent {
    let machine = crate::usage::machine_id();
    let digest = crate::memory::hash(
        format!(
            "{}\0{}\0{machine}\0{recorded_at}",
            kind_name(kind),
            entry.unwrap_or("")
        )
        .as_bytes(),
    );
    LifecycleEvent {
        id: digest[..16].to_string(),
        kind,
        entry: entry.map(str::to_string),
        superseded_by: None,
        quote: None,
        note: None,
        recorded_at: recorded_at.to_string(),
        recorded_machine: Some(machine.to_string()),
        recorded_machine_source: Some(crate::usage::machine_id_source().to_string()),
        harness,
    }
}

/// Insert one event by id. A lost response is settled by reading the id back: found means it
/// landed, absent means it did not, and an unreadable answer is `OutcomeUnknown` with the id.
fn write_event(ledger: &Ledger, project: &str, event: &LifecycleEvent) -> Result<(), LedgerError> {
    let body = serde_json::to_string(event).map_err(|e| LedgerError::Message(e.to_string()))?;
    let args = vec![
        Cell::Text(project.into()),
        Cell::Text(event.id.clone()),
        Cell::Text(body),
    ];
    match ledger.db().execute(INSERT, args) {
        Ok(result) if result.affected_row_count == 1 => Ok(()),
        Ok(result) if result.affected_row_count == 0 => Err(refuse(
            "memory_event_exists",
            format!(
                "lifecycle event {} already exists; nothing was written",
                event.id
            ),
            json!({"id": event.id}),
        )),
        Ok(_) => Err(LedgerError::Message(
            "insert affected more than one row".into(),
        )),
        Err(error) if outcome_unknown(error.kind) => match ledger.db().execute(
            FIND,
            vec![Cell::Text(project.into()), Cell::Text(event.id.clone())],
        ) {
            Ok(found) if !found.rows.is_empty() => Ok(()),
            Ok(_) => Err(LedgerError::NotRecorded { kind: error.kind }),
            Err(_) => Err(LedgerError::OutcomeUnknown {
                id: event.id.clone(),
                // The entry the event acts on (none for the switch): the CLI names it.
                key: event.entry.clone(),
                kind: error.kind,
            }),
        },
        Err(error) => Err(LedgerError::NotRecorded { kind: error.kind }),
    }
}

fn with_unreadable(mut out: Value, unreadable: usize) -> Value {
    if unreadable > 0 {
        out["lifecycle_unreadable"] = json!(unreadable);
        out["lifecycle_hint"] = json!(crate::memory::LIFECYCLE_UNREADABLE_HINT);
    }
    out
}

/// The entry as the fold leaves it once `written` has landed.
fn entry_after(loaded: &Loaded, entry: &MemoryEntry, written: &LifecycleEvent) -> Value {
    let mut events = loaded.events.clone();
    events.push(written.clone());
    let view = fold(&loaded.data, events, loaded.unreadable, loaded.now, None);
    let mut out = json!({"id": entry.id, "key": entry.key});
    if let Some(s) = view.entries.get(&entry.id) {
        out["state"] = json!(s.state.as_str());
        if let Some(due) = s.due_at_ns {
            out["due_at"] = json!(due.to_string());
            out["due_at_utc"] = json!(crate::memory_tiers::utc_iso(due));
        }
    }
    out
}

/// The entry named by `id` (a full id or unique hex prefix over all ledger entries), or why it
/// cannot be acted on.
fn find_active<'a>(data: &'a MemoryLedger, id: &str) -> Result<&'a MemoryEntry, LedgerError> {
    let full = resolve_one(data, id)?;
    if let Some(entry) = active_entries(data).into_iter().find(|e| e.id == full) {
        return Ok(entry);
    }
    let successor = active_successor(data, &full);
    let named = successor
        .as_deref()
        .unwrap_or("an entry the ledger no longer lists");
    Err(refuse(
        "memory_entry_superseded",
        format!("{full} was superseded by {named}; act on {named} instead"),
        json!({"id": full, "successor": successor}),
    ))
}

/// Follow `supersedes`/`resolves` from `id` to the entry that is active now (a -> b -> c names c).
fn active_successor(data: &MemoryLedger, id: &str) -> Option<String> {
    let active = active_entries(data);
    let mut current = id.to_string();
    // A chain cannot be longer than the ledger; the bound only stops a malformed cycle.
    for _ in 0..=data.entries.len() {
        let next = data.entries.iter().find(|e| {
            e.supersedes.as_deref() == Some(current.as_str()) || e.resolves.contains(&current)
        })?;
        if active.iter().any(|e| e.id == next.id) {
            return Some(next.id.clone());
        }
        current = next.id.clone();
    }
    None
}

/// A `--superseded-by` target: another entry that is active and not hidden. Returns its full id.
fn check_successor(loaded: &Loaded, entry: &MemoryEntry, raw: &str) -> Result<String, LedgerError> {
    let target = resolve_one(&loaded.data, raw)?;
    let needs = |why: String| {
        refuse(
            "memory_archive_needs_release",
            format!(
                "--superseded-by {target} {why}; name an active entry that covers it, or pass --quote with the user's words"
            ),
            json!({"id": entry.id, "superseded_by": target}),
        )
    };
    if target == entry.id {
        return Err(needs("is the entry itself".into()));
    }
    if !active_entries(&loaded.data).iter().any(|e| e.id == target) {
        return Err(needs("is itself superseded".into()));
    }
    if loaded.view.state_of(&target).is_some_and(State::is_hidden) {
        return Err(needs("is archived or dropped".into()));
    }
    Ok(target)
}

pub fn act(
    root: &Path,
    verb: Verb,
    id: &str,
    release: Release,
    note: Option<String>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    act_inner(root, &Ledger::open()?, verb, id, release, note, harness)
}

pub fn act_with(
    root: &Path,
    ledger: &Ledger,
    verb: Verb,
    id: &str,
    release: Release,
    note: Option<String>,
) -> Result<Value, LedgerError> {
    act_inner(root, ledger, verb, id, release, note, None)
}

fn act_inner(
    root: &Path,
    ledger: &Ledger,
    verb: Verb,
    id: &str,
    release: Release,
    note: Option<String>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    // Answerable from the arguments alone: refused before any network round trip.
    check_note(note.as_deref())?;
    check_quote(release.quote.as_deref())?;
    if !verb.releases() && (release.superseded_by.is_some() || release.quote.is_some()) {
        return Err(refuse(
            "memory_release_unexpected",
            "--superseded-by and --quote belong to archive and drop",
            json!({}),
        ));
    }
    if release.superseded_by.is_some() && release.quote.is_some() {
        return Err(refuse(
            "memory_archive_needs_release",
            "pass --superseded-by or --quote, not both",
            json!({}),
        ));
    }
    let project = project(root)?;
    let loaded = load(ledger, &project)?;
    if !loaded.view.enabled {
        return Err(off_error());
    }
    let entry = find_active(&loaded.data, id)?;
    let state = loaded
        .view
        .state_of(&entry.id)
        .ok_or_else(|| LedgerError::Message(format!("{} has no lifecycle state", entry.id)))?;
    let kind = verb.kind();
    if !allowed(state).contains(&kind) {
        let verbs: Vec<&str> = allowed(state).iter().map(|k| kind_name(*k)).collect();
        return Err(refuse(
            "memory_lifecycle_not_allowed",
            format!(
                "{} cannot {} from state={}; allowed: {}",
                entry.id,
                kind_name(kind),
                state.as_str(),
                verbs.join(", ")
            ),
            json!({"id": entry.id, "state": state.as_str(), "allowed": verbs}),
        ));
    }
    let superseded_by = match release.superseded_by.as_deref() {
        Some(raw) => Some(check_successor(&loaded, entry, raw)?),
        None => None,
    };
    if verb.releases()
        && superseded_by.is_none()
        && release.quote.is_none()
        && mentions_user(&entry.key, &entry.claim)
    {
        return Err(refuse(
            "memory_archive_needs_release",
            format!(
                "{} mentions the user; pass --superseded-by <id of the entry that replaces it> or --quote '<the user's words>'",
                entry.id
            ),
            json!({"id": entry.id}),
        ));
    }
    let at = stamp(loaded.now, &loaded.events, Some(&entry.id));
    let mut event = new_event(kind, Some(&entry.id), at, harness);
    event.superseded_by = superseded_by;
    event.quote = release.quote;
    event.note = note;
    write_event(ledger, &project, &event)?;
    let out = json!({
        "outcome": verb.outcome(),
        "entry": entry_after(&loaded, entry, &event),
        "event": serde_json::to_value(&event).unwrap_or(Value::Null),
    });
    Ok(with_unreadable(out, loaded.unreadable))
}

pub fn switch(
    root: &Path,
    on: bool,
    note: Option<String>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    switch_inner(root, &Ledger::open()?, on, note, harness)
}

pub fn switch_with(
    root: &Path,
    ledger: &Ledger,
    on: bool,
    note: Option<String>,
) -> Result<Value, LedgerError> {
    switch_inner(root, ledger, on, note, None)
}

fn switch_inner(
    root: &Path,
    ledger: &Ledger,
    on: bool,
    note: Option<String>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    check_note(note.as_deref())?;
    let project = project(root)?;
    let loaded = load(ledger, &project)?;
    if loaded.view.enabled == on {
        let mut out = json!({"outcome": "unchanged", "enabled": on});
        if note.is_some() {
            out["note_recorded"] = json!(false);
            out["message"] = json!(
                "the switch already stood there; nothing was written, so --note was not recorded"
            );
        }
        return Ok(with_unreadable(out, loaded.unreadable));
    }
    let kind = if on { Kind::On } else { Kind::Off };
    let at = stamp(loaded.now, &loaded.events, None);
    let mut event = new_event(kind, None, at, harness);
    event.note = note;
    write_event(ledger, &project, &event)?;
    let mut events = loaded.events.clone();
    events.push(event.clone());
    let view = fold(&loaded.data, events, loaded.unreadable, loaded.now, None);
    let out = json!({
        "outcome": if on { "lifecycle_on" } else { "lifecycle_off" },
        "enabled": view.enabled,
        "epoch": view.epoch_ns.map(|e| e.to_string()),
        "epoch_utc": view.epoch_ns.map(crate::memory_tiers::utc_iso),
        "event": serde_json::to_value(&event).unwrap_or(Value::Null),
    });
    Ok(with_unreadable(out, loaded.unreadable))
}

/// `--lean` for the lifecycle verbs, one line, no empty field: a verb prints
/// `<outcome> <entry id> <state>`; `lifecycle` prints `lifecycle_on` / `lifecycle_off`, or
/// `unchanged on|off` when the switch already stood there. When the read could not use some
/// lifecycle rows, ` lifecycle_unreadable=<n>` is appended (the JSON's hint is not repeated:
/// `cort memory recall --all` finds anything hidden).
pub fn render_lean(verb: &str, payload: &Value) -> String {
    let outcome = lean_field(payload["outcome"].as_str().unwrap_or(""));
    let mut line = if verb == "lifecycle" {
        if payload["outcome"] == "unchanged" {
            let state = if payload["enabled"] == true {
                "on"
            } else {
                "off"
            };
            format!("{outcome} {state}")
        } else {
            outcome
        }
    } else {
        let entry = &payload["entry"];
        format!(
            "{outcome} {} {}",
            lean_field(entry["id"].as_str().unwrap_or("")),
            lean_field(entry["state"].as_str().unwrap_or(""))
        )
    };
    if let Some(n) = payload["lifecycle_unreadable"].as_u64().filter(|n| *n > 0) {
        line.push_str(&format!(" lifecycle_unreadable={n}"));
    }
    line.push('\n');
    line
}
