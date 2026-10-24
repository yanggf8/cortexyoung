use cort::telemetry::command_log_columns;
use cort::usage::{kick_counters_at, record_command_at, CommandRecord, INSTANCE_KEY};
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::{Duration, Instant};

const DAY_MS: i64 = 86_400_000;
const NOW_MS: i64 = 1_800_000_000_000;

fn record(command: &str, now_ms: i64) -> CommandRecord {
    CommandRecord {
        now_ms,
        project_id: None,
        command: command.into(),
        args_summary: "{}".into(),
        status: "ok".into(),
        error_code: None,
        read_source: None,
        requested_content_mode: None,
        effective_content_mode: None,
        receipt_hit: None,
        index_stale: None,
        bytes_out: 0,
        saved_bytes: 0,
    }
}

fn db(path: &Path) -> Connection {
    record_command_at(path, &record("impact", NOW_MS));
    Connection::open(path).unwrap()
}

fn health_row(conn: &Connection, ts: i64) {
    conn.execute(
        "INSERT INTO index_health_log
         (ts, machine_id, machine_source, local_project_id, status)
         VALUES (?1, 'machine', 'test', 'local', 'ok')",
        [ts],
    )
    .unwrap();
}

#[test]
fn schema_adds_health_table_and_partial_index() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db(&dir.path().join("usage.db"));
    for name in [
        "index_health_log",
        "idx_index_health_log_local",
        "idx_command_log_not_measure",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing {name}");
    }
}

#[test]
fn command_log_columns_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let _conn = db(&path);
    assert_eq!(
        command_log_columns(&path).unwrap(),
        "id,id,ts,project_id,machine_id,machine_source,command,args_summary,status,error_code,read_source,requested_content_mode,effective_content_mode,receipt_hit,index_stale,bytes_out,saved_bytes"
    );
}

#[test]
fn instance_id_is_stamped_once() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.db");
    let second = dir.path().join("second.db");
    let conn = db(&first);
    let id: String = conn
        .query_row(
            "SELECT value FROM _usage_meta WHERE key = ?1",
            [INSTANCE_KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(id.len(), 32);
    assert!(id.bytes().all(|c| c.is_ascii_hexdigit()));
    drop(conn);
    let conn = db(&first);
    let reopened: String = conn
        .query_row(
            "SELECT value FROM _usage_meta WHERE key = ?1",
            [INSTANCE_KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(id, reopened);
    let other = db(&second);
    let second_id: String = other
        .query_row(
            "SELECT value FROM _usage_meta WHERE key = ?1",
            [INSTANCE_KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(id, second_id);
}

#[test]
fn kick_counters_excludes_measure_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    for command in ["impact", "hook-refresh", "measure", "measure"] {
        record_command_at(&path, &record(command, NOW_MS));
    }
    let conn = Connection::open(&path).unwrap();
    health_row(&conn, NOW_MS);
    drop(conn);
    let counters = kick_counters_at(&path).unwrap();
    assert_eq!(counters.max_command_id, 2);
    assert_eq!(counters.max_health_id, 1);
    assert_eq!(counters.instance_id.len(), 32);
}

#[test]
fn kick_counters_none_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    assert!(kick_counters_at(&path).is_none());
    assert!(!path.exists());

    let conn = db(&path);
    conn.execute("DELETE FROM _usage_meta WHERE key = ?1", [INSTANCE_KEY])
        .unwrap();
    assert!(kick_counters_at(&path).is_none());
    conn.execute(
        "INSERT INTO _usage_meta (key, value) VALUES (?1, 'instance')",
        [INSTANCE_KEY],
    )
    .unwrap();
    conn.execute_batch("DROP INDEX idx_command_log_not_measure")
        .unwrap();
    assert!(kick_counters_at(&path).is_none());
    conn.execute_batch("DROP TABLE index_health_log").unwrap();
    assert!(kick_counters_at(&path).is_none());
}

#[test]
fn partial_index_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db(&dir.path().join("usage.db"));
    let plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT MAX(id) FROM command_log WHERE command <> 'measure'",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("idx_command_log_not_measure"), "{plan}");
}

#[test]
fn kick_counters_cost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let mut conn = db(&path);
    let tx = conn.transaction().unwrap();
    {
        let mut insert = tx
            .prepare("INSERT INTO command_log (ts, command, args_summary, status, bytes_out) VALUES (?1, ?2, '{}', 'ok', 0)")
            .unwrap();
        for i in 1..200_000 {
            let command = if i >= 150_000 { "measure" } else { "impact" };
            insert.execute(params![NOW_MS, command]).unwrap();
        }
    }
    tx.commit().unwrap();
    drop(conn);

    let mut samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let counters = kick_counters_at(&path).unwrap();
        assert_eq!(counters.max_command_id, 150_000);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let median = samples[50];
    println!(
        "kick_counters_cost median: {:.3} ms",
        median.as_secs_f64() * 1000.0
    );
    assert!(median < Duration::from_millis(20), "median {median:?}");
}

#[test]
fn health_rows_pruned_with_command_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let conn = db(&path);
    // `db` already pruned on NOW_MS's day, so the next prune runs one day later.
    let prune_at = NOW_MS + DAY_MS;
    health_row(&conn, prune_at - 91 * DAY_MS);
    health_row(&conn, prune_at - 89 * DAY_MS);
    drop(conn);
    record_command_at(&path, &record("impact", prune_at));
    let conn = Connection::open(&path).unwrap();
    let timestamps: Vec<i64> = conn
        .prepare("SELECT ts FROM index_health_log ORDER BY ts")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(timestamps, vec![prune_at - 89 * DAY_MS]);
}
