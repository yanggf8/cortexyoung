use cort::memory_store::{self, Ledger, LedgerError};
use cort::remote::{RemoteError, SqliteTransport, Transport};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Action {
    Fail(u16),
    ForwardThenTimeout,
    ForwardThenFail(u16),
    Sql(String),
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

impl Wrapped {
    fn timeout(
        &self,
        status: u16,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> RemoteError {
        let transport = SqliteTransport::new().unwrap();
        transport.fail_next_status(status);
        transport.post(url, body, headers).unwrap_err()
    }

    fn sql(&self, url: &str, sql: &str, headers: &[(&str, &str)]) {
        let body =
            json!({"requests":[{"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]})
                .to_string();
        self.inner.post(url, body.as_bytes(), headers).unwrap();
    }
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
            Some(Action::Fail(status)) => Err(self.timeout(status, url, body, headers)),
            Some(Action::ForwardThenTimeout) => {
                self.inner.post(url, body, headers)?;
                Err(self.timeout(504, url, body, headers))
            }
            Some(Action::ForwardThenFail(status)) => {
                self.inner.post(url, body, headers)?;
                Err(self.timeout(status, url, body, headers))
            }
            Some(Action::Sql(sql)) => {
                self.sql(url, &sql, headers);
                self.inner.post(url, body, headers)
            }
            None => self.inner.post(url, body, headers),
        }
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    ledger: Ledger,
    control: Arc<Mutex<Control>>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("decision.md"), "one\ntwo\nthree\n").unwrap();
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
        ] {
            assert!(Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        assert!(Command::new("git")
            .current_dir(root)
            .args(["remote", "add", "origin", "https://example.test/ledger.git"])
            .status()
            .unwrap()
            .success());
        let inner = Arc::new(SqliteTransport::new().unwrap());
        let control = Arc::new(Mutex::new(Control::default()));
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(Wrapped {
                inner,
                control: control.clone(),
            }),
        )
        .unwrap();
        Self {
            _temp: temp,
            ledger,
            control,
        }
    }
    fn root(&self) -> &Path {
        self._temp.path()
    }
    fn project(&self) -> String {
        memory_store::project(self.root()).unwrap()
    }
    fn arm(&self, offset: usize, action: Action) {
        let mut control = self.control.lock().unwrap();
        let post = control.posts + offset;
        control.next.push((post, action));
    }
    fn record(
        &self,
        key: &str,
        claim: &str,
        supersedes: Option<&str>,
    ) -> Result<cort::memory::MemoryEntry, LedgerError> {
        memory_store::record_with(
            self.root(),
            key,
            claim,
            Path::new("decision.md"),
            1,
            supersedes,
            &self.ledger,
        )
    }
}

fn competing_sql(id: &str) -> String {
    let body = json!({"id":id,"key":"choice","claim":"other","source":"decision.md",
        "source_line":1,"source_sha256":"hash","recorded_at":"1"})
    .to_string()
    .replace('\'', "''");
    format!("INSERT INTO project_memory (project, id, body) SELECT project, '{id}', '{body}' FROM project_memory LIMIT 1")
}

#[test]
fn round_trip_and_supersedes_chain() {
    let f = Fixture::new();
    let a = f.record("choice", "A", None).unwrap();
    let b = f.record("choice", "B", Some(&a.id)).unwrap();
    let c = f.record("choice", "C", Some(&b.id)).unwrap();
    let rows = f.ledger.read(&f.project()).unwrap();
    assert_eq!(rows.entries.len(), 3);
    assert_eq!(c.supersedes.as_deref(), Some(b.id.as_str()));
}

#[test]
fn pre_read_and_definite_write_failures_save_no_row() {
    let f = Fixture::new();
    f.arm(1, Action::Fail(503));
    assert!(matches!(
        f.record("one", "A", None),
        Err(LedgerError::NotRecorded { .. })
    ));
    // 401 is a definite refusal, not an unknown outcome: no reread follows the failed write.
    let before = f.control.lock().unwrap().posts;
    f.arm(2, Action::Fail(401));
    assert!(matches!(
        f.record("two", "B", None),
        Err(LedgerError::NotRecorded {
            kind: cort::remote::ErrorKind::Unauthorized
        })
    ));
    assert_eq!(f.control.lock().unwrap().posts, before + 2);
    assert!(f.ledger.read(&f.project()).unwrap().entries.is_empty());
}

#[test]
fn import_that_fails_its_first_batch_definitely_is_not_recorded() {
    let f = Fixture::new();
    let first = f.record("choice", "A", None).unwrap();
    let mut legacy = first.clone();
    legacy.id = "legacy".into();
    fs::create_dir(f.root().join(".claudecat")).unwrap();
    fs::write(
        f.root().join(".claudecat/memory.json"),
        json!({"schema":1,"entries":[legacy]}).to_string(),
    )
    .unwrap();
    // Post 1 is the pre-read; the first batch is post 2 and nothing reads after it.
    let before = f.control.lock().unwrap().posts;
    f.arm(2, Action::Fail(401));
    assert!(matches!(
        memory_store::import_legacy_with(f.root(), &f.ledger),
        Err(LedgerError::NotRecorded {
            kind: cort::remote::ErrorKind::Unauthorized
        })
    ));
    assert_eq!(f.control.lock().unwrap().posts, before + 2);
    assert_eq!(f.ledger.read(&f.project()).unwrap().entries.len(), 1);
}

#[test]
fn timeout_after_forward_is_reconciled() {
    let f = Fixture::new();
    f.arm(2, Action::ForwardThenTimeout);
    assert!(f.record("choice", "A", None).is_ok());
    assert_eq!(f.ledger.read(&f.project()).unwrap().entries.len(), 1);
}

#[test]
fn timeout_after_forward_and_failed_reread_is_unknown() {
    let f = Fixture::new();
    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::Fail(503));
    let result = f.record("choice", "A", None);
    let Err(LedgerError::OutcomeUnknown {
        id: Some(id),
        key: Some(key),
        kind,
    }) = result
    else {
        panic!("expected outcome unknown: {result:?}");
    };
    assert_eq!(key, "choice");
    assert_eq!(kind, cort::remote::ErrorKind::Timeout);
    assert!(f
        .ledger
        .read(&f.project())
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.id == id));
}

#[test]
fn committed_write_with_failed_reread_keeps_row() {
    let f = Fixture::new();
    f.arm(3, Action::Fail(503));
    let result = f.record("choice", "A", None);
    let Err(LedgerError::CommittedUnverified { id: Some(id), .. }) = result else {
        panic!("expected committed unverified: {result:?}");
    };
    assert!(f
        .ledger
        .read(&f.project())
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.id == id));
}

