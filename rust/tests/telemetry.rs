//! Event sync: ids must equal the ones claudecat already uploaded, byte for byte.
//!
//! The fixtures below are built here, written to disk, and were fed to `claudecat::telemetry::scan`
//! to obtain the golden (payload, id) pairs asserted further down. See `dump_fixtures`.

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
         VALUES (1,1760000000000,'proj-1','mach-A','etc-machine-id','impact','--symbol foo','ok',NULL,NULL,NULL,NULL,1,0,1234,56)",
        [],
    )
    .unwrap();
    // NULLs, non-ASCII, quotes and control characters in text
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,error_code,bytes_out,saved_bytes)
         VALUES (2,1760000001000,NULL,'mach-B','env','read',?1,'error','not_found',0,0)",
        params!["caf\u{e9} \u{65e5}\u{672c}\u{8a9e} \"q\" \\ \n\t<&>\u{1}"],
    )
    .unwrap();
    // float, huge float, non-finite float, and a BLOB in text columns
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,read_source,receipt_hit,index_stale,bytes_out,saved_bytes)
         VALUES (3,1760000002000,'proj-1','mach-A','etc-machine-id','impact','x','ok',?1,0.5,1e21,2.5,9e999)",
        params![vec![0xdeu8, 0xad, 0xbe, 0xef, 0x00]],
    )
    .unwrap();
    // defaulted stamp -> legacy: machine from _usage_meta, machine columns dropped from payload
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,command,args_summary,status,bytes_out,saved_bytes)
         VALUES (7,1760000003000,'proj-2','search','legacy-default','ok',10,3)",
        [],
    )
    .unwrap();
    // explicit legacy_unknown stamp, more columns populated (so the removal order matters)
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,error_code,read_source,requested_content_mode,effective_content_mode,receipt_hit,index_stale,bytes_out,saved_bytes)
         VALUES (9,1760000004000,'proj-2','legacy_unknown','legacy_no_stamp','read','legacy-explicit','ok','e','src','full','slice',1,1,77,5)",
        [],
    )
    .unwrap();
    // large rowid, float ts (time_ms must stay None)
    c.execute(
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,bytes_out,saved_bytes)
         VALUES (9000000000,1760000005000.5,'proj-3','mach-A','etc-machine-id','x','y','ok',1,0)",
        [],
    )
    .unwrap();
}

fn build_pre_v2(path: &Path) {
    let c = Connection::open(path).unwrap();
    c.execute_batch(PRE_V2_SCHEMA).unwrap();
    c.execute_batch(
        "INSERT INTO _usage_meta VALUES ('MACHINE_ID','old-machine'),('MACHINE_ID_SOURCE','hostname');
         INSERT INTO command_log VALUES (1,1750000000000,'p-old','impact','--symbol bar','ok',NULL,321,0);
         INSERT INTO command_log VALUES (2,1750000001000,NULL,'read','\u{4e2d}\u{6587}','error','boom',0,12);",
    )
    .unwrap();
}

/// A key order deliberately not alphabetical, with nesting, floats, escapes and a repeated key.
const COST_FILE: &str = r#"{"zeta":1,"machine":{"source":"env","id":"cost-machine"},"alpha":[1.5,{"b":null,"a":"café 日"},true],"n":1e21,"zeta":2,"s":"line\nbreak \"q\""}"#;

fn build_cost(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("2026-10-22.json"), COST_FILE).unwrap();
    fs::write(dir.join("ignored.txt"), "not json").unwrap();
}

/// Not a test: writes the fixtures to `$CORT_FIXTURE_OUT` (v2/usage.db, prev2/usage.db, cost/).
/// Golden values below were produced by running `claudecat::telemetry::scan` on these exact
/// files, never by reading claudecat's source. Run:
/// `CORT_FIXTURE_OUT=<dir> cargo test --test telemetry dump_fixtures -- --ignored`
#[test]
#[ignore]
fn dump_fixtures() {
    let out = PathBuf::from(std::env::var("CORT_FIXTURE_OUT").unwrap());
    for d in ["v2", "prev2"] {
        fs::create_dir_all(out.join(d)).unwrap();
    }
    build_v2(&out.join("v2/usage.db"));
    build_pre_v2(&out.join("prev2/usage.db"));
    build_cost(&out.join("cost"));
}

