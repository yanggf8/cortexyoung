use cort::question::*;

const DAY: i128 = 86_400_000_000_000;

fn ev(id: &str, kind: EventKind, question: Option<&str>, at: i128) -> QuestionEvent {
    serde_json::from_value(serde_json::json!({
        "id": id, "kind": kind, "question": question, "recorded_at": at.to_string()
    }))
    .unwrap()
}
fn ask(id: &str, text: &str, at: i128) -> QuestionEvent {
    let mut e = ev(id, EventKind::Ask, None, at);
    e.text = Some(text.to_string());
    e
}
fn answer(id: &str, q: &str, text: &str, replaces: &[&str], at: i128) -> QuestionEvent {
    let mut e = ev(id, EventKind::Answer, Some(q), at);
    e.answer = Some(text.to_string());
    e.replaces = replaces.iter().map(|s| s.to_string()).collect();
    e
}
fn one(events: Vec<QuestionEvent>) -> Question {
    fold(events, 100 * DAY, 14).questions.remove(0)
}
fn qs(list: &[(&str, &str)]) -> Vec<Question> {
    let events = list.iter().map(|(i, t)| ask(i, t, 1)).collect();
    fold(events, 2, 14).questions
}

#[test]
fn fold_ask_is_open() {
    let q = one(vec![ask("q1", "why", 1)]);
    assert_eq!(q.state, State::Open);
    assert!(!q.answer_conflict);
}

#[test]
fn fold_answer_makes_answered() {
    let q = one(vec![ask("q1", "why", 1), answer("a1", "q1", "x", &[], 2)]);
    assert_eq!(q.state, State::Answered);
    assert_eq!(q.current_answers.len(), 1);
}

#[test]
fn fold_close_wins_over_answer() {
    let q = one(vec![
        ask("q1", "why", 1),
        answer("a1", "q1", "x", &[], 2),
        ev("c1", EventKind::Close, Some("q1"), 3),
    ]);
    assert_eq!(q.state, State::Closed);
    assert!(q.close.is_some());
}

#[test]
fn two_unreplaced_answers_conflict() {
    let q = one(vec![
        ask("q1", "why", 1),
        answer("a1", "q1", "x", &[], 2),
        answer("a2", "q1", "y", &[], 2),
    ]);
    assert!(q.answer_conflict);
    assert_eq!(q.current_answers.len(), 2);
}

#[test]
fn replace_naming_both_leaves_one_current() {
    let q = one(vec![
        ask("q1", "why", 1),
        answer("a1", "q1", "x", &[], 2),
        answer("a2", "q1", "y", &[], 2),
        answer("a3", "q1", "z", &["a1", "a2"], 3),
    ]);
    assert!(!q.answer_conflict);
    assert_eq!(q.current_answers.len(), 1);
    assert_eq!(q.current_answers[0].id, "a3");
}

#[test]
fn two_replaces_of_one_answer_conflict_again() {
    let q = one(vec![
        ask("q1", "why", 1),
        answer("a1", "q1", "x", &[], 2),
        answer("a2", "q1", "y", &["a1"], 3),
        answer("a3", "q1", "z", &["a1"], 3),
    ]);
    assert!(q.answer_conflict);
    assert_eq!(q.current_answers.len(), 2);
}

#[test]
fn orphan_answer_is_reported_not_dropped() {
    let f = fold(
        vec![ask("q1", "why", 1), answer("a1", "nope", "x", &[], 2)],
        3,
        14,
    );
    assert_eq!(f.orphan_events.len(), 1);
    assert_eq!(f.questions[0].state, State::Open);
}

#[test]
fn stale_only_when_open_past_window() {
    let now = 100 * DAY;
    let stale = |events| fold(events, now, 14).questions[0].stale;
    assert!(stale(vec![ask("q1", "why", now - 15 * DAY)]));
    assert!(!stale(vec![ask("q1", "why", now - 13 * DAY)]));
    assert!(!stale(vec![
        ask("q1", "why", now - 30 * DAY),
        answer("a1", "q1", "x", &[], now - 30 * DAY + 1),
    ]));
}

#[test]
fn cjk_terms_are_bigrams() {
    assert_eq!(terms("保留期限"), vec!["保留", "留期", "期限"]);
    assert_eq!(terms("The 保 retention of x"), vec!["保", "retention"]);
    assert!(terms("the of a").is_empty());
}

#[test]
fn classify_exists_when_all_terms_in_earlier_text() {
    let list = qs(&[
        ("q1", "Keep the recall deadline at 3800 ms?"),
        ("q2", "保留期限是多久"),
    ]);
    assert_eq!(
        classify("recall deadline", &list),
        Dedup::Exists(vec!["q1".into()])
    );
    assert_eq!(
        classify("保留期限", &list),
        Dedup::Exists(vec!["q2".into()])
    );
    let mut closed = list;
    closed[0].state = State::Closed;
    assert!(matches!(
        classify("recall deadline", &closed),
        Dedup::Exists(_)
    ));
}

