use cort::remote::{ErrorKind, RemoteDb, StepOutcome, Stmt, UreqTransport};
use std::time::Duration;

struct Cleanup<'a> {
    db: &'a RemoteDb,
    active: bool,
}
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .db
                .execute("DROP TABLE IF EXISTS cort_spike_tx", vec![]);
        }
    }
}

#[test]
#[ignore]
fn live_transaction_probe() {
    let (Ok(url), Ok(token)) = (
        std::env::var("CORT_REMOTE_PROBE_URL"),
        std::env::var("CORT_REMOTE_PROBE_TOKEN"),
    ) else {
        eprintln!("SKIP: CORT_REMOTE_PROBE_URL and CORT_REMOTE_PROBE_TOKEN are required");
        return;
    };
    let db = RemoteDb::new(
        &url,
        token,
        Box::new(UreqTransport {
            timeout: Duration::from_secs(30),
        }),
    )
    .unwrap();
    let mut cleanup = Cleanup {
        db: &db,
        active: true,
    };
    db.execute("DROP TABLE IF EXISTS cort_spike_tx", vec![])
        .unwrap();
    db.execute("CREATE TABLE cort_spike_tx(k INTEGER PRIMARY KEY)", vec![])
        .unwrap();
    let outcomes = db
        .transaction_steps(&[
            Stmt::new("INSERT INTO cort_spike_tx(k) VALUES(1)", vec![]),
            Stmt::new("INSERT INTO cort_spike_tx(k) VALUES(1)", vec![]),
        ])
        .unwrap();
    assert!(matches!(&outcomes[0], StepOutcome::Ok(_)));
    assert!(matches!(&outcomes[1], StepOutcome::Ok(_)));
    assert!(matches!(&outcomes[2],StepOutcome::Error(e) if e.kind==ErrorKind::Constraint));
    assert!(matches!(&outcomes[3], StepOutcome::Skipped));
    assert!(matches!(&outcomes[4], StepOutcome::Ok(_)));
    match db.execute("INSERT INTO cort_spike_tx(k) VALUES(1)", vec![]) {
        Ok(_) => {}
        Err(e) if e.kind == ErrorKind::Constraint => panic!("partial commit: {e}"),
        Err(e) => panic!("probe invalid, rerun: {e}"),
    }
    db.pipeline(&[
        Stmt::new("BEGIN", vec![]),
        Stmt::new("INSERT INTO cort_spike_tx(k) VALUES(2)", vec![]),
    ])
    .unwrap();
    match db.execute("INSERT INTO cort_spike_tx(k) VALUES(2)", vec![]) {
        Ok(_) => {}
        Err(e) if e.kind == ErrorKind::Constraint => {
            panic!("uncommitted pipeline insert persisted: {e}")
        }
        Err(e) => panic!("probe invalid, rerun: {e}"),
    }
    let result = db
        .execute("INSERT OR IGNORE INTO cort_spike_tx(k) VALUES(2)", vec![])
        .unwrap();
    // affected_row_count is 0 for an ignored row by definition; the billing question is whether
    // the server still counts it in rows_written.
    println!(
        "ignored INSERT: affected_row_count={} rows_written={:?} rows_read={:?}",
        result.affected_row_count, result.rows_written, result.rows_read
    );
    assert_eq!(result.rows_written, Some(0));
    db.execute("DROP TABLE cort_spike_tx", vec![]).unwrap();
    let remaining = db
        .execute(
            "SELECT count(*) FROM sqlite_master WHERE name='cort_spike_tx'",
            vec![],
        )
        .unwrap();
    assert_eq!(remaining.rows, vec![vec![cort::remote::Cell::Int(0)]]);
    cleanup.active = false;
}
