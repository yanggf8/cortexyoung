//! `cort memory recall` under the lifecycle fold, against the fake transport only.

use cort::memory_lifecycle::{fold, State};
use cort::memory_store::{self, Ledger, LedgerError};
use cort::remote::{SqliteTransport, Transport};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

const NS: i128 = 1_000_000_000;
const DAY: i128 = 86_400 * NS;

struct Shared(Arc<SqliteTransport>);

impl Transport for Shared {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, cort::remote::RemoteError> {
        self.0.post(url, body, headers)
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    ledger: Ledger,
    inner: Arc<SqliteTransport>,
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
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(Shared(inner.clone())),
        )
        .unwrap();
        let f = Self {
            _temp: temp,
            ledger,
            inner,
        };
        // A read provisions the tables, exactly as `recall` does.
        f.ledger.read(&f.project()).unwrap();
        f
    }
    fn root(&self) -> &Path {
        self._temp.path()
    }
    fn project(&self) -> String {
        memory_store::project(self.root()).unwrap()
    }
    fn sql(&self, sql: &str) {
        let body =
            json!({"requests":[{"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]})
                .to_string();
        let response = self
            .inner
            .post("https://example.test", body.as_bytes(), &[])
            .unwrap();
        let value: Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(value["results"][0]["type"], "ok", "{value}");
    }
    fn entry(&self, id: &str, key: &str, claim: &str, at_ns: i128) {
        let entry = json!({"id":id,"key":key,"claim":claim,"source":format!(".cort-unsourced/{id}"),
            "source_line":1,"source_sha256":"","unsourced":true,"recorded_at":at_ns.to_string()});
        self.sql(&format!(
            "INSERT INTO project_memory VALUES ('{}', '{id}', '{}')",
            self.project(),
            entry.to_string().replace('\'', "''")
        ));
    }
    fn lifecycle(&self, id: &str, kind: &str, entry: Option<&str>, at_ns: i128) {
        let event = json!({"id":id,"kind":kind,"entry":entry,"recorded_at":at_ns.to_string()});
        self.lifecycle_body(id, &event.to_string());
    }
    fn lifecycle_body(&self, id: &str, body: &str) {
        self.sql(&format!(
            "INSERT INTO project_memory_lifecycle VALUES ('{}', '{id}', '{}')",
            self.project(),
            body.replace('\'', "''")
        ));
    }
    fn recall(&self, query: &str, include_hidden: bool) -> Value {
        serde_json::to_value(
            memory_store::recall_all_with(self.root(), query, &self.ledger, include_hidden)
                .unwrap(),
        )
        .unwrap()
    }
}

fn ids(recall: &Value) -> Vec<String> {
    recall["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn recall_hides_archived_on_enabled_project() {
    let f = Fixture::new();
    let now = now_ns();
    f.entry("live0001", "k-live", "alpha live", now - DAY);
    f.entry("shelf001", "k-shelf", "alpha shelved", now - DAY);
    f.lifecycle("on000001", "on", None, now - 2 * DAY);
    f.lifecycle("arc00001", "archive", Some("shelf001"), now - DAY / 2);

    let shown = f.recall("alpha", false);
    assert_eq!(ids(&shown), ["live0001"]);
    assert!(shown["hits"][0].get("lifecycle").is_none());
    assert!(shown.get("lifecycle_unreadable").is_none());
    // `recall_with` is the same as include_hidden = false.
    let with =
        serde_json::to_value(memory_store::recall_with(f.root(), "alpha", &f.ledger).unwrap())
            .unwrap();
    assert_eq!(with, shown);

    let all = f.recall("alpha", true);
    assert_eq!(ids(&all), ["live0001", "shelf001"]);
    assert_eq!(all["hits"][1]["lifecycle"], "archived");
    assert!(all["hits"][0].get("lifecycle").is_none());
}

#[test]
fn disabled_project_ignores_archive_rows() {
    let f = Fixture::new();
    let now = now_ns();
    f.entry("shelf001", "k-shelf", "alpha shelved", 5);
    f.lifecycle("arc00001", "archive", Some("shelf001"), now - DAY / 2);
    assert_eq!(f.recall("alpha", false), golden_hit("shelf001", "k-shelf"));
}

/// The recall JSON of one unsourced entry recorded at 5 ns, as the pre-lifecycle path printed it.
fn golden_hit(id: &str, key: &str) -> Value {
    json!({
        "ledger_status": "present",
        "hits": [{
            "id": id, "key": key, "claim": "alpha shelved", "unsourced": true,
            "recorded_at": "5", "recorded_at_utc": "1970-01-01T00:00:00Z",
            "evidence_status": "unsourced"
        }]
    })
}

#[test]
fn disabled_project_without_rows_is_the_pre_lifecycle_answer() {
    let f = Fixture::new();
    f.entry("shelf001", "k-shelf", "alpha shelved", 5);
    for include_hidden in [false, true] {
        assert_eq!(
            f.recall("alpha", include_hidden),
            golden_hit("shelf001", "k-shelf")
        );
    }
}

#[test]
fn disabled_project_still_reports_a_bad_row() {
    let f = Fixture::new();
    f.entry("shelf001", "k-shelf", "alpha shelved", 5);
    f.lifecycle_body("bad00001", "{not json");
    let shown = f.recall("alpha", false);
    assert_eq!(shown["lifecycle_unreadable"], 1);
    assert!(shown["lifecycle_hint"]
        .as_str()
        .unwrap()
        .contains("review --all"));
    // Apart from the two disclosure fields the answer is the pre-lifecycle one.
    let mut rest = shown.clone();
    rest.as_object_mut().unwrap().remove("lifecycle_unreadable");
    rest.as_object_mut().unwrap().remove("lifecycle_hint");
    assert_eq!(rest, golden_hit("shelf001", "k-shelf"));
}

#[test]
fn recall_reports_unreadable_rows() {
    let f = Fixture::new();
    let now = now_ns();
    f.entry("live0001", "k-live", "alpha live", now - DAY);
    f.lifecycle("on000001", "on", None, now - 2 * DAY);
    f.lifecycle_body("bad00001", "{not json");
    f.lifecycle_body(
        "bad00002",
        &json!({"id":"bad00002","kind":"archive","recorded_at":"5"}).to_string(),
    );
    let recall = f.recall("alpha", false);
    assert_eq!(ids(&recall), ["live0001"]);
    assert_eq!(recall["lifecycle_unreadable"], 2);
    let hint = recall["lifecycle_hint"].as_str().unwrap();
    assert!(hint.contains("cort memory review --all") && hint.contains("cort memory recall --all"));
}

#[test]
fn record_replace_on_archived_key_starts_short_term() {
    let f = Fixture::new();
    let first = memory_store::record_with(
        f.root(),
        "choice",
        "old",
        Path::new("decision.md"),
        1,
        None,
        &f.ledger,
    )
    .unwrap();
    let now = now_ns();
    f.lifecycle("on000001", "on", None, now - 2 * DAY);
    f.lifecycle("arc00001", "archive", Some(&first.id), now);
    assert!(f.recall("choice", false)["hits"]
        .as_array()
        .unwrap()
        .is_empty());

    // The archived entry keeps the key: a plain record is refused, `--replace` works.
    let plain = memory_store::record_options_with(
        f.root(),
        "choice",
        "new",
        Path::new("decision.md"),
        1,
        &[],
        false,
        &f.ledger,
    );
    assert!(
        // The key is still taken (this is the refusal `main.rs` shows, not the race-lost
        // `ChainTaken`): it names the archived entry and asks for --supersedes or --replace.
        matches!(&plain, Err(LedgerError::Message(m))
            if m.contains("an active entry has this key") && m.contains(&first.id)
                && m.contains("--replace")),
        "{plain:?}"
    );
    let replacement = memory_store::record_options_with(
        f.root(),
        "choice",
        "new",
        Path::new("decision.md"),
        1,
        &[],
        true,
        &f.ledger,
    )
    .unwrap();
    assert_eq!(replacement.supersedes.as_deref(), Some(first.id.as_str()));

    let shown = f.recall("choice", false);
    assert_eq!(ids(&shown), std::slice::from_ref(&replacement.id));
    assert!(shown["hits"][0].get("lifecycle").is_none());

    let (ledger, rows) = f.ledger.read_with_lifecycle(&f.project()).unwrap();
    let view = fold(&ledger, rows.events, rows.unreadable, now_ns(), None);
    assert_eq!(view.state_of(&replacement.id), Some(State::ShortTerm));
    // The archive event now names a superseded entry: listed there, not as an orphan.
    assert!(view.orphans.is_empty(), "{:?}", view.orphans);
    assert!(
        view.superseded
            .iter()
            .any(|(id, events)| *id == first.id && events.iter().any(|e| e.id == "arc00001")),
        "{:?}",
        view.superseded
    );
}