// ---------------------------------------------------------------------------------------------
// Golden values: what `claudecat::telemetry::scan` printed for the fixtures above.
// Produced 2026-10-23 with a throwaway test in a local claudecat checkout (archived, never
// committed) that called `claudecat::telemetry::scan(Some(<fx>/usage.db), Some(<absent>),
// Some(<cost dir or absent>))` and printed `payload` and `id` per event.
// ---------------------------------------------------------------------------------------------

struct Golden {
    fx: &'static str,
    id: &'static str,
    machine_id: &'static str,
    machine_source: &'static str,
    project_id: Option<&'static str>,
    time_ms: Option<i64>,
    payload: &'static str,
}

const GOLDEN: &[Golden] = &[
    Golden {
        fx: "v2",
        id: "0d8f34901eec05f73e6339f0034413ea363f6b2d8294f9a51d2993615ca584d0",
        machine_id: "mach-A",
        machine_source: "etc-machine-id",
        project_id: Some("proj-1"),
        time_ms: Some(1760000000000),
        payload: r####"{"args_summary":"--symbol foo","bytes_out":1234,"command":"impact","effective_content_mode":null,"error_code":null,"id":1,"index_stale":0,"machine_id":"mach-A","machine_source":"etc-machine-id","project_id":"proj-1","read_source":null,"receipt_hit":1,"requested_content_mode":null,"saved_bytes":56,"status":"ok","ts":1760000000000}"####,
    },
    Golden {
        fx: "v2",
        id: "5579ee2b2b4de0f01e2432e4218f6d95ddb508107869740406fd6b6142e77af8",
        machine_id: "mach-B",
        machine_source: "env",
        project_id: None,
        time_ms: Some(1760000001000),
        payload: r####"{"args_summary":"café 日本語 \"q\" \\ \n\t<&>\u0001","bytes_out":0,"command":"read","effective_content_mode":null,"error_code":"not_found","id":2,"index_stale":null,"machine_id":"mach-B","machine_source":"env","project_id":null,"read_source":null,"receipt_hit":null,"requested_content_mode":null,"saved_bytes":0,"status":"error","ts":1760000001000}"####,
    },
    Golden {
        fx: "v2",
        id: "5465a20f2d39cb28eedb2e67c9490b3752664e47e946c066a5d8be85a21e6043",
        machine_id: "mach-A",
        machine_source: "etc-machine-id",
        project_id: Some("proj-1"),
        time_ms: Some(1760000002000),
        payload: r####"{"args_summary":"x","bytes_out":2.5,"command":"impact","effective_content_mode":null,"error_code":null,"id":3,"index_stale":1e+21,"machine_id":"mach-A","machine_source":"etc-machine-id","project_id":"proj-1","read_source":"blob:deadbeef00","receipt_hit":0.5,"requested_content_mode":null,"saved_bytes":null,"status":"ok","ts":1760000002000}"####,
    },
    Golden {
        fx: "v2",
        id: "da4d2d890257e5aa24c4fd36b9e4396ac327ea1cbfc2a47c8ecc377aa1ffba67",
        machine_id: "meta-machine",
        machine_source: "env",
        project_id: Some("proj-2"),
        time_ms: Some(1760000003000),
        payload: r####"{"args_summary":"legacy-default","bytes_out":10,"command":"search","effective_content_mode":null,"error_code":null,"id":7,"index_stale":null,"project_id":"proj-2","read_source":null,"receipt_hit":null,"requested_content_mode":null,"saved_bytes":3,"status":"ok","ts":1760000003000}"####,
    },
    Golden {
        fx: "v2",
        id: "2090e6398314490f415999a8dcf5eaa0d4db9d2ee1aac8f3bf566d030579857c",
        machine_id: "meta-machine",
        machine_source: "env",
        project_id: Some("proj-2"),
        time_ms: Some(1760000004000),
        payload: r####"{"args_summary":"legacy-explicit","bytes_out":77,"command":"read","effective_content_mode":"slice","error_code":"e","id":9,"index_stale":1,"project_id":"proj-2","read_source":"src","receipt_hit":1,"requested_content_mode":"full","saved_bytes":5,"status":"ok","ts":1760000004000}"####,
    },
    Golden {
        fx: "v2",
        id: "7556a3cca579374af28c6a40034965d982c8fe423abadbbe96d3d4f5e85dff44",
        machine_id: "mach-A",
        machine_source: "etc-machine-id",
        project_id: Some("proj-3"),
        time_ms: None,
        payload: r####"{"args_summary":"y","bytes_out":1,"command":"x","effective_content_mode":null,"error_code":null,"id":9000000000,"index_stale":null,"machine_id":"mach-A","machine_source":"etc-machine-id","project_id":"proj-3","read_source":null,"receipt_hit":null,"requested_content_mode":null,"saved_bytes":0,"status":"ok","ts":1760000005000.5}"####,
    },
    Golden {
        fx: "prev2",
        id: "5d6e04bd44090863fe5eaf2122112ddc5b1c620d0cbc40959b1dd84446921bbc",
        machine_id: "old-machine",
        machine_source: "hostname",
        project_id: Some("p-old"),
        time_ms: Some(1750000000000),
        payload: r####"{"args_summary":"--symbol bar","bytes_out":321,"command":"impact","error_code":null,"id":1,"project_id":"p-old","saved_bytes":0,"status":"ok","ts":1750000000000}"####,
    },
    Golden {
        fx: "prev2",
        id: "55ad3f37ceb268a2c8230f4f727d989db0fd2104d34c3bebed092c09b8c49bdc",
        machine_id: "old-machine",
        machine_source: "hostname",
        project_id: None,
        time_ms: Some(1750000001000),
        payload: r####"{"args_summary":"中文","bytes_out":0,"command":"read","error_code":"boom","id":2,"project_id":null,"saved_bytes":12,"status":"error","ts":1750000001000}"####,
    },
    Golden {
        fx: "cost",
        id: "ef7290455fc940d9cfcb35e80d74a4e5b91aeb00426dc145a968cb422ec26d41",
        machine_id: "cost-machine",
        machine_source: "env",
        project_id: None,
        time_ms: None,
        payload: r####"{"alpha":[1.5,{"a":"café 日","b":null},true],"machine":{"id":"cost-machine","source":"env"},"n":1e+21,"s":"line\nbreak \"q\"","zeta":2}"####,
    },
];

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

