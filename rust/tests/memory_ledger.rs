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
    SqlThenFail(String, u16),
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
            Some(Action::SqlThenFail(sql, status)) => {
                self.sql(url, &sql, headers);
                Err(self.timeout(status, url, body, headers))
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
    fn project(&self) -> String {
        memory_store::project(self.root()).unwrap()
    }
    fn arm(&self, offset: usize, action: Action) {
        let mut control = self.control.lock().unwrap();
        let post = control.posts + offset;
        control.next.push((post, action));
    }
    fn sql(&self, sql: &str) {
        let body =
            json!({"requests":[{"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]})
                .to_string();
        let response = self
            .inner
            .post("https://example.test", body.as_bytes(), &[])
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(value["results"][0]["type"], "ok", "{value}");
    }
    fn alias(&self, a: &str, b: &str) {
        self.sql(&format!(
            "INSERT INTO project_aliases VALUES ('{a}', '{b}', 'now', 'test')"
        ));
    }
    fn insert(&self, project: &str, entry: &cort::memory::MemoryEntry) {
        let body = serde_json::to_string(entry).unwrap().replace('\'', "''");
        self.sql(&format!(
            "INSERT INTO project_memory VALUES ('{project}', '{}', '{body}')",
            entry.id
        ));
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
fn recall_from_subdirectory_matches_repository_root() {
    let f = Fixture::new();
    f.record("choice", "A", None).unwrap();
    let sub = f.root().join("sub");
    fs::create_dir(&sub).unwrap();
    let top = memory_store::recall_with(f.root(), "choice", &f.ledger).unwrap();
    let nested = memory_store::recall_with(&sub, "choice", &f.ledger).unwrap();
    assert_eq!(top.hits[0].evidence_status, "source_unchanged");
    assert_eq!(nested.hits[0].evidence_status, top.hits[0].evidence_status);
    assert_eq!(
        nested.hits[0].source_line_status,
        top.hits[0].source_line_status
    );
}

#[cfg(unix)]
#[test]
fn record_from_subdirectory_stores_repository_relative_source() {
    let f = Fixture::new();
    let sub = f.root().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("nested.md"), "nested\n").unwrap();
    assert!(Command::new("git")
        .current_dir(f.root())
        .args(["add", "sub/nested.md"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .current_dir(f.root())
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "nested"
        ])
        .status()
        .unwrap()
        .success());
    let entry = memory_store::record_with(
        &sub,
        "choice",
        "A",
        Path::new("../decision.md"),
        1,
        None,
        &f.ledger,
    )
    .unwrap();
    assert_eq!(entry.source, "decision.md");
    let nested = memory_store::record_with(
        &sub,
        "nested",
        "B",
        Path::new("nested.md"),
        1,
        None,
        &f.ledger,
    )
    .unwrap();
    assert_eq!(nested.source, "sub/nested.md");
    // An existing file outside the work tree, reached through a symlink, is refused for where
    // it is, not because it cannot be opened.
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("outside.md"), "outside\n").unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("outside.md"), sub.join("escape.md")).unwrap();
    let error = memory_store::record_with(
        &sub,
        "outside",
        "C",
        Path::new("escape.md"),
        1,
        None,
        &f.ledger,
    )
    .unwrap_err();
    assert!(
        matches!(error, LedgerError::Message(message) if message.contains("source must be a relative path inside the repository"))
    );
    fs::write(sub.join("nested.md"), "changed\n").unwrap();
    let error = memory_store::record_with(
        &sub,
        "dirty",
        "D",
        Path::new("nested.md"),
        1,
        None,
        &f.ledger,
    )
    .unwrap_err();
    assert!(
        matches!(error, LedgerError::Message(message) if message.contains("uncommitted changes"))
    );
}

#[cfg(unix)]
#[test]
fn record_resolves_a_parent_step_after_a_symlink_through_the_filesystem() {
    let f = Fixture::new();
    for dir in ["sub", "a/b"] {
        fs::create_dir_all(f.root().join(dir)).unwrap();
    }
    fs::write(f.root().join("a/decision.md"), "real\n").unwrap();
    fs::write(f.root().join("sub/decision.md"), "lexical\n").unwrap();
    std::os::unix::fs::symlink("../a/b", f.root().join("sub/link")).unwrap();
    for args in [
        &["add", "."][..],
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "link",
        ],
    ] {
        assert!(Command::new("git")
            .current_dir(f.root())
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let sub = f.root().join("sub");
    let entry = memory_store::record_with(
        &sub,
        "choice",
        "A",
        Path::new("link/../decision.md"),
        1,
        None,
        &f.ledger,
    )
    .unwrap();
    // `link` is followed before `..`, as reading the file does: a lexical pop would cite
    // sub/decision.md, a different committed file.
    assert_eq!(entry.source, "a/decision.md");
}

#[test]
fn session_recall_from_payload_subdirectory_checks_repository_source() {
    let f = Fixture::new();
    let entry = f.record("choice", "A", None).unwrap();
    let sub = f.root().join("sub");
    fs::create_dir(&sub).unwrap();
    let db = cort::remote::RemoteDb::new(
        "https://example.test",
        "test",
        Box::new(Wrapped {
            inner: f.inner.clone(),
            control: f.control.clone(),
        }),
    )
    .unwrap();
    let (recall, _, _, _) = memory_store::session_recall_with(
        &sub,
        &db,
        "self",
        (&f.project(), &f.project()),
        std::time::Instant::now(),
    )
    .unwrap();
    assert_eq!(recall.hits[0].entry.id, entry.id);
    assert_eq!(recall.hits[0].evidence_status, "source_unchanged");
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
fn supersedes_prefixes_are_resolved_and_validated() {
    let f = Fixture::new();
    let first = f.record("choice", "A", None).unwrap();
    let next = f.record("choice", "B", Some(&first.id[..8])).unwrap();
    assert_eq!(next.supersedes.as_deref(), Some(first.id.as_str()));
    assert!(matches!(
        f.record("choice", "C", Some(&next.id[..7])),
        Err(LedgerError::Message(_))
    ));
    assert!(matches!(
        f.record("choice", "C", Some("ffffffffffffffff")),
        Err(LedgerError::SupersedesMissing { .. })
    ));
    let mut other = next.clone();
    other.id = format!("{}{}", &first.id[..8], "0".repeat(56));
    if other.id == first.id {
        other.id.replace_range(63..64, "1");
    }
    other.key = "other".into();
    other.supersedes = None;
    f.insert(&f.project(), &other);
    let Err(LedgerError::Message(message)) = f.record("choice", "C", Some(&first.id[..8])) else {
        panic!("expected ambiguity")
    };
    assert!(message.contains(&first.id) && message.contains(&other.id));
}

#[test]
fn active_entry_refusal_names_id() {
    let f = Fixture::new();
    let first = f.record("choice", "A", None).unwrap();
    let Err(LedgerError::Message(message)) = f.record("choice", "B", None) else {
        panic!("expected refusal")
    };
    assert!(message.contains(&first.id) && message.contains("--replace"));
}

#[test]
fn replace_and_repeated_supersedes_resolve_conflict() {
    let f = Fixture::new();
    let write = |ids: &[String], replace| {
        memory_store::record_options_with(
            f.root(),
            "choice",
            "new",
            Path::new("decision.md"),
            1,
            ids,
            replace,
            &f.ledger,
        )
    };
    assert!(
        matches!(write(&[], true), Err(LedgerError::Message(message)) if message.contains("drop --replace"))
    );
    let first = f.record("choice", "old", None).unwrap();
    assert!(
        matches!(write(std::slice::from_ref(&first.id), true), Err(LedgerError::Message(message)) if message.contains("conflicts"))
    );
    let replacement = write(&[], true).unwrap();
    assert_eq!(replacement.supersedes.as_deref(), Some(first.id.as_str()));
    let mut other = first.clone();
    other.id = "a".repeat(64);
    other.claim = "competing".into();
    f.insert(&f.project(), &other);
    let Err(LedgerError::Message(message)) = write(&[], true) else {
        panic!("expected conflict")
    };
    assert!(
        message.contains(&replacement.id)
            && message.contains(&other.id)
            && message.contains("--supersedes")
    );
    let ids = vec![replacement.id[..8].to_string(), other.id[..8].to_string()];
    assert!(
        matches!(write(&[ids[0].clone(), ids[0].clone()], false), Err(LedgerError::Message(message)) if message.contains("exactly once"))
    );
    let resolved = write(&ids, false).unwrap();
    assert_eq!(resolved.resolves, vec![replacement.id, other.id]);
    let status = memory_store::status_with(f.root(), &f.ledger, "test").unwrap();
    assert_eq!(status.active_entries, 1);
}

#[test]
fn alias_components_are_bidirectional_transitive_and_cycle_safe() {
    let f = Fixture::new();
    let a = f.project();
    let entry = f.record("one", "A", None).unwrap();
    let mut b_entry = entry.clone();
    b_entry.id = "b-id".into();
    b_entry.key = "two".into();
    f.insert("B", &b_entry);
    assert_eq!(f.ledger.read(&a).unwrap().entries.len(), 1);
    assert_eq!(f.ledger.read("B").unwrap().entries.len(), 1);
    f.alias(&a, "B");
    assert_eq!(f.ledger.read(&a).unwrap().entries.len(), 2);
    assert_eq!(f.ledger.read("B").unwrap().entries.len(), 2);
    let mut c_entry = entry.clone();
    c_entry.id = "c-id".into();
    c_entry.key = "three".into();
    f.insert("C", &c_entry);
    f.alias("B", "C");
    f.alias("B", &a);
    assert_eq!(f.ledger.read("C").unwrap().entries.len(), 3);
    let status = memory_store::status_with(f.root(), &f.ledger, "test").unwrap();
    assert_eq!(status.project_ids, vec![a, "B".into(), "C".into()]);
    assert_eq!((status.total_entries, status.active_entries), (3, 3));
    assert!(status.conflicts.is_empty());
}

#[test]
fn alias_add_guards_transitive_and_reverse_links() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let current = f.project();
    let entry = f.record("one", "A", None).unwrap();
    let add =
        |id: &str| memory_store::alias_add_with(f.root(), AliasFrom::Project(id.into()), &f.ledger);
    assert!(
        matches!(add(&current), Err(LedgerError::Message(message)) if message.contains("itself"))
    );
    assert!(
        matches!(add("empty"), Err(LedgerError::Message(message)) if message.contains("URL may be wrong") && message.contains("--from-project") && message.contains("cort memory status"))
    );
    f.insert("A", &entry);
    f.alias("A", "B");
    assert_eq!(add("B").unwrap().outcome, "linked");
    assert_eq!(add("B").unwrap().outcome, "already_linked");
    // A reaches the current id only through B: still linked, and no second row.
    assert_eq!(add("A").unwrap().outcome, "already_linked");
    let list = memory_store::alias_list_with(f.root(), &f.ledger).unwrap();
    assert_eq!(list.aliases.len(), 2);
    let mut expected = vec![(current.as_str(), 1), ("A", 1), ("B", 0)];
    expected.sort();
    assert_eq!(
        list.projects
            .iter()
            .map(|p| (p.project_id.as_str(), p.row_count))
            .collect::<Vec<_>>(),
        expected
    );
    let g = Fixture::new();
    let id = g.project();
    let row = g.record("one", "A", None).unwrap();
    g.insert("old", &row);
    g.alias(&id, "old");
    assert_eq!(
        memory_store::alias_add_with(g.root(), AliasFrom::Project("old".into()), &g.ledger)
            .unwrap()
            .outcome,
        "already_linked"
    );
    assert_eq!(
        memory_store::alias_list_with(g.root(), &g.ledger)
            .unwrap()
            .aliases
            .len(),
        1
    );
}

#[test]
fn alias_remote_normalization_and_write_outcomes() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let row = f.record("one", "A", None).unwrap();
    // Derive the remote id using the same origin path as production.
    let other = Fixture::new();
    assert!(Command::new("git")
        .current_dir(other.root())
        .args([
            "remote",
            "set-url",
            "origin",
            "https://github.com/example/old.git"
        ])
        .status()
        .unwrap()
        .success());
    let old = other.project();
    f.insert(&old, &row);
    f.arm(2, Action::ForwardThenTimeout);
    let linked = memory_store::alias_add_with(
        f.root(),
        AliasFrom::Remote("git@github.com:example/old.git".into()),
        &f.ledger,
    )
    .unwrap();
    assert_eq!(linked.outcome, "linked");
    assert!(linked.compatibility_note.is_some());
    assert_eq!(linked.old_project, old);
    let repeat = memory_store::alias_add_with(
        f.root(),
        AliasFrom::Remote("https://github.com/example/old.git".into()),
        &f.ledger,
    )
    .unwrap();
    assert_eq!(repeat.outcome, "already_linked");
    f.insert("another", &row);
    f.arm(2, Action::Fail(504));
    assert!(matches!(
        memory_store::alias_add_with(f.root(), AliasFrom::Project("another".into()), &f.ledger),
        Err(LedgerError::NotRecorded {
            kind: cort::remote::ErrorKind::Timeout
        })
    ));
}

#[test]
fn alias_add_race_with_reverse_insert_reports_already_linked() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let current = f.project();
    let row = f.record("one", "A", None).unwrap();
    f.insert("old", &row);
    f.arm(
        2,
        Action::Sql(format!(
            "INSERT INTO project_aliases VALUES ('{current}', 'old', 'now', 'test')"
        )),
    );
    let result =
        memory_store::alias_add_with(f.root(), AliasFrom::Project("old".into()), &f.ledger)
            .unwrap();
    assert_eq!(result.outcome, "already_linked");
    assert!(result.compatibility_note.is_none());
    let aliases = memory_store::alias_list_with(f.root(), &f.ledger)
        .unwrap()
        .aliases;
    assert_eq!(aliases.len(), 1);
    assert_eq!(
        (&aliases[0].old_project, &aliases[0].new_project),
        (&current, &"old".to_string())
    );
}

/// Two machines adding the same link: the second insert finds the row instead of hitting the
/// primary key, so it reports `already_linked` rather than `not_recorded`.
#[test]
fn alias_add_race_with_same_direction_insert_reports_already_linked() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let current = f.project();
    let row = f.record("one", "A", None).unwrap();
    f.insert("old", &row);
    f.arm(
        2,
        Action::Sql(format!(
            "INSERT INTO project_aliases VALUES ('old', '{current}', 'now', 'test')"
        )),
    );
    let result =
        memory_store::alias_add_with(f.root(), AliasFrom::Project("old".into()), &f.ledger)
            .unwrap();
    assert_eq!(result.outcome, "already_linked");
    assert_eq!(
        memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .len(),
        1
    );
}

#[test]
fn alias_add_timeout_settles_when_reverse_row_exists() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let current = f.project();
    let row = f.record("one", "A", None).unwrap();
    f.insert("old", &row);
    f.arm(
        2,
        Action::SqlThenFail(
            format!("INSERT INTO project_aliases VALUES ('{current}', 'old', 'now', 'test')"),
            504,
        ),
    );
    let result =
        memory_store::alias_add_with(f.root(), AliasFrom::Project("old".into()), &f.ledger)
            .unwrap();
    assert_eq!(result.outcome, "linked");
    assert_eq!(
        memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .len(),
        1
    );
}

