//! `cort memory promote|archive|drop|restore` and `lifecycle on|off` against the fake transport
//! only; no test touches the network or a shared database.

use cort::lifecycle_store::{act_with, render_lean, switch_with, Release, Verb};
use cort::memory_store::{self, Ledger, LedgerError};
use cort::remote::{SqliteTransport, Transport};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

const NS: i128 = 1_000_000_000;
const DAY: i128 = 86_400 * NS;

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
    sent: Vec<String>,
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
    ) -> Result<Vec<u8>, cort::remote::RemoteError> {
        let action = {
            let mut control = self.control.lock().unwrap();
            control.posts += 1;
            control
                .sent
                .push(String::from_utf8_lossy(body).into_owned());
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
    inner: Arc<SqliteTransport>,
    control: Arc<Mutex<Control>>,
    now: i128,
}

fn now_ns() -> i128 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    i128::try_from(nanos).unwrap()
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("decision.md"), "one\ntwo\nthree\n").unwrap();
        for args in [
            vec!["init"],
            vec!["add", "decision.md"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "source",
            ],
            vec!["remote", "add", "origin", "https://example.test/ledger.git"],
        ] {
            assert!(Command::new("git")
                .current_dir(temp.path())
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
        let f = Self {
            _temp: temp,
            ledger,
            inner,
            control,
            now: now_ns(),
        };
        f.ledger.read(&f.project()).unwrap();
        f
    }

    /// A fixture whose project was switched on 30 days ago.
    fn enabled() -> Self {
        let f = Self::new();
        f.lifecycle("on000001", "on", None, f.now - 30 * DAY);
        f
    }

    fn root(&self) -> &Path {
        self._temp.path()
    }

    /// Fail the `offset`-th ledger post from now (1 = the next one).
    fn arm(&self, offset: usize, action: Action) {
        let mut control = self.control.lock().unwrap();
        let post = control.posts + offset;
        control.next.push((post, action));
    }

    /// Every request body the ledger sent that writes to the lifecycle table.
    fn lifecycle_writes(&self) -> Vec<String> {
        let control = self.control.lock().unwrap();
        control
            .sent
            .iter()
            .filter(|b| b.contains("project_memory_lifecycle") && b.contains("INSERT"))
            .cloned()
            .collect()
    }

    fn project(&self) -> String {
        memory_store::project(self.root()).unwrap()
    }

    fn query(&self, sql: &str) -> Value {
        let body =
            json!({"requests":[{"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]})
                .to_string();
        let response = self
            .inner
            .post("https://example.test", body.as_bytes(), &[])
            .unwrap();
        let value: Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(value["results"][0]["type"], "ok", "{value}");
        value
    }

    fn entry(&self, id: &str, key: &str, claim: &str, at_ns: i128) {
        self.entry_with(id, key, claim, at_ns, json!({}));
    }

    fn entry_with(&self, id: &str, key: &str, claim: &str, at_ns: i128, extra: Value) {
        let mut entry = json!({"id":id,"key":key,"claim":claim,"source":format!(".cort-unsourced/{id}"),
            "source_line":1,"source_sha256":"","unsourced":true,"recorded_at":at_ns.to_string()});
        for (k, v) in extra.as_object().unwrap() {
            entry[k] = v.clone();
        }
        self.query(&format!(
            "INSERT INTO project_memory VALUES ('{}', '{id}', '{}')",
            self.project(),
            entry.to_string().replace('\'', "''")
        ));
    }

    /// Recorded one day ago: short-term, not yet expired.
    fn short(&self, id: &str) {
        self.entry(id, &format!("k-{id}"), "plain claim", self.now - DAY);
    }

    /// Recorded 20 days ago under a 30-day-old epoch: short-term, expired, due for review.
    fn due(&self, id: &str) {
        self.entry(id, &format!("k-{id}"), "plain claim", self.now - 20 * DAY);
    }

    /// Recorded 45 days ago: long-term, not yet a forgetting candidate.
    fn long(&self, id: &str) {
        self.entry(id, &format!("k-{id}"), "plain claim", self.now - 45 * DAY);
    }

    fn lifecycle(&self, id: &str, kind: &str, entry: Option<&str>, at_ns: i128) {
        let event = json!({"id":id,"kind":kind,"entry":entry,"recorded_at":at_ns.to_string()});
        self.query(&format!(
            "INSERT INTO project_memory_lifecycle VALUES ('{}', '{id}', '{}')",
            self.project(),
            event.to_string().replace('\'', "''")
        ));
    }

    fn bodies(&self) -> Vec<Value> {
        let value = self.query("SELECT body FROM project_memory_lifecycle ORDER BY id");
        value["results"][0]["response"]["result"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| serde_json::from_str(row[0]["value"].as_str().unwrap()).unwrap())
            .collect()
    }

    fn count(&self) -> usize {
        self.bodies().len()
    }

    fn act(
        &self,
        verb: Verb,
        id: &str,
        release: Release,
        note: Option<&str>,
    ) -> Result<Value, LedgerError> {
        act_with(
            self.root(),
            &self.ledger,
            verb,
            id,
            release,
            note.map(str::to_string),
        )
    }

    fn plain(&self, verb: Verb, id: &str) -> Result<Value, LedgerError> {
        self.act(verb, id, none(), None)
    }
}

fn none() -> Release {
    Release {
        superseded_by: None,
        quote: None,
    }
}

fn by(id: &str) -> Release {
    Release {
        superseded_by: Some(id.to_string()),
        quote: None,
    }
}

fn quoted(text: &str) -> Release {
    Release {
        superseded_by: None,
        quote: Some(text.to_string()),
    }
}

/// `<code>: <message>` of a refusal.
fn message(result: Result<Value, LedgerError>) -> String {
    match result {
        Err(LedgerError::Refused { code, message, .. }) => format!("{code}: {message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn refusal(result: Result<Value, LedgerError>) -> (&'static str, Value) {
    match result {
        Err(LedgerError::Refused { code, detail, .. }) => (code, detail),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn state(v: &Value) -> &str {
    v["entry"]["state"].as_str().unwrap()
}

#[test]
fn each_verb_writes_one_insert_or_ignore() {
    let f = Fixture::enabled();
    // Full ids are longer than the 8-character prefixes the calls use.
    f.short("aaaaaaa1f0");
    f.short("aaaaaaa2f0");
    f.long("bbbbbbb1f0");
    let start = f.count();

    let v = f.plain(Verb::Promote, "aaaaaaa1").unwrap();
    assert_eq!(state(&v), "long_term");
    assert_eq!(v["entry"]["id"], "aaaaaaa1f0");
    assert_eq!(f.count(), start + 1);

    let v = f.plain(Verb::Archive, "bbbbbbb1").unwrap();
    assert_eq!(state(&v), "archived");
    assert_eq!(f.count(), start + 2);

    let v = f.plain(Verb::Restore, "bbbbbbb1").unwrap();
    assert_eq!(state(&v), "long_term");
    assert_eq!(f.count(), start + 3);

    let v = f.plain(Verb::Drop, "aaaaaaa2").unwrap();
    assert_eq!(state(&v), "dropped");
    assert_eq!(f.count(), start + 4);

    let v = f
        .act(Verb::Restore, "aaaaaaa2", none(), Some("kept after all"))
        .unwrap();
    assert_eq!(state(&v), "short_term");
    assert_eq!(f.count(), start + 5);

    let bodies = f.bodies();
    let kinds: Vec<&str> = bodies
        .iter()
        .filter(|b| b["kind"] != "on")
        .map(|b| b["kind"].as_str().unwrap())
        .collect();
    for kind in ["promote", "archive", "restore", "drop"] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    let last_restore = bodies
        .iter()
        .find(|b| b["kind"] == "restore" && b["entry"] == "aaaaaaa2f0")
        .unwrap();
    assert_eq!(last_restore["note"], "kept after all");
    assert!(last_restore["recorded_machine"].is_string());
    assert_eq!(last_restore["id"].as_str().unwrap().len(), 16);
    // Every stored event names the full entry id, never the prefix typed.
    for b in bodies.iter().filter(|b| b["kind"] != "on") {
        assert!(b["entry"].as_str().unwrap().ends_with("f0"), "{b}");
    }
    // One INSERT OR IGNORE per verb, and only those wrote to the table.
    let writes = f.lifecycle_writes();
    assert_eq!(writes.len(), 5, "{writes:?}");
    for w in &writes {
        assert!(
            w.contains("INSERT OR IGNORE INTO project_memory_lifecycle"),
            "{w}"
        );
    }
}

#[test]
fn refusals_write_nothing() {
    let f = Fixture::enabled();
    f.long("bbbbbbb1");
    f.entry("cccccc01", "k-user-thing", "who", f.now - DAY);
    f.entry("abcdef01aa", "k-amb-a", "x", f.now - DAY);
    f.entry("abcdef01bb", "k-amb-b", "x", f.now - DAY);
    f.entry("dddddd01", "k-old", "old", f.now - DAY);
    f.entry_with(
        "dddddd02",
        "k-old",
        "new",
        f.now - DAY,
        json!({"supersedes":"dddddd01"}),
    );
    let before = f.count();
    let cases: Vec<(&str, Result<Value, LedgerError>)> = vec![
        ("memory_id_unknown", f.plain(Verb::Promote, "deadbeef")),
        ("memory_id_unknown", f.plain(Verb::Promote, "zz")),
        ("memory_id_ambiguous", f.plain(Verb::Promote, "abcdef01")),
        (
            "memory_entry_superseded",
            f.plain(Verb::Archive, "dddddd01"),
        ),
        (
            "memory_lifecycle_not_allowed",
            f.plain(Verb::Restore, "bbbbbbb1"),
        ),
        (
            "memory_lifecycle_not_allowed",
            f.plain(Verb::Promote, "bbbbbbb1"),
        ),
        (
            "memory_archive_needs_release",
            f.plain(Verb::Archive, "cccccc01"),
        ),
        (
            "memory_archive_needs_release",
            f.plain(Verb::Drop, "cccccc01"),
        ),
        (
            "memory_quote_invalid",
            f.act(Verb::Archive, "cccccc01", quoted(""), None),
        ),
        (
            "memory_release_unexpected",
            f.act(Verb::Promote, "abcdef01aa", quoted("words"), None),
        ),
        (
            "memory_release_unexpected",
            f.act(Verb::Restore, "bbbbbbb1", by("cccccc01"), None),
        ),
        (
            "memory_archive_needs_release",
            f.act(
                Verb::Archive,
                "cccccc01",
                Release {
                    superseded_by: Some("abcdef01aa".into()),
                    quote: Some("words".into()),
                },
                None,
            ),
        ),
        (
            "memory_quote_invalid",
            f.act(Verb::Archive, "cccccc01", quoted("\u{200b}\u{200b}"), None),
        ),
        (
            "memory_quote_invalid",
            f.act(Verb::Archive, "cccccc01", quoted("..."), None),
        ),
        (
            "memory_note_invalid",
            f.act(Verb::Promote, "abcdef01aa", none(), Some(&"n".repeat(301))),
        ),
    ];
    for (code, result) in cases {
        let m = message(result);
        assert!(m.starts_with(code), "wanted {code}: {m}");
    }
    let not_allowed = message(f.plain(Verb::Restore, "bbbbbbb1"));
    assert!(
        not_allowed.contains("state=long_term") && not_allowed.contains("archive"),
        "{not_allowed}"
    );
    // The structured detail an agent acts on.
    let (code, detail) = refusal(f.plain(Verb::Restore, "bbbbbbb1"));
    assert_eq!(code, "memory_lifecycle_not_allowed");
    assert_eq!(detail["state"], "long_term");
    assert_eq!(detail["allowed"], json!(["archive"]));
    let (code, detail) = refusal(f.plain(Verb::Archive, "dddddd01"));
    assert_eq!(code, "memory_entry_superseded");
    assert_eq!(detail["successor"], "dddddd02");
    let (code, detail) = refusal(f.plain(Verb::Promote, "abcdef01"));
    assert_eq!(code, "memory_id_ambiguous");
    assert_eq!(detail["matches"], json!(["abcdef01aa", "abcdef01bb"]));
    assert_eq!(f.count(), before);
}

#[test]
fn off_switch_on_a_never_enabled_project_is_unchanged() {
    let f = Fixture::new();
    let out = switch_with(f.root(), &f.ledger, false, Some("why".into())).unwrap();
    assert_eq!(out["outcome"], "unchanged");
    assert_eq!(out["enabled"], false);
    assert_eq!(out["note_recorded"], false);
    assert_eq!(render_lean("lifecycle", &out), "unchanged off\n");
    assert_eq!(f.count(), 0);
}

#[test]
fn superseded_chain_names_the_active_successor() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "k-chain", "one", f.now - 3 * DAY);
    f.entry_with(
        "aaaaaaa2",
        "k-chain",
        "two",
        f.now - 2 * DAY,
        json!({"supersedes":"aaaaaaa1"}),
    );
    f.entry_with(
        "aaaaaaa3",
        "k-chain",
        "three",
        f.now - DAY,
        json!({"supersedes":"aaaaaaa2"}),
    );
    let (code, detail) = refusal(f.plain(Verb::Archive, "aaaaaaa1"));
    assert_eq!(code, "memory_entry_superseded");
    assert_eq!(detail["successor"], "aaaaaaa3");
}

#[test]
fn lean_keeps_the_unreadable_count() {
    let f = Fixture::enabled();
    f.short("aaaaaaa1");
    f.query(&format!(
        "INSERT INTO project_memory_lifecycle VALUES ('{}', 'bad00001', 'not json')",
        f.project()
    ));
    let v = f.plain(Verb::Promote, "aaaaaaa1").unwrap();
    assert_eq!(v["lifecycle_unreadable"], 1);
    assert_eq!(
        render_lean("promote", &v),
        "promoted aaaaaaa1 long_term lifecycle_unreadable=1\n"
    );
}

#[test]
fn entry_output_has_utc_beside_due_and_epoch() {
    let f = Fixture::new();
    f.short("aaaaaaa1");
    let on = switch_with(f.root(), &f.ledger, true, None).unwrap();
    assert!(on["epoch_utc"].as_str().unwrap().ends_with('Z'), "{on}");
    let v = f.plain(Verb::Archive, "aaaaaaa1").unwrap();
    // Hidden entries have no due date; a live one does.
    assert!(v["entry"].get("due_at_utc").is_none());
    let v = f.plain(Verb::Restore, "aaaaaaa1").unwrap();
    let due = v["entry"]["due_at_utc"].as_str().unwrap();
    assert!(due.ends_with('Z') && due.contains('T'), "{v}");
    assert!(v["entry"]["due_at"].is_string());
}

#[test]
fn write_failure_paths() {
    // Post 1 is the read, post 2 the insert, post 3 the read-back.
    let f = Fixture::enabled();
    f.short("aaaaaaa1");
    f.arm(2, Action::ForwardThenTimeout);
    let before = f.count();
    let v = f.plain(Verb::Promote, "aaaaaaa1").unwrap();
    assert_eq!(state(&v), "long_term");
    assert_eq!(f.count(), before + 1);

    f.short("aaaaaaa2");
    f.arm(2, Action::TimeoutOnly);
    let before = f.count();
    match f.plain(Verb::Promote, "aaaaaaa2") {
        Err(LedgerError::NotRecorded { .. }) => {}
        other => panic!("expected NotRecorded, got {other:?}"),
    }
    assert_eq!(f.count(), before);

    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::ForwardThenTimeout);
    match f.plain(Verb::Promote, "aaaaaaa2") {
        Err(LedgerError::OutcomeUnknown { id, key, .. }) => {
            assert_eq!(id.len(), 16);
            assert_eq!(key.as_deref(), Some("aaaaaaa2"));
        }
        other => panic!("expected OutcomeUnknown, got {other:?}"),
    }
    assert_eq!(f.count(), before + 1);
}

#[test]
fn switch_write_failure_paths() {
    let f = Fixture::new();
    f.arm(2, Action::ForwardThenTimeout);
    let out = switch_with(f.root(), &f.ledger, true, None).unwrap();
    assert_eq!(out["outcome"], "lifecycle_on");
    assert_eq!(f.count(), 1);

    f.arm(2, Action::TimeoutOnly);
    match switch_with(f.root(), &f.ledger, false, None) {
        Err(LedgerError::NotRecorded { .. }) => {}
        other => panic!("expected NotRecorded, got {other:?}"),
    }
    assert_eq!(f.count(), 1);

    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::ForwardThenTimeout);
    match switch_with(f.root(), &f.ledger, false, None) {
        Err(LedgerError::OutcomeUnknown { id, key, .. }) => {
            assert_eq!(id.len(), 16);
            assert_eq!(key, None);
        }
        other => panic!("expected OutcomeUnknown, got {other:?}"),
    }
    // It landed: re-running is safe and answers `unchanged`.
    assert_eq!(f.count(), 2);
    let again = switch_with(f.root(), &f.ledger, false, None).unwrap();
    assert_eq!(again["outcome"], "unchanged");
    assert_eq!(f.count(), 2);
}

#[test]
fn disabled_project_writes_nothing_but_on() {
    let f = Fixture::new();
    f.short("aaaaaaa1");
    for verb in [Verb::Promote, Verb::Archive, Verb::Drop, Verb::Restore] {
        let m = message(f.plain(verb, "aaaaaaa1"));
        assert!(m.starts_with("memory_lifecycle_off"), "{m}");
        assert!(m.contains("cort memory lifecycle on"), "{m}");
    }
    assert_eq!(f.count(), 0);

    let on = switch_with(f.root(), &f.ledger, true, None).unwrap();
    assert_eq!(on["outcome"], "lifecycle_on");
    assert_eq!(f.count(), 1);
    // Already on: nothing is written again.
    let again = switch_with(f.root(), &f.ledger, true, None).unwrap();
    assert_eq!(again["outcome"], "unchanged");
    assert_eq!(f.count(), 1);

    assert_eq!(
        state(&f.plain(Verb::Promote, "aaaaaaa1").unwrap()),
        "long_term"
    );
    let off = switch_with(f.root(), &f.ledger, false, Some("pause".into())).unwrap();
    assert_eq!(off["outcome"], "lifecycle_off");
    let n = f.count();
    let m = message(f.plain(Verb::Archive, "aaaaaaa1"));
    assert!(m.starts_with("memory_lifecycle_off"), "{m}");
    assert_eq!(f.count(), n);
}

#[test]
fn guard_user_in_key() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "the-USER-pick", "plain", f.now - 45 * DAY);
    let before = f.count();
    let m = message(f.plain(Verb::Archive, "aaaaaaa1"));
    assert!(m.starts_with("memory_archive_needs_release"), "{m}");
    assert_eq!(f.count(), before);
    let v = f
        .act(
            Verb::Archive,
            "aaaaaaa1",
            quoted("drop it, said the user"),
            None,
        )
        .unwrap();
    assert_eq!(state(&v), "archived");
    assert_eq!(f.count(), before + 1);
}

#[test]
fn guard_user_in_claim() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "k-plain", "the user chose A", f.now - DAY);
    let m = message(f.plain(Verb::Drop, "aaaaaaa1"));
    assert!(m.starts_with("memory_archive_needs_release"), "{m}");
    let v = f
        .act(Verb::Drop, "aaaaaaa1", quoted("not needed"), None)
        .unwrap();
    assert_eq!(state(&v), "dropped");
    let bodies = f.bodies();
    let drop = bodies.iter().find(|b| b["kind"] == "drop").unwrap();
    assert_eq!(drop["quote"], "not needed");
}

