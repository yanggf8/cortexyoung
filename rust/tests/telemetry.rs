//! Event upload, restart boundary, and local watermark behavior.

use cort::measure_progress::{self, Load, Progress};
use cort::measure_store::{MeasureStore, SqliteStore};
use cort::telemetry::*;
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};

const V2_SCHEMA: &str = include_str!("../src/usage_schema.sql");

/// The pre-v2 shape: no machine columns, stamp lives only in `_usage_meta`.
const PRE_V2_SCHEMA: &str = "
CREATE TABLE _usage_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE command_log (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  project_id TEXT,
  command TEXT NOT NULL,
  args_summary TEXT NOT NULL,
  status TEXT NOT NULL,
  error_code TEXT,
  bytes_out INTEGER NOT NULL,
  saved_bytes INTEGER NOT NULL DEFAULT 0
);";

/// Rows exercising every conversion: NULL, integers, floats (incl. non-finite), BLOB, non-ASCII
/// text, a stamped v2 row, and defaulted `legacy_unknown` rows that take the machine from meta.
fn build_v2(path: &Path) {
    let c = Connection::open(path).unwrap();
    c.execute_batch(V2_SCHEMA).unwrap();
    c.execute_batch(
        "INSERT INTO _usage_meta VALUES ('MACHINE_ID','meta-machine'),('MACHINE_ID_SOURCE','env');",
    )
    .unwrap();
    // stamped v2 row, plain
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,error_code,read_source,requested_content_mode,effective_content_mode,receipt_hit,index_stale,bytes_out,saved_bytes)
         VALUES (1,1793116800000,'proj-1','mach-A','etc-machine-id','impact','--symbol foo','ok',NULL,NULL,NULL,NULL,1,0,1234,56)",
        [],
    )
    .unwrap();
    // NULLs, non-ASCII, quotes and control characters in text
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,error_code,bytes_out,saved_bytes)
         VALUES (2,1793116801000,NULL,'mach-B','env','read',?1,'error','not_found',0,0)",
        params!["caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \"q\" \\ \n\t<&>\u{1}"],
    )
    .unwrap();
    // float, huge float, non-finite float, and a BLOB in text columns
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,read_source,receipt_hit,index_stale,bytes_out,saved_bytes)
         VALUES (3,1793116802000,'proj-1','mach-A','etc-machine-id','impact','x','ok',?1,0.5,1e21,2.5,9e999)",
        params![vec![0xdeu8, 0xad, 0xbe, 0xef, 0x00]],
    )
    .unwrap();
    // defaulted stamp -> legacy: machine from _usage_meta, machine columns dropped from payload
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,command,args_summary,status,bytes_out,saved_bytes)
         VALUES (7,1793116803000,'proj-2','search','legacy-default','ok',10,3)",
        [],
    )
    .unwrap();
    // explicit legacy_unknown stamp, more columns populated (so the removal order matters)
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,error_code,read_source,requested_content_mode,effective_content_mode,receipt_hit,index_stale,bytes_out,saved_bytes)
         VALUES (9,1793116804000,'proj-2','legacy_unknown','legacy_no_stamp','read','legacy-explicit','ok','e','src','full','slice',1,1,77,5)",
        [],
    )
    .unwrap();
    // large rowid, float ts (time_ms must stay None)
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,bytes_out,saved_bytes)
         VALUES (9000000000,1793116805000.5,'proj-3','mach-A','etc-machine-id','x','y','ok',1,0)",
        [],
    )
    .unwrap();
}

fn build_pre_v2(path: &Path) {
    let c = Connection::open(path).unwrap();
    c.execute_batch(PRE_V2_SCHEMA).unwrap();
    c.execute_batch(
        "INSERT INTO _usage_meta VALUES ('MACHINE_ID','old-machine'),('MACHINE_ID_SOURCE','hostname');
         INSERT INTO command_log VALUES (1,1793116800000,'p-old','impact','--symbol bar','ok',NULL,321,0);
         INSERT INTO command_log VALUES (2,1793116801000,NULL,'read','\u{4e2d}\u{6587}','error','boom',0,12);",
    )
    .unwrap();
}

/// A key order deliberately not alphabetical, with nesting, floats, escapes and a repeated key.
const COST_FILE: &str = r#"{"zeta":1,"machine":{"source":"env","id":"cost-machine"},"alpha":[1.5,{"b":null,"a":"café 日"},true],"n":1e21,"zeta":2,"s":"line\nbreak \"q\""}"#;

