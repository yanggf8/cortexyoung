//! Usage events, uploaded to the measurement database under the ids claudecat already used.
//!
//! The event id is a hash of `source \0 machine \0 rowid \0 payload`, so "the same event" means
//! "the same bytes". claudecat uploaded the history under its own bytes; an id that differs in
//! one character uploads that history a second time, and a remote row cannot be taken back. Every
//! rule below is therefore copied from what claudecat *produced* (golden values in
//! `tests/telemetry.rs`), including two behaviours its source only implies:
//!
//! * the payload's keys come out **alphabetical**, not in column order (its JSON map is a
//!   `BTreeMap`, whatever the column order of the table), and
//! * `SELECT rowid, * FROM command_log` yields no `rowid` key at all -- SQLite names the rowid
//!   column `id` when the table's primary key is the rowid alias, and the second `id` overwrites
//!   the first -- so claudecat hashed **rowid 0** for every `cort_command` event.
//!
//! The JSON here is written by hand over `BTreeMap`, so cort's own `serde_json` feature set (which
//! decides whether *its* maps keep insertion order) cannot change a byte.
//!
//! Dry-run gate: the first upload for a machine is allowed only after [`verify_events`] has
//! compared local ids with the remote ones, or once the machine already has an event watermark.

use crate::data_dir::write_atomic;
use crate::measure::format_utc;
use crate::measure_progress::{self, Progress};
use crate::measure_store::MeasureStore;
use crate::usage::health_rows_after_at;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{json, Number, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SOURCE_COMMAND: &str = "cort_command";
pub const SOURCE_COST: &str = "cost_daily";
/// Index health observations. They have no claudecat history, so their id is a content key
/// (day + root + the row's payload without `id` or `ts`) rather than a rowid: re-observing the
/// same state on the same day -- including after `usage.db` is replaced -- must not store a
/// second row. `id` is left out because a replaced `usage.db` numbers its rows afresh.
pub const SOURCE_HEALTH: &str = "cort_index_health";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryEvent {
    pub source: String,
    pub event_id: String,
    pub machine_id: String,
    pub machine_source: String,
    pub project_id: Option<String>,
    pub time_ms: Option<i64>,
    pub payload: String,
    /// The row's real SQLite rowid (`None` for sources that have none). Not what the id hashes.
    pub rowid: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRun {
    pub matched: usize,
    /// `(local rowid, description)` for every paired row whose ids differ.
    pub mismatched: Vec<(i64, String)>,
    pub local_only: usize,
    pub remote_only: usize,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digest_parts(source: &str, machine_id: &str, key: &str, payload: &str) -> String {
    hex(&Sha256::digest(
        format!("{source}\0{machine_id}\0{key}\0{payload}").as_bytes(),
    ))
}

/// SHA-256 hex of `source\0machine_id\0rowid\0payload`.
pub fn event_id(source: &str, machine_id: &str, rowid: i64, payload: &str) -> String {
    digest_parts(source, machine_id, &rowid.to_string(), payload)
}

/// SHA-256 hex of `cort_index_health\0machine_id\0utc_day\0root\0payload_without_ts`.
///
/// `root` is `""` for a row that has none. The key goes through [`digest_parts`] as
/// `"<day>\0<root>"`, which puts exactly the separators this contract names between the parts.
pub fn health_event_id(
    machine_id: &str,
    utc_day: &str,
    root: &str,
    payload_without_ts: &str,
) -> String {
    digest_parts(
        SOURCE_HEALTH,
        machine_id,
        &format!("{utc_day}\0{root}"),
        payload_without_ts,
    )
}

/// The compact JSON `cort_command` payloads use: object keys sorted, no spaces -- rendered by the
/// one [`J`] serializer, so a health payload cannot differ from a command payload by one byte.
pub fn sorted_compact_json(value: &Value) -> String {
    // Every `serde_json::Value` deserialises into `J` (its numbers are always finite), so the
    // error arm is unreachable; it renders `null` instead of panicking if the invariant breaks.
    J::deserialize(value).unwrap_or(J::Null).render()
}

// ---------------------------------------------------------------------------------------------
// JSON the way claudecat printed it: objects sorted by key, compact.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(Number),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

impl J {
    fn int(&self) -> Option<i64> {
        match self {
            J::Num(n) => n.as_i64(),
            _ => None,
        }
    }
    fn text(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
    fn write(&self, out: &mut String) {
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            // The serializer, not `Number::to_string`: it is what claudecat's `Value::to_string` ran.
            J::Num(n) => out.push_str(&serde_json::to_string(n).unwrap_or_else(|_| "null".into())),
            J::Str(s) => out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())),
            J::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            J::Obj(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_else(|_| "\"\"".into()));
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
    fn render(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

impl<'de> Deserialize<'de> for J {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = J;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> Result<J, E> {
                Ok(J::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<J, E> {
                Ok(J::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<J, E> {
                Number::from_f64(v)
                    .map(J::Num)
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E>(self, v: &str) -> Result<J, E> {
                Ok(J::Str(v.to_string()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<J, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(J::Arr(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<J, A::Error> {
                let mut out = BTreeMap::new();
                while let Some((k, v)) = map.next_entry::<String, J>()? {
                    out.insert(k, v);
                }
                Ok(J::Obj(out))
            }
        }
        d.deserialize_any(V)
    }
}

/// SQLite value -> JSON, as claudecat converted it. A non-finite float has no JSON form: null.
fn json_of(value: ValueRef<'_>) -> J {
    match value {
        ValueRef::Null => J::Null,
        ValueRef::Integer(n) => J::Num(n.into()),
        ValueRef::Real(n) => Number::from_f64(n).map_or(J::Null, J::Num),
        ValueRef::Text(b) => J::Str(String::from_utf8_lossy(b).into_owned()),
        ValueRef::Blob(b) => J::Str(format!("blob:{}", hex(b))),
    }
}

// ---------------------------------------------------------------------------------------------
// cort_command: the local usage.db
// ---------------------------------------------------------------------------------------------

fn open_readonly(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// `(MACHINE_ID, MACHINE_ID_SOURCE)` from `_usage_meta`; each falls back on its own.
fn meta_stamp(conn: &Connection) -> (String, String) {
    let get = |key: &str| {
        conn.query_row(
            "SELECT value FROM _usage_meta WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "legacy_unknown".into())
    };
    (get("MACHINE_ID"), get("MACHINE_ID_SOURCE"))
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE name = ?2")
        .map_err(|e| e.to_string())?;
    stmt.exists((table, column)).map_err(|e| e.to_string())
}

/// The column names `SELECT rowid, * FROM command_log` returns, joined. They decide every payload
/// (hence every id), so the verify marker records them: a changed schema voids the verification.
pub fn command_log_columns(usage_db: &Path) -> Result<String, String> {
    let conn = open_readonly(usage_db)?;
    let stmt = conn
        .prepare("SELECT rowid, * FROM command_log")
        .map_err(|e| format!("{}: {e}", usage_db.display()))?;
    Ok(stmt.column_names().join(","))
}

/// Every `command_log` row (or those after `after_rowid`) as an event. A missing file is no events.
pub fn cort_events(
    usage_db: &Path,
    after_rowid: Option<i64>,
) -> Result<Vec<TelemetryEvent>, String> {
    read_events(usage_db, after_rowid.map_or(Scope::All, Scope::After), None)
}

/// At most `limit` events after `after_rowid`, in rowid order: one page of an upload.
pub fn cort_events_page(
    usage_db: &Path,
    after_rowid: Option<i64>,
    limit: usize,
) -> Result<Vec<TelemetryEvent>, String> {
    read_events(
        usage_db,
        after_rowid.map_or(Scope::All, Scope::After),
        Some(limit),
    )
}

/// Identity of the row at `rowid` that survives a schema change: its raw `ts` and `command`
/// values. The event id cannot serve, because it hashes every column, so adding a column to the
/// same database would change it and look like a replaced file. `None` if there is no such row.
pub fn row_fingerprint(usage_db: &Path, rowid: i64) -> Result<Option<String>, String> {
    if !usage_db.exists() {
        return Ok(None);
    }
    let conn = open_readonly(usage_db)?;
    let mut stmt = conn
        .prepare("SELECT ts, command FROM command_log WHERE rowid = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([rowid]).map_err(|e| e.to_string())?;
    let Some(row) = rows.next().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let part = |i: usize| -> Result<String, String> {
        Ok(match row.get_ref(i).map_err(|e| e.to_string())? {
            ValueRef::Null => "null".to_string(),
            ValueRef::Integer(n) => n.to_string(),
            ValueRef::Real(n) => n.to_string(),
            ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
            ValueRef::Blob(b) => format!("blob:{}", hex(b)),
        })
    };
    Ok(Some(format!("{}|{}", part(0)?, part(1)?)))
}

/// Raw `ts|root` for the health watermark. SQL NULL root is the empty string.
pub fn health_row_fingerprint(usage_db: &Path, rowid: i64) -> Result<Option<String>, String> {
    if !usage_db.exists() {
        return Ok(None);
    }
    let conn = open_readonly(usage_db)?;
    let mut stmt = conn
        .prepare("SELECT ts, COALESCE(root, '') FROM index_health_log WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([rowid]).map_err(|e| e.to_string())?;
    let Some(row) = rows.next().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let ts: i64 = row.get(0).map_err(|e| e.to_string())?;
    let root: String = row.get(1).map_err(|e| e.to_string())?;
    Ok(Some(format!("{ts}|{root}")))
}

/// One page of local health observations, carrying every `index_health_log` column in the payload.
pub fn health_events(
    usage_db: &Path,
    after_id: i64,
    limit: usize,
) -> Result<Vec<TelemetryEvent>, String> {
    if !usage_db.exists() {
        return Ok(Vec::new());
    }
    health_rows_after_at(usage_db, after_id, limit)?
        .into_iter()
        .map(|row| {
            let mut value = json!({
                "id": row.id, "ts": row.ts, "machine_id": row.machine_id,
                "machine_source": row.machine_source, "root": row.root,
                "local_project_id": row.local_project_id, "project_id": row.project_id,
                "status": row.status, "error": row.error, "fresh": row.fresh,
                "chunks": row.chunks, "relationships": row.relationships,
                "not_chunked": row.not_chunked, "real_gap": row.real_gap, "fts": row.fts,
            });
            let payload = sorted_compact_json(&value);
            if let Value::Object(fields) = &mut value {
                fields.remove("id");
                fields.remove("ts");
            }
            let utc = format_utc(row.ts);
            let event_id = health_event_id(
                &row.machine_id,
                &utc[..10],
                row.root.as_deref().unwrap_or(""),
                &sorted_compact_json(&value),
            );
            Ok(TelemetryEvent {
                source: SOURCE_HEALTH.into(),
                event_id,
                machine_id: row.machine_id,
                machine_source: row.machine_source,
                project_id: row.project_id,
                time_ms: Some(row.ts),
                payload,
                rowid: Some(row.id),
            })
        })
        .collect()
}

enum Scope {
    All,
    After(i64),
}

fn read_events(
    usage_db: &Path,
    scope: Scope,
    limit: Option<usize>,
) -> Result<Vec<TelemetryEvent>, String> {
    if !usage_db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_readonly(usage_db)?;
    let stamp = meta_stamp(&conn);
    let row_stamp = has_column(&conn, "command_log", "machine_id")?
        && has_column(&conn, "command_log", "machine_source")?;
    let filter = match scope {
        Scope::All => "",
        Scope::After(_) => " WHERE rowid > ?1",
    };
    let cap = limit.map_or(String::new(), |n| format!(" LIMIT {n}"));
    let sql = format!("SELECT rowid, * FROM command_log{filter} ORDER BY rowid{cap}");
    let sql = sql.as_str();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("{}: {e}", usage_db.display()))?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = match scope {
        Scope::All => stmt.query([]),
        Scope::After(r) => stmt.query([r]),
    }
    .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let real_rowid = match row.get_ref(0).map_err(|e| e.to_string())? {
            ValueRef::Integer(n) => n,
            other => return Err(format!("rowid is not an integer: {other:?}")),
        };
        // Later columns overwrite earlier ones of the same name, exactly as claudecat's map did.
        let mut object: BTreeMap<String, J> = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            object.insert(
                name.clone(),
                json_of(row.get_ref(i).map_err(|e| e.to_string())?),
            );
        }
        let hashed_rowid = object.get("rowid").and_then(J::int).unwrap_or(0);
        let row_machine = object.get("machine_id").and_then(J::text);
        let legacy = !row_stamp || row_machine.is_none_or(|id| id == "legacy_unknown");
        let (machine_id, machine_source) = if row_stamp && !legacy {
            (
                row_machine.unwrap_or("legacy_unknown").to_string(),
                object
                    .get("machine_source")
                    .and_then(J::text)
                    .unwrap_or("legacy_no_stamp")
                    .to_string(),
            )
        } else {
            stamp.clone()
        };
        let project_id = object
            .get("project_id")
            .and_then(J::text)
            .map(str::to_string);
        let time_ms = object.get("ts").and_then(J::int);
        // Pre-v2 rows were uploaded before these columns existed: their payload and id stay as
        // they were then.
        if legacy && row_stamp {
            object.remove("machine_id");
            object.remove("machine_source");
        }
        let payload = J::Obj(object).render();
        out.push(TelemetryEvent {
            source: SOURCE_COMMAND.into(),
            event_id: event_id(SOURCE_COMMAND, &machine_id, hashed_rowid, &payload),
            machine_id,
            machine_source,
            project_id,
            time_ms,
            payload,
            rowid: Some(real_rowid),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// cost_daily: ~/.local/share/cort-evals/claude-cost/*.json
// ---------------------------------------------------------------------------------------------

/// One event per `*.json` report; the id hashes the file name where commands hash a rowid.
pub fn cost_events(dir: &Path) -> Result<Vec<TelemetryEvent>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for item in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = item.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let body = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let report: J =
            serde_json::from_str(&body).map_err(|e| format!("{}: {e}", path.display()))?;
        let J::Obj(fields) = &report else {
            return Err(format!(
                "{}: cost report has no machine stamp",
                path.display()
            ));
        };
        let machine = fields
            .get("machine")
            .ok_or_else(|| format!("{}: cost report has no machine stamp", path.display()))?;
        let machine_id = match machine {
            J::Obj(m) => m.get("id").and_then(J::text),
            _ => None,
        }
        .ok_or_else(|| format!("{}: cost report machine id missing", path.display()))?
        .to_string();
        let machine_source = match machine {
            J::Obj(m) => m.get("source").and_then(J::text),
            _ => None,
        }
        .unwrap_or("unknown")
        .to_string();
        let payload = report.render();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        out.push(TelemetryEvent {
            source: SOURCE_COST.into(),
            event_id: digest_parts(SOURCE_COST, &machine_id, &name, &payload),
            machine_id,
            machine_source,
            project_id: None,
            time_ms: None,
            payload,
            rowid: None,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// project aliases
// ---------------------------------------------------------------------------------------------

/// Distinct `(machine_id, project_id)` over the WHOLE `command_log`, attributed to machines by the
/// same rules as [`cort_events`] (a row's own stamp unless it is missing or `legacy_unknown`, then
/// `_usage_meta`). A light pass: no payloads, no hashing. Machines `legacy_unknown`/`unknown` drop out.
fn observed_pairs(usage_db: &Path) -> Result<Vec<(String, String)>, String> {
    if !usage_db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_readonly(usage_db)?;
    let stamp = meta_stamp(&conn).0;
    let row_stamp = has_column(&conn, "command_log", "machine_id")?
        && has_column(&conn, "command_log", "machine_source")?;
    let machine = if row_stamp {
        "CASE WHEN typeof(machine_id) = 'text' AND machine_id <> 'legacy_unknown' \
         THEN machine_id END"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT DISTINCT {machine}, project_id FROM command_log WHERE typeof(project_id) = 'text'"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut out = std::collections::BTreeSet::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let machine = row
            .get::<_, Option<String>>(0)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| stamp.clone());
        let project: String = row.get(1).map_err(|e| e.to_string())?;
        if machine != "legacy_unknown" && machine != "unknown" {
            out.insert((machine, project));
        }
    }
    Ok(out.into_iter().collect())
}

/// `(machine_id, local_project_id, shared_project_id)` for every (machine, project) pair anywhere
/// in `usage_db` whose project has an index database beside it and a git origin. Recomputed from
/// the whole file on every call, so an index or origin that appears after the events were uploaded
/// still gets its alias. Unresolvable projects are left out, as claudecat left them out.
pub fn project_aliases(usage_db: &Path) -> Result<Vec<(String, String, String)>, String> {
    let Some(cache_dir) = usage_db.parent().filter(|d| d.is_dir()) else {
        return Ok(Vec::new());
    };
    let mut observed: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for (machine, project) in observed_pairs(usage_db)? {
        observed.entry(project).or_default().insert(machine);
    }
    let mut aliases = std::collections::BTreeSet::new();
    for item in fs::read_dir(cache_dir).map_err(|e| e.to_string())? {
        let path = item.map_err(|e| e.to_string())?.path();
        if path.extension().is_none_or(|ext| ext != "db")
            || path.file_name().is_some_and(|name| name == "usage.db")
        {
            continue;
        }
        let Ok(conn) = open_readonly(&path) else {
            continue;
        };
        let Ok(mut stmt) = conn.prepare("SELECT project_id, path FROM projects") else {
            continue;
        };
        let Ok(mut rows) = stmt.query([]) else {
            continue;
        };
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let local: String = row.get(0).map_err(|e| e.to_string())?;
            let project_path: String = row.get(1).map_err(|e| e.to_string())?;
            let Some(machines) = observed.get(local.as_str()) else {
                continue;
            };
            let Ok(canonical) = fs::canonicalize(&project_path) else {
                continue;
            };
            if crate::db::project_id_for(&canonical.to_string_lossy()) != local {
                continue;
            }
            if let Ok(shared) = crate::memory_store::project_from_git_origin(&canonical) {
                for machine in machines {
                    aliases.insert((machine.clone(), local.clone(), shared.clone()));
                }
            }
        }
    }
    Ok(aliases.into_iter().collect())
}

// ---------------------------------------------------------------------------------------------
// dry run
// ---------------------------------------------------------------------------------------------

/// Compare local `cort_command` events with what the store already holds, without writing.
///
/// Pairing key: `(machine_id, rowid)` -- the id is the thing under test, so it cannot also be the
/// key. Every machine that has local events must overlap the remote at least once; zero overlap
/// proves nothing about the ids. Any paired row whose ids differ refuses the whole run.
pub fn dry_run(store: &mut dyn MeasureStore, local: &[TelemetryEvent]) -> Result<DryRun, String> {
    let mut by_machine: BTreeMap<&str, Vec<&TelemetryEvent>> = BTreeMap::new();
    for e in local.iter().filter(|e| e.source == SOURCE_COMMAND) {
        by_machine.entry(&e.machine_id).or_default().push(e);
    }
    let mut report = DryRun {
        matched: 0,
        mismatched: Vec::new(),
        local_only: 0,
        remote_only: 0,
    };
    let mut no_overlap = Vec::new();
    for (machine, events) in &by_machine {
        let remote = store.remote_ids(machine, SOURCE_COMMAND)?;
        let mut overlap = 0usize;
        let mut seen = std::collections::HashSet::new();
        for e in events {
            let Some(rowid) = e.rowid else { continue };
            seen.insert(rowid);
            match remote.get(&rowid) {
                None => report.local_only += 1,
                Some(rid) if *rid == e.event_id => {
                    overlap += 1;
                    report.matched += 1;
                }
                Some(rid) => {
                    overlap += 1;
                    let remote_payload = store
                        .remote_payload(machine, SOURCE_COMMAND, rid)?
                        .unwrap_or_else(|| "<gone>".into());
                    report.mismatched.push((
                        rowid,
                        format!(
                            "machine {machine} rowid {rowid}: local id {} remote id {rid}\n  local  payload {}\n  remote payload {remote_payload}",
                            e.event_id, e.payload
                        ),
                    ));
                }
            }
        }
        report.remote_only += remote.keys().filter(|r| !seen.contains(r)).count();
        if overlap == 0 {
            no_overlap.push(*machine);
        }
    }
    if !report.mismatched.is_empty() {
        let shown: Vec<&str> = report
            .mismatched
            .iter()
            .take(5)
            .map(|(_, d)| d.as_str())
            .collect();
        return Err(format!(
            "id mismatch on {} of {} paired rows; refusing to upload. First {}:\n{}",
            report.mismatched.len(),
            report.matched + report.mismatched.len(),
            shown.len(),
            shown.join("\n")
        ));
    }
    if !no_overlap.is_empty() {
        return Err(format!(
            "no overlap: nothing proves the ids match (machine(s) {} have local events and none \
             of them exist remotely)",
            no_overlap.join(", ")
        ));
    }
    Ok(report)
}

// ---------------------------------------------------------------------------------------------
// the verified marker, upload, and the gate
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UploadInputs {
    pub usage_db: PathBuf,
    pub cost_dir: PathBuf,
    /// Local file written by a successful `measure verify`; see [`verify_events`].
    pub marker_path: PathBuf,
    /// The machine running this, whose events watermark is kept (rows in a carried-over usage.db
    /// stamped with another machine still upload under their own `machine_id`).
    pub machine_id: String,
}

pub fn default_marker_path() -> PathBuf {
    crate::data_dir::data_dir().join("measure-events-verified.json")
}

pub fn default_cost_dir() -> PathBuf {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => PathBuf::from(d),
        None => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".local/share"))
            .unwrap_or_else(|| PathBuf::from(".local/share")),
    };
    base.join("cort-evals/claude-cost")
}

/// Whether a dry-run has passed for this machine and this `command_log` column layout.
///
/// The marker certifies the ALGORITHM for `(machine, column layout)`, not a particular file.
/// What a dry-run proves is that the id function, run over this layout, reproduces the ids the
/// remote already holds. A replacement `usage.db` with the same layout is hashed by that same
/// verified function, so its ids are right and `INSERT OR IGNORE` absorbs any repeats. Demanding a
/// fresh dry-run per file would also block a recreated database forever: its rows are new, so it
/// can never overlap the remote and the dry-run's overlap rule could never pass again. A changed
/// layout does void it, because that changes the function's output.
fn marker_valid(input: &UploadInputs) -> bool {
    let Ok(text) = fs::read_to_string(&input.marker_path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    v["machine_id"] == input.machine_id.as_str()
        && command_log_columns(&input.usage_db).is_ok_and(|c| v["columns"] == c.as_str())
}

/// Compare every local `cort_command` event with the remote, and on success write the
/// marker that lets the first real upload through.
pub fn verify_events(
    store: &mut dyn MeasureStore,
    input: &UploadInputs,
    now_ms: i64,
) -> Result<DryRun, String> {
    let local = cort_events(&input.usage_db, None)?;
    let report = dry_run(store, &local)?;
    if report.matched > 0 {
        let marker = json!({
            "machine_id": input.machine_id,
            "columns": command_log_columns(&input.usage_db)?,
            "verified_ms": now_ms,
            "matched": report.matched,
        });
        write_atomic(&input.marker_path, &marker.to_string())
            .map_err(|e| format!("cannot write {}: {e}", input.marker_path.display()))?;
    }
    Ok(report)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsReport {
    Uploaded {
        scanned: usize,
        inserted: usize,
        aliases: usize,
        /// Everything was read, not just the rows after the watermark: either there was no
        /// watermark yet (verified by dry-run) or `reason` says why it could not be trusted.
        full_range: bool,
        /// Why a watermark was not used: `watermark_row_changed` (the file was replaced and the row
        /// at that rowid is a different event) or `watermark_row_missing` (replaced with fewer
        /// rows, or the 90-day prune removed it).
        reason: Option<String>,
        watermark: Option<i64>,
    },
    /// The first upload for this machine has not been verified by a dry-run. Nothing was uploaded.
    Refused(String),
    /// Reading or writing failed. Nothing is claimed uploaded beyond what the watermark says.
    Failed(String),
}

impl EventsReport {
    pub fn is_ok(&self) -> bool {
        matches!(self, EventsReport::Uploaded { .. })
    }

    pub fn to_json(&self) -> Value {
        match self {
            EventsReport::Uploaded {
                scanned,
                inserted,
                aliases,
                full_range,
                reason,
                watermark,
            } => json!({"status":"uploaded","scanned":scanned,"inserted":inserted,
                "aliases":aliases,"full_range":full_range,"reason":reason,"watermark":watermark}),
            EventsReport::Refused(r) => json!({"status":"refused","reason":r}),
            EventsReport::Failed(e) => json!({"status":"failed","error":e}),
        }
    }
}

/// Rows read and inserted per page of an upload.
const PAGE: usize = 2000;

/// Write new events into the store. Never returns an error and
/// never panics: a refusal or a failure is a value, so the compaction record beside it still runs.
///
/// With a watermark, the row at that rowid must still be the same row (same raw `ts` and
/// `command`, see [`row_fingerprint`]; the computed event id would change with any new column and
/// re-upload history under new ids); then only later rows are read. If it is different or gone,
/// `usage.db` was replaced (rowids restart) or pruned, so the whole file is read and `INSERT OR IGNORE` dedups. That fallback is
/// not re-gated: see [`marker_valid`]. With no watermark, the whole file is read, but only if
/// [`verify_events`] has passed for this machine and column layout.
pub fn upload_events(store: &mut dyn MeasureStore, input: &UploadInputs) -> EventsReport {
    let path = input.marker_path.with_file_name("measure-progress.json");
    let result = measure_progress::open(store, &path, &input.machine_id, Some(&input.usage_db))
        .and_then(|(mut progress, _)| try_upload(store, input, &mut progress, &path));
    match result {
        Ok(r) => r,
        Err(e) => EventsReport::Failed(e),
    }
}

pub fn upload_events_with_progress(
    store: &mut dyn MeasureStore,
    input: &UploadInputs,
    progress: &mut Progress,
    path: &Path,
) -> EventsReport {
    try_upload(store, input, progress, path).unwrap_or_else(EventsReport::Failed)
}

/// Copy local health observations to the store without the command dry-run gate. `progress_path`
/// is the progress file to use. Production collection goes through `record_with_scan_until`
/// instead; usage.db lives in the cache directory, so the path is never derived from it.
pub fn upload_health_events(
    store: &mut dyn MeasureStore,
    usage_db: &Path,
    machine_id: &str,
    progress_path: &Path,
) -> EventsReport {
    let path = progress_path;
    let result = measure_progress::open(store, path, machine_id, Some(usage_db)).and_then(
        |(mut progress, _)| try_upload_health(store, usage_db, machine_id, &mut progress, path),
    );
    match result {
        Ok(report) => report,
        Err(error) => EventsReport::Failed(error),
    }
}

pub fn upload_health_events_with_progress(
    store: &mut dyn MeasureStore,
    usage_db: &Path,
    machine_id: &str,
    progress: &mut Progress,
    path: &Path,
) -> EventsReport {
    try_upload_health(store, usage_db, machine_id, progress, path)
        .unwrap_or_else(EventsReport::Failed)
}

type Fingerprint = fn(&Path, i64) -> Result<Option<String>, String>;
type EventPage = fn(&Path, Option<i64>, usize) -> Result<Vec<TelemetryEvent>, String>;
type WatermarkCheck = (Option<i64>, Option<String>, Option<i64>);

struct UploadSource<'a> {
    usage_db: &'a Path,
    machine: &'a str,
    source: &'static str,
    fingerprint: Fingerprint,
    read_page: EventPage,
}

fn health_events_page(
    path: &Path,
    after: Option<i64>,
    limit: usize,
) -> Result<Vec<TelemetryEvent>, String> {
    health_events(path, after.unwrap_or(0), limit)
}

fn upload_cursor(progress: &Progress, config: &UploadSource<'_>) -> Result<WatermarkCheck, String> {
    let stored = progress.watermark(config.machine, config.source);
    let (cursor, reason) = match &stored {
        Some(prior) => match (config.fingerprint)(config.usage_db, prior.rowid)? {
            Some(current) if current == prior.fingerprint => (Some(prior.rowid), None),
            Some(_) => (None, Some("watermark_row_changed".into())),
            None => (None, Some("watermark_row_missing".into())),
        },
        None => (None, None),
    };
    Ok((cursor, reason, stored.map(|prior| prior.rowid)))
}

fn upload_pages(
    store: &mut dyn MeasureStore,
    config: &UploadSource<'_>,
    progress: &mut Progress,
    path: &Path,
    mut cursor: Option<i64>,
    mut watermark: Option<i64>,
) -> Result<(usize, usize, Option<i64>), String> {
    let mut scanned = 0;
    let mut inserted = 0;
    loop {
        let page = (config.read_page)(config.usage_db, cursor, PAGE)?;
        let Some(last) = page.last() else { break };
        scanned += page.len();
        inserted += store.insert_events(&page)?;
        let last_rowid = last.rowid.ok_or("event without a rowid")?;
        let current = (config.fingerprint)(config.usage_db, last_rowid)?
            .ok_or("watermark row vanished while uploading")?;
        // A prune hint only: failing to write it keeps rows longer, the safe direction, so it
        // never fails an upload that already landed.
        let _ = crate::usage::uploaded_floor_at(config.usage_db, config.source, last_rowid);
        progress.set_watermark(config.machine, config.source, last_rowid, current);
        progress.save(path)?;
        watermark = Some(last_rowid);
        cursor = Some(last_rowid);
        if page.len() < PAGE {
            break;
        }
    }
    Ok((scanned, inserted, watermark))
}

fn try_upload_health(
    store: &mut dyn MeasureStore,
    usage_db: &Path,
    machine: &str,
    progress: &mut Progress,
    path: &Path,
) -> Result<EventsReport, String> {
    let config = UploadSource {
        usage_db,
        machine,
        source: SOURCE_HEALTH,
        fingerprint: health_row_fingerprint,
        read_page: health_events_page,
    };
    let (cursor, reason, watermark) = upload_cursor(progress, &config)?;
    let full_range = cursor.is_none();
    let (scanned, inserted, watermark) =
        upload_pages(store, &config, progress, path, cursor, watermark)?;
    Ok(EventsReport::Uploaded {
        scanned,
        inserted,
        aliases: 0,
        full_range,
        reason,
        watermark,
    })
}

fn try_upload(
    store: &mut dyn MeasureStore,
    input: &UploadInputs,
    progress: &mut Progress,
    path: &Path,
) -> Result<EventsReport, String> {
    let machine = input.machine_id.as_str();
    let config = UploadSource {
        usage_db: &input.usage_db,
        machine,
        source: SOURCE_COMMAND,
        fingerprint: row_fingerprint,
        read_page: cort_events_page,
    };
    let (cursor, reason, watermark) = upload_cursor(progress, &config)?;
    if watermark.is_none() && !marker_valid(input) {
        return Ok(EventsReport::Refused(format!(
            "no event watermark for machine {machine} and no passing verify: run `cort \
             measure verify` once; nothing was uploaded"
        )));
    }
    let full_range = cursor.is_none();
    let mut scanned = 0usize;
    let mut inserted = 0usize;
    // Cost reports have no rowid to watermark; INSERT OR IGNORE is their only dedup, so they go
    // under the same gate.
    let cost = cost_events(&input.cost_dir)?;
    if !cost.is_empty() {
        scanned += cost.len();
        inserted += store.insert_events(&cost)?;
    }
    // The shared pager advances local progress only after each remote page succeeds.
    let (page_scanned, page_inserted, watermark) =
        upload_pages(store, &config, progress, path, cursor, watermark)?;
    scanned += page_scanned;
    inserted += page_inserted;
    // Every run, over the whole file: an index or git origin may have appeared since the events
    // were uploaded.
    let aliases = project_aliases(&input.usage_db)?;
    store.upsert_aliases(&aliases)?;
    Ok(EventsReport::Uploaded {
        scanned,
        inserted,
        aliases: aliases.len(),
        full_range,
        reason,
        watermark,
    })
}
