//! Storage for `cort remind`: append-only reminder events in the ledger's Turso database.
//! Reads cover the project's alias component; writes go under the current project id and use
//! the ledger's checked-write rule (insert by id, read the id back when the response is lost).

use crate::memory_store::{
    outcome_unknown, project, Ledger, LedgerError, COMPONENT, CREATE_ALIASES, CREATE_REMINDERS,
};
use crate::question;
use crate::question::Dedup;
use crate::remind::{
    self, check_not_past, classify, fold, kind_name, parse_on, resolve_prefix, validate_add,
    validate_note, EventKind, Folded, Reminder, ReminderEvent, State,
};
use crate::remote::{Cell, Stmt};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const INSERT: &str = "INSERT OR IGNORE INTO project_reminders (project, id, body) VALUES (?, ?, ?)";
const FIND: &str = "SELECT 1 FROM project_reminders WHERE project = ? AND id = ?";
const EVENTS_REPORTED_ABOVE: usize = 2000;

pub struct AddArgs {
    pub text: String,
    pub on: String,
    pub lead: u32,
    pub blocks: Option<String>,
    pub harness: Option<String>,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListFilter {
    Open,
    Closed,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Done,
    Moot,
    Duplicate,
}

impl CloseReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Moot => "moot",
            Self::Duplicate => "duplicate",
        }
    }
}

fn now_ns() -> Result<i128, LedgerError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| LedgerError::Message(format!("clock error: {e}")))?
        .as_nanos();
    i128::try_from(nanos).map_err(|_| LedgerError::Message("clock error: overflow".into()))
}

/// A read before a write: a failure there means nothing was written.
fn not_recorded(error: LedgerError) -> LedgerError {
    match error {
        LedgerError::Remote(kind) => LedgerError::NotRecorded { kind },
        other => other,
    }
}

/// Every reminder event of the component; provisions the table (and the alias table the
/// component query reads) first, so a fresh database answers with no events rather than an
/// error.
pub fn read_events_with(ledger: &Ledger, project: &str) -> Result<Vec<ReminderEvent>, LedgerError> {
    let sql = format!("SELECT project, body FROM project_reminders WHERE project IN ({COMPONENT})");
    let results = ledger
        .db()
        .pipeline(&[
            Stmt::new(CREATE_REMINDERS, vec![]),
            Stmt::new(CREATE_ALIASES, vec![]),
            Stmt::new(sql, vec![Cell::Text(project.into())]),
        ])
        .map_err(|e| LedgerError::Remote(e.kind))?;
    let rows = &results
        .get(2)
        .ok_or_else(|| LedgerError::Message("missing remote reminder result".into()))?
        .rows;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(Cell::Text(body)) = row.get(1) else {
            return Err(LedgerError::Message("invalid remote reminder row".into()));
        };
        let event = remind::parse_event(body)
            .map_err(|e| LedgerError::Message(format!("invalid remote reminder row: {e}")))?;
        events.push(event);
    }
    Ok(events)
}

/// The folded component, the event count, the fold's `now`, and the clock reading taken just
/// before the read was sent (the snapshot's `read_at`).
fn load(ledger: &Ledger, project: &str) -> Result<(Folded, usize, i128, i128), LedgerError> {
    let read_at = now_ns()?;
    let events = read_events_with(ledger, project)?;
    let total = events.len();
    let now = now_ns()?;
    Ok((fold(events, now), total, now, read_at))
}

/// What a successful read leaves for the prompt hook: the local reminder snapshot. Built from
/// the events of the reminders that are not closed, plus -- after a write was confirmed -- the
/// event it wrote. Every error is ignored: a snapshot problem never changes a command's output.
struct Snap<'a> {
    project: &'a str,
    read_at: i128,
    now: i128,
    events: Vec<ReminderEvent>,
}

impl<'a> Snap<'a> {
    fn new(project: &'a str, folded: &Folded, read_at: i128, now: i128) -> Self {
        Snap {
            project,
            read_at,
            now,
            events: crate::remind_snapshot::snapshot_events(&folded.reminders),
        }
    }

