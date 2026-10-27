use cort::remote::{
    Cell, ErrorKind, RemoteDb, RemoteError, SqliteTransport, StepOutcome, Stmt, Transport,
    UreqTransport,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn client() -> (RemoteDb, Arc<SqliteTransport>) {
    let transport = Arc::new(SqliteTransport::new().unwrap());
    struct Shared(Arc<SqliteTransport>);
    impl Transport for Shared {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, RemoteError> {
            self.0.post(url, body, headers)
        }
    }
    (
        RemoteDb::new(
            "turso://unit.test",
            "SECRET_TOKEN",
            Box::new(Shared(transport.clone())),
        )
        .unwrap()
        .with_sleeper(|_| {}),
        transport,
    )
}
#[test]
fn cells_and_schemes() {
    let (db, transport) = client();
    let row = db
        .execute(
            "SELECT ?1 AS n, ?2 AS s, ?3 AS empty",
            vec![Cell::Int(2164), Cell::Text("hello".into()), Cell::Null],
        )
        .unwrap();
    assert_eq!(
        row.rows,
        vec![vec![
            Cell::Int(2164),
            Cell::Text("hello".into()),
            Cell::Null
        ]]
    );
    assert_eq!(db.pipeline_url(), "https://unit.test/v2/pipeline");
    for scheme in ["libsql://", "https://"] {
        let url = format!("{scheme}unit.test/");
        assert_eq!(
            RemoteDb::new(&url, "x", Box::new(SqliteTransport::new().unwrap()))
                .unwrap()
                .pipeline_url(),
            "https://unit.test/v2/pipeline"
        );
    }
    assert!(RemoteDb::new(
        "http://unit.test",
        "x",
        Box::new(SqliteTransport::new().unwrap())
    )
    .is_err());
    for cell in [
        json!({"type":"float","value":"1.5"}),
        json!({"type":"mystery","value":"x"}),
    ] {
        transport.return_next_cell(cell);
        assert_eq!(
            db.execute("SELECT 1", vec![]).unwrap_err().kind,
            ErrorKind::Sql
        );
    }
    transport.return_next_cell(json!({"type":"integer","value":"2164"}));
    assert_eq!(
        db.execute("SELECT 1", vec![]).unwrap().rows[0][0],
        Cell::Int(2164)
    );
}
#[test]
fn pipeline_reports_first_failure() {
    let (db, _) = client();
    let err = db
        .pipeline(&[
            Stmt::new("SELECT 1", vec![]),
            Stmt::new("bad sql", vec![]),
            Stmt::new("also bad", vec![]),
        ])
        .unwrap_err();
    assert_eq!(err.step, Some(1));
}
#[test]
fn transaction_commits_and_rolls_back() {
    let (db, _) = client();
    db.execute("CREATE TABLE t(k INTEGER PRIMARY KEY)", vec![])
        .unwrap();
    let inserted = db
        .transaction(&[Stmt::new("INSERT INTO t VALUES(1)", vec![])])
        .unwrap();
    assert_eq!(inserted[0].rows_written, Some(1));
    let ignored = db
        .execute("INSERT OR IGNORE INTO t VALUES(1)", vec![])
        .unwrap();
    assert_eq!(
        (ignored.affected_row_count, ignored.rows_written),
        (0, Some(0))
    );
    let err = db
        .transaction(&[
            Stmt::new("INSERT INTO t VALUES(2)", vec![]),
            Stmt::new("INSERT INTO t VALUES(1)", vec![]),
        ])
        .unwrap_err();
    assert_eq!((err.kind, err.step), (ErrorKind::Constraint, Some(2)));
    assert_eq!(
        db.execute("SELECT k FROM t ORDER BY k", vec![])
            .unwrap()
            .rows,
        vec![vec![Cell::Int(1)]]
    );
}
#[test]
fn pipeline_close_rolls_back_uncommitted_insert() {
    let (db, _) = client();
    db.execute("CREATE TABLE t(k INTEGER PRIMARY KEY)", vec![])
        .unwrap();
    db.pipeline(&[
        Stmt::new("BEGIN", vec![]),
        Stmt::new("INSERT INTO t VALUES(1)", vec![]),
    ])
    .unwrap();
    assert!(db.execute("INSERT INTO t VALUES(1)", vec![]).is_ok());
}
#[test]
fn rollback_error_does_not_mask_first_step() {
    let (db, transport) = client();
    transport.fail_next_step(0, "SQLITE_BUSY");
    let outcomes = db
        .transaction_steps(&[Stmt::new("SELECT 1", vec![])])
        .unwrap();
    assert!(matches!(&outcomes[0],StepOutcome::Error(e) if e.kind==ErrorKind::Busy));
    assert!(matches!(&outcomes[3], StepOutcome::Error(_)));
}
#[test]
fn kinds_and_secret_redaction() {
    let (db, transport) = client();
    assert!(!format!("{db:?}").contains("SECRET_TOKEN"));
    transport.fail_next_status(401);
    let err = db.execute("SELECT 1", vec![]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Unauthorized);
    for code in [
        "BLOCKED",
        "SQLITE_BUSY",
        "SQLITE_CONSTRAINT_UNIQUE",
        "SQLITE_ERROR",
    ] {
        transport.fail_next_code(code);
        let err = db.execute("SELECT 1", vec![]).unwrap_err();
        assert_eq!(
            err.kind,
            match code {
                "BLOCKED" => ErrorKind::Blocked,
                "SQLITE_BUSY" => ErrorKind::Busy,
                "SQLITE_CONSTRAINT_UNIQUE" => ErrorKind::Constraint,
                _ => ErrorKind::Sql,
            }
        );
        assert!(!format!("{err}").contains("SECRET_TOKEN"));
        assert!(!format!("{err:?}").contains("SECRET_TOKEN"));
    }
}
#[test]
fn server_message_is_not_exposed() {
    struct Canned;
    impl Transport for Canned {
        fn post(&self, _: &str, _: &[u8], _: &[(&str, &str)]) -> Result<Vec<u8>, RemoteError> {
            Ok(json!({"results":[{"type":"error","error":{"code":"SQLITE_ERROR","message":"fake-token-123"}},
                {"type":"ok","response":null}]}).to_string().into_bytes())
        }
    }
    let db = RemoteDb::new("turso://unit.test", "x", Box::new(Canned)).unwrap();
    let err = db.execute("SELECT 1", vec![]).unwrap_err();
    assert!(!format!("{err}").contains("fake-token-123"));
    assert!(!format!("{err:?}").contains("fake-token-123"));
    assert!(!err.detail().contains("fake-token-123"));
}
#[test]
fn counters_accept_numbers_and_strings() {
    struct Canned;
    impl Transport for Canned {
        fn post(&self, _: &str, _: &[u8], _: &[(&str, &str)]) -> Result<Vec<u8>, RemoteError> {
            Ok(json!({"results":[{"type":"ok","response":{"result":{"cols":[{"name":null},{"name":""}],"rows":[],
                "affected_row_count":0,"rows_read":3,"rows_written":"1"}}},{"type":"ok","response":null}]}).to_string().into_bytes())
        }
    }
    let db = RemoteDb::new("turso://unit.test", "x", Box::new(Canned)).unwrap();
    let result = db.execute("SELECT 1", vec![]).unwrap();
    assert_eq!(result.cols, vec!["", ""]);
    assert_eq!((result.rows_read, result.rows_written), (Some(3), Some(1)));
}
#[test]
fn busy_retry() {
    let (db, transport) = client();
    for _ in 0..4 {
        transport.fail_next_step(0, "SQLITE_BUSY");
    }
    assert_eq!(
        db.transaction(&[Stmt::new("SELECT 1", vec![])])
            .unwrap_err()
            .kind,
        ErrorKind::Busy
    );
    let (db, transport) = client();
    transport.fail_next_step(0, "SQLITE_BUSY");
    assert!(db.transaction(&[Stmt::new("SELECT 1", vec![])]).is_ok());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saved = calls.clone();
    let db = db.with_sleeper(move |d| saved.lock().unwrap().push(d));
    transport.fail_next_step(0, "SQLITE_BUSY");
    db.transaction(&[Stmt::new("SELECT 1", vec![])]).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![Duration::from_millis(100)]);
}
#[test]
fn busy_retry_user_and_commit_steps() {
    for step in [1, 2] {
        let (db, transport) = client();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let saved = calls.clone();
        let db = db.with_sleeper(move |d| saved.lock().unwrap().push(d));
        for _ in 0..4 {
            transport.fail_next_step(step, "SQLITE_BUSY");
        }
        let err = db
            .transaction(&[Stmt::new("SELECT 1", vec![])])
            .unwrap_err();
        assert_eq!((err.kind, err.step), (ErrorKind::Busy, Some(step)));
        assert_eq!(
            *calls.lock().unwrap(),
            [100, 300, 900].map(Duration::from_millis)
        );
    }
}
#[test]
fn close_error_after_success_is_ignored() {
    let (db, transport) = client();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saved = calls.clone();
    let db = db.with_sleeper(move |d| saved.lock().unwrap().push(d));
    transport.fail_next_close("SQLITE_BUSY");
    assert!(db.transaction(&[Stmt::new("SELECT 1", vec![])]).is_ok());
    assert!(calls.lock().unwrap().is_empty());
    transport.fail_next_close("SQLITE_BUSY");
    assert!(db.pipeline(&[Stmt::new("SELECT 1", vec![])]).is_ok());
    assert!(calls.lock().unwrap().is_empty());
}
#[test]
fn injection_shape_is_checked() {
    let (db, transport) = client();
    transport.fail_next_code("BLOCKED");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || db.pipeline(&[Stmt::new("SELECT 1", vec![]), Stmt::new("SELECT 2", vec![])])
    ))
    .is_err());
    let (db, transport) = client();
    transport.fail_next_step(0, "SQLITE_BUSY");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || db.execute("SELECT 1", vec![])
    ))
    .is_err());
    let (db, transport) = client();
    transport.return_next_cell(json!({"type":"null"}));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || db.pipeline(&[Stmt::new("SELECT 1", vec![]), Stmt::new("SELECT 2", vec![])])
    ))
    .is_err());
}
#[test]
fn sqlite_constraint_retains_extended_code() {
    let (db, _) = client();
    db.execute(
        "CREATE TABLE t(k INTEGER PRIMARY KEY, v TEXT UNIQUE NOT NULL)",
        vec![],
    )
    .unwrap();
    db.execute("INSERT INTO t VALUES(1, 'a')", vec![]).unwrap();
    let error = db
        .execute("INSERT INTO t VALUES(2, 'a')", vec![])
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Constraint);
    assert!(error.detail().contains("SQLITE_CONSTRAINT_UNIQUE"));
}
#[test]
fn overall_timeout_after_accept() {
    use std::net::TcpListener;
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("SKIP: sandbox denies loopback bind");
            return;
        }
        Err(error) => panic!("loopback bind failed: {error}"),
    };
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(900));
    });
    let transport = UreqTransport {
        timeout: Duration::from_millis(400),
    };
    let err = transport
        .post(&format!("http://{address}"), b"{}", &[])
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Timeout);
    worker.join().unwrap();
}
#[test]
fn timeout_while_reading_body() {
    use std::io::Write;
    use std::net::TcpListener;
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("SKIP: sandbox denies loopback bind");
            return;
        }
        Err(error) => panic!("loopback bind failed: {error}"),
    };
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
            .unwrap();
        std::thread::sleep(Duration::from_millis(900));
    });
    let transport = UreqTransport {
        timeout: Duration::from_millis(400),
    };
    let err = transport
        .post(&format!("http://{address}"), b"{}", &[])
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Timeout);
    worker.join().unwrap();
}

/// The turso client builds its TLS config with `ClientConfig::builder()`, which panics when more
/// than one rustls provider is compiled in; that panic hung every replica test (C1, e7ea29d6).
#[test]
fn rustls_has_one_process_provider() {
    let _ = rustls::ClientConfig::builder();
}
