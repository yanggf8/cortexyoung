use cort::measure::copy_events;
use cort::measure_store::{MeasureStore, RemoteStore, SqliteStore};
use cort::remote::SqliteTransport;
use cort::telemetry::TelemetryEvent;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn open(dir: &tempfile::TempDir, name: &str) -> SqliteStore {
    SqliteStore::open(&dir.path().join(name)).unwrap()
}

fn ev(source: &str, id: &str, machine: &str, n: i64) -> TelemetryEvent {
    TelemetryEvent {
        source: source.into(),
        event_id: id.into(),
        machine_id: machine.into(),
        machine_source: "env".into(),
        project_id: Some(format!("proj-{n}")),
        time_ms: Some(1_000 + n),
        payload: format!("{{\"id\":{n}}}"),
        rowid: Some(n),
    }
}

fn as_copied(e: &TelemetryEvent) -> TelemetryEvent {
    TelemetryEvent {
        rowid: None,
        ..e.clone()
    }
}

/// Every event in a store, in primary-key order.
fn all(s: &mut SqliteStore) -> Vec<TelemetryEvent> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = s.events_page(after.clone(), 7).unwrap();
        if page.is_empty() {
            return out;
        }
        let last = page.last().unwrap();
        after = Some((last.source.clone(), last.event_id.clone()));
        out.extend(page);
    }
}

fn seed(s: &mut SqliteStore) -> Vec<TelemetryEvent> {
    let mut events = vec![
        ev("claude", "a1", "m1", 1),
        ev("claude", "a2", "m1", 2),
        ev("claude", "b1", "m2", 3),
        ev("cort_command", "c1", "m1", 4),
        ev("cort_command", "c2", "m2", 5),
        ev("cort_command", "c3", "m2", 6),
    ];
    events[2].project_id = None;
    events[3].time_ms = None;
    events[4].payload = "{\"cmd\":\"查詢 – ünïcode 🙂\"}".into();
    s.insert_events(&events).unwrap();
    events
}

#[test]
fn copy_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    seed(&mut from);
    let first = copy_events(&mut from, &mut to).unwrap().groups;
    let before = all(&mut to);
    let second = copy_events(&mut from, &mut to).unwrap().groups;
    assert_eq!(first, second);
    assert_eq!(all(&mut to), before);
    assert_eq!(before.len(), 6);
}

#[test]
fn copy_reports_counts_by_source_and_machine() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    seed(&mut from);
    // The destination already holds one of m2's cort_command rows plus an unrelated machine.
    to.insert_events(&[
        ev("cort_command", "c2", "m2", 5),
        ev("cort_command", "z1", "m9", 9),
    ])
    .unwrap();
    let mut report = copy_events(&mut from, &mut to).unwrap().groups;
    report.sort();
    let s = |a: &str, b: &str, x, y| (a.to_string(), b.to_string(), x, y);
    assert_eq!(
        report,
        vec![
            s("claude", "m1", 2, 2),
            s("claude", "m2", 1, 1),
            s("cort_command", "m1", 1, 1),
            s("cort_command", "m2", 2, 2),
        ]
    );
}

#[test]
fn copy_keeps_event_ids_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    let seeded = seed(&mut from);
    copy_events(&mut from, &mut to).unwrap();
    let mut expected: Vec<_> = seeded.iter().map(as_copied).collect();
    expected.sort_by(|a, b| (&a.source, &a.event_id).cmp(&(&b.source, &b.event_id)));
    let got = all(&mut to);
    assert_eq!(got, expected);
    assert!(got.iter().any(|e| e.project_id.is_none()));
    assert!(got.iter().any(|e| e.time_ms.is_none()));
    assert!(got.iter().all(|e| e.rowid.is_none()));
}

