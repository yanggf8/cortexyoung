//! Pure core of `cort question`: event model, fold, term matching and write bounds.
//! Nothing here touches the network or the disk.

use crate::memory::PARTIAL_IGNORED;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const NS_PER_DAY: i128 = 86_400_000_000_000;
const MAX_TEXT: usize = 600;
const MAX_OPTION: usize = 200;
const MAX_LABEL: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Ask,
    Answer,
    Close,
    /// A kind written by a newer cort. The fold keeps it in its question's history (or the
    /// orphans) and lets it change nothing: not the state, the answers, nor the last activity.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub key: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionEvent {
    pub id: String,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<QuestionOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
    pub recorded_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_machine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_machine_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Open,
    Answered,
    Closed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Question {
    pub ask: QuestionEvent,
    pub events: Vec<QuestionEvent>,
    pub state: State,
    pub current_answers: Vec<QuestionEvent>,
    pub answer_conflict: bool,
    pub close: Option<QuestionEvent>,
    pub last_activity: String,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Folded {
    pub questions: Vec<Question>,
    pub orphan_events: Vec<QuestionEvent>,
}

/// One stored event body. A row of a kind this version does not know is read leniently, so a
/// newer cort's fields cannot break an older one: whatever ids it carries (`question`,
/// `replaces`, `of`) are kept where they are strings. A body that is not JSON, or a known kind
/// whose fields do not parse, is still an error.
pub fn parse_event(body: &str) -> Result<QuestionEvent, String> {
    let error = match serde_json::from_str::<QuestionEvent>(body) {
        Ok(event) => return Ok(event),
        Err(error) => error.to_string(),
    };
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| error.clone())?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    match value.get("kind").and_then(|v| v.as_str()) {
        Some("ask" | "answer" | "close") | None => return Err(error),
        Some(_) => {}
    }
    let id = text("id").ok_or_else(|| error.clone())?;
    Ok(QuestionEvent {
        id,
        kind: EventKind::Unknown,
        question: text("question"),
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
        replaces: value
            .get("replaces")
            .and_then(|v| v.as_array())
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        reason: None,
        of: text("of"),
        recorded_at: text("recorded_at").unwrap_or_else(|| "0".to_string()),
        recorded_machine: text("recorded_machine"),
        recorded_machine_source: text("recorded_machine_source"),
        harness: text("harness"),
    })
}

fn ns(event: &QuestionEvent) -> i128 {
    event.recorded_at.parse().unwrap_or(0)
}

fn order(a: &QuestionEvent, b: &QuestionEvent) -> std::cmp::Ordering {
    ns(a).cmp(&ns(b)).then_with(|| a.id.cmp(&b.id))
}

pub fn fold(events: Vec<QuestionEvent>, now_ns: i128, stale_days: u64) -> Folded {
    let mut asks: Vec<QuestionEvent> = Vec::new();
    let mut rest: Vec<QuestionEvent> = Vec::new();
    for event in events {
        if event.kind == EventKind::Ask {
            if !asks.iter().any(|a| a.id == event.id) {
                asks.push(event);
            }
        } else {
            rest.push(event);
        }
    }
    let mut by_question: HashMap<String, Vec<QuestionEvent>> = HashMap::new();
    let mut orphan_events = Vec::new();
    for event in rest {
        match event.question.clone() {
            Some(q) if asks.iter().any(|a| a.id == q) => {
                by_question.entry(q).or_default().push(event)
            }
            _ => orphan_events.push(event),
        }
    }
    orphan_events.sort_by(order);
    let window = i128::from(stale_days).saturating_mul(NS_PER_DAY);
    let mut questions: Vec<Question> = asks
        .into_iter()
        .map(|ask| {
            let mut events = by_question.remove(&ask.id).unwrap_or_default();
            events.sort_by(order);
            let answers: Vec<&QuestionEvent> = events
                .iter()
                .filter(|e| e.kind == EventKind::Answer)
                .collect();
            let current_answers: Vec<QuestionEvent> = answers
                .iter()
                .filter(|a| {
                    !answers
                        .iter()
                        .any(|b| b.id != a.id && b.replaces.contains(&a.id))
                })
                .map(|a| (*a).clone())
                .collect();
            let close = events.iter().rfind(|e| e.kind == EventKind::Close).cloned();
            let state = if close.is_some() {
                State::Closed
            } else if !current_answers.is_empty() {
                State::Answered
            } else {
                State::Open
            };
            let last = events
                .iter()
                .filter(|e| e.kind != EventKind::Unknown)
                .map(ns)
                .chain([ns(&ask)])
                .max()
                .unwrap_or(0);
            let stale = state == State::Open && now_ns.saturating_sub(last) > window;
            Question {
                answer_conflict: current_answers.len() > 1,
                current_answers,
                close,
                state,
                last_activity: last.to_string(),
                stale,
                events,
                ask,
            }
        })
        .collect();
    questions.sort_by(|a, b| {
        let (x, y) = (
            a.last_activity.parse::<i128>().unwrap_or(0),
            b.last_activity.parse::<i128>().unwrap_or(0),
        );
        y.cmp(&x).then_with(|| a.ask.id.cmp(&b.ask.id))
    });
    Folded {
        questions,
        orphan_events,
    }
}