#[test]
fn event_ids_match_claudecat_golden() {
    let fx = Fx::new();
    for name in ["v2", "prev2", "cost"] {
        let want: Vec<&Golden> = GOLDEN.iter().filter(|g| g.fx == name).collect();
        let got = fx.events(name);
        assert_eq!(got.len(), want.len(), "{name}: event count");
        for (g, w) in got.iter().zip(&want) {
            // The payload string first: it asserts key order and every value conversion.
            assert_eq!(g.payload, w.payload, "{name}: payload");
            assert_eq!(g.event_id, w.id, "{name}: id of {}", w.payload);
            assert_eq!(g.machine_id, w.machine_id, "{name}: machine_id");
            assert_eq!(g.machine_source, w.machine_source, "{name}: machine_source");
            assert_eq!(g.project_id.as_deref(), w.project_id, "{name}: project_id");
            assert_eq!(g.time_ms, w.time_ms, "{name}: time_ms");
            assert_eq!(
                g.source,
                if name == "cost" {
                    "cost_daily"
                } else {
                    "cort_command"
                }
            );
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
    let cols = command_log_columns(&fx.p("v2/usage.db")).unwrap();
    let col_order: Vec<&str> = cols.split(',').collect();
    assert_ne!(
        col_order, keys,
        "fixture would not distinguish sorted from column order"
    );
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
    fs::write(dir.join("x.json"), r#"{"total":1}"#).unwrap();
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
        marker_path: fx.p("verified.json"),
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
fn dry_run_refuses_on_mismatch() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    let mut local = fx.events("v2");
    // Same row, but the id was computed from a different byte.
    local[0].event_id = event_id("cort_command", "mach-A", 0, "not the payload");
    let err = dry_run(&mut s, &local).unwrap_err();
    assert!(err.contains("id mismatch on 1 of"), "{err}");
    assert!(err.contains("rowid 1"), "{err}");
    assert!(err.contains("local  payload"), "{err}");
    assert!(err.contains("remote payload"), "{err}");
}

#[test]
fn dry_run_refuses_empty_overlap() {
    let fx = Fx::new();
    let mut s = store(&fx);
    // Remote holds two of the three machines; the third has none of its rows there.
    let v2 = fx.events("v2");
    s.insert_events(&[v2[0].clone(), v2[1].clone()]).unwrap();
    let err = dry_run(&mut s, &v2).unwrap_err();
    assert!(
        err.contains("no overlap: nothing proves the ids match"),
        "{err}"
    );
    assert!(err.contains("meta-machine"), "{err}");
    // And a wholly empty remote.
    let mut empty = SqliteStore::open(&fx.p("empty.db")).unwrap();
    assert!(dry_run(&mut empty, &v2).unwrap_err().contains("no overlap"));
}

#[test]
fn dry_run_counts_local_only_and_remote_only() {
    let fx = Fx::new();
    let mut s = store(&fx);
    let v2 = fx.events("v2");
    let mach_a: Vec<TelemetryEvent> = v2
        .iter()
        .filter(|e| e.machine_id == "mach-A")
        .cloned()
        .collect();
    assert_eq!(mach_a.len(), 3);
    s.insert_events(&mach_a[..2]).unwrap();
    // A remote row this machine's file no longer has (pruned after 90 days).
    let mut old = mach_a[0].clone();
    old.payload = old.payload.replace("\"id\":1,", "\"id\":99,");
    old.event_id = event_id("cort_command", "mach-A", 0, &old.payload);
    s.insert_events(&[old]).unwrap();
    let report = dry_run(&mut s, &mach_a).unwrap();
    assert_eq!(
        report,
        DryRun {
            matched: 2,
            mismatched: vec![],
            local_only: 1,
            remote_only: 1
        }
    );
}

#[test]
fn verify_never_pushes_and_writes_the_marker() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    s.fail_push(true); // would blow up if verify pushed
    let report = verify_events(&mut s, &inputs(&fx), 42).unwrap();
    assert_eq!(report.matched, 3);
    assert_eq!(s.push_calls(), 0);
    let marker: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fx.p("verified.json")).unwrap()).unwrap();
    assert_eq!(marker["machine_id"], "mach-A");
    assert_eq!(marker["verified_ms"], 42);
    // Verification wrote nothing but the marker.
    assert_eq!(held(&mut s), 3);
    assert_eq!(s.event_watermark("mach-A", "cort_command").unwrap(), None);
}