#[test]
fn guard_cjk_terms() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "k-a", "使用者決定用 A", f.now - 45 * DAY);
    f.entry("aaaaaaa2", "用戶-偏好", "plain", f.now - 45 * DAY);
    for id in ["aaaaaaa1", "aaaaaaa2"] {
        let m = message(f.plain(Verb::Archive, id));
        assert!(m.starts_with("memory_archive_needs_release"), "{id}: {m}");
    }
    // A non-user entry needs no release.
    f.long("bbbbbbb1");
    assert_eq!(
        state(&f.plain(Verb::Archive, "bbbbbbb1").unwrap()),
        "archived"
    );
}

#[test]
fn superseded_by_self_hidden_superseded_unknown_refused() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "k-user-a", "x", f.now - 45 * DAY);
    f.long("bbbbbbb1"); // not a user entry
    f.long("hhhhhhh1");
    f.entry("dddddd01", "k-old", "old", f.now - DAY);
    f.entry_with(
        "dddddd02",
        "k-old",
        "new",
        f.now - DAY,
        json!({"supersedes":"dddddd01"}),
    );
    f.plain(Verb::Archive, "hhhhhhh1").unwrap();
    let before = f.count();
    // Refused for a user entry and for one that mentions nobody: a given --superseded-by must be valid.
    for id in ["aaaaaaa1", "bbbbbbb1"] {
        for target in ["aaaaaaa1", "bbbbbbb1", "hhhhhhh1", "dddddd01"] {
            if target == id {
                let m = message(f.act(Verb::Archive, id, by(target), None));
                assert!(m.starts_with("memory_archive_needs_release"), "{m}");
                continue;
            }
            if target == "aaaaaaa1" || target == "bbbbbbb1" {
                continue; // a valid, active, distinct target
            }
            let m = message(f.act(Verb::Archive, id, by(target), None));
            assert!(
                m.starts_with("memory_archive_needs_release") && m.contains(target),
                "{id} -> {target}: {m}"
            );
        }
        let m = message(f.act(Verb::Archive, id, by("deadbeef"), None));
        assert!(m.starts_with("memory_id_unknown"), "{m}");
    }
    assert_eq!(f.count(), before);

    // A valid target, given as a prefix, is stored as the full resolved id.
    f.entry("eeeeeeee99", "k-new", "replacement", f.now - DAY);
    let v = f
        .act(Verb::Archive, "aaaaaaa1", by("eeeeeeee"), None)
        .unwrap();
    assert_eq!(state(&v), "archived");
    let bodies = f.bodies();
    let archive = bodies
        .iter()
        .find(|b| b["kind"] == "archive" && b["entry"] == "aaaaaaa1")
        .unwrap();
    assert_eq!(archive["superseded_by"], "eeeeeeee99");
}

