//! Task 2 tests for the reminder store (`cort::remind_store`): table, checked writes, verbs.
//!
//! The store reads the real clock (`SystemTime::now` inside `now_ns`), so dates are computed
//! from today rather than pinned: a pinned future date would rot into `remind_date_past`.

use cort::memory_store::{self, AliasFrom, Ledger, LedgerError};
use cort::remind_store::{self, AddArgs, CloseReason, ListFilter};
use cort::remote::{RemoteError, SqliteTransport, Transport};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
enum Action {
    /// Forward the post (so it commits), then lose both answers.
    ForwardThenTimeout,
    /// Lose the answer without forwarding: the write never lands.
    TimeoutOnly,
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
            Some(Action::TimeoutOnly) => {
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

/// The store writes a reminder snapshot under `data_dir()` after every successful read; every
/// test in this file points it at one temp directory (set once, before any fixture runs) so
/// none of them touches the real one. Fixtures that share an origin share a project id and so a
/// snapshot file: a test that reads its snapshot back takes its own origin.
fn snapshot_dir() -> &'static Path {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CORT_MEMORY_DIR", dir.path());
        dir
    })
    .path()
}

impl Fixture {
    fn new() -> Self {
        Self::with_origin("reminders")
    }

    fn with_origin(name: &str) -> Self {
        snapshot_dir();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let origin = format!("https://example.test/{name}.git");
        for args in [
            vec!["init"],
            vec!["remote", "add", "origin", origin.as_str()],
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
    fn posts(&self) -> usize {
        self.control.lock().unwrap().posts
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
        self.query("SELECT id FROM project_reminders")
            .map(|rows| rows.len())
            .unwrap_or(0)
    }
    /// Writes one event straight through the fixture's transport, as other tests seed rows
    /// (an `add` with a past due_at cannot go through `add_with`).
    fn insert_event(&self, event: &Value) {
        let project = memory_store::project(self.root()).unwrap();
        let body = event.to_string().replace('\'', "''");
        self.query(&format!(
            "INSERT INTO project_reminders VALUES ('{project}', '{}', '{body}')",
            event["id"].as_str().unwrap()
        ))
        .unwrap();
    }
    fn add_args(&self, args: AddArgs) -> Result<Value, LedgerError> {
        remind_store::add_with(self.root(), args, &self.ledger)
    }
    fn add_on(&self, text: &str, on: &str) -> Value {
        self.add_args(args(text, on)).unwrap()
    }
    fn defer(
        &self,
        id: &str,
        on: &str,
        note: Option<String>,
        harness: Option<String>,
    ) -> Result<Value, LedgerError> {
        remind_store::defer_with(self.root(), id, on, note, harness, &self.ledger)
    }
    fn close(
        &self,
        id: &str,
        reason: CloseReason,
        of: Option<String>,
        note: Option<String>,
        harness: Option<String>,
    ) -> Result<Value, LedgerError> {
        remind_store::close_with(self.root(), id, reason, of, note, harness, &self.ledger)
    }
    fn list(&self, filter: ListFilter) -> Value {
        remind_store::list_with(self.root(), filter, None, None, &self.ledger).unwrap()
    }
}

fn args(text: &str, on: &str) -> AddArgs {
    AddArgs {
        text: text.into(),
        on: on.into(),
        lead: 1,
        blocks: None,
        harness: None,
        force: false,
    }
}

/// `YYYY-MM-DD+08:00` for the UTC day `days` from today: an explicit offset keeps `due_at`
/// deterministic whatever the host's zone is set to.
fn on_in_days(days: i64) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i128;
    let day = secs.div_euclid(86_400) + i128::from(days);
    // Howard Hinnant's `civil_from_days`, the inverse of the core's `days_from_civil`.
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i128::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}+08:00")
}

fn added_id(out: &Value) -> String {
    assert_eq!(out["outcome"], "added", "{out}");
    out["reminder"]["id"].as_str().unwrap().to_string()
}

fn message(err: LedgerError) -> String {
    match err {
        LedgerError::Message(m) => m,
        other => panic!("expected a message, got {other:?}"),
    }
}