#[test]
fn failed_verify_leaves_no_marker() {
    let fx = Fx::new();
    let mut s = store(&fx); // empty remote: no overlap
    assert!(verify_events(&mut s, &inputs(&fx), 1).is_err());
    assert!(!fx.p("verified.json").exists());
}

#[test]
fn upload_refuses_without_watermark_or_passing_dry_run() {
    let fx = Fx::new();
    let mut s = store(&fx);
    let report = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(&report, EventsReport::Refused(r) if r.contains("--dry-run")),
        "{report:?}"
    );
    assert_eq!(held(&mut s), 0);
    assert_eq!(s.event_watermark("mach-A", "cort_command").unwrap(), None);
    assert_eq!(s.push_calls(), 0);
}

#[test]
fn marker_for_another_machine_or_schema_does_not_open_the_gate() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
    let mut other = inputs(&fx);
    other.machine_id = "mach-Z".into();
    assert!(matches!(
        upload_events(&mut s, &other),
        EventsReport::Refused(_)
    ));
    // Changing the column layout changes every payload, so the old comparison proves nothing.
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch("ALTER TABLE command_log ADD COLUMN extra TEXT")
        .unwrap();
    assert!(matches!(
        upload_events(&mut s, &inputs(&fx)),
        EventsReport::Refused(_)
    ));
    assert_eq!(held(&mut s), 3);
}