/// Lowercase match terms: ASCII words (stopwords and single characters dropped) and
/// overlapping bigrams of each non-ASCII run (a one-character run is kept whole).
pub fn terms(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: String| {
        if !out.contains(&t) {
            out.push(t);
        }
    };
    let mut word = String::new();
    let mut run: Vec<char> = Vec::new();
    fn flush_word(word: &mut String, push: &mut impl FnMut(String)) {
        if word.len() > 1 && !PARTIAL_IGNORED.contains(&word.as_str()) {
            push(std::mem::take(word));
        } else {
            word.clear();
        }
    }
    fn flush_run(run: &mut Vec<char>, push: &mut impl FnMut(String)) {
        match run.len() {
            0 => {}
            1 => push(run[0].to_string()),
            _ => {
                for pair in run.windows(2) {
                    push(pair.iter().collect());
                }
            }
        }
        run.clear();
    }
    for c in lower.chars() {
        if c.is_ascii() {
            flush_run(&mut run, &mut push);
            if c.is_ascii_alphanumeric() {
                word.push(c);
            } else {
                flush_word(&mut word, &mut push);
            }
        } else if c.is_alphanumeric() {
            flush_word(&mut word, &mut push);
            run.push(c);
        } else {
            flush_word(&mut word, &mut push);
            flush_run(&mut run, &mut push);
        }
    }
    flush_word(&mut word, &mut push);
    flush_run(&mut run, &mut push);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dedup {
    Exists(Vec<String>),
    Similar(Vec<String>),
    New,
}

pub(crate) fn occurs(haystack: &str, term: &str) -> bool {
    if term.is_ascii() {
        haystack
            .match_indices(term)
            .any(|(start, _)| start == 0 || !haystack.as_bytes()[start - 1].is_ascii_alphanumeric())
    } else {
        haystack.contains(term)
    }
}

pub fn classify(new_text: &str, questions: &[Question]) -> Dedup {
    let wanted = terms(new_text);
    if wanted.is_empty() {
        return Dedup::New;
    }
    let exists: Vec<String> = questions
        .iter()
        .filter(|q| {
            let have = terms(q.ask.text.as_deref().unwrap_or(""));
            wanted.iter().all(|t| have.contains(t))
        })
        .map(|q| q.ask.id.clone())
        .collect();
    if !exists.is_empty() {
        return Dedup::Exists(exists);
    }
    let similar: Vec<String> = questions
        .iter()
        .filter(|q| {
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
            let hits = wanted.iter().filter(|t| occurs(&hay, t)).count();
            hits * 2 >= wanted.len()
        })
        .map(|q| q.ask.id.clone())
        .collect();
    if similar.is_empty() {
        Dedup::New
    } else {
        Dedup::Similar(similar)
    }
}

fn check(what: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        return Err(format!("question_control_chars: {what}"));
    }
    if value.chars().count() > max {
        return Err(format!(
            "question_too_long: {what} exceeds {max} characters"
        ));
    }
    Ok(())
}

pub fn validate_ask(
    text: &str,
    options: &[QuestionOption],
    recommend: Option<&str>,
    blocks: Option<&str>,
    by: Option<&str>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("question_text_empty".to_string());
    }
    check("text", text, MAX_TEXT)?;
    for o in options {
        check("option key", &o.key, MAX_LABEL)?;
        check("option text", &o.text, MAX_OPTION)?;
    }
    if let Some(b) = blocks {
        check("blocks", b, MAX_LABEL)?;
    }
    if let Some(b) = by {
        check("by", b, MAX_LABEL)?;
    }
    if let Some(r) = recommend {
        if !options.iter().any(|o| o.key == r) {
            return Err(format!("question_recommend_unknown: {r}"));
        }
    }
    Ok(())
}

pub fn resolve_prefix<'a>(
    ids: impl Iterator<Item = &'a str>,
    prefix: &str,
) -> Result<String, String> {
    if prefix.len() < 8 {
        return Err("question_id_too_short".to_string());
    }
    if !prefix
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err("question_not_found".to_string());
    }
    let mut hits: Vec<&str> = ids.filter(|id| id.starts_with(prefix)).collect();
    hits.sort_unstable();
    hits.dedup();
    match hits.as_slice() {
        [] => Err("question_not_found".to_string()),
        [one] => Ok((*one).to_string()),
        _ => Err("question_id_ambiguous".to_string()),
    }
}
