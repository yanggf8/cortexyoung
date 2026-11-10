use cort::memory_store::{self, AliasFrom, Ledger, LedgerError};
use cort::question::QuestionOption;
use cort::question_store::{self, AddArgs, CloseReason, ListFilter};
use cort::remote::{RemoteError, SqliteTransport, Transport};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Action {
    ForwardThenTimeout,
}

#[derive(Default)]
struct Control {
    posts: usize,
    next: Vec<(usize, Action)>,
}

struct Wrapped {
    inner: Arc<SqliteTransport>,
    control: Arc<Mutex<Control>>,
}

impl Transport for Wrapped {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        let action = {
            let mut control = self.control.lock().unwrap();
            control.posts += 1;
            control
                .next
                .iter()
                .position(|(n, _)| *n == control.posts)
                .map(|index| control.next.remove(index).1)
        };
        match action {
            Some(Action::ForwardThenTimeout) => {
                self.inner.post(url, body, headers)?;
                let transport = SqliteTransport::new().unwrap();
                transport.fail_next_status(504);
                Err(transport.post(url, body, headers).unwrap_err())
            }
            None => self.inner.post(url, body, headers),
        }
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    ledger: Ledger,
    control: Arc<Mutex<Control>>,
    inner: Arc<SqliteTransport>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for args in [
            vec!["init"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.test/questions.git",
            ],
        ] {
            assert!(Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        let inner = Arc::new(SqliteTransport::new().unwrap());
        let control = Arc::new(Mutex::new(Control::default()));
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(Wrapped {
                inner: inner.clone(),
                control: control.clone(),
            }),
        )
        .unwrap();
        Self {
            _temp: temp,
            ledger,
            control,
            inner,
        }
    }
    fn root(&self) -> &Path {
        self._temp.path()
    }
    fn arm(&self, offset: usize, action: Action) {
        let mut control = self.control.lock().unwrap();
        let post = control.posts + offset;
        control.next.push((post, action));
    }
    fn query(&self, sql: &str) -> Result<Vec<Value>, String> {
        let body =
            json!({"requests":[{"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]})
                .to_string();
        let response = self
            .inner
            .post("https://example.test", body.as_bytes(), &[])
            .unwrap();
        let value: Value = serde_json::from_slice(&response).unwrap();
        if value["results"][0]["type"] != "ok" {
            return Err(value.to_string());
        }
        Ok(value["results"][0]["response"]["result"]["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
    fn rows(&self) -> usize {
        self.query("SELECT id FROM project_questions")
            .map(|rows| rows.len())
            .unwrap_or(0)
    }
    fn insert_event(&self, event: &Value) {
        let project = memory_store::project(self.root()).unwrap();
        let body = event.to_string().replace('\'', "''");
        self.query(&format!(
            "INSERT INTO project_questions VALUES ('{project}', '{}', '{body}')",
            event["id"].as_str().unwrap()
        ))
        .unwrap();
    }
    fn add(&self, text: &str) -> Value {
        self.add_args(args(text)).unwrap()
    }
    fn add_args(&self, args: AddArgs) -> Result<Value, LedgerError> {
        question_store::add_with(self.root(), args, &self.ledger)
    }
    fn answer(
        &self,
        id: &str,
        answer: &str,
        quote: Option<&str>,
        replace: Option<Vec<String>>,
    ) -> Result<Value, LedgerError> {
        question_store::answer_with(
            self.root(),
            id,
            answer,
            quote,
            None,
            replace,
            None,
            &self.ledger,
        )
    }
    fn list(&self, filter: ListFilter) -> Value {
        question_store::list_with(self.root(), filter, None, None, &self.ledger).unwrap()
    }
}

fn args(text: &str) -> AddArgs {
    AddArgs {
        text: text.into(),
        options: Vec::new(),
        recommend: None,
        blocks: None,
        by: None,
        harness: None,
        force: false,
    }
}

fn added_id(out: &Value) -> String {
    assert_eq!(out["outcome"], "added", "{out}");
    out["question"]["id"].as_str().unwrap().to_string()
}

fn message(err: LedgerError) -> String {
    match err {
        LedgerError::Message(m) => m,
        other => panic!("expected a message, got {other:?}"),
    }
}

#[test]
fn add_then_list_open() {
    let f = Fixture::new();
    let mut a = args("Keep the recall deadline at 3800 ms?");
    a.options = vec![
        QuestionOption {
            key: "A".into(),
            text: "keep".into(),
        },
        QuestionOption {
            key: "B".into(),
            text: "move to 4300".into(),
        },
    ];
    a.recommend = Some("A".into());
    a.blocks = Some("recall-timeout".into());
    let id = added_id(&f.add_args(a).unwrap());
    assert_eq!(id.len(), 16);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    let listed = f.list(ListFilter::Open);
    let questions = listed["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 1, "{listed}");
    assert_eq!(questions[0]["id"], id.as_str());
    assert_eq!(questions[0]["state"], "open");
    assert_eq!(questions[0]["stale"], false);
    assert_eq!(questions[0]["recommend"], "A");
    assert_eq!(questions[0]["blocks"], "recall-timeout");
    assert_eq!(questions[0]["options"][1]["key"], "B");
    assert!(listed.get("orphan_events").is_none());
    assert!(listed.get("events_total").is_none());
    assert_eq!(
        f.list(ListFilter::Answered)["questions"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn add_exists_writes_nothing() {
    let f = Fixture::new();
    let id = added_id(&f.add("Should the recall deadline move to 4300 ms?"));
    let before = f.rows();
    let out = f.add("recall deadline 4300?");
    assert_eq!(out["outcome"], "exists", "{out}");
    assert_eq!(out["questions"][0]["id"], id.as_str());
    assert_eq!(out["questions"][0]["state"], "open");
    assert_eq!(f.rows(), before);
}

#[test]
fn add_similar_writes_and_lists_similar() {
    let f = Fixture::new();
    let first = added_id(&f.add("Should the recall deadline move to 4300 ms?"));
    let before = f.rows();
    let out = f.add("Should the notes deadline move?");
    let id = added_id(&out);
    assert_eq!(f.rows(), before + 1);
    let similar = out["similar"].as_array().unwrap();
    assert_eq!(similar.len(), 1, "{out}");
    assert_eq!(similar[0]["id"], first.as_str());
    let command = similar[0]["close_command"].as_str().unwrap();
    assert!(
        command.contains(&format!("cort question close {id}")),
        "{command}"
    );
    assert!(
        command.contains(&format!("--reason duplicate --of {first}")),
        "{command}"
    );
}

#[test]
fn add_force_stores_reasks() {
    let f = Fixture::new();
    let first = added_id(&f.add("Should the recall deadline move to 4300 ms?"));
    let mut a = args("recall deadline 4300?");
    a.force = true;
    let out = f.add_args(a).unwrap();
    added_id(&out);
    assert_eq!(out["question"]["reasks"], json!([first]));
    assert_eq!(f.rows(), 2);
}

#[test]
fn answer_without_quote_stores_quoted_false() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    let out = f.answer(&id[..8], "keep it", None, None).unwrap();
    assert_eq!(out["outcome"], "answered", "{out}");
    assert_eq!(out["answer"]["quoted"], false);
    assert!(out["answer"].get("quote").is_none());
    let listed = f.list(ListFilter::Answered);
    assert_eq!(listed["questions"][0]["state"], "answered");
    assert_eq!(
        listed["questions"][0]["current_answers"][0]["quoted"],
        false
    );
    let quoted = Fixture::new();
    let id = added_id(&quoted.add("Keep the deadline?"));
    let out = quoted
        .answer(&id, "keep it", Some("  keep it, 就這樣  "), None)
        .unwrap();
    assert_eq!(out["answer"]["quoted"], true);
    assert_eq!(out["answer"]["quote"], "  keep it, 就這樣  ");
}

#[test]
fn answer_twice_is_question_answered_with_retry_command() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    let first = f.answer(&id, "keep it", None, None).unwrap();
    let answer_id = first["answer"]["id"].as_str().unwrap().to_string();
    let before = f.rows();
    let err = message(f.answer(&id, "move it", None, None).unwrap_err());
    assert!(err.starts_with("question_answered"), "{err}");
    assert!(err.contains("keep it"), "{err}");
    assert!(err.contains(&answer_id), "{err}");
    assert!(
        err.contains(&format!(
            "cort question answer {id} --answer 'move it' --replace {answer_id}"
        )),
        "{err}"
    );
    assert_eq!(f.rows(), before);
    let replaced = f.answer(&id, "move it", None, Some(vec![])).unwrap();
    assert_eq!(replaced["answer"]["replaces"], json!([answer_id]));
    let listed = f.list(ListFilter::Answered);
    assert_eq!(
        listed["questions"][0]["current_answers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        listed["questions"][0]["current_answers"][0]["answer"],
        "move it"
    );
}

fn racing_answers(f: &Fixture) -> (String, String, String) {
    let id = added_id(&f.add("Keep the deadline?"));
    let first = f.answer(&id, "keep it", None, None).unwrap();
    let a = first["answer"]["id"].as_str().unwrap().to_string();
    let b = "b0b0b0b0b0b0b0b0".to_string();
    f.insert_event(
        &json!({"id": b, "kind": "answer", "question": id, "answer": "move it",
        "quoted": false, "recorded_at": "99999999999999999999"}),
    );
    (id, a, b)
}

#[test]
fn bare_replace_with_two_current_is_refused() {
    let f = Fixture::new();
    let (id, a, b) = racing_answers(&f);
    let listed = f.list(ListFilter::Answered);
    assert_eq!(listed["questions"][0]["answer_conflict"], true);
    let before = f.rows();
    let err = message(f.answer(&id, "settle", None, Some(vec![])).unwrap_err());
    assert!(err.starts_with("question_answer_conflict"), "{err}");
    assert!(err.contains(&a) && err.contains(&b), "{err}");
    assert_eq!(f.rows(), before);
    let out = f
        .answer(&id, "settle", None, Some(vec![b[..8].into(), a.clone()]))
        .unwrap();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(out["answer"]["replaces"], json!(both));
}

#[test]
fn replace_subset_is_question_answer_conflict() {
    let f = Fixture::new();
    let (id, a, b) = racing_answers(&f);
    let before = f.rows();
    let err = message(
        f.answer(&id, "settle", None, Some(vec![a.clone()]))
            .unwrap_err(),
    );
    assert!(err.starts_with("question_answer_conflict"), "{err}");
    assert!(err.contains(&b), "{err}");
    assert_eq!(f.rows(), before);
}

#[test]
fn answer_closed_is_question_closed() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    question_store::close_with(f.root(), &id, CloseReason::Moot, None, None, &f.ledger).unwrap();
    let before = f.rows();
    let err = message(f.answer(&id, "keep", None, None).unwrap_err());
    assert!(err.starts_with("question_closed"), "{err}");
    assert_eq!(f.rows(), before);
}

#[test]
fn answer_output_has_next_record_command() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the user's deadline?"));
    let out = f.answer(&id, "yes, keep", None, None).unwrap();
    let next = out["next"].as_str().unwrap();
    assert_eq!(
        next,
        format!(
            "cort memory record --key 'question {}' --claim 'Keep the user'\\''s deadline? -> yes, keep'",
            &id[..8]
        )
    );
}

#[test]
fn close_duplicate_requires_existing_of() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    let other = added_id(&f.add("Rename the notes table?"));
    let before = f.rows();
    for of in [None, Some(id.as_str()), Some("ffffffffffff")] {
        let err =
            question_store::close_with(f.root(), &id, CloseReason::Duplicate, of, None, &f.ledger)
                .unwrap_err();
        assert!(matches!(err, LedgerError::Message(_)), "{err:?}");
    }
    assert_eq!(f.rows(), before);
    let out = question_store::close_with(
        f.root(),
        &id,
        CloseReason::Duplicate,
        Some(&other[..8]),
        None,
        &f.ledger,
    )
    .unwrap();
    assert_eq!(out["outcome"], "closed", "{out}");
    assert_eq!(out["question"]["close"]["of"], other.as_str());
    assert_eq!(out["question"]["close"]["reason"], "duplicate");
}

#[test]
fn close_records_machine() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    question_store::close_with(
        f.root(),
        &id,
        CloseReason::Stale,
        None,
        Some("codex".into()),
        &f.ledger,
    )
    .unwrap();
    let closed = f.list(ListFilter::Closed);
    let close = &closed["questions"][0]["close"];
    assert_eq!(close["reason"], "stale");
    assert_eq!(close["recorded_machine"], cort::usage::machine_id());
    assert_eq!(close["harness"], "codex");
    assert_eq!(closed["questions"][0]["state"], "closed");
}

#[test]
fn add_after_lost_response_reports_the_landed_id() {
    let f = Fixture::new();
    // Post 1 reads the events, post 2 is the insert.
    f.arm(2, Action::ForwardThenTimeout);
    let out = f.add("Keep the deadline?");
    let id = added_id(&out);
    assert_eq!(f.rows(), 1);
    let rows = f.query("SELECT id FROM project_questions").unwrap();
    assert_eq!(rows[0][0]["value"], id.as_str());
}

#[test]
fn memory_recall_never_returns_question_events() {
    let f = Fixture::new();
    added_id(&f.add("Should the recall deadline move to 4300 ms?"));
    let recall = memory_store::recall_with(f.root(), "recall deadline", &f.ledger).unwrap();
    assert!(recall.hits.is_empty());
}

#[test]
fn memory_status_provisions_questions_table() {
    let f = Fixture::new();
    assert!(f.query("SELECT 1 FROM project_questions").is_err());
    memory_store::status_with(f.root(), &f.ledger, "https://example.test").unwrap();
    assert!(f.query("SELECT 1 FROM project_questions").is_ok());
}

#[test]
fn list_match_reports_all_and_partial() {
    let f = Fixture::new();
    let a = added_id(&f.add("Keep the recall deadline at 3800 ms?"));
    let b = added_id(&f.add("Rename the notes table?"));
    f.answer(&b, "only after the recall deadline lands", None, None)
        .unwrap();
    let out = question_store::list_with(
        f.root(),
        ListFilter::Open,
        None,
        Some("recall deadline 3800"),
        &f.ledger,
    )
    .unwrap();
    let qs = out["questions"].as_array().unwrap();
    assert_eq!(qs.len(), 2, "{out}");
    assert_eq!(qs[0]["id"], a.as_str());
    assert_eq!(qs[0]["match"], "all");
    assert_eq!(qs[1]["id"], b.as_str());
    assert_eq!(qs[1]["match"], "partial");
}

#[test]
fn list_one_id_shows_events_and_orphans_are_reported() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    f.answer(&id, "keep", None, None).unwrap();
    f.insert_event(
        &json!({"id": "0rphan0000000000", "kind": "close", "question": "nope",
        "reason": "moot", "recorded_at": "1"}),
    );
    let one =
        question_store::list_with(f.root(), ListFilter::Open, Some(&id[..8]), None, &f.ledger)
            .unwrap();
    assert_eq!(one["questions"][0]["id"], id.as_str());
    assert_eq!(one["questions"][0]["events"].as_array().unwrap().len(), 1);
    let all = f.list(ListFilter::All);
    assert_eq!(all["orphan_events"][0]["id"], "0rphan0000000000");
}

/// Moves every question event to `old`, as if they were filed before a repository rename.
fn move_events_to(f: &Fixture, old: &str) {
    f.query(&format!("UPDATE project_questions SET project = '{old}'"))
        .unwrap();
}

fn alias_from(f: &Fixture, old: &str) -> Result<memory_store::AliasOutcome, LedgerError> {
    memory_store::alias_add_with(f.root(), AliasFrom::Project(old.into()), &f.ledger)
}

#[test]
fn alias_add_accepts_questions_only_old_project() {
    let f = Fixture::new();
    added_id(&f.add("Keep the deadline?"));
    move_events_to(&f, "old-questions-only");
    let linked = alias_from(&f, "old-questions-only").unwrap();
    assert_eq!(linked.outcome, "linked");
    let list = memory_store::alias_list_with(f.root(), &f.ledger).unwrap();
    let old = list
        .projects
        .iter()
        .find(|p| p.project_id == "old-questions-only")
        .unwrap();
    assert_eq!(old.row_count, 1);
    let empty = Fixture::new();
    let refused = message(alias_from(&empty, "nothing-here").unwrap_err());
    assert!(refused.contains("no memory or question rows"), "{refused}");
}

#[test]
fn answer_across_alias_folds_into_the_ask() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    move_events_to(&f, "old-repo");
    alias_from(&f, "old-repo").unwrap();
    let out = f.answer(&id[..8], "keep", None, None).unwrap();
    assert_eq!(out["outcome"], "answered", "{out}");
    let listed = f.list(ListFilter::All);
    assert_eq!(listed["questions"][0]["id"], id.as_str());
    assert_eq!(listed["questions"][0]["state"], "answered");
    assert!(listed.get("orphan_events").is_none(), "{listed}");
}

#[test]
fn alias_remove_refuses_when_answer_would_split_from_ask() {
    let f = Fixture::new();
    let id = added_id(&f.add("Keep the deadline?"));
    move_events_to(&f, "old-repo");
    alias_from(&f, "old-repo").unwrap();
    f.answer(&id, "keep", None, None).unwrap();
    let refused = message(
        memory_store::alias_remove_with(f.root(), AliasFrom::Project("old-repo".into()), &f.ledger)
            .unwrap_err(),
    );
    assert!(refused.starts_with("alias_in_use"), "{refused}");
    assert!(refused.contains(&id), "{refused}");
    assert_eq!(
        memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .len(),
        1
    );
}