#[test]
fn alias_remove_deletes_duplicate_directions() {
    use memory_store::AliasFrom;
    for timeout in [false, true] {
        let f = Fixture::new();
        let current = f.project();
        let row = f.record("one", "A", None).unwrap();
        let mut old_row = row.clone();
        old_row.id = "old-id".into();
        old_row.key = "old-key".into();
        f.insert("old", &old_row);
        f.alias("old", &current);
        f.alias(&current, "old");
        assert_eq!(f.ledger.read(&current).unwrap().entries.len(), 2);
        if timeout {
            f.arm(2, Action::ForwardThenTimeout);
        }
        let result =
            memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger)
                .unwrap();
        assert_eq!(result.outcome, "removed");
        assert!(memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .is_empty());
        let entries = f.ledger.read(&current).unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, row.id);
    }
}

/// Another machine changes one direction between the snapshot and the DELETE: the count no
/// longer matches, but the unlink still succeeded, so it is reported as `removed`.
#[test]
fn alias_remove_settles_a_concurrent_change_of_one_direction() {
    use memory_store::AliasFrom;
    for reverse_added in [false, true] {
        let f = Fixture::new();
        let current = f.project();
        let row = f.record("one", "A", None).unwrap();
        let mut old_row = row.clone();
        old_row.id = "old-id".into();
        old_row.key = "old-key".into();
        f.insert("old", &old_row);
        f.alias("old", &current);
        let race = if reverse_added {
            format!("INSERT INTO project_aliases VALUES ('{current}', 'old', 'now', 'test')")
        } else {
            f.alias(&current, "old");
            format!(
                "DELETE FROM project_aliases WHERE old_project = '{current}' AND new_project = 'old'"
            )
        };
        f.arm(2, Action::Sql(race));
        let result =
            memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger)
                .unwrap();
        assert_eq!(result.outcome, "removed", "reverse_added={reverse_added}");
        assert!(memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .is_empty());
    }
}

