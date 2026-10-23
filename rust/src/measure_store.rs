//! Compaction measurement rows, stored in a Turso database of their own.
//!
//! Two implementations of one trait: [`SqliteStore`] is a plain rusqlite file (tests, and any
//! caller that wants the logic without a network), and the Turso store behind
//! [`with_turso_store`] is a sync replica that must only be opened through the `turso` crate.
//! Both run the same [`MEASURE_SCHEMA`] and the same SQL text.

use crate::memory_store::{self, Config};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowCounts {
    pub tool_calls: i64,
    pub rereads: i64,
    pub repeated_commands: i64,
    pub transcript_lookups: i64,
    pub human_messages: i64,
    pub corrections: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionRow {
    pub machine_id: String,
    pub boundary_uuid: String,
    pub session_id: String,
    pub time_ms: i64,
    pub project_id: Option<String>,
    pub trigger: String,
    pub pre_tokens: Option<i64>,
    pub post_tokens: Option<i64>,
    pub pre: WindowCounts,
    pub post: WindowCounts,
    pub post_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineHost {
    pub machine_id: String,
    pub host: String,
    pub last_seen_ms: i64,
}

pub trait MeasureStore {
    /// Insert, or replace the row with the same `(machine_id, boundary_uuid)`.
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String>;
    /// The session that already owns this boundary, if any.
    fn compaction_session(
        &mut self,
        machine_id: &str,
        boundary_uuid: &str,
    ) -> Result<Option<String>, String>;
    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String>;
    /// Record that `host` was seen for `machine_id`; the latest `seen_ms` wins.
    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String>;
    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String>;
    fn push(&mut self) -> Result<(), String>;
    fn pull(&mut self) -> Result<(), String>;
}

/// Every CREATE statement; both implementations run all of them on open.
pub const MEASURE_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS compactions (
        machine_id TEXT NOT NULL,
        boundary_uuid TEXT NOT NULL,
        session_id TEXT NOT NULL,
        time_ms INTEGER NOT NULL,
        project_id TEXT,
        trigger_kind TEXT NOT NULL,
        pre_tokens INTEGER,
        post_tokens INTEGER,
        pre_tool_calls INTEGER NOT NULL,
        pre_rereads INTEGER NOT NULL,
        pre_repeated_commands INTEGER NOT NULL,
        pre_transcript_lookups INTEGER NOT NULL,
        pre_human_messages INTEGER NOT NULL,
        pre_corrections INTEGER NOT NULL,
        post_tool_calls INTEGER NOT NULL,
        post_rereads INTEGER NOT NULL,
        post_repeated_commands INTEGER NOT NULL,
        post_transcript_lookups INTEGER NOT NULL,
        post_human_messages INTEGER NOT NULL,
        post_corrections INTEGER NOT NULL,
        post_complete INTEGER NOT NULL,
        PRIMARY KEY (machine_id, boundary_uuid)
    )",
    "CREATE INDEX IF NOT EXISTS compactions_time ON compactions (time_ms)",
    "CREATE TABLE IF NOT EXISTS machine_hosts (
        machine_id TEXT NOT NULL,
        host TEXT NOT NULL,
        last_seen_ms INTEGER NOT NULL,
        PRIMARY KEY (machine_id, host)
    )",
];

const COMPACTION_COLUMNS: &str = "machine_id, boundary_uuid, session_id, time_ms, project_id, \
    trigger_kind, pre_tokens, post_tokens, \
    pre_tool_calls, pre_rereads, pre_repeated_commands, pre_transcript_lookups, \
    pre_human_messages, pre_corrections, \
    post_tool_calls, post_rereads, post_repeated_commands, post_transcript_lookups, \
    post_human_messages, post_corrections, post_complete";

const UPSERT_COMPACTION: &str = "INSERT OR REPLACE INTO compactions (machine_id, boundary_uuid, \
    session_id, time_ms, project_id, trigger_kind, pre_tokens, post_tokens, \
    pre_tool_calls, pre_rereads, pre_repeated_commands, pre_transcript_lookups, \
    pre_human_messages, pre_corrections, \
    post_tool_calls, post_rereads, post_repeated_commands, post_transcript_lookups, \
    post_human_messages, post_corrections, post_complete) \
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
const SELECT_SESSION: &str =
    "SELECT session_id FROM compactions WHERE machine_id = ? AND boundary_uuid = ?";
const UPSERT_HOST: &str = "INSERT INTO machine_hosts (machine_id, host, last_seen_ms) \
    VALUES (?, ?, ?) ON CONFLICT (machine_id, host) DO UPDATE SET \
    last_seen_ms = MAX(last_seen_ms, excluded.last_seen_ms)";