    fn save(&self, written: Option<&ReminderEvent>) {
        let events = match written {
            None => self.events.clone(),
            Some(event) => {
                let mut all = self.events.clone();
                all.push(event.clone());
                crate::remind_snapshot::snapshot_events(&fold(all, self.now).reminders)
            }
        };
        let snapshot = crate::remind_snapshot::Snapshot::new(self.project, self.read_at, events);
        let _ = crate::remind_snapshot::write_snapshot(&crate::data_dir::data_dir(), &snapshot);
    }

    /// Write `event`, then settle the snapshot: confirmed adds it, a refusal keeps the read's
    /// events, and an unknown outcome writes nothing.
    fn write(&self, ledger: &Ledger, event: &ReminderEvent) -> Result<(), LedgerError> {
        let result = write_event(ledger, self.project, event);
        match &result {
            Ok(()) => self.save(Some(event)),
            Err(LedgerError::OutcomeUnknown { .. }) => {}
            Err(_) => self.save(None),
        }
        result
    }
}

fn new_event(
    kind: EventKind,
    seed: &str,
    harness: Option<String>,
) -> Result<ReminderEvent, LedgerError> {
    let timestamp = now_ns()?;
    let machine = crate::usage::machine_id();
    let digest = crate::memory::hash(
        format!("{}\0{seed}\0{machine}\0{timestamp}", kind_name(kind)).as_bytes(),
    );
    Ok(ReminderEvent {
        id: digest[..16].to_string(),
        kind,
        reminder: None,
        text: None,
        blocks: None,
        lead: None,
        on: None,
        utc_offset: None,
        due_at: None,
        reason: None,
        of: None,
        note: None,
        recorded_at: timestamp.to_string(),
        recorded_machine: Some(machine.to_string()),
        recorded_machine_source: Some(crate::usage::machine_id_source().to_string()),
        harness,
    })
}

/// Insert one event by id. A lost response is settled by reading the id back: found means it
/// landed, absent means it did not, and an unreadable answer is `OutcomeUnknown` with the id.
fn write_event(ledger: &Ledger, project: &str, event: &ReminderEvent) -> Result<(), LedgerError> {
    let body = serde_json::to_string(event).map_err(|e| LedgerError::Message(e.to_string()))?;
    let args = vec![
        Cell::Text(project.into()),
        Cell::Text(event.id.clone()),
        Cell::Text(body),
    ];
    match ledger.db().execute(INSERT, args) {
        Ok(result) if result.affected_row_count == 1 => Ok(()),
        Ok(result) if result.affected_row_count == 0 => Err(LedgerError::Message(format!(
            "reminder event {} already exists; nothing was written",
            event.id
        ))),
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
                key: None,
                kind: error.kind,
            }),
        },
        Err(error) => Err(LedgerError::NotRecorded { kind: error.kind }),
    }
}

fn event_json(event: &ReminderEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap_or_else(|_| json!({"id": event.id}));
    if let Some(map) = value.as_object_mut() {
        map.remove("kind");
    }
    value
}

/// One reminder as it stands: the effective date (the newest defer before the close, else the
/// add), the state and label at `now`, and the add's text and lead.
fn reminder_json(r: &Reminder, with_events: bool) -> Value {
    let mut value = json!({
        "id": r.add.id,
        "on": r.effective.on,
        "utc_offset": r.effective.utc_offset,
        "due_at": r.effective.due_at.to_string(),
        "state": r.state,
        "label": r.label,
        "text": r.add.text.clone().unwrap_or_default(),
        "lead": r.lead,
        "last_activity": r.last_activity,
    });
    if let Some(blocks) = &r.add.blocks {
        value["blocks"] = json!(blocks);
    }
    if let Some(days) = r.overdue_days {
        value["overdue_days"] = json!(days);
    }
    if let Some(close) = &r.close {
        value["close"] = event_json(close);
    }
    if with_events {
        value["events"] = Value::Array(
            r.events
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                .collect(),
        );
    }
    value
}

/// `r` as it stands once `written` has landed: its events plus the new one, folded again, so
/// a reply reports the effective date and state after the write rather than before it.
fn after_write(r: &Reminder, written: &ReminderEvent, now: i128) -> Value {
    let mut events = Vec::with_capacity(r.events.len() + 2);
    events.push(r.add.clone());
    events.extend(r.events.iter().cloned());
    events.push(written.clone());
    match fold(events, now).reminders.into_iter().next() {
        Some(folded) => reminder_json(&folded, false),
        None => reminder_json(r, false),
    }
}