#[test]
fn missing_watermark_full_range_inserts_no_duplicates() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
    let r = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        r,
        EventsReport::Uploaded {
            scanned: 6,
            inserted: 3,
            aliases: 0,
            already_uploaded: 3,
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
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
    upload_events(&mut s, &inputs(&fx));
    assert_eq!(wm(&mut s, "mach-A"), Some(9_000_000_000));
    // Nothing new: nothing read, nothing inserted.
    let idle = upload_events(&mut s, &inputs(&fx));
    assert_eq!(
        idle,
        EventsReport::Uploaded {
            scanned: 0,
            inserted: 0,
            aliases: 0,
            already_uploaded: 0,
            full_range: false,
            reason: None,
            watermark: Some(9_000_000_000)
        }
    );
    // A later row is the only one read, and the gate no longer needs the marker.
    fs::remove_file(fx.p("verified.json")).unwrap();
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute(
            "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (9000000001,1,'mach-A','etc-machine-id','x','y','ok',1)",
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
            already_uploaded: 0,
            full_range: false,
            reason: None,
            watermark: Some(9_000_000_001)
        }
    );
    assert_eq!(held(&mut s), 7);
}

#[test]
fn watermark_set_replaces_so_a_recreated_db_can_restart_lower() {
    let fx = Fx::new();
    let mut s = store(&fx);
    s.set_event_watermark("m", "cort_command", 10, "id10")
        .unwrap();
    s.set_event_watermark("m", "cort_command", 5, "id5")
        .unwrap();
    assert_eq!(
        s.event_watermark("m", "cort_command").unwrap(),
        Some((5, "id5".to_string()))
    );
    assert_eq!(s.event_watermark("m", "cost_daily").unwrap(), None);
}

#[test]
fn cost_events_upload_under_the_same_gate() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
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
        "INSERT INTO command_log (id,ts,project_id,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,1,?2,?3,'env','x','y','ok',1)",
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
        marker_path: fx.p("verified.json"),
        machine_id: "mach-A".into(),
    };
    verify_events(&mut s, &up, 1).unwrap();
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

/// `--help` is answered globally (there is no per-verb help), so the retention boundary and the
/// dry-run flag live in the `measure` line of the usage text.
#[test]
fn help_states_the_retention_boundary() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cort"))
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sync --dry-run"), "{text}");
    assert!(text.contains("90 days in a row"), "{text}");
    assert!(text.contains("idle for more than 90 days"), "{text}");
    assert!(!text.contains("record not running"), "{text}");
}

fn wm(s: &mut SqliteStore, machine: &str) -> Option<i64> {
    s.event_watermark(machine, "cort_command")
        .unwrap()
        .map(|w| w.0)
}

/// Replace `usage.db` by a brand-new file holding rows `(id, command)` for machine mach-A.
fn replace_usage_db(fx: &Fx, rows: &[(i64, &str)]) {
    let path = fx.p("v2/usage.db");
    fs::remove_file(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute_batch(V2_SCHEMA).unwrap();
    for (id, cmd) in rows {
        c.execute(
            "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,1,'mach-A','etc-machine-id',?2,'y','ok',1)",
            params![id, cmd],
        )
        .unwrap();
    }
}

fn uploaded_s(fx: &Fx) -> SqliteStore {
    let mut s = store(fx);
    seed_one_per_machine(&mut s, fx);
    verify_events(&mut s, &inputs(fx), 1).unwrap();
    upload_events(&mut s, &inputs(fx));
    assert_eq!(wm(&mut s, "mach-A"), Some(9_000_000_000));
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
            already_uploaded: 0,
            full_range: true,
            reason: Some("watermark_row_missing".into()),
            watermark: Some(3)
        }
    );
    assert_eq!(wm(&mut s, "mach-A"), Some(3));
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
    fs::remove_file(fx.p("verified.json")).unwrap(); // no marker: the fallback must not need one
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(&r, EventsReport::Uploaded { scanned: 5, inserted: 0, full_range: true,
            reason: Some(why), watermark: Some(9), .. } if why == "watermark_row_missing"),
        "{r:?}"
    );
    assert_eq!(held(&mut s), 6, "nothing duplicated, nothing lost");
}