#[test]
fn add_then_list_shows_due_order() {
    let f = Fixture::new();
    let soon = on_in_days(3);
    let later = on_in_days(10);
    // Added out of order: the list is by effective due_at ascending, not by insertion.
    let b = added_id(&f.add_on("ThinkPad hotspot plan", &later));
    let a = added_id(&f.add_on("NUC boot auto-recovery", &soon));
    assert_eq!(a.len(), 16);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    let listed = f.list(ListFilter::Open);
    let reminders = listed["reminders"].as_array().unwrap();
    assert_eq!(reminders.len(), 2, "{listed}");
    assert_eq!(reminders[0]["id"], a.as_str());
    assert_eq!(reminders[0]["on"], json!(&soon[..10]));
    assert_eq!(reminders[0]["utc_offset"], "+08:00");
    assert_eq!(
        reminders[0]["due_at"],
        cort::remind::parse_on(&soon).unwrap().due_at.to_string()
    );
    assert_eq!(reminders[0]["text"], "NUC boot auto-recovery");
    assert_eq!(reminders[0]["lead"], 1);
    assert!(reminders[0]["state"].is_string());
    assert!(reminders[0]["label"].is_string());
    assert!(reminders[0].get("blocks").is_none());
    assert_eq!(reminders[1]["id"], b.as_str());
    assert!(listed.get("orphan_events").is_none());
    assert!(listed.get("events_total").is_none());
}

#[test]
fn add_exists_returns_earlier_without_write() {
    let f = Fixture::new();
    let on = on_in_days(5);
    let first = added_id(&f.add_on("Reboot the NUC", &on));
    let before = f.rows();
    let out = f.add_args(args("reboot NUC", &on)).unwrap();
    assert_eq!(out["outcome"], "exists", "{out}");
    assert_eq!(out["reminder"]["id"], first.as_str());
    assert_eq!(out["reminder"]["on"], json!(&on[..10]));
    assert_eq!(f.rows(), before);
    // The same task filed for another day is written, with the earlier one as similar.
    let other = f.add_args(args("reboot NUC", &on_in_days(12))).unwrap();
    let second = added_id(&other);
    let similar = other["similar"].as_array().unwrap();
    assert_eq!(similar.len(), 1, "{other}");
    assert_eq!(similar[0]["id"], first.as_str());
    let command = similar[0]["duplicate_command"].as_str().unwrap();
    assert!(
        command.contains(&format!("cort remind close {second}")),
        "{command}"
    );
    assert!(
        command.contains(&format!("--reason duplicate --of {first}")),
        "{command}"
    );
    assert_eq!(f.rows(), before + 1);
}

#[test]
fn add_past_refused_before_write() {
    let f = Fixture::new();
    let err = message(f.add_args(args("Too late", &on_in_days(-30))).unwrap_err());
    assert!(err.starts_with("remind_date_past"), "{err}");
    assert_eq!(f.rows(), 0);
    // Refused before any network round trip, not just before the write.
    assert_eq!(f.posts(), 0);
}

#[test]
fn defer_moves_date_and_reply_has_offset() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    let moved = on_in_days(17);
    let out = f
        .defer(
            &id[..8],
            &moved,
            Some("not this Saturday, the next one".into()),
            None,
        )
        .unwrap();
    assert_eq!(out["outcome"], "deferred", "{out}");
    assert_eq!(out["reminder"]["id"], id.as_str());
    assert_eq!(out["reminder"]["on"], json!(&moved[..10]));
    assert_eq!(out["reminder"]["utc_offset"], "+08:00");
    assert_eq!(
        out["reminder"]["due_at"],
        cort::remind::parse_on(&moved).unwrap().due_at.to_string()
    );
    let one =
        remind_store::list_with(f.root(), ListFilter::Open, Some(&id), None, &f.ledger).unwrap();
    assert_eq!(one["reminders"][0]["on"], json!(&moved[..10]), "{one}");
    let events = one["reminders"][0]["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{one}");
    assert_eq!(events[0]["kind"], "defer");
    assert_eq!(events[0]["note"], "not this Saturday, the next one");
}

#[test]
fn defer_closed_refused() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    f.close(&id, CloseReason::Done, None, None, None).unwrap();
    let before = f.rows();
    let err = message(f.defer(&id, &on_in_days(10), None, None).unwrap_err());
    assert!(err.starts_with("remind_closed"), "{err}");
    assert_eq!(f.rows(), before);
}