/// Review Focus 3: a target that is itself due for review is active and not hidden, so it is
/// allowed.
#[test]
fn superseded_by_due_entry_allowed() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "k-user-a", "x", f.now - 45 * DAY);
    f.due("eeeeeee1f0");
    // The target really is due for review, which is the case under test.
    let (data, rows) = f.ledger.read_with_lifecycle(&f.project()).unwrap();
    let view = cort::memory_lifecycle::fold(&data, rows.events, rows.unreadable, f.now, None);
    assert_eq!(
        view.state_of("eeeeeee1f0"),
        Some(cort::memory_lifecycle::State::ShortTermExpired)
    );
    let v = f
        .act(Verb::Archive, "aaaaaaa1", by("eeeeeee1"), None)
        .unwrap();
    assert_eq!(state(&v), "archived");
    let bodies = f.bodies();
    let archive = bodies
        .iter()
        .find(|b| b["kind"] == "archive" && b["entry"] == "aaaaaaa1")
        .unwrap();
    assert_eq!(archive["superseded_by"], "eeeeeee1f0");
}

#[test]
fn quote_empty_blank_601_bytes_refused() {
    let f = Fixture::enabled();
    f.entry("aaaaaaa1", "k-user-a", "x", f.now - 45 * DAY);
    let before = f.count();
    for bad in ["", "   \n\t ", &"a".repeat(601)] {
        let m = message(f.act(Verb::Archive, "aaaaaaa1", quoted(bad), None));
        assert!(m.starts_with("memory_quote_invalid"), "{m}");
    }
    // 601 bytes in fewer characters is still too long.
    let wide = "用".repeat(201);
    assert_eq!(wide.len(), 603);
    let m = message(f.act(Verb::Archive, "aaaaaaa1", quoted(&wide), None));
    assert!(m.starts_with("memory_quote_invalid"), "{m}");
    assert_eq!(f.count(), before);
    let v = f
        .act(Verb::Archive, "aaaaaaa1", quoted(&"a".repeat(600)), None)
        .unwrap();
    assert_eq!(state(&v), "archived");
}