#[test]
fn classify_terms_only_in_an_answer_are_not_exists() {
    let events = vec![
        ask("q1", "deadline choice", 1),
        answer("a1", "q1", "use zebra mode", &[], 2),
    ];
    let list = fold(events, 3, 14).questions;
    let d = classify("zebra mode", &list);
    assert!(matches!(d, Dedup::Similar(_) | Dedup::New));
}

#[test]
fn classify_partial_is_similar() {
    let list = qs(&[("q1", "Keep the recall deadline at 3800 ms?")]);
    assert_eq!(classify("recall timeout banana", &list), Dedup::New);
    assert_eq!(
        classify("recall deadline banana", &list),
        Dedup::Similar(vec!["q1".into()])
    );
    let cjk = qs(&[("q2", "保留期限是多久")]);
    assert_eq!(
        classify("保留期間是多久", &cjk),
        Dedup::Similar(vec!["q2".into()])
    );
}

#[test]
fn validate_refuses_control_chars_and_overlength() {
    let opt = |k: &str, t: &str| QuestionOption {
        key: k.into(),
        text: t.into(),
    };
    assert!(validate_ask("ok", &[], None, None, None).is_ok());
    assert!(validate_ask(&"x".repeat(600), &[], None, None, None).is_ok());
    assert!(validate_ask(&"x".repeat(601), &[], None, None, None).is_err());
    assert!(validate_ask("a\nb", &[], None, None, None).is_err());
    assert!(validate_ask("ok", &[opt("A", &"x".repeat(201))], None, None, None).is_err());
    assert!(validate_ask("ok", &[opt("A", "yes")], Some("B"), None, None).is_err());
    assert!(validate_ask("ok", &[opt("A", "yes")], Some("A"), None, None).is_ok());
    assert!(validate_ask("ok", &[], None, Some(&"x".repeat(65)), None).is_err());
    assert!(validate_ask("ok", &[], None, None, Some("a\tb")).is_err());
}

#[test]
fn prefix_rules() {
    let ids = ["3f9a1c2e0001", "3f9a1c2e0002", "abcdef120000"];
    let r = |p: &str| resolve_prefix(ids.iter().copied(), p);
    assert_eq!(r("3f9a1c2"), Err("question_id_too_short".into()));
    assert_eq!(r("3f9a1c2e"), Err("question_id_ambiguous".into()));
    assert_eq!(r("abcdef12"), Ok("abcdef120000".into()));
    assert_eq!(r("ABCDEF12"), Err("question_not_found".into()));
    assert_eq!(r("deadbeef"), Err("question_not_found".into()));
}

#[test]
fn unknown_kind_is_kept_in_history_without_changing_state() {
    let future: QuestionEvent = serde_json::from_value(serde_json::json!({
        "id": "u1", "kind": "comment", "question": "q1", "recorded_at": (50 * DAY).to_string()
    }))
    .unwrap();
    assert_eq!(future.kind, EventKind::Unknown);
    let loose: QuestionEvent = serde_json::from_value(serde_json::json!({
        "id": "u2", "kind": "reopen", "recorded_at": "3"
    }))
    .unwrap();
    let base = vec![ask("q1", "why", 1), answer("a1", "q1", "x", &[], 2)];
    let plain = fold(base.clone(), 100 * DAY, 14);
    let mut with = base;
    with.push(future);
    with.push(loose);
    let folded = fold(with, 100 * DAY, 14);
    let (q, p) = (&folded.questions[0], &plain.questions[0]);
    assert_eq!(q.state, p.state);
    assert_eq!(q.current_answers, p.current_answers);
    assert_eq!(q.answer_conflict, p.answer_conflict);
    assert_eq!(q.last_activity, p.last_activity);
    assert_eq!(q.stale, p.stale);
    assert!(q.events.iter().any(|e| e.id == "u1"));
    assert_eq!(folded.orphan_events.len(), 1);
    assert_eq!(folded.orphan_events[0].id, "u2");
}

#[test]
fn parse_event_reads_an_unknown_kind_with_unfamiliar_fields() {
    // A future kind whose fields this version types differently still yields its ids.
    let body = r#"{"id":"u3","kind":"comment","question":"q1","text":{"rich":true},
        "replaces":["a1",7],"of":"q2","recorded_at":"9"}"#;
    let event = parse_event(body).unwrap();
    assert_eq!(event.kind, EventKind::Unknown);
    assert_eq!(event.question.as_deref(), Some("q1"));
    assert_eq!(event.replaces, vec!["a1".to_string()]);
    assert_eq!(event.of.as_deref(), Some("q2"));
    assert!(event.text.is_none());
    // A known kind with a broken field, or a body that is not JSON, is still an error.
    assert!(parse_event(r#"{"id":"x","kind":"answer","answer":7,"recorded_at":"1"}"#).is_err());
    assert!(parse_event("not json").is_err());
}
