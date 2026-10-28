use cort::telemetry::command_log_columns;
use cort::usage::{
    health_rows_after_at, insert_health_row_at, kick_counters_at, last_health_rows_at,
    record_command_at, touched_project_ids_at, CommandRecord, HealthRow, INSTANCE_KEY,
};
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

fn sample_health(local_project_id: &str, status: &str) -> HealthRow {
    HealthRow {
        id: -1,
        ts: NOW_MS,
        machine_id: "machine".into(),
        machine_source: "test".into(),
        root: Some("/project".into()),
        local_project_id: local_project_id.into(),
        project_id: Some("origin".into()),
        status: status.into(),
        error: Some("detail".into()),
        fresh: Some("STALE".into()),
        chunks: Some(3),
        relationships: Some(4),
        not_chunked: Some(5),
        real_gap: Some(6),
        fts: Some("drift=2".into()),
    }
}

#[test]
fn health_row_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let _conn = db(&path);
    let row = sample_health("local", "ok");
    let id = insert_health_row_at(&path, &row).unwrap();
    assert!(id > 0);
    assert_eq!(
        health_rows_after_at(&path, 0, 10, i64::MIN).unwrap(),
        vec![HealthRow { id, ..row }]
    );
}

#[test]
fn health_rows_after_pages_in_id_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let _conn = db(&path);
    let ids: Vec<i64> = (0..5)
        .map(|i| {
            let mut row = sample_health("local", "ok");
            row.ts += i;
            insert_health_row_at(&path, &row).unwrap()
        })
        .collect();
    let page = health_rows_after_at(&path, ids[1], 2, i64::MIN).unwrap();
    assert_eq!(page.iter().map(|row| row.id).collect::<Vec<_>>(), ids[2..4]);
    assert!(health_rows_after_at(&path, ids[4], 2, i64::MIN)
        .unwrap()
        .is_empty());
}

#[test]
fn last_health_rows_returns_latest_per_project() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let _conn = db(&path);
    let first = sample_health("first", "ok");
    insert_health_row_at(&path, &first).unwrap();
    let second = sample_health("second", "no_index");
    let second_id = insert_health_row_at(&path, &second).unwrap();
    let latest = sample_health("first", "unreadable");
    let latest_id = insert_health_row_at(&path, &latest).unwrap();
    let rows = last_health_rows_at(&path, "machine").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows["first"],
        HealthRow {
            id: latest_id,
            ..latest
        }
    );
    assert_eq!(
        rows["second"],
        HealthRow {
            id: second_id,
            ..second
        }
    );
}

#[test]
fn last_health_rows_ignore_other_machines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let _conn = db(&path);
    let mut older = sample_health("shared", "ok");
    older.machine_id = "machine-b".into();
    let older_id = insert_health_row_at(&path, &older).unwrap();
    let mut newer = sample_health("shared", "no_index");
    newer.machine_id = "machine-a".into();
    insert_health_row_at(&path, &newer).unwrap();
    let rows = last_health_rows_at(&path, "machine-b").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows["shared"],
        HealthRow {
            id: older_id,
            ..older
        }
    );
    assert!(last_health_rows_at(&path, "machine-c").unwrap().is_empty());
}

#[test]
fn touched_project_ids_skip_measure_null_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    for (command, project_id) in [
        ("measure", Some("ignored")),
        ("impact", None),
        ("impact", Some("_unknown")),
        ("impact", Some("alpha")),
        ("hook-refresh", Some("beta")),
        ("impact", Some("alpha")),
        ("measure", Some("ignored")),
        ("impact", Some("beta")),
        ("impact", None),
    ] {
        let mut rec = record(command, NOW_MS);
        rec.project_id = project_id.map(str::to_owned);
        record_command_at(&path, &rec);
    }
    let (mut ids, max_id) = touched_project_ids_at(&path, 0).unwrap();
    ids.sort();
    assert_eq!(ids, vec!["alpha", "beta"]);
    assert_eq!(max_id, 9);
    assert_eq!(
        touched_project_ids_at(&path, max_id).unwrap(),
        (vec![], max_id)
    );
}

/// The cursor row itself is excluded even when it names a project: `>=` would re-observe the last
/// project of every pass. Row 6 is `alpha`, so only `beta` (row 8) is new after it.
#[test]
fn touched_project_ids_exclude_the_cursor_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    for (command, project_id) in [
        ("measure", Some("ignored")),
        ("impact", None),
        ("impact", Some("_unknown")),
        ("impact", Some("alpha")),
        ("hook-refresh", Some("beta")),
        ("impact", Some("alpha")),
        ("measure", Some("ignored")),
        ("impact", Some("beta")),
        ("impact", None),
    ] {
        let mut rec = record(command, NOW_MS);
        rec.project_id = project_id.map(str::to_owned);
        record_command_at(&path, &rec);
    }
    assert_eq!(
        touched_project_ids_at(&path, 6).unwrap(),
        (vec!["beta".to_string()], 9)
    );
}

/// A first write through the health path creates `usage.db` private, like `record_command_at`:
/// it holds roots and the machine id. SQLite creates the file at open, before the chmod runs.
/// Under a umask of 077 the file is private anyway and this cannot catch a dropped chmod; the
/// umask is not set here because it is process-wide and the tests run in parallel.
#[cfg(unix)]
#[test]
fn health_insert_creates_a_private_database() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh").join("usage.db");
    insert_health_row_at(&path, &sample_health("local", "ok")).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode {mode:o}");
}

#[test]
fn invalid_status_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let _conn = db(&path);
    assert!(insert_health_row_at(&path, &sample_health("local", "bogus")).is_err());
    assert!(health_rows_after_at(&path, 0, 10, i64::MIN)
        .unwrap()
        .is_empty());
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
    assert_eq!(counters.max_any_command_id, 4);
    assert_eq!(counters.max_health_id, 1);
    assert_eq!(counters.instance_id.len(), 32);
}

/// The two maxima answer different questions and must be read separately: the filtered one says
/// how many genuine rows are pending, the unfiltered one says whether a cursor has outrun the
/// database. A cursor that only outran the measure rows is the ordinary post-batch state.
#[test]
fn kick_counters_read_max_over_all_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    for command in ["impact", "hook-suggest", "measure", "measure", "measure"] {
        record_command_at(&path, &record(command, NOW_MS));
    }
    let counters = kick_counters_at(&path).unwrap();
    assert_eq!(counters.max_command_id, 2);
    assert_eq!(counters.max_any_command_id, 5);
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
    cort::usage::uploaded_floor_at(&path, "cort_index_health", 1).unwrap();
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