#[test]
fn prefix_exact_non_hex_id() {
    let f = Fixture::enabled();
    f.entry("imported-note", "k-imp", "plain", f.now - DAY);
    let v = f.plain(Verb::Promote, "imported-note").unwrap();
    assert_eq!(v["entry"]["id"], "imported-note");
    // A prefix of it is not hex, so it names nothing.
    let m = message(f.plain(Verb::Archive, "imported"));
    assert!(m.starts_with("memory_id_unknown"), "{m}");
}

#[test]
fn prefix_ambiguous() {
    let f = Fixture::enabled();
    f.entry("abcdef01aa", "k-a", "x", f.now - DAY);
    f.entry("abcdef01bb", "k-b", "x", f.now - DAY);
    let before = f.count();
    let m = message(f.plain(Verb::Promote, "abcdef01"));
    assert!(m.starts_with("memory_id_ambiguous"), "{m}");
    assert!(m.contains("abcdef01aa") && m.contains("abcdef01bb"), "{m}");
    assert_eq!(f.count(), before);
    let v = f.plain(Verb::Promote, "abcdef01a").unwrap();
    assert_eq!(v["entry"]["id"], "abcdef01aa");
}

#[test]
fn prefix_of_superseded_entry_refused_with_successor() {
    let f = Fixture::enabled();
    f.entry("dddddd01aa", "k-old", "old", f.now - DAY);
    f.entry_with(
        "dddddd02bb",
        "k-old",
        "new",
        f.now - DAY,
        json!({"supersedes":"dddddd01aa"}),
    );
    let before = f.count();
    let m = message(f.plain(Verb::Archive, "dddddd01"));
    assert!(m.starts_with("memory_entry_superseded"), "{m}");
    assert!(m.contains("dddddd02bb"), "{m}");
    assert_eq!(f.count(), before);
}