fn build_cost(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("2026-10-28.json"), COST_FILE).unwrap();
    fs::write(dir.join("ignored.txt"), "not json").unwrap();
}

struct Fx {
    tmp: tempfile::TempDir,
}

impl Fx {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        for d in ["v2", "prev2"] {
            fs::create_dir_all(tmp.path().join(d)).unwrap();
        }
        build_v2(&tmp.path().join("v2/usage.db"));
        build_pre_v2(&tmp.path().join("prev2/usage.db"));
        build_cost(&tmp.path().join("cost"));
        Fx { tmp }
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }
    fn events(&self, fx: &str) -> Vec<TelemetryEvent> {
        match fx {
            "v2" | "prev2" => cort_events(&self.p(&format!("{fx}/usage.db")), None).unwrap(),
            "cost" => cost_events(&self.p("cost")).unwrap(),
            other => panic!("unknown fixture {other}"),
        }
    }
}

/// claudecat sorted keys; the table's column order is not what reaches the hash. Pinned both ways
/// so a "tidy-up" to column order (or a `preserve_order` feature flip) fails here.
#[test]
fn payload_keys_are_sorted_not_in_column_order() {
    let fx = Fx::new();
    let first = &fx.events("v2")[0];
    let keys: Vec<&str> = first
        .payload
        .trim_matches(|c| c == '{' || c == '}')
        .split(",\"")
        .map(|kv| kv.trim_start_matches('"').split('"').next().unwrap())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "keys must be alphabetical");
    assert!(
        first.payload.starts_with("{\"args_summary\":"),
        "{}",
        first.payload
    );
}

/// `SELECT rowid, *` produces no `rowid` key (SQLite names it `id`), so claudecat hashed rowid 0
/// for every command event. Real rowids give different ids; ported verbatim means 0.
#[test]
fn command_event_ids_hash_rowid_zero() {
    let fx = Fx::new();
    let e = &fx.events("v2")[0];
    assert_eq!(e.rowid, Some(1));
    assert_eq!(
        event_id("cort_command", "mach-A", 0, &e.payload),
        e.event_id
    );
    assert_ne!(
        event_id("cort_command", "mach-A", 1, &e.payload),
        e.event_id
    );
    assert!(!e.payload.contains("\"rowid\""));
}

#[test]
fn legacy_row_takes_machine_from_meta_and_drops_stamp() {
    let fx = Fx::new();
    let v2 = fx.events("v2");
    for (idx, rowid) in [(3usize, 7i64), (4, 9)] {
        let e = &v2[idx];
        assert_eq!(e.rowid, Some(rowid));
        assert_eq!(
            (e.machine_id.as_str(), e.machine_source.as_str()),
            ("meta-machine", "env")
        );
        assert!(!e.payload.contains("machine_id"), "{}", e.payload);
        assert!(!e.payload.contains("machine_source"), "{}", e.payload);
    }
    // A stamped row keeps its own machine and both columns.
    assert_eq!(v2[0].machine_id, "mach-A");
    assert!(v2[0].payload.contains("\"machine_id\":\"mach-A\""));
    // Pre-v2 table: no machine columns at all, stamp from _usage_meta.
    for e in fx.events("prev2") {
        assert_eq!(e.machine_id, "old-machine");
        assert_eq!(e.machine_source, "hostname");
    }
    // `after_rowid` is exclusive.
    let after = cort_events(&fx.p("v2/usage.db"), Some(7)).unwrap();
    assert_eq!(
        after.iter().filter_map(|e| e.rowid).collect::<Vec<_>>(),
        vec![9, 9_000_000_000]
    );
}