const SELECT_HOSTS: &str = "SELECT machine_id, host, last_seen_ms FROM machine_hosts";

fn select_since() -> String {
    format!("SELECT {COMPACTION_COLUMNS} FROM compactions WHERE time_ms >= ? ORDER BY time_ms, machine_id, boundary_uuid")
}

/// A column value neutral between rusqlite and turso, so row <-> SQL mapping is written once.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Text(String),
}

fn opt_int(v: Option<i64>) -> Cell {
    v.map_or(Cell::Null, Cell::Int)
}

fn compaction_cells(r: &CompactionRow) -> Vec<Cell> {
    let mut cells = vec![
        Cell::Text(r.machine_id.clone()),
        Cell::Text(r.boundary_uuid.clone()),
        Cell::Text(r.session_id.clone()),
        Cell::Int(r.time_ms),
        r.project_id.clone().map_or(Cell::Null, Cell::Text),
        Cell::Text(r.trigger.clone()),
        opt_int(r.pre_tokens),
        opt_int(r.post_tokens),
    ];
    for w in [&r.pre, &r.post] {
        cells.extend(
            [
                w.tool_calls,
                w.rereads,
                w.repeated_commands,
                w.transcript_lookups,
                w.human_messages,
                w.corrections,
            ]
            .map(Cell::Int),
        );
    }
    cells.push(Cell::Int(i64::from(r.post_complete)));
    cells
}

fn int(c: &Cell) -> Result<i64, String> {
    match c {
        Cell::Int(i) => Ok(*i),
        other => Err(format!("expected integer column, got {other:?}")),
    }
}

fn opt_i(c: &Cell) -> Result<Option<i64>, String> {
    match c {
        Cell::Null => Ok(None),
        c => int(c).map(Some),
    }
}

fn text(c: &Cell) -> Result<String, String> {
    match c {
        Cell::Text(s) => Ok(s.clone()),
        other => Err(format!("expected text column, got {other:?}")),
    }
}

fn window(cells: &[Cell]) -> Result<WindowCounts, String> {
    Ok(WindowCounts {
        tool_calls: int(&cells[0])?,
        rereads: int(&cells[1])?,
        repeated_commands: int(&cells[2])?,
        transcript_lookups: int(&cells[3])?,
        human_messages: int(&cells[4])?,
        corrections: int(&cells[5])?,
    })
}

fn compaction_from_cells(c: &[Cell]) -> Result<CompactionRow, String> {
    if c.len() != 21 {
        return Err(format!(
            "compactions row has {} columns, expected 21",
            c.len()
        ));
    }
    Ok(CompactionRow {
        machine_id: text(&c[0])?,
        boundary_uuid: text(&c[1])?,
        session_id: text(&c[2])?,
        time_ms: int(&c[3])?,
        project_id: match &c[4] {
            Cell::Null => None,
            other => Some(text(other)?),
        },
        trigger: text(&c[5])?,
        pre_tokens: opt_i(&c[6])?,
        post_tokens: opt_i(&c[7])?,
        pre: window(&c[8..14])?,
        post: window(&c[14..20])?,
        post_complete: int(&c[20])? != 0,
    })
}

fn host_from_cells(c: &[Cell]) -> Result<MachineHost, String> {
    Ok(MachineHost {
        machine_id: text(&c[0])?,
        host: text(&c[1])?,
        last_seen_ms: int(&c[2])?,
    })
}

/// A plain SQLite file implementing the store; `push`/`pull` have nothing to sync.
pub struct SqliteStore {
    conn: rusqlite::Connection,
    fail_push: bool,
}

fn sql_err(e: rusqlite::Error) -> String {
    e.to_string()
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path).map_err(sql_err)?;
        for stmt in MEASURE_SCHEMA {
            conn.execute(stmt, []).map_err(sql_err)?;
        }
        Ok(Self {
            conn,
            fail_push: false,
        })
    }

    /// Test hook: make `push` return an error while keeping local data.
    pub fn fail_push(&mut self, fail: bool) {
        self.fail_push = fail;
    }

    fn query(&self, sql: &str, params: Vec<Cell>, cols: usize) -> Result<Vec<Vec<Cell>>, String> {
        use rusqlite::types::{Value, ValueRef};
        let mut stmt = self.conn.prepare(sql).map_err(sql_err)?;
        let params: Vec<Value> = params.into_iter().map(to_sqlite).collect();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params))
            .map_err(sql_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(sql_err)? {
            let mut cells = Vec::with_capacity(cols);
            for i in 0..cols {
                cells.push(match row.get_ref(i).map_err(sql_err)? {
                    ValueRef::Null => Cell::Null,
                    ValueRef::Integer(i) => Cell::Int(i),
                    ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
                    other => return Err(format!("unexpected column type {other:?}")),
                });
            }
            out.push(cells);
        }
        Ok(out)
    }

    fn exec(&self, sql: &str, params: Vec<Cell>) -> Result<(), String> {
        let params: Vec<rusqlite::types::Value> = params.into_iter().map(to_sqlite).collect();
        self.conn
            .execute(sql, rusqlite::params_from_iter(params))
            .map_err(sql_err)?;
        Ok(())
    }
}