#[test]
fn write_stamps_after_later_event() {
    let f = Fixture::enabled();
    f.long("bbbbbbb1");
    let later = f.now + DAY;
    f.lifecycle("arc00001", "archive", Some("bbbbbbb1"), later);
    let v = f.plain(Verb::Restore, "bbbbbbb1").unwrap();
    assert_eq!(state(&v), "long_term");
    let bodies = f.bodies();
    let restore = bodies.iter().find(|b| b["kind"] == "restore").unwrap();
    assert_eq!(restore["recorded_at"], (later + 1).to_string());
}

#[test]
fn switch_stamps_after_a_later_switch_event() {
    let f = Fixture::enabled();
    let later = f.now + DAY;
    f.lifecycle("off00001", "off", None, later);
    let out = switch_with(f.root(), &f.ledger, true, None).unwrap();
    assert_eq!(out["outcome"], "lifecycle_on");
    let bodies = f.bodies();
    let on = bodies
        .iter()
        .filter(|b| b["kind"] == "on")
        .max_by_key(|b| b["recorded_at"].as_str().unwrap().parse::<i128>().unwrap())
        .unwrap();
    assert_eq!(on["recorded_at"], (later + 1).to_string());
}

#[test]
fn lean_lines_have_no_empty_field() {
    let f = Fixture::enabled();
    f.short("aaaaaaa1");
    let v = f.plain(Verb::Promote, "aaaaaaa1").unwrap();
    let line = render_lean("promote", &v);
    assert_eq!(line, "promoted aaaaaaa1 long_term\n");
    let on = switch_with(f.root(), &f.ledger, false, None).unwrap();
    assert_eq!(render_lean("lifecycle", &on), "lifecycle_off\n");
    let off_again = switch_with(f.root(), &f.ledger, false, None).unwrap();
    assert_eq!(render_lean("lifecycle", &off_again), "unchanged off\n");
    switch_with(f.root(), &f.ledger, true, None).unwrap();
    let on_again = switch_with(f.root(), &f.ledger, true, None).unwrap();
    assert_eq!(render_lean("lifecycle", &on_again), "unchanged on\n");
}
