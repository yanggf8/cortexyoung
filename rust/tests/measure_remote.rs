use cort::audit_import::ImportedUsage;
use cort::measure_store::{CompactionRow, MeasureStore, RemoteStore, WindowCounts};
use cort::remote::{RemoteError, SqliteTransport, Transport};
use cort::telemetry::TelemetryEvent;
use std::sync::{Arc, Mutex};

struct RecordingTransport {
    inner: SqliteTransport,
    sql: Arc<Mutex<Vec<String>>>,
}

impl Transport for RecordingTransport {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        let value: serde_json::Value = serde_json::from_slice(body).expect("Hrana request");
        let mut seen = self.sql.lock().expect("SQL log");
        for request in value["requests"].as_array().expect("requests") {
            if let Some(sql) = request["stmt"]["sql"].as_str() {
                seen.push(sql.into());
            }
            if let Some(steps) = request["batch"]["steps"].as_array() {
                for step in steps {
                    if let Some(sql) = step["stmt"]["sql"].as_str() {
                        seen.push(sql.into());
                    }
                }
            }
        }
        drop(seen);
        self.inner.post(url, body, headers)
    }
}

fn store(schema: bool) -> (RemoteStore, Arc<Mutex<Vec<String>>>) {
    let sql = Arc::new(Mutex::new(Vec::new()));
    let transport = RecordingTransport {
        inner: SqliteTransport::new().unwrap(),
        sql: sql.clone(),
    };
    let store = RemoteStore::with_transport(
        "libsql://test.invalid",
        "test-token",
        Box::new(transport),
        schema,
    )
    .unwrap();
    (store, sql)
}

fn event(source: &str, id: &str, time_ms: Option<i64>) -> TelemetryEvent {
    TelemetryEvent {
        source: source.into(),
        event_id: id.into(),
        machine_id: "machine".into(),
        machine_source: "test".into(),
        project_id: None,
        time_ms,
        payload: format!("{{\"id\":{}}}", id.parse::<i64>().unwrap()),
        rowid: None,
    }
}

#[test]
fn remote_store_runs_shared_sql_through_hrana() {
    let (mut store, sql) = store(true);
    let first = CompactionRow {
        machine_id: "machine".into(),
        boundary_uuid: "boundary".into(),
        session_id: "session-1".into(),
        time_ms: 10,
        project_id: None,
        trigger: "auto".into(),
        pre_tokens: Some(7),
        post_tokens: None,
        pre: WindowCounts::default(),
        post: WindowCounts {
            tool_calls: 3,
            ..Default::default()
        },
        post_complete: true,
    };
    assert_eq!(
        store.compaction_session("machine", "boundary").unwrap(),
        None
    );
    store.upsert_compaction(&first).unwrap();
    let mut second = first.clone();
    second.session_id = "session-2".into();
    second.time_ms = 20;
    store.upsert_compaction(&second).unwrap();
    assert_eq!(
        store.compaction_session("machine", "boundary").unwrap(),
        Some("session-2".into())
    );
    assert_eq!(store.compactions_since(15).unwrap(), vec![second]);
    assert!(store.compactions_since(21).unwrap().is_empty());

    for seen in [500, 900, 700] {
        store.upsert_machine_host("machine", "host", seen).unwrap();
    }
    assert_eq!(store.machine_hosts().unwrap()[0].last_seen_ms, 900);

    let events = [
        event("a", "1", Some(1)),
        event("a", "2", Some(2)),
        event("b", "3", None),
    ];
    assert_eq!(store.insert_events(&events).unwrap(), 3);
    assert_eq!(store.insert_events(&events).unwrap(), 0);
    assert_eq!(store.remote_ids("machine", "a").unwrap().len(), 2);
    assert_eq!(
        store.remote_payload("machine", "a", "1").unwrap(),
        Some(events[0].payload.clone())
    );
    assert_eq!(
        store
            .remote_payloads_in_range("machine", "a", 2, 3)
            .unwrap(),
        vec![events[1].payload.clone()]
    );
    assert_eq!(
        store.remote_payloads_without_time("machine", "b").unwrap(),
        vec![events[2].payload.clone()]
    );
    assert_eq!(store.events_page(None, 2).unwrap(), events[..2]);
    assert_eq!(
        store
            .events_page(Some(("a".into(), "1".into())), 3)
            .unwrap(),
        events[1..]
    );
    assert_eq!(
        store
            .events_in_key_range(&("a".into(), "2".into()), &("b".into(), "3".into()))
            .unwrap(),
        events[1..]
    );
    assert_eq!(
        store.event_counts().unwrap(),
        vec![
            ("a".into(), "machine".into(), 2),
            ("b".into(), "machine".into(), 1)
        ]
    );
    assert!(sql
        .lock()
        .unwrap()
        .iter()
        .any(|stmt| stmt == "BEGIN TRANSACTION"));

    assert_eq!(store.event_watermark("machine", "a").unwrap(), None);
    store
        .set_event_watermark("machine", "a", 10, "fingerprint-10")
        .unwrap();
    store
        .set_event_watermark("machine", "a", 4, "fingerprint-4")
        .unwrap();
    assert_eq!(
        store.event_watermark("machine", "a").unwrap(),
        Some((4, "fingerprint-4".into()))
    );

    let alias = ("machine".into(), "local".into(), "shared".into());
    store
        .upsert_aliases(&[alias.clone(), alias.clone()])
        .unwrap();
    assert_eq!(store.aliases().unwrap(), vec![alias]);

    let usage = ImportedUsage {
        day: "2026-10-27".into(),
        host: "host".into(),
        commands_window: Some(2),
        core_window: None,
        deep_window: None,
        commands_7d: None,
        deep_7d: None,
        decline_top: None,
        window_days: 30,
    };
    assert_eq!(
        store
            .upsert_imported_usage(std::slice::from_ref(&usage))
            .unwrap()
            .inserted,
        1
    );
    assert_eq!(
        store
            .upsert_imported_usage(std::slice::from_ref(&usage))
            .unwrap()
            .unchanged,
        1
    );
    let mut changed = usage.clone();
    changed.commands_window = Some(3);
    assert_eq!(
        store
            .upsert_imported_usage(&[changed])
            .unwrap()
            .conflicts
            .len(),
        1
    );
    assert_eq!(store.imported_usage().unwrap(), vec![usage]);
}

#[test]
fn read_only_constructor_issues_no_ddl() {
    let (_store, sql) = store(false);
    assert!(sql.lock().unwrap().is_empty());
}

#[test]
fn timeout_error_keeps_network_prefix_and_hides_token() {
    let transport = SqliteTransport::new().unwrap();
    transport.fail_next_status(408);
    let result = RemoteStore::with_transport(
        "libsql://test.invalid",
        "secret-test-token",
        Box::new(transport),
        true,
    );
    let error = match result {
        Ok(_) => panic!("expected timeout"),
        Err(error) => error,
    };
    assert!(error.starts_with("network timeout after "), "{error}");
    assert!(error.contains("timeout"));
    assert!(!error.contains("secret-test-token"));
}