fn find<'a>(folded: &'a Folded, prefix: &str) -> Result<&'a Reminder, LedgerError> {
    let id = resolve_prefix(folded.reminders.iter().map(|r| r.add.id.as_str()), prefix)
        .map_err(|code| LedgerError::Message(format!("{code}: {prefix}")))?;
    folded
        .reminders
        .iter()
        .find(|r| r.add.id == id)
        .ok_or_else(|| LedgerError::Message(format!("remind_not_found: {prefix}")))
}

pub fn add(root: &Path, args: AddArgs) -> Result<Value, LedgerError> {
    add_with(root, args, &Ledger::open()?)
}

pub fn add_with(root: &Path, args: AddArgs, ledger: &Ledger) -> Result<Value, LedgerError> {
    validate_add(&args.text, args.blocks.as_deref(), args.lead)?;
    let due = parse_on(&args.on)?;
    let project = project(root)?;
    // Refused before any network round trip: a past date needs no read to answer.
    check_not_past(&due, now_ns()?)?;
    let (folded, _, now, read_at) = load(ledger, &project).map_err(not_recorded)?;
    let snap = Snap::new(&project, &folded, read_at, now);
    let by_id = |id: &String| folded.reminders.iter().find(|r| &r.add.id == id);
    let similar = match classify(&args.text, due.due_at, &folded.reminders) {
        Dedup::Exists(ids) if !args.force => {
            snap.save(None);
            let earlier = by_id(ids.first().ok_or_else(|| {
                LedgerError::Message("matched reminder is missing from the fold".into())
            })?)
            .ok_or_else(|| {
                LedgerError::Message("matched reminder is missing from the fold".into())
            })?;
            return Ok(json!({
                "outcome": "exists",
                "reminder": reminder_json(earlier, false),
            }));
        }
        // `--force` writes past an exists without reporting one, as question does.
        Dedup::Exists(_) | Dedup::New => Vec::new(),
        Dedup::Similar(ids) => ids,
    };
    let mut event = new_event(EventKind::Add, &args.text, args.harness)?;
    event.text = Some(args.text);
    event.blocks = args.blocks;
    event.lead = Some(args.lead);
    event.on = Some(due.on.clone());
    event.utc_offset = Some(due.utc_offset.clone());
    event.due_at = Some(due.due_at.to_string());
    snap.write(ledger, &event)?;
    let similar: Vec<Value> = similar
        .iter()
        .filter_map(by_id)
        .map(|r| {
            let mut value = reminder_json(r, false);
            value["duplicate_command"] = json!(format!(
                "cort remind close {} --reason duplicate --of {}",
                event.id, r.add.id
            ));
            value
        })
        .collect();
    let reminder = fold(vec![event.clone()], now)
        .reminders
        .into_iter()
        .next()
        .map(|r| reminder_json(&r, false))
        .unwrap_or(Value::Null);
    let mut out = json!({"outcome": "added", "reminder": reminder});
    if !similar.is_empty() {
        out["similar"] = Value::Array(similar);
    }
    Ok(out)
}

pub fn list(
    root: &Path,
    filter: ListFilter,
    id: Option<&str>,
    matching: Option<&str>,
) -> Result<Value, LedgerError> {
    list_with(root, filter, id, matching, &Ledger::open()?)
}

