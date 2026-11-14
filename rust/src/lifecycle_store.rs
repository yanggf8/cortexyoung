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

const STATES: [State; 6] = [
    State::ShortTerm,
    State::ShortTermExpired,
    State::LongTerm,
    State::ForgettingCandidate,
    State::Archived,
    State::Dropped,
];

pub fn review(root: &Path, all: bool) -> Result<Value, LedgerError> {
    review_with(root, &Ledger::open()?, all, now_ns()?)
}

/// What is due for review, and how to settle each item. Read-only. A project whose lifecycle is
/// off answers `lifecycle: "off"` with a what-if: the fold with the epoch set to `now_ns` and
/// every existing event applied, which is what `cort memory lifecycle on` would place today.
pub fn review_with(
    root: &Path,
    ledger: &Ledger,
    all: bool,
    now_ns: i128,
) -> Result<Value, LedgerError> {
    let project = project(root)?;
    let (data, rows) = ledger.read_with_lifecycle(&project)?;
    let unreadable = rows.unreadable;
    let mut view = fold(&data, rows.events.clone(), unreadable, now_ns, None);
    let enabled = view.enabled;
    if !enabled {
        view = fold(&data, rows.events, unreadable, now_ns, Some(now_ns));
    }
    let entries = active_entries(&data);
    let mut counts = serde_json::Map::new();
    for state in STATES {
        let n = entries
            .iter()
            .filter(|e| view.entries.get(&e.id).is_some_and(|s| s.state == state))
            .count();
        counts.insert(state.as_str().to_string(), json!(n));
    }
    let recorded_utc = |entry: &MemoryEntry| {
        crate::memory_tiers::parse_recorded_ns(&entry.recorded_at).map(crate::memory_tiers::utc_iso)
    };
    let mut due: Vec<(i128, &MemoryEntry, State)> = entries
        .iter()
        .filter_map(|e| {
            let s = view.entries.get(&e.id)?;
            s.state
                .is_due()
                .then(|| (s.due_at_ns.unwrap_or(i128::MAX), *e, s.state))
        })
        .collect();
    due.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let items: Vec<Value> = due
        .iter()
        .map(|(due_at, entry, state)| {
            let mut item = json!({
                "id": entry.id,
                "key": entry.key,
                "claim": entry.claim,
                "state": state.as_str(),
                "mentions_user": mentions_user(&entry.key, &entry.claim),
                "id_ref": id_ref(&data, &entry.id),
                "commands": if enabled {
                    commands(&id_ref(&data, &entry.id), entry, *state)
                } else {
                    Vec::new()
                },
            });
            if let Some(at) = recorded_utc(entry) {
                item["recorded_at_utc"] = json!(at);
            }
            if *due_at != i128::MAX {
                item["due_at_utc"] = json!(crate::memory_tiers::utc_iso(*due_at));
            }
            item
        })
        .collect();
    let mut out = json!({
        "lifecycle": if enabled { "on" } else { "off" },
        "epoch_utc": view.epoch_ns.map(crate::memory_tiers::utc_iso),
        "counts": counts,
        "items": items,
    });
    if !enabled {
        out["what_if"] = json!(
            "epoch = now: what `cort memory lifecycle on` would place today; nothing is switched on or written"
        );
    }
    if all {
        let mut hidden: Vec<&MemoryEntry> = entries
            .iter()
            .copied()
            .filter(|e| view.entries.get(&e.id).is_some_and(|s| s.state.is_hidden()))
            .collect();
        hidden.sort_by(|a, b| a.id.cmp(&b.id));
        out["hidden"] = json!(hidden
            .iter()
            .filter_map(|entry| {
                let s = view.entries.get(&entry.id)?;
                let mut item = json!({"id": entry.id, "id_ref": id_ref(&data, &entry.id), "key": entry.key, "state": s.state.as_str()});
                if let Some(at) = recorded_utc(entry) {
                    item["recorded_at_utc"] = json!(at);
                }
                if let Some(last) = last_applied_release(s.state, &s.history) {
                    for (field, value) in [
                        ("quote", &last.quote),
                        ("superseded_by", &last.superseded_by),
                        ("note", &last.note),
                    ] {
                        if let Some(v) = value {
                            item[field] = json!(v);
                        }
                    }
                }
                Some(item)
            })
            .collect::<Vec<_>>());
        out["superseded"] = json!(view
            .superseded
            .iter()
            .map(|(id, events)| json!({"id": id, "events": events.len()}))
            .collect::<Vec<_>>());
        out["orphans"] = json!(view
            .orphans
            .iter()
            .map(|e| {
                let mut orphan = json!({"id": e.id, "kind": kind_name(e.kind)});
                if let Some(entry) = &e.entry {
                    orphan["entry"] = json!(entry);
                }
                orphan
            })
            .collect::<Vec<_>>());
    }
    Ok(with_unreadable(out, unreadable))
}