#[test]
fn close_done_then_close_again_refused() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    let out = f
        .close(
            &id,
            CloseReason::Done,
            None,
            None,
            Some("claude-code".into()),
        )
        .unwrap();
    assert_eq!(out["outcome"], "closed", "{out}");
    assert_eq!(out["reminder"]["state"], "closed");
    assert_eq!(out["reminder"]["label"], "done");
    assert_eq!(out["reminder"]["close"]["reason"], "done");
    assert_eq!(out["reminder"]["close"]["harness"], "claude-code");
    assert_eq!(
        out["reminder"]["close"]["recorded_machine"],
        cort::usage::machine_id()
    );
    let before = f.rows();
    let err = message(
        f.close(&id, CloseReason::Moot, None, None, None)
            .unwrap_err(),
    );
    assert!(err.starts_with("remind_closed"), "{err}");
    assert_eq!(f.rows(), before);
    assert_eq!(
        f.list(ListFilter::Closed)["reminders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn duplicate_requires_of() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("Reboot the NUC", &on_in_days(3)));
    let other = added_id(&f.add_on("Upgrade cort on the NUC", &on_in_days(4)));
    let before = f.rows();
    let posts = f.posts();
    let err = message(
        f.close(&id, CloseReason::Duplicate, None, None, None)
            .unwrap_err(),
    );
    assert!(err.starts_with("remind_of_required"), "{err}");
    // `--of` on done or moot is refused: it belongs to duplicate only.
    for reason in [CloseReason::Done, CloseReason::Moot] {
        let err = message(
            f.close(&id, reason, Some(other.clone()), None, None)
                .unwrap_err(),
        );
        assert!(err.starts_with("remind_of_unexpected"), "{err}");
    }
    // Both shapes are refused from the arguments alone, before any read.
    assert_eq!(f.posts(), posts);
    assert_eq!(f.rows(), before);
    let out = f
        .close(
            &id,
            CloseReason::Duplicate,
            Some(other[..8].to_string()),
            None,
            None,
        )
        .unwrap();
    assert_eq!(out["outcome"], "closed", "{out}");
    assert_eq!(out["reminder"]["close"]["of"], other.as_str());
    assert_eq!(out["reminder"]["close"]["reason"], "duplicate");
}

#[test]
fn lost_response_reports_outcome_unknown() {
    let f = Fixture::new();
    // Post 1 reads the events, post 2 is the insert, post 3 the read-back: the insert is
    // forwarded (so it commits) and then both answers are lost.
    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::ForwardThenTimeout);
    let err = f
        .add_args(args("NUC boot auto-recovery", &on_in_days(3)))
        .unwrap_err();
    match err {
        LedgerError::OutcomeUnknown { id, .. } => {
            assert_eq!(id.len(), 16);
            assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        }
        other => panic!("expected OutcomeUnknown, got {other:?}"),
    }
    // The write landed; only the answers were lost.
    assert_eq!(f.rows(), 1);
    assert_eq!(
        f.list(ListFilter::Open)["reminders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn lost_response_read_back_found_is_ok() {
    let f = Fixture::new();
    // Post 1 reads the events, post 2 is the insert: forwarded (so it commits), then the
    // answer is lost; the read-back finds the id and the add reports success.
    f.arm(2, Action::ForwardThenTimeout);
    let out = f
        .add_args(args("NUC boot auto-recovery", &on_in_days(3)))
        .unwrap();
    assert_eq!(out["outcome"], "added", "{out}");
    assert_eq!(f.rows(), 1);
    let rows = f.query("SELECT id FROM project_reminders").unwrap();
    assert_eq!(rows[0][0]["value"], out["reminder"]["id"], "{out}");
}

#[test]
fn lost_response_read_back_absent_is_not_recorded() {
    let f = Fixture::new();
    // Post 1 reads the events, post 2 is the insert: it times out without landing, and the
    // read-back finds nothing.
    f.arm(2, Action::TimeoutOnly);
    let err = f
        .add_args(args("NUC boot auto-recovery", &on_in_days(3)))
        .unwrap_err();
    match err {
        LedgerError::NotRecorded { .. } => {}
        other => panic!("expected NotRecorded, got {other:?}"),
    }
    assert_eq!(f.rows(), 0);
}

#[test]
fn defer_past_refused() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    let before = f.rows();
    let posts = f.posts();
    let err = message(f.defer(&id, &on_in_days(-5), None, None).unwrap_err());
    assert!(err.starts_with("remind_date_past"), "{err}");
    assert_eq!(f.rows(), before);
    // Refused before the read, exactly as `add` refuses the past.
    assert_eq!(f.posts(), posts);
}

#[test]
fn force_writes_past_exists() {
    let f = Fixture::new();
    let on = on_in_days(6);
    let first = added_id(&f.add_on("Reboot the NUC", &on));
    let mut a = args("reboot NUC", &on);
    a.force = true;
    let second = added_id(&f.add_args(a).unwrap());
    assert_ne!(first, second);
    assert_eq!(f.rows(), 2);
}

#[test]
fn orphan_events_listed() {
    let f = Fixture::new();
    added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    f.insert_event(&json!({
        "id": "0rphan0000000000", "kind": "close", "reminder": "nope",
        "reason": "moot", "recorded_at": "1",
    }));
    let all = f.list(ListFilter::All);
    assert_eq!(all["reminders"].as_array().unwrap().len(), 1, "{all}");
    assert_eq!(all["orphan_events"][0]["id"], "0rphan0000000000", "{all}");
}

#[test]
fn fresh_database_lists_empty() {
    let f = Fixture::new();
    let listed = f.list(ListFilter::Open);
    assert_eq!(listed["reminders"].as_array().unwrap().len(), 0, "{listed}");
    assert!(listed["project_id"].is_string());
    assert!(listed.get("orphan_events").is_none());
    assert!(listed.get("events_total").is_none());
}

#[test]
fn list_match_searches_closed() {
    let f = Fixture::new();
    let nuc = added_id(&f.add_on("Reboot the NUC", &on_in_days(3)));
    let upgrade = added_id(&f.add_on("Upgrade cort on the ThinkPad", &on_in_days(4)));
    f.close(&upgrade, CloseReason::Done, None, None, None)
        .unwrap();
    // The default list drops the closed one; --match searches every state.
    assert_eq!(
        f.list(ListFilter::Open)["reminders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let out =
        remind_store::list_with(f.root(), ListFilter::Open, None, Some("nuc"), &f.ledger).unwrap();
    let hits = out["reminders"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{out}");
    assert_eq!(hits[0]["id"], nuc.as_str());
    assert_eq!(hits[0]["match"], "all");
    let out = remind_store::list_with(
        f.root(),
        ListFilter::Closed,
        None,
        Some("thinkpad"),
        &f.ledger,
    )
    .unwrap();
    let hits = out["reminders"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{out}");
    assert_eq!(hits[0]["id"], upgrade.as_str());
    assert_eq!(hits[0]["state"], "closed");
    assert_eq!(hits[0]["match"], "all");
}

#[test]
fn lean_add_prints_id_on_offset() {
    let f = Fixture::new();
    let on = on_in_days(7);
    let out = f.add_args(args("NUC boot auto-recovery", &on)).unwrap();
    let id = added_id(&out);
    let line = remind_store::render_lean("add", &out);
    assert_eq!(line, format!("added {id} {} +08:00\n", &on[..10]), "{line}");
    // exists carries the earlier record's on and state.
    let exists = f.add_args(args("nuc boot auto recovery", &on)).unwrap();
    assert_eq!(exists["outcome"], "exists", "{exists}");
    let line = remind_store::render_lean("add", &exists);
    let fields: Vec<&str> = line.trim_end().split(' ').collect();
    assert_eq!(fields.len(), 4, "{line}");
    assert_eq!(fields[0], "exists");
    assert_eq!(fields[1], id.as_str());
    assert_eq!(fields[2], &on[..10]);
    assert!(!fields[3].is_empty());
}

#[test]
fn lean_list_has_no_empty_field() {
    let f = Fixture::new();
    let mut a = args("NUC boot auto-recovery", &on_in_days(3));
    a.blocks = Some("nuc-boot".into());
    f.add_args(a).unwrap();
    f.add_on("ThinkPad hotspot plan", &on_in_days(5));
    // A genuinely overdue row, seeded straight through the transport (`add` refuses the
    // past): 3 days ago, at any offset, is overdue on every read after this one.
    let now_ns = i128::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )
    .unwrap();
    f.insert_event(&json!({
        "id": "deadbeef0000000f", "kind": "add", "text": "Rotate the NUC log",
        "on": "2026-11-11", "utc_offset": "+00:00",
        "due_at": (now_ns - 3 * 86_400_000_000_000).to_string(),
        "lead": 1, "recorded_at": "1",
    }));
    let listed = f.list(ListFilter::Open);
    let lean = remind_store::render_lean("list", &listed);
    let lines: Vec<&str> = lean.lines().collect();
    assert_eq!(lines.len(), 3, "{lean}");
    let mut saw_dash = false;
    let mut saw_overdue = false;
    for line in &lines {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "{line}");
        assert!(
            fields.iter().all(|field| !field.is_empty()),
            "empty field in {line:?}"
        );
        saw_dash |= fields[5] == "-";
        assert_eq!(fields[1], fields[1].trim(), "{line}");
        if fields[2] == "overdue" {
            saw_overdue = true;
            // The label is one field (overdue_3d): a space would split the row for `read`.
            assert_eq!(fields[3], "overdue_3d", "{lean}");
            assert!(!fields[3].contains(' '), "{lean}");
        }
    }
    assert!(saw_dash, "the blocks-less row must print -, got {lean}");
    assert!(saw_overdue, "the overdue row must be listed, got {lean}");
}

/// Moves every reminder event to `old`, as if they were filed before a repository rename.
fn move_events_to(f: &Fixture, old: &str) {
    f.query(&format!("UPDATE project_reminders SET project = '{old}'"))
        .unwrap();
}

fn alias_from(f: &Fixture, old: &str) -> Result<memory_store::AliasOutcome, LedgerError> {
    memory_store::alias_add_with(f.root(), AliasFrom::Project(old.into()), &f.ledger)
}

#[test]
fn reminders_only_old_id_accepts_alias_add() {
    let f = Fixture::new();
    added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    move_events_to(&f, "old-reminders-only");
    let linked = alias_from(&f, "old-reminders-only").unwrap();
    assert_eq!(linked.outcome, "linked");
    // The old id's reminders are read through the alias under the current one.
    assert_eq!(
        f.list(ListFilter::Open)["reminders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let empty = Fixture::new();
    let refused = message(alias_from(&empty, "nothing-here").unwrap_err());
    assert!(
        refused.contains("no memory, question or reminder rows"),
        "{refused}"
    );
}

#[test]
fn alias_remove_across_defer_is_alias_in_use() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    move_events_to(&f, "old-repo");
    alias_from(&f, "old-repo").unwrap();
    // The defer lands under the current id while its add lives under the old one.
    f.defer(&id, &on_in_days(10), None, None).unwrap();
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

#[test]
fn existing_orphan_does_not_block_alias_remove() {
    let f = Fixture::new();
    added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    move_events_to(&f, "old-repo");
    alias_from(&f, "old-repo").unwrap();
    // A defer and a duplicate close whose `reminder`/`of` ids no component holds at all --
    // orphans an older cort already split off. They name nothing the split could separate,
    // so the removal goes through; only an id present on the other side counts.
    let on = on_in_days(9);
    let due_at = cort::remind::parse_on(&on).unwrap().due_at.to_string();
    f.insert_event(&json!({
        "id": "d1fe000000000001", "kind": "defer", "reminder": "gone0000000000ff",
        "on": &on[..10], "utc_offset": "+08:00", "due_at": due_at, "recorded_at": "5",
    }));
    f.insert_event(&json!({
        "id": "c105e000000000002", "kind": "close", "reminder": "gone0000000000ff",
        "reason": "duplicate", "of": "l0st00000000000f", "recorded_at": "6",
    }));
    let removed =
        memory_store::alias_remove_with(f.root(), AliasFrom::Project("old-repo".into()), &f.ledger)
            .unwrap();
    assert_eq!(removed.outcome, "removed");
    assert_eq!(
        memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .len(),
        0
    );
}

#[test]
fn listed_under_new_id_after_alias() {
    let f = Fixture::new();
    let id = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    move_events_to(&f, "old-repo");
    alias_from(&f, "old-repo").unwrap();
    let moved = on_in_days(17);
    let out = f.defer(&id[..8], &moved, None, None).unwrap();
    assert_eq!(out["outcome"], "deferred", "{out}");
    let listed = f.list(ListFilter::All);
    assert_eq!(listed["reminders"][0]["id"], id.as_str(), "{listed}");
    assert_eq!(
        listed["reminders"][0]["on"],
        json!(&moved[..10]),
        "{listed}"
    );
    assert!(listed.get("orphan_events").is_none(), "{listed}");
}

fn snapshot_ids(f: &Fixture) -> Vec<String> {
    let project = memory_store::project(f.root()).unwrap();
    let snap = cort::remind_snapshot::read_snapshot(snapshot_dir(), &project)
        .expect("a snapshot after a successful read");
    snap.events.iter().map(|e| e.id.clone()).collect()
}

fn snapshot_file(f: &Fixture) -> std::path::PathBuf {
    let project = memory_store::project(f.root()).unwrap();
    snapshot_dir().join(format!("remind-snapshot/{project}.json"))
}

#[test]
fn remind_list_and_close_refresh_snapshot() {
    let f = Fixture::with_origin("snap-list-close");
    // A list of an empty component writes an empty snapshot rather than skipping it.
    f.list(ListFilter::Open);
    assert!(snapshot_ids(&f).is_empty());
    let a = added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    assert_eq!(snapshot_ids(&f), vec![a.clone()]);
    let b = added_id(&f.add_on("ThinkPad hotspot plan", &on_in_days(10)));
    let mut ids = snapshot_ids(&f);
    ids.sort();
    let mut want = vec![a.clone(), b.clone()];
    want.sort();
    assert_eq!(ids, want);
    // An event from another writer shows up on the next list: insert it straight through the
    // transport, then list.
    let c = "c0c0c0c0c0c0c0c0";
    let ns = |days: i64| {
        (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i128
            + i128::from(days) * 86_400_000_000_000)
            .to_string()
    };
    f.insert_event(
        &json!({"id": c, "kind": "add", "text": "Rotate the Turso token",
        "on": "2030-01-01", "utc_offset": "+00:00", "due_at": ns(30), "lead": 1,
        "recorded_at": ns(0)}),
    );
    assert_eq!(snapshot_ids(&f).len(), 2);
    f.list(ListFilter::Closed);
    assert!(snapshot_ids(&f).contains(&c.to_string()));
    // A close drops the closed reminder with its events; a defer keeps add and defer.
    f.close(&a, CloseReason::Done, None, None, None).unwrap();
    let ids = snapshot_ids(&f);
    assert!(!ids.contains(&a), "{ids:?}");
    assert!(ids.contains(&b) && ids.contains(&c.to_string()), "{ids:?}");
    f.defer(&b, &on_in_days(12), None, None).unwrap();
    let ids = snapshot_ids(&f);
    assert_eq!(ids.len(), 3, "{ids:?}");
    // An `exists` answer refreshes it too, and a refusal before the read writes nothing.
    let before = std::fs::read(snapshot_file(&f)).unwrap();
    let out = f
        .add_args(args("ThinkPad hotspot plan", &on_in_days(12)))
        .unwrap();
    assert_eq!(out["outcome"], "exists", "{out}");
    assert_ne!(std::fs::read(snapshot_file(&f)).unwrap(), before);
    let before = std::fs::read(snapshot_file(&f)).unwrap();
    assert!(f.add_args(args("Too late", "2001-01-01")).is_err());
    assert_eq!(std::fs::read(snapshot_file(&f)).unwrap(), before);
}

#[test]
fn outcome_unknown_leaves_snapshot() {
    let f = Fixture::with_origin("snap-unknown");
    added_id(&f.add_on("NUC boot auto-recovery", &on_in_days(3)));
    let before = std::fs::read(snapshot_file(&f)).unwrap();
    // Post 1 reads the events, post 2 is the insert, post 3 the read-back: the insert lands and
    // both answers are lost, so the outcome is unknown.
    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::ForwardThenTimeout);
    let err = f
        .add_args(args("ThinkPad hotspot plan", &on_in_days(10)))
        .unwrap_err();
    assert!(matches!(err, LedgerError::OutcomeUnknown { .. }), "{err:?}");
    assert_eq!(f.rows(), 2);
    assert_eq!(std::fs::read(snapshot_file(&f)).unwrap(), before);
}