#[test]
fn alias_remove_both_directions_and_timeout() {
    use memory_store::AliasFrom;
    for reverse in [false, true] {
        let f = Fixture::new();
        let current = f.project();
        let row = f.record("one", "A", None).unwrap();
        f.insert("old", &row);
        if reverse {
            f.alias(&current, "old");
        } else {
            f.alias("old", &current);
        }
        f.arm(2, Action::ForwardThenTimeout);
        memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger)
            .unwrap();
        assert!(memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .is_empty());
        assert_eq!(f.ledger.read(&current).unwrap().entries.len(), 1);
        assert_eq!(f.ledger.read("old").unwrap().entries.len(), 1);
        assert!(
            matches!(memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger), Err(LedgerError::Message(message)) if message.contains("not linked"))
        );
    }
}

#[test]
fn alias_writes_report_unknown_and_uncommitted_remove() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let row = f.record("one", "A", None).unwrap();
    f.insert("old", &row);
    f.arm(2, Action::ForwardThenTimeout);
    f.arm(3, Action::Fail(503));
    assert!(matches!(
        memory_store::alias_add_with(f.root(), AliasFrom::Project("old".into()), &f.ledger),
        Err(LedgerError::OutcomeUnknown {
            kind: cort::remote::ErrorKind::Timeout,
            ..
        })
    ));
    f.arm(2, Action::Fail(504));
    assert!(matches!(
        memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger),
        Err(LedgerError::NotRecorded {
            kind: cort::remote::ErrorKind::Timeout
        })
    ));
    assert_eq!(
        memory_store::alias_list_with(f.root(), &f.ledger)
            .unwrap()
            .aliases
            .len(),
        1
    );
}