#[test]
fn copy_never_writes_to_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("from.db");
    let mut from = SqliteStore::open(&path).unwrap();
    seed(&mut from);
    from.upsert_aliases(&[("m1".into(), "l".into(), "s".into())])
        .unwrap();
    let mut to = open(&dir, "to.db");
    let before = fs::read(&path).unwrap();
    copy_events(&mut from, &mut to).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn copy_pages_across_keyset_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    // 4_500 rows: three pages of 2_000, with ids that sort differently as text and as numbers.
    let events: Vec<_> = (0..4_500)
        .map(|n| {
            ev(
                if n % 3 == 0 { "a" } else { "b" },
                &format!("id{n}"),
                "m",
                n,
            )
        })
        .collect();
    from.insert_events(&events).unwrap();
    let report = copy_events(&mut from, &mut to).unwrap().groups;
    assert_eq!(
        report,
        vec![
            ("a".to_string(), "m".to_string(), 1_500, 1_500),
            ("b".to_string(), "m".to_string(), 3_000, 3_000)
        ]
    );
    let mut expected: Vec<_> = events.iter().map(as_copied).collect();
    expected.sort_by(|a, b| (&a.source, &a.event_id).cmp(&(&b.source, &b.event_id)));
    assert_eq!(all(&mut to), expected);
}

#[test]
fn events_page_resumes_strictly_after_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = open(&dir, "s.db");
    seed(&mut s);
    let p = s
        .events_page(Some(("claude".into(), "a2".into())), 2)
        .unwrap();
    let keys: Vec<_> = p
        .iter()
        .map(|e| (e.source.as_str(), e.event_id.as_str()))
        .collect();
    assert_eq!(keys, vec![("claude", "b1"), ("cort_command", "c1")]);
}

#[test]
fn copy_copies_aliases_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    seed(&mut from);
    let rows = vec![
        (
            "m1".to_string(),
            "local-1".to_string(),
            "shared-1".to_string(),
        ),
        (
            "m1".to_string(),
            "local-2".to_string(),
            "shared-1".to_string(),
        ),
        ("m2".to_string(), "本地".to_string(), "shared-2".to_string()),
    ];
    from.upsert_aliases(&rows).unwrap();
    to.upsert_aliases(&[("m9".into(), "keep".into(), "mine".into())])
        .unwrap();
    copy_events(&mut from, &mut to).unwrap();
    copy_events(&mut from, &mut to).unwrap();
    let mut got = to.aliases().unwrap();
    got.sort();
    let mut want = rows;
    want.push(("m9".into(), "keep".into(), "mine".into()));
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn failed_remote_source_read_is_tagged_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let transport = std::sync::Arc::new(SqliteTransport::new().unwrap());
    let mut from = RemoteStore::with_transport(
        "libsql://test.invalid",
        "test-token",
        Box::new(Shared(transport.clone())),
        true,
    )
    .unwrap();
    let mut to = open(&dir, "to.db");
    transport.fail_next_code("BLOCKED");
    let err = copy_events(&mut from, &mut to).unwrap_err();
    assert!(err.starts_with("remote read: "), "{err}");
    assert_eq!(cort::measure_store::remote_error_kind(&err), "blocked");
    assert!(all(&mut to).is_empty());
}

struct Shared(std::sync::Arc<SqliteTransport>);

impl cort::remote::Transport for Shared {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, cort::remote::RemoteError> {
        self.0.post(url, body, headers)
    }
}

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

#[test]
fn copy_events_without_config_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let home = tmp.path().join("home");
    let cache = tmp.path().join("cache");
    for d in [&data, &home, &cache] {
        fs::create_dir_all(d).unwrap();
    }
    let out = Command::new(cort_bin())
        .args(["measure", "copy-events"])
        .env("CORT_MEMORY_DIR", &data)
        .env("HOME", &home)
        .env("CORT_CACHE_DIR", &cache)
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("measure_copy_failed"), "{stdout}");
    assert!(stdout.contains("not configured"), "{stdout}");
    let left: Vec<_> = fs::read_dir(&data)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "copy-events left behind {left:?}");
}

#[test]
fn identical_preexisting_row_is_not_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    let seeded = seed(&mut from);
    to.insert_events(&[seeded[0].clone()]).unwrap();
    let report = copy_events(&mut from, &mut to).unwrap();
    assert_eq!(report.conflicts, 0);
    assert!(report.conflict_examples.is_empty());
}

#[test]
fn same_key_with_different_columns_is_a_conflict_and_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    let seeded = seed(&mut from);
    let mut other = seeded[1].clone();
    other.project_id = Some("someone-else".into());
    other.payload = "{\"id\":99}".into();
    to.insert_events(&[other.clone()]).unwrap();
    let report = copy_events(&mut from, &mut to).unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(
        report.conflict_examples,
        vec![(
            "claude".to_string(),
            "a2".to_string(),
            vec!["project_id", "payload"]
        )]
    );
    let kept = to
        .events_in_key_range(
            &("claude".into(), "a2".into()),
            &("claude".into(), "a2".into()),
        )
        .unwrap();
    assert_eq!(kept, vec![as_copied(&other)]);
    // Counts alone would have looked fine.
    assert_eq!(report.groups.iter().map(|g| g.2).sum::<usize>(), 6);
    assert_eq!(report.groups.iter().map(|g| g.3).sum::<usize>(), 6);
}