#[test]
fn cost_report_without_machine_stamp_is_an_error() {
    let fx = Fx::new();
    let dir = fx.p("badcost");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("2026-10-28.json"), r#"{"total":1}"#).unwrap();
    let err = cost_events(&dir).unwrap_err();
    assert!(err.contains("no machine stamp"), "{err}");
    assert!(cost_events(&fx.p("absent")).unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------
// dry run, watermark, gate
// ---------------------------------------------------------------------------------------------

fn store(fx: &Fx) -> SqliteStore {
    SqliteStore::open(&fx.p("store.db")).unwrap()
}

/// Events claudecat "already uploaded": one per machine of the v2 fixture.
fn seed_one_per_machine(s: &mut SqliteStore, fx: &Fx) {
    let v2 = fx.events("v2");
    // rows 1 (mach-A), 2 (mach-B), 7 (meta-machine)
    let seed = vec![v2[0].clone(), v2[1].clone(), v2[3].clone()];
    assert_eq!(s.insert_events(&seed).unwrap(), 3);
}

fn inputs(fx: &Fx) -> UploadInputs {
    UploadInputs {
        usage_db: fx.p("v2/usage.db"),
        cost_dir: fx.p("no-cost-here"),
        progress_path: fx.p("measure-progress.json"),
        machine_id: "mach-A".into(),
    }
}

fn held(s: &mut SqliteStore) -> usize {
    ["mach-A", "mach-B", "meta-machine"]
        .iter()
        .map(|m| s.remote_ids(m, "cort_command").unwrap().len())
        .sum()
}

#[test]
fn command_boundary_and_empty_remote_rebuild() {
    let fx = Fx::new();
    let db = fx.p("boundary.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(V2_SCHEMA).unwrap();
    for (id, ts) in [(1, MEASURE_START_MS - 1), (2, MEASURE_START_MS)] {
        conn.execute(
            "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,?2,'boundary','test','impact','x','ok',0)",
            params![id, ts],
        )
        .unwrap();
    }
    let input = UploadInputs {
        usage_db: db.clone(),
        cost_dir: fx.p("missing-cost"),
        progress_path: fx.p("boundary-progress.json"),
        machine_id: "boundary".into(),
    };
    assert_eq!(cort_events(&db, None).unwrap().len(), 1);
    assert_eq!(cort_events_page(&db, None, 1).unwrap()[0].rowid, Some(2));
    let mut remote = SqliteStore::open(&fx.p("empty-boundary-store.db")).unwrap();
    fs::write(&input.progress_path, "corrupt progress").unwrap();
    assert!(matches!(
        upload_events(&mut remote, &input),
        EventsReport::Uploaded { inserted: 1, .. }
    ));
    assert_eq!(remote.event_count("boundary", SOURCE_COMMAND).unwrap(), 1);
    fs::write(&input.progress_path, "corrupt progress").unwrap();
    let report = upload_events(&mut remote, &input);
    assert!(
        matches!(report, EventsReport::Uploaded { inserted: 0, .. }),
        "{report:?}"
    );
    assert_eq!(remote.event_count("boundary", SOURCE_COMMAND).unwrap(), 1);
}

#[test]
fn all_old_command_rows_advance_watermark_before_new_event() {
    let fx = Fx::new();
    let db = fx.p("old-only.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(V2_SCHEMA).unwrap();
    for id in 1..=3 {
        conn.execute(
            "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,?2,'boundary','test','impact','x','ok',0)",
            params![id, MEASURE_START_MS - 1],
        )
        .unwrap();
    }
    let input = UploadInputs {
        usage_db: db.clone(),
        cost_dir: fx.p("missing-cost"),
        progress_path: fx.p("old-progress.json"),
        machine_id: "boundary".into(),
    };
    let mut remote = SqliteStore::open(&fx.p("old-store.db")).unwrap();
    assert!(matches!(
        upload_events(&mut remote, &input),
        EventsReport::Uploaded {
            scanned: 0,
            inserted: 0,
            watermark: Some(3),
            ..
        }
    ));
    conn.execute(
        "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (4,?1,'boundary','test','impact','x','ok',0)",
        [MEASURE_START_MS],
    )
    .unwrap();
    assert!(matches!(
        upload_events(&mut remote, &input),
        EventsReport::Uploaded {
            inserted: 1,
            watermark: Some(4),
            ..
        }
    ));
    assert_eq!(remote.event_count("boundary", SOURCE_COMMAND).unwrap(), 1);
}

#[test]
fn missing_watermark_full_range_inserts_no_duplicates() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    let r = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        r,
        EventsReport::Uploaded {
            scanned: 6,
            inserted: 3,
            aliases: 0,
            full_range: true,
            reason: None,
            watermark: Some(9_000_000_000)
        }
    );
    assert_eq!(held(&mut s), 6);
    // The watermark was lost (a second, fully seeded store has none): the full range runs again
    // and the primary key keeps it duplicate-free.
    let mut again = SqliteStore::open(&fx.p("again.db")).unwrap();
    again.insert_events(&fx.events("v2")).unwrap();
    fs::remove_file(fx.p("measure-progress.json")).unwrap();
    let r2 = upload_events(&mut again, &inputs(&fx));
    assert!(
        matches!(
            r2,
            EventsReport::Uploaded {
                inserted: 0,
                full_range: true,
                ..
            }
        ),
        "{r2:?}"
    );
    assert_eq!(held(&mut again), 6);
}

#[test]
fn watermark_skips_already_uploaded() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    upload_events(&mut s, &inputs(&fx));
    assert_eq!(wm(&fx, "mach-A"), Some(9_000_000_000));
    // Nothing new: nothing read, nothing inserted.
    let idle = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        idle,
        EventsReport::Uploaded {
            scanned: 0,
            inserted: 0,
            aliases: 0,
            full_range: false,
            reason: None,
            watermark: Some(9_000_000_000)
        }
    );
    // A later row is the only one read.
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute(
            "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (9000000001,1793116800000,'mach-A','etc-machine-id','x','y','ok',1)",
            [],
        )
        .unwrap();
    let next = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        next,
        EventsReport::Uploaded {
            scanned: 1,
            inserted: 1,
            aliases: 0,
            full_range: false,
            reason: None,
            watermark: Some(9_000_000_001)
        }
    );
    assert_eq!(held(&mut s), 7);
}