fn to_sqlite(c: Cell) -> rusqlite::types::Value {
    match c {
        Cell::Null => rusqlite::types::Value::Null,
        Cell::Int(i) => rusqlite::types::Value::Integer(i),
        Cell::Text(s) => rusqlite::types::Value::Text(s),
    }
}

impl MeasureStore for SqliteStore {
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String> {
        self.exec(UPSERT_COMPACTION, compaction_cells(row))
    }

    fn compaction_session(
        &mut self,
        machine_id: &str,
        boundary_uuid: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.query(
            SELECT_SESSION,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(boundary_uuid.into()),
            ],
            1,
        )?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String> {
        self.query(&select_since(), vec![Cell::Int(since_ms)], 21)?
            .iter()
            .map(|r| compaction_from_cells(r))
            .collect()
    }

    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String> {
        self.exec(
            UPSERT_HOST,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(host.into()),
                Cell::Int(seen_ms),
            ],
        )
    }

    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String> {
        self.query(SELECT_HOSTS, vec![], 3)?
            .iter()
            .map(|r| host_from_cells(r))
            .collect()
    }

    fn push(&mut self) -> Result<(), String> {
        if self.fail_push {
            return Err("push failed (fail_push set)".into());
        }
        Ok(())
    }

    fn pull(&mut self) -> Result<(), String> {
        Ok(())
    }
}

const SUBDIR: &str = "turso-measure";

fn config_path() -> PathBuf {
    crate::data_dir::data_dir().join("turso-measure.json")
}

/// Whether `cort measure configure` has been run on this machine (the config file exists).
pub fn is_configured() -> bool {
    config_path().exists()
}

/// Store the measurement database's URL and token, private to this user.
pub fn configure(url: &str, token: &str) -> Result<PathBuf, String> {
    let config = memory_store::validated_config(url, token)?;
    memory_store::write_private_config(&config_path(), &config, "Turso measure config")
}

fn config() -> Result<Config, String> {
    memory_store::read_config(
        &config_path(),
        "Turso measure config",
        "Turso measurement is not configured; run `cort measure configure --url URL --token-stdin`",
    )
}

fn with_config_lock<T>(action: impl FnOnce(Config) -> Result<T, String>) -> Result<T, String> {
    let config = config()?;
    let _lock = memory_store::lock_in(SUBDIR, &config, "measure.lock")?;
    action(config)
}

/// Hold the exclusive `measure.lock` for the configured database while `action` runs.
pub fn with_measure_lock<T>(action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    with_config_lock(|_| action())
}

/// The Turso sync replica as a [`MeasureStore`]. `push`/`pull` talk to the network.
struct TursoStore {
    rt: tokio::runtime::Runtime,
    db: turso::sync::Database,
    conn: turso::Connection,
}

fn tv(c: Cell) -> turso::Value {
    match c {
        Cell::Null => turso::Value::Null,
        Cell::Int(i) => turso::Value::Integer(i),
        Cell::Text(s) => turso::Value::Text(s),
    }
}

fn te(e: turso::Error) -> String {
    e.to_string()
}

async fn turso_exec(conn: &turso::Connection, sql: &str, params: Vec<Cell>) -> Result<(), String> {
    conn.execute(sql, turso::params_from_iter(params.into_iter().map(tv)))
        .await
        .map_err(te)?;
    Ok(())
}

async fn turso_query(
    conn: &turso::Connection,
    sql: &str,
    params: Vec<Cell>,
    cols: usize,
) -> Result<Vec<Vec<Cell>>, String> {
    let mut rows = conn
        .query(sql, turso::params_from_iter(params.into_iter().map(tv)))
        .await
        .map_err(te)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(te)? {
        let mut cells = Vec::with_capacity(cols);
        for i in 0..cols {
            cells.push(match row.get_value(i).map_err(te)? {
                turso::Value::Null => Cell::Null,
                turso::Value::Integer(i) => Cell::Int(i),
                turso::Value::Text(t) => Cell::Text(t),
                other => return Err(format!("unexpected column type {other:?}")),
            });
        }
        out.push(cells);
    }
    Ok(out)
}