#[test]
fn alias_remove_allows_a_reference_stored_on_both_sides() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let current = f.project();
    let first = f.record("choice", "old", None).unwrap();
    f.insert("old", &first);
    f.alias("old", &current);
    f.record("choice", "new", Some(&first.id)).unwrap();
    // The superseded id stays under the current project, so splitting the component strands nothing.
    memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger).unwrap();
    assert_eq!(f.ledger.read(&current).unwrap().entries.len(), 2);
    assert_eq!(f.ledger.read("old").unwrap().entries.len(), 1);
}

#[test]
fn alias_remove_refuses_cross_component_supersede_and_resolve() {
    use memory_store::AliasFrom;
    for resolution in [false, true] {
        let f = Fixture::new();
        let current = f.project();
        let first = f.record("choice", "old", None).unwrap();
        f.insert("old", &first);
        f.sql(&format!(
            "DELETE FROM project_memory WHERE project = '{current}'"
        ));
        f.alias("old", &current);
        f.alias(&current, "old");
        if resolution {
            let mut second = first.clone();
            second.id = "second".into();
            second.claim = "other".into();
            f.insert(&current, &second);
            memory_store::resolve_with(
                f.root(),
                "choice",
                "resolved",
                Path::new("decision.md"),
                1,
                &[first.id.clone(), second.id],
                &f.ledger,
            )
            .unwrap();
        } else {
            f.record("choice", "new", Some(&first.id)).unwrap();
        }
        assert!(
            matches!(memory_store::alias_remove_with(f.root(), AliasFrom::Project("old".into()), &f.ledger), Err(LedgerError::Message(message)) if message.contains("alias_in_use") && message.contains(&first.id))
        );
        assert_eq!(
            memory_store::alias_list_with(f.root(), &f.ledger)
                .unwrap()
                .aliases
                .len(),
            2
        );
    }
}