#[test]
fn watermark_set_replaces_so_a_recreated_db_can_restart_lower() {
    let mut progress = Progress::default();
    progress.set_watermark("m", "cort_command", 10, "id10".into());
    progress.set_watermark("m", "cort_command", 5, "id5".into());
    assert_eq!(
        progress
            .watermark("m", "cort_command")
            .map(|w| (w.rowid, w.fingerprint.as_str())),
        Some((5, "id5"))
    );
    assert!(progress.watermark("m", "cost_daily").is_none());
}

#[test]
fn cost_events_upload_after_start() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    let mut with_cost = inputs(&fx);
    with_cost.cost_dir = fx.p("cost");
    let r = upload_events(&mut s, &with_cost);
    assert!(
        matches!(
            r,
            EventsReport::Uploaded {
                scanned: 7,
                inserted: 4,
                ..
            }
        ),
        "{r:?}"
    );
    // The cost file is read again every run; the primary key absorbs it.
    let r = upload_events(&mut s, &with_cost);
    assert!(
        matches!(
            r,
            EventsReport::Uploaded {
                scanned: 1,
                inserted: 0,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(
        s.remote_ids("cost-machine", "cost_daily").unwrap().len(),
        0,
        "no `id` key, no rowid pairing"
    );
}

#[test]
fn cost_day_spanning_start_is_not_uploaded() {
    let fx = Fx::new();
    fs::write(fx.p("cost/2026-10-27.json"), COST_FILE).unwrap();
    fs::write(fx.p("cost/undated.json"), COST_FILE).unwrap();
    let events = cost_events(&fx.p("cost")).unwrap();
    assert_eq!(events.len(), 1);
    let mut remote = store(&fx);
    let mut input = inputs(&fx);
    input.cost_dir = fx.p("cost");
    assert!(matches!(
        upload_events(&mut remote, &input),
        EventsReport::Uploaded { .. }
    ));
    assert_eq!(remote.event_count("cost-machine", SOURCE_COST).unwrap(), 1);
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// A repo directory plus its row in an index database next to `usage.db`; returns its project id.
fn indexed_repo(cache: &Path, dir: &Path, origin: Option<&str>) -> String {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q"]);
    if let Some(url) = origin {
        git(dir, &["remote", "add", "origin", url]);
    }
    let index = Connection::open(cache.join("abc.db")).unwrap();
    index
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (project_id TEXT PRIMARY KEY, name TEXT, path TEXT)",
        )
        .unwrap();
    let canon = fs::canonicalize(dir).unwrap();
    let id = cort::db::project_id_for(&canon.to_string_lossy());
    index
        .execute(
            "INSERT INTO projects VALUES (?1,'n',?2)",
            params![id, canon.to_string_lossy()],
        )
        .unwrap();
    id
}

fn log_row(db: &Path, id: i64, machine: &str, project: Option<&str>) {
    let c = Connection::open(db).unwrap();
    c.execute_batch(V2_SCHEMA).ok(); // idempotent: IF NOT EXISTS
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,1793116800000,?2,?3,'env','x','y','ok',1)",
        params![id, project, machine],
    )
    .unwrap();
}

