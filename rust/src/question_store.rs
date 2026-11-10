//! Storage for `cort question`: append-only question events in the ledger's Turso database.
//! Reads cover the project's alias component; writes go under the current project id and use
//! the ledger's checked-write rule (insert by id, read the id back when the response is lost).

use crate::memory_store::{
    outcome_unknown, project, Ledger, LedgerError, COMPONENT, CREATE_ALIASES, CREATE_QUESTIONS,
};
use crate::question::{
    self, classify, fold, resolve_prefix, validate_ask, Dedup, EventKind, Folded, Question,
    QuestionEvent, QuestionOption, State,
};
use crate::remote::{Cell, Stmt};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const INSERT: &str = "INSERT OR IGNORE INTO project_questions (project, id, body) VALUES (?, ?, ?)";
const FIND: &str = "SELECT 1 FROM project_questions WHERE project = ? AND id = ?";
const NS_PER_DAY: i128 = 86_400_000_000_000;
const DEFAULT_STALE_DAYS: u64 = 14;
const EVENTS_REPORTED_ABOVE: usize = 2000;

pub struct AddArgs {
    pub text: String,
    pub options: Vec<QuestionOption>,
    pub recommend: Option<String>,
    pub blocks: Option<String>,
    pub by: Option<String>,
    pub harness: Option<String>,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListFilter {
    Open,
    Answered,
    Closed,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Stale,
    Moot,
    Duplicate,
}

impl CloseReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Moot => "moot",
            Self::Duplicate => "duplicate",
        }
    }
}

/// `CORT_QUESTION_STALE_DAYS`; unset, invalid or zero falls back to 14.
pub fn stale_days() -> u64 {
    std::env::var("CORT_QUESTION_STALE_DAYS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_STALE_DAYS)
}

fn now_ns() -> Result<i128, LedgerError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| LedgerError::Message(format!("clock error: {e}")))?
        .as_nanos();
    i128::try_from(nanos).map_err(|_| LedgerError::Message("clock error: overflow".into()))
}

fn days_since(now: i128, recorded_at: &str) -> i128 {
    recorded_at
        .parse::<i128>()
        .map_or(0, |ns| (now.saturating_sub(ns) / NS_PER_DAY).max(0))
}

/// Single-quote for a POSIX shell: the result is one word whatever `s` holds.
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A read before a write: a failure there means nothing was written.
fn not_recorded(error: LedgerError) -> LedgerError {
    match error {
        LedgerError::Remote(kind) => LedgerError::NotRecorded { kind },
        other => other,
    }
}

struct Events {
    events: Vec<QuestionEvent>,
    total: usize,
}

/// Every event of the component; provisions the table (and the alias table the component
/// query reads) first, so a fresh database answers with no events rather than an error.
fn read_events(ledger: &Ledger, project: &str) -> Result<Events, LedgerError> {
    let sql = format!("SELECT project, body FROM project_questions WHERE project IN ({COMPONENT})");
    let results = ledger
        .db()
        .pipeline(&[
            Stmt::new(CREATE_QUESTIONS, vec![]),
            Stmt::new(CREATE_ALIASES, vec![]),
            Stmt::new(sql, vec![Cell::Text(project.into())]),
        ])
        .map_err(|e| LedgerError::Remote(e.kind))?;
    let rows = &results
        .get(2)
        .ok_or_else(|| LedgerError::Message("missing remote question result".into()))?
        .rows;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(Cell::Text(body)) = row.get(1) else {
            return Err(LedgerError::Message("invalid remote question row".into()));
        };
        let event: QuestionEvent = serde_json::from_str(body)
            .map_err(|e| LedgerError::Message(format!("invalid remote question row: {e}")))?;
        events.push(event);
    }
    Ok(Events {
        total: events.len(),
        events,
    })
}

fn load(ledger: &Ledger, project: &str) -> Result<(Folded, usize, i128), LedgerError> {
    let read = read_events(ledger, project)?;
    let now = now_ns()?;
    Ok((fold(read.events, now, stale_days()), read.total, now))
}