#[test]
fn unforwarded_timeout_with_good_reread_is_not_recorded() {
    let f = Fixture::new();
    f.arm(2, Action::Fail(504));
    assert!(matches!(
        f.record("choice", "A", None),
        Err(LedgerError::NotRecorded {
            kind: cort::remote::ErrorKind::Timeout
        })
    ));
    assert!(f.ledger.read(&f.project()).unwrap().entries.is_empty());
}

#[test]
fn superseded_row_deleted_before_write_is_missing() {
    let f = Fixture::new();
    let first = f.record("choice", "A", None).unwrap();
    f.arm(
        2,
        Action::Sql(format!(
            "DELETE FROM project_memory WHERE id = '{}'",
            first.id
        )),
    );
    assert!(matches!(
        f.record("choice", "B", Some(&first.id)),
        Err(LedgerError::SupersedesMissing { .. })
    ));
    assert!(f.ledger.read(&f.project()).unwrap().entries.is_empty());
}

#[test]
fn competing_active_entry_reports_conflict_and_keeps_write() {
    let f = Fixture::new();
    // Seed another key so the SQL can copy this project's id without a hard-coded hash.
    f.record("seed", "seed", None).unwrap();
    f.arm(2, Action::Sql(competing_sql("competing")));
    let result = f.record("choice", "A", None);
    let Err(LedgerError::Conflict { id, active_ids, .. }) = result else {
        panic!("expected conflict: {result:?}");
    };
    assert!(active_ids.contains(&id));
    assert!(active_ids.contains(&"competing".to_string()));
    assert!(f
        .ledger
        .read(&f.project())
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.id == id));
}

#[test]
fn resolve_missing_predecessor_and_new_conflict() {
    let f = Fixture::new();
    f.record("seed", "seed", None).unwrap();
    f.arm(2, Action::Sql(competing_sql("competing")));
    let Err(LedgerError::Conflict { id, .. }) = f.record("choice", "A", None) else {
        panic!("expected conflict");
    };
    let ids = vec![id, "competing".into()];
    f.arm(
        2,
        Action::Sql("DELETE FROM project_memory WHERE id = 'competing'".into()),
    );
    let result = memory_store::resolve_with(
        f.root(),
        "choice",
        "resolved",
        Path::new("decision.md"),
        2,
        &ids,
        &f.ledger,
    );
    assert!(matches!(result, Err(LedgerError::SupersedesMissing { .. })));
    // Put the missing row back, then insert a third active claim immediately before resolution.
    f.arm(1, Action::Sql(competing_sql("competing")));
    f.ledger.read(&f.project()).unwrap();
    f.arm(2, Action::Sql(competing_sql("third")));
    let result = memory_store::resolve_with(
        f.root(),
        "choice",
        "resolved",
        Path::new("decision.md"),
        2,
        &ids,
        &f.ledger,
    );
    let Err(LedgerError::Conflict { id, active_ids, .. }) = result else {
        panic!("expected conflict: {result:?}");
    };
    assert!(active_ids.contains(&id));
    assert!(active_ids.contains(&"third".to_string()));
    assert!(f
        .ledger
        .read(&f.project())
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.id == id));
}