#[test]
fn project_aliases_map_observed_pairs() {
    let fx = Fx::new();
    let cache = fx.p("cache");
    fs::create_dir_all(&cache).unwrap();
    let with = indexed_repo(
        &cache,
        &fx.p("with-origin"),
        Some("https://example.com/a/b.git"),
    );
    let without = indexed_repo(&cache, &fx.p("without-origin"), None);
    let db = cache.join("usage.db");
    log_row(&db, 1, "mach-A", Some(&with));
    log_row(&db, 2, "mach-A", Some(&without));
    log_row(&db, 3, "legacy_unknown", Some(&with)); // no _usage_meta: machine stays unknown
    log_row(&db, 4, "mach-A", Some("unindexed-project"));
    Connection::open(&db)
        .unwrap()
        .execute(
            "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (5,?1,?2,'old-only','env','x','y','ok',0)",
            params![MEASURE_START_MS - 1, &with],
        )
        .unwrap();
    let aliases = project_aliases(&db).unwrap();
    assert_eq!(aliases.len(), 1, "{aliases:?}");
    assert_eq!(
        (aliases[0].0.as_str(), aliases[0].1.as_str()),
        ("mach-A", with.as_str())
    );
    assert_eq!(aliases[0].2.len(), 64);
}

#[test]
fn alias_appears_on_a_later_run_after_its_events_were_watermarked() {
    let fx = Fx::new();
    let cache = fx.p("cache");
    fs::create_dir_all(&cache).unwrap();
    let repo = fx.p("late-repo");
    fs::create_dir_all(&repo).unwrap();
    let canon = fs::canonicalize(&repo).unwrap();
    let project = cort::db::project_id_for(&canon.to_string_lossy());
    let db = cache.join("usage.db");
    log_row(&db, 1, "mach-A", Some(&project));
    let mut s = store(&fx);
    s.insert_events(&cort_events(&db, None).unwrap()).unwrap(); // "already uploaded by claudecat"
    let up = UploadInputs {
        usage_db: db.clone(),
        cost_dir: fx.p("no-cost-here"),
        progress_path: fx.p("measure-progress.json"),
        machine_id: "mach-A".into(),
    };
    let first = upload_events(&mut s, &up);
    assert!(
        matches!(first, EventsReport::Uploaded { aliases: 0, .. }),
        "{first:?}"
    );
    // The index and the origin appear only now; no new events.
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &["remote", "add", "origin", "https://example.com/late.git"],
    );
    Connection::open(cache.join("late.db"))
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE projects (project_id TEXT PRIMARY KEY, name TEXT, path TEXT);
             INSERT INTO projects VALUES ('{project}','n','{}')",
            canon.to_string_lossy()
        ))
        .unwrap();
    let second = upload_events(&mut s, &up);
    assert!(
        matches!(
            second,
            EventsReport::Uploaded {
                scanned: 0,
                aliases: 1,
                full_range: false,
                ..
            }
        ),
        "{second:?}"
    );
    let n: i64 = Connection::open(fx.p("store.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM telemetry_project_aliases", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 1);
}

/// Not a test of ours alone: `usage_schema.sql` carries the warning, and this keeps it there.
#[test]
fn schema_file_warns_that_column_order_is_hashed() {
    let schema = include_str!("../src/usage_schema.sql");
    assert!(
        schema.contains("event_id"),
        "usage_schema.sql must say its columns feed event_id"
    );
}

/// `--help` is answered globally, so the restart and retention boundaries live in its usage text.
#[test]
fn help_states_the_retention_boundary() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cort"))
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("uploads start at 2026-10-28 00:00 Taiwan time"),
        "{text}"
    );
    assert!(
        !text.contains("measure <configure|show|record|collect|verify"),
        "{text}"
    );
    assert!(text.contains("rows not yet uploaded 180 days"), "{text}");
    assert!(text.contains("pruned_unuploaded_rows"), "{text}");
    assert!(!text.contains("record not running"), "{text}");
}