fn cort(
    data: &std::path::Path,
    home: &std::path::Path,
    args: &[&str],
    stdin: &str,
) -> std::process::Output {
    let mut child = Command::new(cort_bin())
        .args(args)
        .env("CORT_MEMORY_DIR", data)
        .env("HOME", home)
        .env("CORT_CACHE_DIR", home.join("cache"))
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let home = tmp.path().join("home");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(home.join("cache")).unwrap();
    (tmp, data, home)
}

/// Every path under `data`, recursively, so a replica or lock nested in an existing directory counts.
fn files_left(data: &std::path::Path) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(data, data, &mut out);
    out.sort();
    out
}

#[test]
fn copy_events_refuses_when_both_configs_name_one_database() {
    let (_t, data, home) = dirs();
    let m = cort(
        &data,
        &home,
        &[
            "memory",
            "configure",
            "--url",
            "libsql://x.turso.io",
            "--token-stdin",
        ],
        "tok",
    );
    assert!(m.status.success(), "{m:?}");
    let m = cort(
        &data,
        &home,
        &[
            "measure",
            "configure",
            "--url",
            "turso://X.turso.io/",
            "--token-stdin",
        ],
        "tok",
    );
    assert!(m.status.success(), "{m:?}");
    let before = files_left(&data);
    let out = cort(&data, &home, &["measure", "copy-events"], "");
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("measure_copy_failed"), "{stdout}");
    assert!(stdout.contains("same database"), "{stdout}");
    assert_eq!(files_left(&data), before, "no replica or lock may appear");
}

#[test]
fn copy_events_with_memory_configured_but_measure_not_fails_cleanly() {
    let (_t, data, home) = dirs();
    let m = cort(
        &data,
        &home,
        &[
            "memory",
            "configure",
            "--url",
            "libsql://x.turso.io",
            "--token-stdin",
        ],
        "tok",
    );
    assert!(m.status.success(), "{m:?}");
    let before = files_left(&data);
    let out = cort(&data, &home, &["measure", "copy-events"], "");
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("measure_copy_failed"), "{stdout}");
    assert!(stdout.contains("cort measure configure"), "{stdout}");
    assert_eq!(files_left(&data), before, "no replica or lock may appear");
}

#[test]
fn row_the_destination_silently_drops_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to_store) = (open(&dir, "from.db"), open(&dir, "to.db"));
    seed(&mut from);
    {
        let raw = rusqlite::Connection::open(dir.path().join("to.db")).unwrap();
        raw.execute_batch(
            "CREATE TRIGGER drop_one BEFORE INSERT ON telemetry_events \
             WHEN NEW.event_id = 'b1' BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    }
    let err = copy_events(&mut from, &mut to_store).unwrap_err();
    assert!(err.contains("b1") && err.contains("1 event"), "{err}");
}

#[test]
fn range_lookup_spans_two_sources() {
    let dir = tempfile::tempdir().unwrap();
    let (mut from, mut to) = (open(&dir, "from.db"), open(&dir, "to.db"));
    // A page that starts in source "a" and ends in "b"; the destination holds a different row
    // in each and extra rows inside the range.
    let a = ev("a", "z9", "m", 1);
    let b = ev("b", "a1", "m", 2);
    from.insert_events(&[a.clone(), b.clone()]).unwrap();
    let mut a_other = a.clone();
    a_other.payload = "{\"x\":1}".into();
    to.insert_events(&[a_other, ev("a", "zz", "m", 3), ev("b", "a0", "m", 4)])
        .unwrap();
    let range = to
        .events_in_key_range(&("a".into(), "z9".into()), &("b".into(), "a1".into()))
        .unwrap();
    let keys: Vec<_> = range
        .iter()
        .map(|e| (e.source.as_str(), e.event_id.as_str()))
        .collect();
    assert_eq!(keys, vec![("a", "z9"), ("a", "zz"), ("b", "a0")]);
    let report = copy_events(&mut from, &mut to).unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.conflict_examples[0].1, "z9");
}