#[test]
fn alias_repairs_missing_superseded_id_after_race() {
    use memory_store::AliasFrom;
    let f = Fixture::new();
    let current = f.project();
    let old = f.record("choice", "old", None).unwrap();
    f.insert("old", &old);
    f.sql(&format!(
        "DELETE FROM project_memory WHERE project = '{current}'"
    ));
    let mut newer = old.clone();
    newer.id = "newer".into();
    newer.claim = "new".into();
    newer.supersedes = Some(old.id.clone());
    f.insert(&current, &newer);
    assert!(
        matches!(f.ledger.read(&current), Err(LedgerError::Message(message)) if message.contains("supersedes missing id") && message.contains("cort memory alias add"))
    );
    memory_store::alias_add_with(f.root(), AliasFrom::Project("old".into()), &f.ledger).unwrap();
    assert_eq!(f.ledger.read(&current).unwrap().entries.len(), 2);
}

#[test]
fn supersedes_across_alias_requires_link() {
    let f = Fixture::new();
    let a = f.project();
    let entry = f.record("choice", "old", None).unwrap();
    f.insert("B", &entry);
    f.sql(&format!("DELETE FROM project_memory WHERE project = '{a}'"));
    assert!(matches!(
        f.record("choice", "new", Some(&entry.id)),
        Err(LedgerError::SupersedesMissing { .. })
    ));
    f.alias(&a, "B");
    let updated = f.record("choice", "new", Some(&entry.id[..8])).unwrap();
    assert_eq!(updated.supersedes.as_deref(), Some(entry.id.as_str()));
    let status = memory_store::status_with(f.root(), &f.ledger, "test").unwrap();
    assert_eq!((status.total_entries, status.active_entries), (2, 1));
}