/// `id` shows one reminder with its event history whatever `filter` says; `matching` searches
/// every state, as `--match` is specified to.
pub fn list_with(
    root: &Path,
    filter: ListFilter,
    id: Option<&str>,
    matching: Option<&str>,
    ledger: &Ledger,
) -> Result<Value, LedgerError> {
    let project = project(root)?;
    let (folded, total, now, read_at) = load(ledger, &project)?;
    Snap::new(&project, &folded, read_at, now).save(None);
    let reminders: Vec<Value> = if let Some(prefix) = id {
        vec![reminder_json(find(&folded, prefix)?, true)]
    } else if let Some(matching) = matching {
        let wanted = question::terms(matching);
        if wanted.is_empty() {
            return Err(LedgerError::Message(format!(
                "remind_match_empty: no searchable term in {matching:?}"
            )));
        }
        let mut hits: Vec<(bool, Value)> = folded
            .reminders
            .iter()
            .filter_map(|r| {
                let mut hay = r.add.text.clone().unwrap_or_default();
                for event in &r.events {
                    if let Some(note) = &event.note {
                        hay.push(' ');
                        hay.push_str(note);
                    }
                }
                let hay = hay.to_lowercase();
                let found = wanted.iter().filter(|t| question::occurs(&hay, t)).count();
                let all = found == wanted.len();
                (all || found * 2 >= wanted.len()).then(|| {
                    let mut value = reminder_json(r, false);
                    value["match"] = json!(if all { "all" } else { "partial" });
                    (all, value)
                })
            })
            .collect();
        // Stable: within each group the fold's due-at-ascending order holds.
        hits.sort_by_key(|(all, _)| !*all);
        hits.into_iter().map(|(_, value)| value).collect()
    } else {
        folded
            .reminders
            .iter()
            .filter(|r| match filter {
                ListFilter::Open => r.state != State::Closed,
                ListFilter::Closed => r.state == State::Closed,
                ListFilter::All => true,
            })
            .map(|r| reminder_json(r, false))
            .collect()
    };
    let mut out = json!({
        "project_id": project,
        "reminders": reminders,
    });
    if !folded.orphan_events.is_empty() {
        out["orphan_events"] = serde_json::to_value(&folded.orphan_events)
            .map_err(|e| LedgerError::Message(e.to_string()))?;
    }
    if total > EVENTS_REPORTED_ABOVE {
        out["events_total"] = json!(total);
    }
    Ok(out)
}

pub fn defer(
    root: &Path,
    id: &str,
    on: &str,
    note: Option<String>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    defer_with(root, id, on, note, harness, &Ledger::open()?)
}

pub fn defer_with(
    root: &Path,
    id: &str,
    on: &str,
    note: Option<String>,
    harness: Option<String>,
    ledger: &Ledger,
) -> Result<Value, LedgerError> {
    if let Some(note) = note.as_deref() {
        validate_note(note)?;
    }
    let due = parse_on(on)?;
    let project = project(root)?;
    // Refused before any network round trip, exactly as `add`: a past date needs no read
    // to answer.
    check_not_past(&due, now_ns()?)?;
    let (folded, _, now, read_at) = load(ledger, &project).map_err(not_recorded)?;
    let snap = Snap::new(&project, &folded, read_at, now);
    let r = find(&folded, id).inspect_err(|_| snap.save(None))?;
    let rid = r.add.id.clone();
    if let Some(close) = &r.close {
        snap.save(None);
        return Err(LedgerError::Message(format!(
            "remind_closed: {rid} is already closed ({})",
            close.reason.as_deref().unwrap_or("-")
        )));
    }
    let mut event = new_event(EventKind::Defer, &format!("{rid}\0{on}"), harness)?;
    event.reminder = Some(rid);
    event.on = Some(due.on.clone());
    event.utc_offset = Some(due.utc_offset.clone());
    event.due_at = Some(due.due_at.to_string());
    event.note = note;
    snap.write(ledger, &event)?;
    Ok(json!({
        "outcome": "deferred",
        "reminder": after_write(r, &event, now),
    }))
}

pub fn close(
    root: &Path,
    id: &str,
    reason: CloseReason,
    of: Option<String>,
    note: Option<String>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    close_with(root, id, reason, of, note, harness, &Ledger::open()?)
}