fn wm(fx: &Fx, machine: &str) -> Option<i64> {
    match measure_progress::load(&fx.p("measure-progress.json")) {
        Load::Loaded(progress) => progress.watermark(machine, "cort_command").map(|w| w.rowid),
        _ => None,
    }
}

/// Replace `usage.db` by a brand-new file holding rows `(id, command)` for machine mach-A.
fn replace_usage_db(fx: &Fx, rows: &[(i64, &str)]) {
    let path = fx.p("v2/usage.db");
    fs::remove_file(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute_batch(V2_SCHEMA).unwrap();
    for (id, cmd) in rows {
        c.execute(
            "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,1793116800000,'mach-A','etc-machine-id',?2,'y','ok',1)",
            params![id, cmd],
        )
        .unwrap();
    }
}

fn uploaded_s(fx: &Fx) -> SqliteStore {
    let mut s = store(fx);
    seed_one_per_machine(&mut s, fx);
    upload_events(&mut s, &inputs(fx));
    assert_eq!(wm(fx, "mach-A"), Some(9_000_000_000));
    s
}

#[test]
fn replaced_db_with_restarted_rowids_is_read_in_full() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    replace_usage_db(&fx, &[(1, "new-a"), (2, "new-b"), (3, "new-c")]);
    let r = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        r,
        EventsReport::Uploaded {
            scanned: 3,
            inserted: 3,
            aliases: 0,
            full_range: true,
            reason: Some("watermark_row_missing".into()),
            watermark: Some(3)
        }
    );
    assert_eq!(wm(&fx, "mach-A"), Some(3));
    assert_eq!(stored(&fx), 9);
}

#[test]
fn replaced_db_already_past_the_watermark_is_read_in_full() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    // The new file already has a row at the old watermark's rowid, and rows beyond it.
    replace_usage_db(
        &fx,
        &[
            (1, "n1"),
            (9_000_000_000, "different"),
            (9_000_000_005, "n3"),
        ],
    );
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(&r, EventsReport::Uploaded { scanned: 3, inserted: 3, full_range: true,
            reason: Some(why), watermark: Some(9_000_000_005), .. } if why == "watermark_row_changed"),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 9);
}

#[test]
fn pruned_watermark_row_falls_back_to_full_range_without_regating() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute("DELETE FROM command_log WHERE id = 9000000000", [])
        .unwrap();
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(&r, EventsReport::Uploaded { scanned: 5, inserted: 0, full_range: true,
            reason: Some(why), watermark: Some(9), .. } if why == "watermark_row_missing"),
        "{r:?}"
    );
    assert_eq!(held(&mut s), 6, "nothing duplicated, nothing lost");
}

#[test]
fn upload_pages_through_a_large_range() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    {
        let c = Connection::open(fx.p("v2/usage.db")).unwrap();
        c.execute_batch("BEGIN").unwrap();
        for i in 1..=4500i64 {
            c.execute(
                "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,1793116800000,'mach-A','etc-machine-id','bulk','y','ok',1)",
                params![10_000_000_000 + i],
            )
            .unwrap();
        }
        c.execute_batch("COMMIT").unwrap();
    }
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            r,
            EventsReport::Uploaded {
                scanned: 4506,
                inserted: 4503,
                watermark: Some(10_000_004_500),
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(wm(&fx, "mach-A"), Some(10_000_004_500));
    let floor: String = Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .query_row(
            "SELECT value FROM _usage_meta WHERE key='uploaded_command_rowid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(floor, "10000004500");
}

#[test]
fn remote_ids_skip_malformed_payloads() {
    let fx = Fx::new();
    let mut s = store(&fx);
    let mut junk = fx.events("v2")[0].clone();
    junk.payload = "not json".into();
    junk.event_id = "junk".into();
    let good = fx.events("v2")[2].clone();
    s.insert_events(&[junk, good.clone()]).unwrap();
    let ids = s.remote_ids("mach-A", "cort_command").unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[&3], good.event_id);
}