/// What an `archive`/`drop` command names an entry by: the 8-character prefix when the id is
/// lowercase hex and no other ledger id starts with it, else the shortest prefix (8 or more) that
/// is unique, else the full id (a non-hex, imported id is only accepted whole).
fn id_ref(data: &MemoryLedger, id: &str) -> String {
    let hex = id
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if hex {
        for n in 8..id.len() {
            let prefix = &id[..n];
            if !data
                .entries
                .iter()
                .any(|e| e.id != id && e.id.starts_with(prefix))
            {
                return prefix.to_string();
            }
        }
    }
    id.to_string()
}

/// The `archive`/`drop` event that put the entry in its hidden state: replay the history, taking
/// only events of the kind the final state names, skipping those that arrive while hidden and
/// letting a `restore` of a hidden entry reopen it. Inert and unknown events never qualify.
fn last_applied_release(state: State, history: &[LifecycleEvent]) -> Option<&LifecycleEvent> {
    let kind = match state {
        State::Archived => Kind::Archive,
        State::Dropped => Kind::Drop,
        _ => return None,
    };
    let mut hidden = false;
    let mut applied = None;
    for event in history {
        if event.kind == kind && !hidden {
            hidden = true;
            applied = Some(event);
        } else if event.kind == Kind::Restore && hidden {
            hidden = false;
        }
    }
    applied
}

/// The commands that settle an item, complete and runnable apart from the two placeholders
/// (`<id>`, `<the user's words>`) an entry that mentions the user needs filled in.
fn commands(id8: &str, entry: &MemoryEntry, state: State) -> Vec<String> {
    let release = |verb: &str| {
        if mentions_user(&entry.key, &entry.claim) {
            vec![
                format!("cort memory {verb} {id8} --superseded-by <id>"),
                format!("cort memory {verb} {id8} --quote '<the user's words>'"),
            ]
        } else {
            vec![format!("cort memory {verb} {id8}")]
        }
    };
    match state {
        State::ShortTermExpired => {
            let mut out = vec![format!("cort memory promote {id8}")];
            out.extend(release("drop"));
            out
        }
        State::ForgettingCandidate => release("archive"),
        _ => Vec::new(),
    }
}

/// `--lean` for `review`: a header `review lifecycle=<on|off> epoch_utc=<..> due=<n>` plus a
/// count per state, then one TSV line per item
/// `<id ref>\t<state>\t<due_at_utc>\t<mentions_user y|n>\t<key>` (the id as the commands name it);
/// with `--all`, one line each for `hidden`, `superseded` and `orphan` entries. No field is ever
/// empty (`-`).
pub fn render_review_lean(report: &Value) -> String {
    let mut out = format!(
        "review lifecycle={} epoch_utc={} due={}",
        lean_field(report["lifecycle"].as_str().unwrap_or("")),
        lean_field(report["epoch_utc"].as_str().unwrap_or("")),
        report["items"].as_array().map_or(0, Vec::len)
    );
    for state in STATES {
        let n = report["counts"][state.as_str()].as_u64().unwrap_or(0);
        out.push_str(&format!(" {}={n}", state.as_str()));
    }
    if let Some(n) = report["lifecycle_unreadable"].as_u64().filter(|n| *n > 0) {
        out.push_str(&format!(" lifecycle_unreadable={n}"));
    }
    out.push('\n');
    for item in report["items"].as_array().into_iter().flatten() {
        let fields = [
            lean_field(item["id_ref"].as_str().unwrap_or("")),
            lean_field(item["state"].as_str().unwrap_or("")),
            lean_field(item["due_at_utc"].as_str().unwrap_or("")),
            if item["mentions_user"] == true {
                "y"
            } else {
                "n"
            }
            .to_string(),
            lean_field(item["key"].as_str().unwrap_or("")),
        ];
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    // `--all` sections: one line each, led by the section name.
    let text = |v: &Value, k: &str| lean_field(v[k].as_str().unwrap_or(""));
    for h in report["hidden"].as_array().into_iter().flatten() {
        let fields = [
            "hidden".to_string(),
            text(h, "id_ref"),
            text(h, "state"),
            text(h, "key"),
        ];
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    for s in report["superseded"].as_array().into_iter().flatten() {
        let fields = [
            "superseded".to_string(),
            text(s, "id"),
            s["events"].as_u64().unwrap_or(0).to_string(),
        ];
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    for o in report["orphans"].as_array().into_iter().flatten() {
        let fields = [
            "orphan".to_string(),
            text(o, "id"),
            text(o, "kind"),
            text(o, "entry"),
        ];
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    out
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