#[test]
fn resolve_component_conflict_counts_distinct_ids() {
    let f = Fixture::new();
    let a = f.project();
    let x = f.record("choice", "X", None).unwrap();
    f.insert("B", &x);
    let mut y = x.clone();
    y.id = "other-id".into();
    y.claim = "Y".into();
    f.insert("B", &y);
    f.alias(&a, "B");
    let status = memory_store::status_with(f.root(), &f.ledger, "test").unwrap();
    assert_eq!((status.total_entries, status.active_entries), (2, 2));
    assert_eq!(status.conflicts.len(), 1);
    assert!(status.conflicts[0].active_ids.contains(&x.id));
    assert!(status.conflicts[0].active_ids.contains(&y.id));
    memory_store::resolve_with(
        f.root(),
        "choice",
        "resolved",
        Path::new("decision.md"),
        2,
        &[x.id, y.id],
        &f.ledger,
    )
    .unwrap();
    let status = memory_store::status_with(f.root(), &f.ledger, "test").unwrap();
    assert_eq!((status.total_entries, status.active_entries), (3, 1));
    assert!(status.conflicts.is_empty());
}

#[test]
fn duplicate_claims_merge_and_collisions_fail() {
    let f = Fixture::new();
    let a = f.project();
    let entry = f.record("choice", "X", None).unwrap();
    f.insert("B", &entry);
    f.alias(&a, "B");
    assert_eq!(f.ledger.read(&a).unwrap().entries.len(), 1);
    f.sql("DELETE FROM project_memory WHERE project = 'B'");
    let mut unstamped = entry.clone();
    unstamped.recorded_machine = None;
    unstamped.recorded_machine_source = None;
    f.insert("B", &unstamped);
    let merged = f.ledger.read(&a).unwrap();
    assert_eq!(merged.entries.len(), 1);
    assert_eq!(merged.entries[0].recorded_machine, entry.recorded_machine);
    f.sql("DELETE FROM project_memory WHERE project = 'B'");
    unstamped.claim = "different".into();
    f.insert("B", &unstamped);
    assert!(
        matches!(f.ledger.read(&a), Err(LedgerError::Message(message)) if message.contains("collision"))
    );
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
fn timeout_reread_ignores_an_id_visible_only_through_an_alias() {
    let f = Fixture::new();
    let project = f.project();
    // The first read creates the alias table.
    f.ledger.read(&project).unwrap();
    f.alias(&project, "renamed");
    f.arm(2, Action::ForwardThenTimeout);
    // Before the reread, move the committed row to the aliased project: the merged ledger
    // still shows the id, but nothing is stored under this project.
    f.arm(
        3,
        Action::Sql(format!(
            "UPDATE project_memory SET project = 'renamed' WHERE project = '{project}'"
        )),
    );
    assert!(matches!(
        f.record("choice", "A", None),
        Err(LedgerError::NotRecorded {
            kind: cort::remote::ErrorKind::Timeout
        })
    ));
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