/// Stored events, counted in SQL: `held` keys by payload id, which collides across a replaced db.
fn stored(fx: &Fx) -> i64 {
    Connection::open(fx.p("store.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn schema_change_in_same_db_continues_after_watermark() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    assert_eq!(stored(&fx), 6);
    // The same database gains a column: every payload, hence every event id, now differs from what
    // was uploaded. That must not look like a replaced file.
    {
        let c = Connection::open(fx.p("v2/usage.db")).unwrap();
        c.execute_batch("ALTER TABLE command_log ADD COLUMN extra TEXT")
            .unwrap();
        for id in [9_000_000_010i64, 9_000_000_011] {
            c.execute(
                "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out,extra) VALUES (?1,1793116800000,'mach-A','etc-machine-id','new','y','ok',1,'e')",
                params![id],
            )
            .unwrap();
        }
    }
    let r = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        r,
        EventsReport::Uploaded {
            scanned: 2,
            inserted: 2,
            aliases: 0,
            full_range: false,
            reason: None,
            watermark: Some(9_000_000_011)
        }
    );
    assert_eq!(
        stored(&fx),
        8,
        "no earlier row was re-uploaded under a new id"
    );
}

#[test]
fn schema_change_then_pruned_watermark_resends_full_range() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    assert_eq!(stored(&fx), 6);
    {
        let c = Connection::open(fx.p("v2/usage.db")).unwrap();
        c.execute_batch("ALTER TABLE command_log ADD COLUMN extra TEXT")
            .unwrap();
        // The retention prune removes the watermark row; two new rows arrive.
        c.execute("DELETE FROM command_log WHERE id = 9000000000", [])
            .unwrap();
        for id in [9_000_000_010i64, 9_000_000_011] {
            c.execute(
                "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out,extra) VALUES (?1,1793116800000,'mach-A','etc-machine-id','new','y','ok',1,'e')",
                params![id],
            )
            .unwrap();
        }
    }
    let r = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        r,
        EventsReport::Uploaded {
            scanned: 7,
            inserted: 7,
            aliases: 0,
            full_range: true,
            reason: Some("watermark_row_missing".into()),
            watermark: Some(9_000_000_011)
        }
    );
    assert_eq!(stored(&fx), 13, "changed payloads have new ids");
}

#[test]
fn same_millisecond_same_command_distinct_rows_both_upload_in_full_range() {
    let fx = Fx::new();
    replace_usage_db(&fx, &[(1, "c"), (2, "c")]);
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch("UPDATE command_log SET command = 'same', args_summary = 'args' || id")
        .unwrap();
    let mut s = store(&fx);
    // Only the first of the two is on the remote already.
    let first = cort_events(&fx.p("v2/usage.db"), None).unwrap()[0].clone();
    assert_eq!(first.rowid, Some(1));
    s.insert_events(&[first]).unwrap();
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            r,
            EventsReport::Uploaded {
                scanned: 2,
                inserted: 1,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 2);
}

/// Make the watermark point at a rowid the database does not have, as the 90-day prune would.
fn lose_watermark_row(fx: &Fx) {
    let path = fx.p("measure-progress.json");
    let Load::Loaded(mut progress) = measure_progress::load(&path) else {
        panic!("missing progress")
    };
    progress.set_watermark("mach-A", "cort_command", 9_000_000_500, "gone".into());
    progress.save(&path).unwrap();
}

#[test]
fn float_ts_row_is_resent_after_column_add_and_pruned_watermark() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx); // includes the float-ts row (no time_ms), id 9000000000
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch("ALTER TABLE command_log ADD COLUMN extra TEXT")
        .unwrap();
    lose_watermark_row(&fx);
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            &r,
            EventsReport::Uploaded {
                scanned: 6,
                inserted: 6,
                full_range: true,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 12);
}

#[test]
fn schema_column_removed_then_pruned_watermark_resends_full_range() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    // The remote payloads carry `read_source`; the local table no longer has it.
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch("ALTER TABLE command_log DROP COLUMN read_source")
        .unwrap();
    let local = cort_events(&fx.p("v2/usage.db"), None).unwrap();
    assert!(local.iter().all(|e| !e.payload.contains("read_source")));
    lose_watermark_row(&fx);
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            &r,
            EventsReport::Uploaded {
                scanned: 6,
                inserted: 6,
                full_range: true,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 12);
}

#[test]
fn simultaneous_add_and_remove_is_the_documented_unmatched_case() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch(
            "ALTER TABLE command_log DROP COLUMN read_source;
             ALTER TABLE command_log ADD COLUMN extra TEXT",
        )
        .unwrap();
    lose_watermark_row(&fx);
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(&r, EventsReport::Uploaded { inserted: 6, .. }),
        "{r:?}"
    );
}