fn new_event(
    kind: EventKind,
    seed: &str,
    harness: Option<String>,
) -> Result<QuestionEvent, LedgerError> {
    let timestamp = now_ns()?;
    let machine = crate::usage::machine_id();
    let kind_name = match kind {
        EventKind::Ask => "ask",
        EventKind::Answer => "answer",
        EventKind::Close => "close",
    };
    let digest =
        crate::memory::hash(format!("{kind_name}\0{seed}\0{machine}\0{timestamp}").as_bytes());
    Ok(QuestionEvent {
        id: digest[..16].to_string(),
        kind,
        question: None,
        text: None,
        options: Vec::new(),
        recommend: None,
        blocks: None,
        by: None,
        reasks: Vec::new(),
        answer: None,
        quote: None,
        quoted: None,
        option: None,
        replaces: Vec::new(),
        reason: None,
        of: None,
        recorded_at: timestamp.to_string(),
        recorded_machine: Some(machine.to_string()),
        recorded_machine_source: Some(crate::usage::machine_id_source().to_string()),
        harness,
    })
}

/// Insert one event by id. A lost response is settled by reading the id back: found means it
/// landed, absent means it did not, and an unreadable answer is `OutcomeUnknown` with the id.
fn write_event(ledger: &Ledger, project: &str, event: &QuestionEvent) -> Result<(), LedgerError> {
    let body = serde_json::to_string(event).map_err(|e| LedgerError::Message(e.to_string()))?;
    let args = vec![
        Cell::Text(project.into()),
        Cell::Text(event.id.clone()),
        Cell::Text(body),
    ];
    match ledger.db().execute(INSERT, args) {
        Ok(result) if result.affected_row_count == 1 => Ok(()),
        Ok(result) if result.affected_row_count == 0 => Err(LedgerError::Message(format!(
            "question event {} already exists; nothing was written",
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

fn event_json(event: &QuestionEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap_or_else(|_| json!({"id": event.id}));
    if let Some(map) = value.as_object_mut() {
        map.remove("kind");
    }
    value
}

fn question_json(q: &Question, now: i128, with_events: bool) -> Value {
    let mut value = event_json(&q.ask);
    let Some(map) = value.as_object_mut() else {
        return value;
    };
    map.insert("state".into(), json!(q.state));
    map.insert("stale".into(), json!(q.stale));
    map.insert("last_activity".into(), json!(q.last_activity));
    map.insert(
        "age_days".into(),
        json!(days_since(now, &q.ask.recorded_at)),
    );
    map.insert(
        "current_answers".into(),
        Value::Array(q.current_answers.iter().map(event_json).collect()),
    );
    map.insert("answer_conflict".into(), json!(q.answer_conflict));
    if let Some(close) = &q.close {
        map.insert("close".into(), event_json(close));
    }
    if q.state == State::Answered {
        if let Some(newest) = q
            .current_answers
            .iter()
            .filter_map(|a| a.recorded_at.parse::<i128>().ok())
            .max()
        {
            map.insert(
                "answered_days_ago".into(),
                json!((now.saturating_sub(newest) / NS_PER_DAY).max(0)),
            );
        }
    }
    if with_events {
        map.insert(
            "events".into(),
            Value::Array(
                q.events
                    .iter()
                    .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
                    .collect(),
            ),
        );
    }
    value
}

/// `q` as it stands once `written` has landed: its events plus the new one, folded again, so
/// a reply reports the current answers and conflict after the write rather than before it.
fn after_write(q: &Question, written: &QuestionEvent, now: i128) -> Value {
    let mut events = Vec::with_capacity(q.events.len() + 2);
    events.push(q.ask.clone());
    events.extend(q.events.iter().cloned());
    events.push(written.clone());
    match fold(events, now, stale_days()).questions.first() {
        Some(folded) => question_json(folded, now, false),
        None => question_json(q, now, false),
    }
}

fn find<'a>(folded: &'a Folded, prefix: &str) -> Result<&'a Question, LedgerError> {
    let id = resolve_prefix(folded.questions.iter().map(|q| q.ask.id.as_str()), prefix)
        .map_err(|code| LedgerError::Message(format!("{code}: {prefix}")))?;
    folded
        .questions
        .iter()
        .find(|q| q.ask.id == id)
        .ok_or_else(|| LedgerError::Message(format!("question_not_found: {prefix}")))
}

pub fn add(root: &Path, args: AddArgs) -> Result<Value, LedgerError> {
    add_with(root, args, &Ledger::open()?)
}

pub fn add_with(root: &Path, args: AddArgs, ledger: &Ledger) -> Result<Value, LedgerError> {
    validate_ask(
        &args.text,
        &args.options,
        args.recommend.as_deref(),
        args.blocks.as_deref(),
        args.by.as_deref(),
    )?;
    let project = project(root)?;
    let (folded, _, now) = load(ledger, &project).map_err(not_recorded)?;
    let by_id = |id: &String| folded.questions.iter().find(|q| &q.ask.id == id);
    let (reasks, similar) = match classify(&args.text, &folded.questions) {
        Dedup::Exists(ids) if !args.force => {
            let matched: Vec<&Question> = ids.iter().filter_map(by_id).collect();
            let mut out = json!({
                "outcome": "exists",
                "questions": matched.iter().map(|q| question_json(q, now, false)).collect::<Vec<_>>(),
            });
            if let Some(answered) = matched.iter().find(|q| q.state == State::Answered) {
                out["answer"] = json!({
                    "question": answered.ask.id,
                    "answers": answered.current_answers.iter().map(event_json).collect::<Vec<_>>(),
                    "answered_days_ago": question_json(answered, now, false)["answered_days_ago"],
                });
            }
            return Ok(out);
        }
        Dedup::Exists(ids) => (ids, Vec::new()),
        Dedup::Similar(ids) => (Vec::new(), ids),
        Dedup::New => (Vec::new(), Vec::new()),
    };
    let mut event = new_event(EventKind::Ask, &args.text, args.harness)?;
    event.text = Some(args.text);
    event.options = args.options;
    event.recommend = args.recommend;
    event.blocks = args.blocks;
    event.by = args.by;
    event.reasks = reasks;
    write_event(ledger, &project, &event)?;
    let mut question = event_json(&event);
    question["state"] = json!("open");
    let similar: Vec<Value> = similar
        .iter()
        .filter_map(by_id)
        .map(|q| {
            let mut value = question_json(q, now, false);
            value["close_command"] = json!(format!(
                "cort question close {} --reason duplicate --of {}",
                event.id, q.ask.id
            ));
            value
        })
        .collect();
    Ok(json!({"outcome": "added", "question": question, "similar": similar}))
}

pub fn list(
    root: &Path,
    filter: ListFilter,
    id: Option<&str>,
    matching: Option<&str>,
) -> Result<Value, LedgerError> {
    list_with(root, filter, id, matching, &Ledger::open()?)
}

/// `id` shows one question with its event history whatever `filter` says; `matching` searches
/// every state, as `--match` is specified to.
pub fn list_with(
    root: &Path,
    filter: ListFilter,
    id: Option<&str>,
    matching: Option<&str>,
    ledger: &Ledger,
) -> Result<Value, LedgerError> {
    let project = project(root)?;
    let (folded, total, now) = load(ledger, &project)?;
    let questions: Vec<Value> = if let Some(prefix) = id {
        vec![question_json(find(&folded, prefix)?, now, true)]
    } else if let Some(matching) = matching {
        let wanted = question::terms(matching);
        if wanted.is_empty() {
            return Err(LedgerError::Message(format!(
                "question_match_empty: no searchable term in {matching:?}"
            )));
        }
        let mut hits: Vec<(bool, Value)> = folded
            .questions
            .iter()
            .filter_map(|q| {
                let mut hay = q.ask.text.clone().unwrap_or_default();
                for o in &q.ask.options {
                    hay.push(' ');
                    hay.push_str(&o.text);
                }
                for a in &q.current_answers {
                    hay.push(' ');
                    hay.push_str(a.answer.as_deref().unwrap_or(""));
                }
                let hay = hay.to_lowercase();
                let found = wanted.iter().filter(|t| question::occurs(&hay, t)).count();
                let all = found == wanted.len();
                (all || found * 2 >= wanted.len()).then(|| {
                    let mut value = question_json(q, now, false);
                    value["match"] = json!(if all { "all" } else { "partial" });
                    (all, value)
                })
            })
            .collect();
        // Stable: within each group the fold's newest-activity-first order holds.
        hits.sort_by_key(|(all, _)| !*all);
        hits.into_iter().map(|(_, value)| value).collect()
    } else {
        folded
            .questions
            .iter()
            .filter(|q| match filter {
                ListFilter::Open => q.state == State::Open,
                ListFilter::Answered => q.state == State::Answered,
                ListFilter::Closed => q.state == State::Closed,
                ListFilter::All => true,
            })
            .map(|q| question_json(q, now, false))
            .collect()
    };
    let mut out = json!({
        "project_id": project,
        "stale_days": stale_days(),
        "questions": questions,
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

fn answers_list(answers: &[QuestionEvent]) -> String {
    answers
        .iter()
        .map(|a| {
            format!(
                "{} {:?}{}",
                a.id,
                a.answer.as_deref().unwrap_or(""),
                if a.quoted == Some(true) {
                    ""
                } else {
                    " (no quote)"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn retry_command(question: &str, answer: &str, replaces: &[QuestionEvent]) -> String {
    let ids: Vec<&str> = replaces.iter().map(|a| a.id.as_str()).collect();
    format!(
        "cort question answer {question} --answer {} --replace {}",
        sh(answer),
        ids.join(" ")
    )
}

#[allow(clippy::too_many_arguments)]
pub fn answer(
    root: &Path,
    id: &str,
    answer: &str,
    quote: Option<&str>,
    option: Option<&str>,
    replace: Option<Vec<String>>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    answer_with(
        root,
        id,
        answer,
        quote,
        option,
        replace,
        harness,
        &Ledger::open()?,
    )
}

/// `replace: None` is no flag; `Some(vec![])` is a bare `--replace`.
#[allow(clippy::too_many_arguments)]
pub fn answer_with(
    root: &Path,
    id: &str,
    answer: &str,
    quote: Option<&str>,
    option: Option<&str>,
    replace: Option<Vec<String>>,
    harness: Option<String>,
    ledger: &Ledger,
) -> Result<Value, LedgerError> {
    if answer.trim().is_empty() {
        return Err("question_answer_empty".into());
    }
    let project = project(root)?;
    let (folded, _, now) = load(ledger, &project).map_err(not_recorded)?;
    let q = find(&folded, id)?;
    let qid = q.ask.id.clone();
    if let Some(close) = &q.close {
        return Err(LedgerError::Message(format!(
            "question_closed: {qid} was closed ({}); ask again with cort question add --force",
            close.reason.as_deref().unwrap_or("-")
        )));
    }
    if let Some(key) = option {
        if !q.ask.options.iter().any(|o| o.key == key) {
            return Err(LedgerError::Message(format!(
                "question_option_unknown: {key}"
            )));
        }
    }
    let current = &q.current_answers;
    let mut replaces: Vec<String> = match replace {
        None if current.is_empty() => Vec::new(),
        None => {
            return Err(LedgerError::Message(format!(
                "question_answered: {qid} already has {} current answer(s): {}; to change it: {}",
                current.len(),
                answers_list(current),
                retry_command(&qid, answer, current)
            )))
        }
        Some(named) if named.is_empty() => match current.as_slice() {
            [] => Vec::new(),
            [one] => vec![one.id.clone()],
            _ => {
                return Err(LedgerError::Message(format!(
                "question_answer_conflict: {qid} has {} current answers: {}; name every one: {}",
                current.len(),
                answers_list(current),
                retry_command(&qid, answer, current)
            )))
            }
        },
        Some(named) => {
            let answer_ids = q
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Answer)
                .map(|e| e.id.as_str());
            let mut resolved = Vec::with_capacity(named.len());
            for prefix in &named {
                let full = resolve_prefix(answer_ids.clone(), prefix)
                    .map_err(|code| LedgerError::Message(format!("{code}: answer {prefix}")))?;
                resolved.push(full);
            }
            resolved.sort();
            resolved.dedup();
            let mut wanted: Vec<String> = current.iter().map(|a| a.id.clone()).collect();
            wanted.sort();
            if resolved != wanted {
                return Err(LedgerError::Message(format!(
                    "question_answer_conflict: --replace must name every current answer of {qid}: {}; retry: {}",
                    if current.is_empty() { "none".to_string() } else { answers_list(current) },
                    if current.is_empty() {
                        format!("cort question answer {qid} --answer {}", sh(answer))
                    } else {
                        retry_command(&qid, answer, current)
                    }
                )));
            }
            resolved
        }
    };
    replaces.sort();
    let mut event = new_event(EventKind::Answer, &format!("{qid}\0{answer}"), harness)?;
    event.question = Some(qid.clone());
    event.answer = Some(answer.to_string());
    event.quote = quote.map(str::to_string);
    event.quoted = Some(quote.is_some());
    event.option = option.map(str::to_string);
    event.replaces = replaces;
    write_event(ledger, &project, &event)?;
    let text = q.ask.text.as_deref().unwrap_or("");
    let next = format!(
        "cort memory record --key {} --claim {}",
        sh(&format!("question {}", &qid[..qid.len().min(8)])),
        sh(&format!("{text} -> {answer}"))
    );
    let question = after_write(q, &event, now);
    Ok(json!({
        "outcome": "answered",
        "question": question,
        "answer": event_json(&event),
        "next": next,
    }))
}

pub fn close(
    root: &Path,
    id: &str,
    reason: CloseReason,
    of: Option<&str>,
    harness: Option<String>,
) -> Result<Value, LedgerError> {
    close_with(root, id, reason, of, harness, &Ledger::open()?)
}

pub fn close_with(
    root: &Path,
    id: &str,
    reason: CloseReason,
    of: Option<&str>,
    harness: Option<String>,
    ledger: &Ledger,
) -> Result<Value, LedgerError> {
    let project = project(root)?;
    let (folded, _, now) = load(ledger, &project).map_err(not_recorded)?;
    let q = find(&folded, id)?;
    let qid = q.ask.id.clone();
    if let Some(close) = &q.close {
        return Err(LedgerError::Message(format!(
            "question_closed: {qid} is already closed ({})",
            close.reason.as_deref().unwrap_or("-")
        )));
    }
    let of = match (reason, of) {
        (CloseReason::Duplicate, None) => {
            return Err("question_duplicate_needs_of: --reason duplicate needs --of <id>".into())
        }
        (CloseReason::Duplicate, Some(prefix)) => {
            let other = find(&folded, prefix)?;
            if other.ask.id == qid {
                return Err(LedgerError::Message(format!(
                    "question_duplicate_of_self: {qid} cannot duplicate itself"
                )));
            }
            Some(other.ask.id.clone())
        }
        (_, Some(_)) => {
            return Err("question_of_needs_duplicate: --of is only for --reason duplicate".into())
        }
        (_, None) => None,
    };
    let mut event = new_event(EventKind::Close, &qid, harness)?;
    event.question = Some(qid);
    event.reason = Some(reason.as_str().to_string());
    event.of = of;
    write_event(ledger, &project, &event)?;
    let question = after_write(q, &event, now);
    Ok(json!({"outcome": "closed", "question": question}))
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

fn lean_answers(question: &Value) -> String {
    let answers: Vec<String> = question["current_answers"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|a| {
            let text = a["answer"].as_str().unwrap_or("");
            let text = lean_field(text);
            let quoted = a["quoted"].as_bool() == Some(true);
            match (text.as_str(), quoted) {
                ("-", false) => "(no quote)".to_string(),
                (_, false) => format!("{text} (no quote)"),
                (_, true) => text,
            }
        })
        .collect();
    if answers.is_empty() {
        "-".to_string()
    } else {
        answers.join(" | ")
    }
}

/// `-f lean` for `cort question <verb>`: `add` prints `added <id>` (plus ` similar=<id8>,...`
/// when similar questions exist) or one `exists <id> <state>` line per matched question; `list` one TSV row per question
/// (`<id8> <state> <stale|-> <age>d <text> <answers|(no quote)|->`); `answer` and `close`
/// print `<outcome> <id8>`, and `answer` adds `next: <record command>`. No field is ever empty.
pub fn render_lean(verb: &str, payload: &Value) -> String {
    let mut out = String::new();
    match verb {
        "add" if payload["outcome"] == "exists" => {
            for q in payload["questions"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                out.push_str(&format!(
                    "exists {} {}\n",
                    lean_field(q["id"].as_str().unwrap_or("")),
                    lean_field(q["state"].as_str().unwrap_or(""))
                ));
            }
        }
        "add" => {
            out.push_str(&format!(
                "{} {}",
                lean_field(payload["outcome"].as_str().unwrap_or("")),
                lean_field(payload["question"]["id"].as_str().unwrap_or(""))
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
        "list" => {
            for q in payload["questions"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}d\t{}\t{}\n",
                    lean_id8(q),
                    lean_field(q["state"].as_str().unwrap_or("")),
                    if q["stale"].as_bool() == Some(true) {
                        "stale"
                    } else {
                        "-"
                    },
                    q["age_days"].as_i64().unwrap_or(0),
                    lean_field(q["text"].as_str().unwrap_or("")),
                    lean_answers(q)
                ));
            }
        }
        _ => {
            out.push_str(&format!(
                "{} {}\n",
                lean_field(payload["outcome"].as_str().unwrap_or("")),
                lean_id8(&payload["question"])
            ));
            if let Some(next) = payload["next"].as_str() {
                out.push_str(&format!("next: {}\n", lean_field(next)));
            }
        }
    }
    out
}