#[test]
fn marker_certifies_algorithm_not_file() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
    // Same machine, same column layout, entirely different contents (and so no overlap with the
    // remote at all): the verified algorithm still applies, so the gate stays open.
    replace_usage_db(&fx, &[(1, "other"), (2, "rows")]);
    let mut fresh = SqliteStore::open(&fx.p("fresh.db")).unwrap();
    let r = upload_events(&mut fresh, &inputs(&fx));
    assert!(
        matches!(
            r,
            EventsReport::Uploaded {
                scanned: 2,
                inserted: 2,
                full_range: true,
                ..
            }
        ),
        "{r:?}"
    );
}

#[test]
fn upload_pages_through_a_large_range() {
    let fx = Fx::new();
    let mut s = store(&fx);
    seed_one_per_machine(&mut s, &fx);
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
    {
        let c = Connection::open(fx.p("v2/usage.db")).unwrap();
        c.execute_batch("BEGIN").unwrap();
        for i in 1..=4500i64 {
            c.execute(
                "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out) VALUES (?1,1,'mach-A','etc-machine-id','bulk','y','ok',1)",
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
    assert_eq!(wm(&mut s, "mach-A"), Some(10_000_004_500));
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
                "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out,extra) VALUES (?1,1,'mach-A','etc-machine-id','new','y','ok',1,'e')",
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
            already_uploaded: 0,
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
fn schema_change_then_pruned_watermark_does_not_reupload() {
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
                "INSERT INTO command_log (id,ts,machine_id,machine_source,command,args_summary,status,bytes_out,extra) VALUES (?1,2,'mach-A','etc-machine-id','new','y','ok',1,'e')",
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
            inserted: 2,
            aliases: 0,
            already_uploaded: 5,
            full_range: true,
            reason: Some("watermark_row_missing".into()),
            watermark: Some(9_000_000_011)
        }
    );
    assert_eq!(stored(&fx), 8, "only the new rows were added");
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
    verify_events(&mut s, &inputs(&fx), 1).unwrap();
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            r,
            EventsReport::Uploaded {
                scanned: 2,
                inserted: 1,
                already_uploaded: 1,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 2);
}

/// Make the watermark point at a rowid the database does not have, as the 90-day prune would.
fn lose_watermark_row(s: &mut SqliteStore) {
    s.set_event_watermark("mach-A", "cort_command", 9_000_000_500, "gone")
        .unwrap();
}

#[test]
fn float_ts_row_is_not_reuploaded_after_column_add_and_pruned_watermark() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx); // includes the float-ts row (no time_ms), id 9000000000
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch("ALTER TABLE command_log ADD COLUMN extra TEXT")
        .unwrap();
    lose_watermark_row(&mut s);
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            &r,
            EventsReport::Uploaded {
                scanned: 6,
                inserted: 0,
                already_uploaded: 6,
                full_range: true,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 6);
}

#[test]
fn schema_column_removed_then_pruned_watermark_does_not_reupload() {
    let fx = Fx::new();
    let mut s = uploaded_s(&fx);
    // The remote payloads carry `read_source`; the local table no longer has it.
    Connection::open(fx.p("v2/usage.db"))
        .unwrap()
        .execute_batch("ALTER TABLE command_log DROP COLUMN read_source")
        .unwrap();
    let local = cort_events(&fx.p("v2/usage.db"), None).unwrap();
    assert!(local.iter().all(|e| !e.payload.contains("read_source")));
    lose_watermark_row(&mut s);
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            &r,
            EventsReport::Uploaded {
                scanned: 6,
                inserted: 0,
                already_uploaded: 6,
                full_range: true,
                ..
            }
        ),
        "{r:?}"
    );
    assert_eq!(stored(&fx), 6);
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
    lose_watermark_row(&mut s);
    let r = upload_events(&mut s, &inputs(&fx));
    assert!(
        matches!(
            &r,
            EventsReport::Uploaded {
                already_uploaded: 0,
                inserted: 6,
                ..
            }
        ),
        "{r:?}"
    );
}