pub fn close_with(
    root: &Path,
    id: &str,
    reason: CloseReason,
    of: Option<String>,
    note: Option<String>,
    harness: Option<String>,
    ledger: &Ledger,
) -> Result<Value, LedgerError> {
    if let Some(note) = note.as_deref() {
        validate_note(note)?;
    }
    // Refused before any read: the `--of` shape is answerable from the arguments alone.
    // Only a duplicate's `--of` prefix needs the fold to resolve.
    let of_prefix = match (reason, of) {
        (CloseReason::Duplicate, None) => {
            return Err("remind_of_required: --reason duplicate needs --of <id>".into());
        }
        (CloseReason::Duplicate, Some(prefix)) => Some(prefix),
        (_, Some(_)) => {
            return Err("remind_of_unexpected: --of is only for --reason duplicate".into());
        }
        (_, None) => None,
    };
    let project = project(root)?;
    let (folded, _, now, read_at) = load(ledger, &project).map_err(not_recorded)?;
    let snap = Snap::new(&project, &folded, read_at, now);
    let r = find(&folded, id).inspect_err(|_| snap.save(None))?;
    let rid = r.add.id.clone();
    if let Some(close) = &r.close {
        snap.save(None);
        return Err(LedgerError::Message(format!(
            "remind_closed: {rid} is already closed ({})",
            close.reason.as_deref().unwrap_or("-")
        )));
    }
    let of = match of_prefix {
        Some(prefix) => {
            let other = find(&folded, &prefix).inspect_err(|_| snap.save(None))?;
            if other.add.id == rid {
                snap.save(None);
                return Err(LedgerError::Message(format!(
                    "remind_of_unexpected: {rid} cannot be a duplicate of itself"
                )));
            }
            Some(other.add.id.clone())
        }
        None => None,
    };
    let mut event = new_event(EventKind::Close, &rid, harness)?;
    event.reminder = Some(rid);
    event.reason = Some(reason.as_str().to_string());
    event.of = of;
    event.note = note;
    snap.write(ledger, &event)?;
    Ok(json!({
        "outcome": "closed",
        "reminder": after_write(r, &event, now),
    }))
}

/// One lean field: control characters (a tab or newline would split the row) become spaces,
/// and an empty value prints as `-`, because `read` collapses runs of tabs.
fn lean_field(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

fn lean_id8(value: &Value) -> String {
    let id: String = value["id"].as_str().unwrap_or("").chars().take(8).collect();
    lean_field(&id)
}

/// `-f lean` for `cort remind <verb>`: `add` prints `added <id> <on> <utc_offset>` (plus
/// ` similar=<id8>,...` when similar reminders exist) or `exists <id> <on> <state>`; `defer`
/// prints `deferred <id> <on> <utc_offset>` (the offset is always shown so the agent can check
/// it against the user's zone); `close` prints `closed <id> <reason>`; `list` one TSV row per
/// reminder (`<id8> <on> <state> <label> <text> <blocks|->`, labels with `_` for spaces). No
/// field is ever empty.
pub fn render_lean(verb: &str, payload: &Value) -> String {
    let mut out = String::new();
    match verb {
        "add" if payload["outcome"] == "exists" => {
            let r = &payload["reminder"];
            out.push_str(&format!(
                "exists {} {} {}\n",
                lean_field(r["id"].as_str().unwrap_or("")),
                lean_field(r["on"].as_str().unwrap_or("")),
                lean_field(r["state"].as_str().unwrap_or(""))
            ));
        }
        "add" | "defer" => {
            let r = &payload["reminder"];
            out.push_str(&format!(
                "{} {} {} {}",
                lean_field(payload["outcome"].as_str().unwrap_or("")),
                lean_field(r["id"].as_str().unwrap_or("")),
                lean_field(r["on"].as_str().unwrap_or("")),
                lean_field(r["utc_offset"].as_str().unwrap_or(""))
            ));
            let similar: Vec<String> = payload["similar"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(lean_id8)
                .collect();
            if !similar.is_empty() {
                out.push_str(&format!(" similar={}", similar.join(",")));
            }
            out.push('\n');
        }
        "close" => {
            let reason = payload["reminder"]["close"]["reason"]
                .as_str()
                .unwrap_or("");
            out.push_str(&format!(
                "closed {} {}\n",
                lean_field(payload["reminder"]["id"].as_str().unwrap_or("")),
                lean_field(reason)
            ));
        }
        "list" => {
            for r in payload["reminders"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let label = lean_field(r["label"].as_str().unwrap_or("")).replace(' ', "_");
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    lean_id8(r),
                    lean_field(r["on"].as_str().unwrap_or("")),
                    lean_field(r["state"].as_str().unwrap_or("")),
                    label,
                    lean_field(r["text"].as_str().unwrap_or("")),
                    lean_field(r["blocks"].as_str().unwrap_or("")),
                ));
            }
        }
        _ => {
            out.push_str(&format!(
                "{} {}\n",
                lean_field(payload["outcome"].as_str().unwrap_or("")),
                lean_id8(&payload["reminder"])
            ));
        }
    }
    out
}