#[test]
fn import_is_idempotent_forgives_absent_stamp_and_refuses_collision() {
    let f = Fixture::new();
    let first = f.record("choice", "A", None).unwrap();
    let mut legacy = first.clone();
    legacy.recorded_machine = None;
    legacy.recorded_machine_source = None;
    fs::create_dir(f.root().join(".claudecat")).unwrap();
    let path = f.root().join(".claudecat/memory.json");
    fs::write(&path, json!({"schema":1,"entries":[legacy]}).to_string()).unwrap();
    assert_eq!(
        memory_store::import_legacy_with(f.root(), &f.ledger).unwrap(),
        0
    );
    let mut added = first.clone();
    added.id = "legacy".into();
    added.supersedes = None;
    fs::write(&path, json!({"schema":1,"entries":[added]}).to_string()).unwrap();
    assert_eq!(
        memory_store::import_legacy_with(f.root(), &f.ledger).unwrap(),
        1
    );
    assert_eq!(
        memory_store::import_legacy_with(f.root(), &f.ledger).unwrap(),
        0
    );
    added.claim = "tampered".into();
    fs::write(&path, json!({"schema":1,"entries":[added]}).to_string()).unwrap();
    assert!(
        matches!(memory_store::import_legacy_with(f.root(), &f.ledger), Err(LedgerError::Message(message)) if message == "memory id collision with different content")
    );
}

#[test]
fn resolve_settles_two_active_entries() {
    let f = Fixture::new();
    f.record("seed", "seed", None).unwrap();
    f.arm(2, Action::Sql(competing_sql("competing")));
    let Err(LedgerError::Conflict { id, .. }) = f.record("choice", "A", None) else {
        panic!("expected conflict");
    };
    let resolved = memory_store::resolve_with(
        f.root(),
        "choice",
        "resolved",
        Path::new("decision.md"),
        2,
        &[id.clone(), "competing".into()],
        &f.ledger,
    )
    .unwrap();
    let after = f.ledger.read(&f.project()).unwrap();
    let replaced = |id: &str| {
        after.entries.iter().any(|other| {
            other.supersedes.as_deref() == Some(id) || other.resolves.iter().any(|r| r == id)
        })
    };
    let active: Vec<_> = after
        .entries
        .iter()
        .filter(|entry| entry.key == "choice" && !replaced(&entry.id))
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, resolved.id);
    assert_eq!(resolved.resolves, vec![id, "competing".to_string()]);
}

/// A 400 classifies as `Sql`, which a malformed response to a committed write also produces, so
/// it is settled by the reread instead of being reported as not recorded.
#[test]
fn sql_error_after_a_committed_write_is_settled_by_the_reread() {
    let f = Fixture::new();
    f.arm(2, Action::ForwardThenFail(400));
    assert!(f.record("choice", "A", None).is_ok());
    assert_eq!(f.ledger.read(&f.project()).unwrap().entries.len(), 1);
    f.arm(2, Action::Fail(400));
    let result = f.record("other", "B", None);
    assert!(
        matches!(
            result,
            Err(LedgerError::NotRecorded {
                kind: cort::remote::ErrorKind::Sql
            })
        ),
        "{result:?}"
    );
    assert_eq!(f.ledger.read(&f.project()).unwrap().entries.len(), 1);
}

fn legacy_entries(f: &Fixture, count: usize) -> Vec<cort::memory::MemoryEntry> {
    let template = f.record("template", "T", None).unwrap();
    (0..count)
        .map(|index| {
            let mut entry = template.clone();
            entry.id = format!("legacy-{index:03}");
            entry.key = format!("legacy-{index:03}");
            entry
        })
        .collect()
}

#[test]
fn import_writes_every_batch_and_names_a_later_batch_failure() {
    let f = Fixture::new();
    let entries = legacy_entries(&f, 120);
    fs::create_dir(f.root().join(".claudecat")).unwrap();
    let path = f.root().join(".claudecat/memory.json");
    fs::write(&path, json!({"schema":1,"entries":entries}).to_string()).unwrap();
    // Post 1 is the pre-read and posts 2-4 are the three batches: the third is refused.
    f.arm(4, Action::Fail(401));
    let result = memory_store::import_legacy_with(f.root(), &f.ledger);
    let Err(LedgerError::Message(message)) = result else {
        panic!("expected a partial-import message: {result:?}");
    };
    assert_eq!(
        message,
        "legacy import stopped after 2 of 3 batches (unauthorized); re-run import-legacy, it is idempotent"
    );
    assert_eq!(f.ledger.read(&f.project()).unwrap().entries.len(), 101);
    assert_eq!(
        memory_store::import_legacy_with(f.root(), &f.ledger).unwrap(),
        20
    );
    assert_eq!(f.ledger.read(&f.project()).unwrap().entries.len(), 121);
}

#[test]
fn import_with_an_unknown_batch_and_a_failed_reread_is_unknown() {
    let f = Fixture::new();
    let entries = legacy_entries(&f, 3);
    fs::create_dir(f.root().join(".claudecat")).unwrap();
    fs::write(
        f.root().join(".claudecat/memory.json"),
        json!({"schema":1,"entries":entries}).to_string(),
    )
    .unwrap();
    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::Fail(503));
    assert!(matches!(
        memory_store::import_legacy_with(f.root(), &f.ledger),
        Err(LedgerError::OutcomeUnknown {
            id: None,
            kind: cort::remote::ErrorKind::Timeout,
            ..
        })
    ));
}