impl TursoStore {
    fn open(config: &Config) -> Result<Self, String> {
        let rt = memory_store::runtime()?;
        let (db, conn) = rt.block_on(async {
            let db = memory_store::open_replica_in(SUBDIR, config, "measure.db").await?;
            let conn = db.connect().await.map_err(te)?;
            for stmt in MEASURE_SCHEMA {
                conn.execute(stmt, ()).await.map_err(te)?;
            }
            Ok::<_, String>((db, conn))
        })?;
        Ok(Self { rt, db, conn })
    }
}

impl MeasureStore for TursoStore {
    fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String> {
        self.rt.block_on(turso_exec(
            &self.conn,
            UPSERT_COMPACTION,
            compaction_cells(row),
        ))
    }

    fn compaction_session(
        &mut self,
        machine_id: &str,
        boundary_uuid: &str,
    ) -> Result<Option<String>, String> {
        let rows = self.rt.block_on(turso_query(
            &self.conn,
            SELECT_SESSION,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(boundary_uuid.into()),
            ],
            1,
        ))?;
        rows.first().map(|r| text(&r[0])).transpose()
    }

    fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String> {
        self.rt
            .block_on(turso_query(
                &self.conn,
                &select_since(),
                vec![Cell::Int(since_ms)],
                21,
            ))?
            .iter()
            .map(|r| compaction_from_cells(r))
            .collect()
    }

    fn upsert_machine_host(
        &mut self,
        machine_id: &str,
        host: &str,
        seen_ms: i64,
    ) -> Result<(), String> {
        self.rt.block_on(turso_exec(
            &self.conn,
            UPSERT_HOST,
            vec![
                Cell::Text(machine_id.into()),
                Cell::Text(host.into()),
                Cell::Int(seen_ms),
            ],
        ))
    }

    fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String> {
        self.rt
            .block_on(turso_query(&self.conn, SELECT_HOSTS, vec![], 3))?
            .iter()
            .map(|r| host_from_cells(r))
            .collect()
    }

    fn push(&mut self) -> Result<(), String> {
        self.rt.block_on(self.db.push()).map_err(te)
    }

    fn pull(&mut self) -> Result<(), String> {
        self.rt.block_on(self.db.pull()).map(|_| ()).map_err(te)
    }
}

/// Open the configured replica under `measure.lock` and run `f` against it.
pub fn with_turso_store<T>(
    f: impl FnOnce(&mut dyn MeasureStore) -> Result<T, String>,
) -> Result<T, String> {
    with_config_lock(|config| {
        let mut store = TursoStore::open(&config)?;
        f(&mut store)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared SQL text must run on Turso's engine too, not only on SQLite.
    #[test]
    fn shared_sql_runs_on_turso_local() {
        let rt = memory_store::runtime().unwrap();
        rt.block_on(async {
            let db = turso::Builder::new_local(":memory:").build().await.unwrap();
            let conn = db.connect().unwrap();
            for stmt in MEASURE_SCHEMA {
                conn.execute(stmt, ()).await.unwrap();
            }
            let mk = |session: &str, t: i64| CompactionRow {
                machine_id: "m".into(),
                boundary_uuid: "b".into(),
                session_id: session.into(),
                time_ms: t,
                project_id: None,
                trigger: "auto".into(),
                pre_tokens: Some(5),
                post_tokens: None,
                pre: WindowCounts::default(),
                post: WindowCounts {
                    tool_calls: 3,
                    ..Default::default()
                },
                post_complete: true,
            };
            turso_exec(&conn, UPSERT_COMPACTION, compaction_cells(&mk("s1", 1)))
                .await
                .unwrap();
            turso_exec(&conn, UPSERT_COMPACTION, compaction_cells(&mk("s2", 2)))
                .await
                .unwrap();
            let rows = turso_query(&conn, &select_since(), vec![Cell::Int(0)], 21)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(compaction_from_cells(&rows[0]).unwrap(), mk("s2", 2));
            for seen in [500, 900, 700] {
                let p = vec![
                    Cell::Text("m".into()),
                    Cell::Text("h".into()),
                    Cell::Int(seen),
                ];
                turso_exec(&conn, UPSERT_HOST, p).await.unwrap();
            }
            let hosts = turso_query(&conn, SELECT_HOSTS, vec![], 3).await.unwrap();
            assert_eq!(host_from_cells(&hosts[0]).unwrap().last_seen_ms, 900);
        });
    }
}
