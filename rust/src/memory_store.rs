//! Cortexyoung-owned cross-machine project memory backed by a shared Turso database.

use crate::memory::{self, MemoryEntry, MemoryLedger, MemoryRecall};
use crate::remote::{Cell, ErrorKind, RemoteDb, Stmt, Transport, UreqTransport};
use crate::{notes, notes_upload, usage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))";
const CREATE_ALIASES: &str = "CREATE TABLE IF NOT EXISTS project_aliases (old_project TEXT NOT NULL, new_project TEXT NOT NULL, created_at TEXT NOT NULL, machine TEXT NOT NULL, PRIMARY KEY (old_project, new_project))";
const COMPONENT: &str = "WITH RECURSIVE g(p) AS (SELECT ? UNION SELECT CASE WHEN a.old_project = g.p THEN a.new_project ELSE a.old_project END FROM project_aliases a JOIN g ON a.old_project = g.p OR a.new_project = g.p) SELECT p FROM g";
#[derive(Debug, Serialize)]
pub struct MemoryConflict {
    pub key: String,
    pub active_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryStatus {
    pub project_id: String,
    pub project_ids: Vec<String>,
    pub project_id_source: &'static str,
    pub database_url: String,
    pub remote: &'static str,
    pub total_entries: usize,
    pub active_entries: usize,
    pub conflicts: Vec<MemoryConflict>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LedgerError {
    Message(String),
    Remote(ErrorKind),
    NotRecorded {
        kind: ErrorKind,
    },
    CommittedUnverified {
        id: String,
        kind: ErrorKind,
    },
    OutcomeUnknown {
        id: String,
        key: Option<String>,
        kind: ErrorKind,
    },
    SupersedesMissing {
        superseded: Vec<String>,
    },
    Conflict {
        id: String,
        key: String,
        active_ids: Vec<String>,
    },
}

impl From<String> for LedgerError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for LedgerError {
    fn from(value: &str) -> Self {
        Self::Message(value.into())
    }
}

pub struct Ledger {
    db: RemoteDb,
}

impl Ledger {
    pub fn open() -> Result<Self, LedgerError> {
        let config = config()?;
        let timeout =
            Duration::from_millis(crate::collect::CollectConfig::from_env().net_timeout_ms);
        Self::with_transport(
            &config.url,
            &config.token,
            Box::new(UreqTransport { timeout }),
        )
    }

    #[doc(hidden)]
    pub fn with_transport(
        url: &str,
        token: &str,
        transport: Box<dyn Transport>,
    ) -> Result<Self, LedgerError> {
        Ok(Self {
            db: RemoteDb::new(url, token, transport).map_err(|e| LedgerError::Remote(e.kind))?,
        })
    }

    fn read_bodies(&self, project: &str) -> Result<Vec<String>, LedgerError> {
        let sql = format!("SELECT body FROM project_memory WHERE project IN ({COMPONENT})");
        let results = self
            .db
            .pipeline(&[
                Stmt::new(CREATE_TABLE, vec![]),
                Stmt::new(CREATE_ALIASES, vec![]),
                Stmt::new(&sql, vec![Cell::Text(project.into())]),
            ])
            .map_err(|e| LedgerError::Remote(e.kind))?;
        let bodies: Vec<String> = results[2]
            .rows
            .iter()
            .map(|row| match row.first() {
                Some(Cell::Text(body)) => Ok(body.clone()),
                _ => Err(LedgerError::Message("invalid remote memory row".into())),
            })
            .collect::<Result<_, _>>()?;
        merge_bodies(bodies).map_err(LedgerError::Message)
    }

    fn project_ids(&self, project: &str) -> Result<Vec<String>, LedgerError> {
        let result = self
            .db
            .execute(COMPONENT, vec![Cell::Text(project.into())])
            .map_err(|e| LedgerError::Remote(e.kind))?;
        let mut ids = result
            .rows
            .iter()
            .map(|row| match row.first() {
                Some(Cell::Text(id)) => Ok(id.clone()),
                _ => Err(LedgerError::Message("invalid remote project id row".into())),
            })
            .collect::<Result<Vec<_>, _>>()?;
        ids.sort();
        ids.retain(|id| id != project);
        ids.insert(0, project.to_string());
        Ok(ids)
    }

    pub fn read(&self, project: &str) -> Result<MemoryLedger, LedgerError> {
        ledger_from_bodies(self.read_bodies(project)?).map_err(|message| {
            if message.contains("supersedes missing id") {
                LedgerError::Message(format!(
                    "{message}; re-add the alias with cort memory alias add"
                ))
            } else {
                LedgerError::Message(message)
            }
        })
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Config {
    pub(crate) url: String,
    pub(crate) token: String,
}

pub(crate) fn digest(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) fn config_path() -> PathBuf {
    crate::data_dir::data_dir().join("turso-memory.json")
}

/// Validate a URL/token pair the way every cort Turso client does.
pub(crate) fn validated_config(url: &str, token: &str) -> Result<Config, String> {
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("turso://") || url.starts_with("libsql://"))
        || token.trim().is_empty()
    {
        return Err("provide a Turso database URL and a nonempty auth token".into());
    }
    Ok(Config {
        url: url.to_string(),
        token: token.trim().to_string(),
    })
}

/// Write a credential file: private to its owner, never through a symlink, atomically renamed.
/// `label` names the file in the symlink error.
pub(crate) fn write_private_config(
    path: &Path,
    config: &Config,
    label: &str,
) -> Result<PathBuf, String> {
    let body = serde_json::to_string(config).map_err(|e| e.to_string())?;
    // A credential file is local to this machine and private to its owner.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{label} must not be a symlink"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), stamp));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    crate::data_dir::write_atomic(path, &body).map_err(|e| e.to_string())?;
    Ok(path.to_path_buf())
}

pub fn configure(url: &str, token: &str) -> Result<PathBuf, String> {
    let config = validated_config(url, token)?;
    write_private_config(&config_path(), &config, "Turso memory config")
}

/// Move a machine's existing ClaudeCat connection to cort without printing the credential.
/// Both clients can address the same remote `project_memory` rows during the handoff.
pub fn import_config(from: &Path) -> Result<PathBuf, String> {
    if fs::symlink_metadata(from).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("memory config source must not be a symlink".into());
    }
    let body = fs::read_to_string(from).map_err(|e| format!("cannot read memory config: {e}"))?;
    let old: Config =
        serde_json::from_str(&body).map_err(|e| format!("invalid memory config: {e}"))?;
    configure(&old.url, &old.token)
}

fn config() -> Result<Config, String> {
    read_config(
        &config_path(),
        "Turso memory config",
        "Turso memory is not configured; run `cort memory configure --url URL --token-stdin`",
    )
}

/// Session hook's absent config is distinct from an unreadable or malformed config.
pub fn session_config() -> Result<Option<(String, String)>, String> {
    let path = config_path();
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    config().map(|c| Some((c.url, c.token)))
}

/// Legacy replica directory name, retained to find its lock and detect older clients.
pub(crate) const SUBDIR: &str = "turso-memory";

/// Read a credential file written by `write_private_config`, refusing a symlink.
pub(crate) fn read_config(path: &Path, label: &str, unconfigured: &str) -> Result<Config, String> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{label} must not be a symlink"));
    }
    let body = fs::read_to_string(path).map_err(|_| unconfigured.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("invalid {label}: {e}"))
}

pub fn project(root: &Path) -> Result<String, String> {
    for name in ["CORT_MEMORY_PROJECT", "CLAUDECAT_MEMORY_PROJECT"] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Ok(digest(value.trim()));
            }
        }
    }
    project_from_git_origin(root)
}

/// Resolve origin once: the ledger may use an explicit project, while notes always use origin.
pub fn session_project_ids(root: &Path) -> Result<(String, String), String> {
    let origin = project_from_git_origin(root).ok();
    let ledger = ["CORT_MEMORY_PROJECT", "CLAUDECAT_MEMORY_PROJECT"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
        .map(|v| digest(v.trim()))
        .or_else(|| origin.clone())
        .ok_or_else(|| "project needs an origin remote or CORT_MEMORY_PROJECT".to_string())?;
    // Without an origin, an explicit ledger project remains readable; no notes use an override.
    Ok((ledger, origin.unwrap_or_default()))
}

/// Resolve an indexed project's identity from its own origin, ignoring the process-wide
/// override that applies only to the current memory command's chosen project.
pub fn project_from_git_origin(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot identify project Git remote: {e}"))?;
    if !output.status.success() {
        return Err("project needs an origin remote or CORT_MEMORY_PROJECT".into());
    }
    let remote = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let remote = remote.trim();
    if remote.is_empty() {
        return Err("empty origin remote".into());
    }
    Ok(digest(&normalize_remote(remote)))
}

fn project_source() -> &'static str {
    if ["CORT_MEMORY_PROJECT", "CLAUDECAT_MEMORY_PROJECT"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|v| !v.trim().is_empty()))
    {
        "explicit"
    } else {
        "git_origin"
    }
}

fn normalize_remote(remote: &str) -> String {
    // Match common Git SSH and HTTPS spellings. A caller can set an explicit project key
    // when its host has case-sensitive paths or uses another remote syntax.
    let remote = remote.trim().trim_end_matches(".git");
    let normalized = if let Some(s) = remote.strip_prefix("git@") {
        s.replace(':', "/")
    } else if let Some(s) = remote.strip_prefix("ssh://git@") {
        s.to_string()
    } else if let Some(s) = remote.strip_prefix("https://") {
        s.to_string()
    } else {
        remote.to_string()
    };
    normalized.to_lowercase()
}

fn require_committed_source(root: &Path, source: &Path) -> Result<(), String> {
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(source)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot check source in Git: {e}"))?;
    if !tracked.status.success() {
        return Err("cloud memory source must be a tracked, committed file".into());
    }
    let clean = Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--"])
        .arg(source)
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot compare source with HEAD: {e}"))?;
    if !clean.success() {
        return Err("cloud memory source has uncommitted changes; commit it first".into());
    }
    Ok(())
}

fn merge_bodies(bodies: Vec<String>) -> Result<Vec<String>, String> {
    let mut groups: BTreeMap<String, Vec<(MemoryEntry, String)>> = BTreeMap::new();
    for body in bodies {
        let entry: MemoryEntry =
            serde_json::from_str(&body).map_err(|e| format!("invalid remote memory row: {e}"))?;
        let group = groups.entry(entry.id.clone()).or_default();
        if group.iter().any(|(_, old)| !same_claim(old, &body)) {
            return Err(format!(
                "memory id collision: {} is stored under two aliased projects with different content",
                entry.id
            ));
        }
        group.push((entry, body));
    }
    Ok(groups
        .into_values()
        .filter_map(|group| {
            group
                .into_iter()
                .max_by_key(|(entry, _)| {
                    usize::from(entry.recorded_machine.is_some())
                        + usize::from(entry.recorded_machine_source.is_some())
                })
                .map(|(_, body)| body)
        })
        .collect())
}

fn ledger_from_bodies(bodies: Vec<String>) -> Result<MemoryLedger, String> {
    let mut ledger = MemoryLedger::default();
    for body in bodies {
        ledger.entries.push(
            serde_json::from_str(&body).map_err(|e| format!("invalid remote memory row: {e}"))?,
        );
    }
    // Cloud rows have no append order. Order by supersession, not machine clocks: a correction
    // written on a machine with a slow clock can legitimately predate the entry it replaces.
    ledger.entries.sort_by(|a, b| a.id.cmp(&b.id));
    let mut pending = std::mem::take(&mut ledger.entries);
    let all_ids: BTreeSet<_> = pending.iter().map(|entry| entry.id.clone()).collect();
    for entry in &pending {
        for old in entry.supersedes.iter().chain(entry.resolves.iter()) {
            if !all_ids.contains(old) {
                return Err(format!("memory {} supersedes missing id {old}", entry.id));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|entry| {
            entry
                .supersedes
                .iter()
                .chain(entry.resolves.iter())
                .all(|old| seen.contains(old))
        }) else {
            return Err("memory supersession has a missing id or cycle".into());
        };
        let entry = pending.remove(index);
        seen.insert(entry.id.clone());
        ledger.entries.push(entry);
    }
    memory::validate(&ledger)?;
    Ok(ledger)
}

fn not_recorded_read(error: LedgerError) -> LedgerError {
    match error {
        LedgerError::Remote(kind) => LedgerError::NotRecorded { kind },
        other => other,
    }
}

/// Whether a failed write may still have committed. Timeout and Unreachable can arrive after the
/// server applied the statement; so can `Sql`, which also names an undecodable or mis-shaped
/// response and a 5xx whose body carries an `SQLITE_*` code. The other kinds are refusals the
/// server reports before or instead of applying it.
fn outcome_unknown(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Timeout | ErrorKind::Unreachable | ErrorKind::Sql
    )
}

fn entry_args(project: &str, entry: &MemoryEntry) -> Result<Vec<Cell>, LedgerError> {
    let body = serde_json::to_string(entry).map_err(|e| LedgerError::Message(e.to_string()))?;
    Ok(vec![
        Cell::Text(project.into()),
        Cell::Text(entry.id.clone()),
        Cell::Text(body),
    ])
}

impl Ledger {
    fn write_checked(
        &self,
        project: &str,
        entry: &MemoryEntry,
        sql: &str,
        args: Vec<Cell>,
        superseded: &[String],
        resolution: bool,
    ) -> Result<(), LedgerError> {
        match self.db.execute(sql, args) {
            Ok(result) if result.affected_row_count == 1 => {}
            Ok(result) if result.affected_row_count == 0 => {
                return if superseded.is_empty() {
                    Err(LedgerError::Message("insert affected no row".into()))
                } else {
                    Err(LedgerError::SupersedesMissing {
                        superseded: superseded.to_vec(),
                    })
                };
            }
            Ok(_) => {
                return Err(LedgerError::Message(
                    "insert affected more than one row".into(),
                ))
            }
            // Ask for the row itself, not the merged ledger: through an alias the same id may
            // already be visible under another project while nothing was written under this one.
            Err(error) if outcome_unknown(error.kind) => match self.db.execute(
                "SELECT 1 FROM project_memory WHERE project = ? AND id = ?",
                vec![Cell::Text(project.into()), Cell::Text(entry.id.clone())],
            ) {
                Ok(found) if !found.rows.is_empty() => {}
                Ok(_) => return Err(LedgerError::NotRecorded { kind: error.kind }),
                Err(_) => {
                    return Err(LedgerError::OutcomeUnknown {
                        id: entry.id.clone(),
                        key: Some(entry.key.clone()),
                        kind: error.kind,
                    })
                }
            },
            Err(error) => return Err(LedgerError::NotRecorded { kind: error.kind }),
        }
        let after = match self.read(project) {
            Ok(after) => after,
            Err(LedgerError::Remote(kind)) => {
                return Err(LedgerError::CommittedUnverified {
                    id: entry.id.clone(),
                    kind,
                })
            }
            Err(LedgerError::Message(message)) => {
                return Err(LedgerError::Message(format!(
                    "{} was committed, but the ledger read back is invalid: {message}",
                    entry.id
                )))
            }
            Err(other) => return Err(other),
        };
        let active_ids: Vec<_> = memory::active_entries(&after)
            .into_iter()
            .filter(|item| item.key == entry.key)
            .map(|item| item.id.clone())
            .collect();
        if (resolution && active_ids.len() != 1) || (!resolution && active_ids.len() > 1) {
            return Err(LedgerError::Conflict {
                id: entry.id.clone(),
                key: entry.key.clone(),
                active_ids,
            });
        }
        Ok(())
    }
}

const SESSION_NOTES: &str = "SELECT n.note_id, n.machine, m.host, n.file_name, n.updated_at FROM harness_notes n LEFT JOIN harness_machines m ON m.machine=n.machine WHERE n.project=? AND n.machine<>? AND n.deleted=0 ORDER BY n.updated_at DESC, n.machine, n.source_dir, n.file_name LIMIT 15";
const SESSION_NOTES_COUNT: &str =
    "SELECT COUNT(*) FROM harness_notes WHERE project=? AND machine<>? AND deleted=0";

/// Read only. Retry a step-zero SQL error with an unaliased full pipeline, then retry a
/// notes SQL error with ledger only, subject to the deadline before each retry.
#[doc(hidden)]
pub fn session_recall_with(
    root: &Path,
    db: &RemoteDb,
    machine: &str,
    projects: (&str, &str),
    started: Instant,
) -> Result<(MemoryRecall, usize, Vec<notes_upload::NoteBrief>, usize), ErrorKind> {
    // Before the network wait: nothing after it may spend the watchdog's margin on a subprocess.
    let top = memory::source_root(root);
    let unaliased = Stmt::new(
        "SELECT body FROM project_memory WHERE project = ?",
        vec![Cell::Text(projects.0.into())],
    );
    let aliased = Stmt::new(
        format!("SELECT body FROM project_memory WHERE project IN ({COMPONENT})"),
        vec![Cell::Text(projects.0.into())],
    );
    let note_args = || vec![Cell::Text(projects.1.into()), Cell::Text(machine.into())];
    let mut ledger_stmt = aliased;
    let mut with_notes = true;
    let mut retried_alias = false;
    let result = loop {
        if crate::remote::RECALL_DEADLINE.saturating_sub(started.elapsed())
            < Duration::from_millis(200)
        {
            return Err(ErrorKind::Timeout);
        }
        let mut statements = vec![ledger_stmt.clone()];
        if with_notes {
            statements.push(Stmt::new(SESSION_NOTES, note_args()));
            statements.push(Stmt::new(SESSION_NOTES_COUNT, note_args()));
        }
        match db.pipeline(&statements) {
            Err(error)
                if error.kind == ErrorKind::Sql
                    && error.step == Some(0)
                    && !retried_alias
                    && with_notes =>
            {
                ledger_stmt = unaliased.clone();
                retried_alias = true;
            }
            Err(error)
                if error.kind == ErrorKind::Sql
                    && error.step.is_some_and(|n| n >= 1)
                    && with_notes =>
            {
                with_notes = false;
            }
            result => break result.map_err(|error| error.kind)?,
        }
    };
    let (rows, notes, total) = match result {
        mut results if with_notes => {
            if results.len() != 3 {
                return Err(ErrorKind::Sql);
            }
            let count = results.pop().ok_or(ErrorKind::Sql)?;
            let note_rows = results.pop().ok_or(ErrorKind::Sql)?;
            let ledger = results.pop().ok_or(ErrorKind::Sql)?;
            let total = match count.rows.first().and_then(|r| r.first()) {
                Some(Cell::Int(n)) if *n >= 0 => *n as usize,
                _ => return Err(ErrorKind::Sql),
            };
            (ledger.rows, note_rows.rows, total)
        }
        mut results => (results.pop().ok_or(ErrorKind::Sql)?.rows, Vec::new(), 0),
    };
    let bodies = rows
        .iter()
        .map(|row| match row.first() {
            Some(Cell::Text(s)) => Ok(s.clone()),
            _ => Err(ErrorKind::Sql),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bodies = merge_bodies(bodies).map_err(|_| ErrorKind::Sql)?;
    let ledger = ledger_from_bodies(bodies).map_err(|_| ErrorKind::Sql)?;
    let (recall, omitted) = memory::recall_ledger_for_hook(&top, &ledger);
    let briefs = notes
        .iter()
        .map(|row| {
            let text = |i| match row.get(i) {
                Some(Cell::Text(s)) => Ok(s.clone()),
                _ => Err(ErrorKind::Sql),
            };
            let host = match row.get(2) {
                Some(Cell::Text(s)) => Some(s.clone()),
                Some(Cell::Null) => None,
                _ => return Err(ErrorKind::Sql),
            };
            Ok(notes_upload::NoteBrief {
                note_id: text(0)?,
                machine: text(1)?,
                host,
                file_name: text(3)?,
                updated_at: text(4)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((recall, omitted, briefs, total))
}

fn notes_machine() -> Value {
    json!({"id":usage::machine_id(),"source":usage::machine_id_source()})
}

fn claude_home() -> Result<PathBuf, String> {
    crate::measure::claude_projects_dir()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Claude projects directory has no parent".to_string())
}

pub fn notes_opt_in(root: &Path, opt_in: bool) -> Result<Value, String> {
    let project = project_from_git_origin(root)?;
    let path = notes_upload::opt_in_path();
    let mut projects = notes_upload::read_opt_in(&path)?;
    if opt_in {
        projects.insert(project.clone());
    } else {
        projects.remove(&project);
    }
    notes_upload::write_opt_in(&path, &projects)?;
    let mut result = json!({"status":if opt_in {"opted_in"} else {"opted_out"},"project":project,"opt_in_file":path});
    if !opt_in {
        result["remote_rows"] = json!("kept");
    }
    Ok(result)
}

pub fn notes_status() -> Result<Value, String> {
    let opted_in = notes_upload::read_opt_in(&notes_upload::opt_in_path())?;
    let state = config()
        .ok()
        .and_then(|config| load_notes_state(&notes_state_path(), &config, usage::machine_id()));
    Ok(
        json!({"opted_in":opted_in,"memory_configured":config().is_ok(),"claude_home":claude_home()?,"machine":notes_machine(),
            "tombstones_unknown":state.as_ref().is_none_or(|s| s.tombstones_unknown),
            "uploaded_notes":state.as_ref().map_or(0, |s| s.notes.len())}),
    )
}

fn notes_now() -> Result<String, String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;
    let (year, month, day) = usage::civil_from_unix_days(seconds.div_euclid(86_400));
    let hour = seconds.rem_euclid(86_400) / 3_600;
    let minute = seconds.rem_euclid(3_600) / 60;
    let second = seconds.rem_euclid(60);
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

const NOTES_HARNESS: &str = "claude-code";
const NOTES_EXISTING: &str = "SELECT project, source_dir, file_name, sha256, deleted FROM harness_notes WHERE machine = ? AND harness = ?";
const NOTES_UPSERT: &str = "INSERT INTO harness_notes (project, machine, harness, source_dir, file_name, note_id, sha256, title, description, kind, body, deleted, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?) ON CONFLICT(project, machine, harness, source_dir, file_name) DO UPDATE SET note_id=excluded.note_id, sha256=excluded.sha256, title=excluded.title, description=excluded.description, kind=excluded.kind, body=excluded.body, deleted=0, updated_at=excluded.updated_at";
const NOTES_TOMBSTONE: &str = "UPDATE harness_notes SET deleted=1, body='', title=NULL, description=NULL, kind=NULL, updated_at=? WHERE project=? AND machine=? AND harness=? AND source_dir=? AND file_name=? AND sha256=?";
const NOTES_HOST: &str = "INSERT INTO harness_machines (machine, host, updated_at) VALUES (?, ?, ?) ON CONFLICT(machine) DO UPDATE SET host=excluded.host, updated_at=excluded.updated_at WHERE host<>excluded.host";
const NOTES_LIST: &str = "SELECT n.project, n.machine, n.harness, n.source_dir, n.file_name, n.note_id, n.sha256, n.title, n.description, n.kind, n.updated_at, m.host FROM harness_notes n LEFT JOIN harness_machines m ON m.machine=n.machine WHERE n.project=? AND n.deleted=0 ORDER BY n.updated_at DESC, n.machine, n.harness, n.source_dir, n.file_name";
const NOTES_SHOW: &str = "SELECT n.project, n.machine, n.harness, n.source_dir, n.file_name, n.note_id, n.sha256, n.title, n.description, n.kind, n.updated_at, m.host, n.body FROM harness_notes n LEFT JOIN harness_machines m ON m.machine=n.machine WHERE n.note_id=? AND n.deleted=0";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UploadedNote {
    project: String,
    harness: String,
    source_dir: String,
    file_name: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NotesUploaded {
    version: u8,
    database: String,
    machine: String,
    host: String,
    tombstones_unknown: bool,
    notes: Vec<UploadedNote>,
}

impl NotesUploaded {
    fn empty(config: &Config, machine: &str) -> Self {
        Self {
            version: 1,
            database: crate::measure_store::database_identity(&config.url),
            machine: machine.into(),
            host: String::new(),
            tombstones_unknown: false,
            notes: Vec::new(),
        }
    }
    fn existing(&self) -> Vec<notes_upload::ExistingRow> {
        self.notes
            .iter()
            .map(|n| notes_upload::ExistingRow {
                project: n.project.clone(),
                source_dir: n.source_dir.clone(),
                file_name: n.file_name.clone(),
                sha256: n.sha256.clone(),
                deleted: false,
            })
            .collect()
    }
    fn remove(&mut self, project: &str, source_dir: &str, file_name: &str) {
        self.notes.retain(|n| {
            n.project != project
                || n.source_dir != source_dir
                || n.file_name != file_name
                || n.harness != NOTES_HARNESS
        });
    }
}

fn notes_state_path() -> PathBuf {
    crate::data_dir::data_dir().join("notes-uploaded.json")
}

fn load_notes_state(path: &Path, config: &Config, machine: &str) -> Option<NotesUploaded> {
    if fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    let body = fs::read_to_string(path).ok()?;
    let state: NotesUploaded = serde_json::from_str(&body).ok()?;
    (state.version == 1
        && state.database == crate::measure_store::database_identity(&config.url)
        && state.machine == machine)
        .then_some(state)
}

fn save_notes_state(path: &Path, state: &NotesUploaded) -> Result<(), String> {
    let body = serde_json::to_string(state).map_err(|e| e.to_string())?;
    write_private_bytes(path, body.as_bytes(), "notes upload state")
}

fn write_private_bytes(path: &Path, body: &[u8], label: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{label} must not be a symlink"));
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), stamp));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        file.write_all(body).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    crate::data_dir::write_atomic(path, &String::from_utf8_lossy(body))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn notes_db(config: &Config) -> Result<RemoteDb, String> {
    let timeout = Duration::from_millis(crate::collect::CollectConfig::from_env().net_timeout_ms);
    RemoteDb::new(
        &config.url,
        &config.token,
        Box::new(UreqTransport { timeout }),
    )
    .map_err(|e| e.to_string())
}

/// The notes tables and indexes. They ride at the front of the first transaction of a run that
/// writes, so a run with nothing to write sends nothing, and a database recreated at the same URL
/// gets its tables back on the next write instead of being remembered as initialized.
const NOTES_SCHEMA: [&str; 6] = [
    notes_upload::CREATE_TABLE,
    notes_upload::CREATE_MACHINES,
    notes_upload::CREATE_INDEX,
    notes_upload::CREATE_MACHINE_INDEX,
    notes_upload::CREATE_ID_INDEX,
    notes_upload::CREATE_PROJECT_TIME_INDEX,
];

/// Read this machine's own rows. An upload creates the table first, because a database with no
/// notes table holds no rows to recover and must not leave `tombstones_unknown` set; a dry run
/// writes nothing, so it reads only.
fn remote_existing(
    db: &RemoteDb,
    machine: &str,
    create: bool,
) -> Result<Vec<notes_upload::ExistingRow>, ErrorKind> {
    let select = Stmt::new(
        NOTES_EXISTING,
        vec![Cell::Text(machine.into()), Cell::Text(NOTES_HARNESS.into())],
    );
    let mut stmts = Vec::new();
    if create {
        stmts.push(Stmt::new(notes_upload::CREATE_TABLE, vec![]));
    }
    stmts.push(select);
    let rows = db
        .pipeline(&stmts)
        .map_err(|e| e.kind)?
        .pop()
        .ok_or(ErrorKind::Sql)?
        .rows;
    rows.iter()
        .map(|r| {
            let string = |i| match r.get(i) {
                Some(Cell::Text(s)) => Ok(s.clone()),
                _ => Err(ErrorKind::Sql),
            };
            Ok(notes_upload::ExistingRow {
                project: string(0)?,
                source_dir: string(1)?,
                file_name: string(2)?,
                sha256: string(3)?,
                deleted: match r.get(4) {
                    Some(Cell::Int(n)) => *n != 0,
                    _ => return Err(ErrorKind::Sql),
                },
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn notes_upload_output(
    plan: notes_upload::Plan,
    complete: bool,
    dry_run: bool,
    verbose: bool,
    planned_from: &str,
    host_written: bool,
    tombstone_skipped: usize,
    state: &str,
    tombstones_unknown: bool,
    read_error: Option<ErrorKind>,
) -> Value {
    let counts = &plan.counts;
    let mut result = json!({
        "dry_run":dry_run,
        "counts":{"scanned":counts.scanned,"upload":counts.upload,"unresolved":counts.unresolved,
            "not_opted_in":counts.not_opted_in,"skipped_size":counts.skipped_size,
            "skipped_other":counts.skipped_other,"skipped_secret":counts.skipped_secret,
            "unchanged":counts.unchanged,"upserts":counts.upserts,"tombstones":counts.tombstones},
        "listing":if dry_run || verbose {plan.listing.lines().map(str::to_owned).collect::<Vec<_>>()} else {Vec::new()},
        "complete":complete,"uploaded":!dry_run,"machine":notes_machine(),
        "host_written":host_written,"tombstone_skipped":tombstone_skipped,
        "state":state,"tombstones_unknown":tombstones_unknown
    });
    if dry_run {
        result["planned_from"] = json!(planned_from);
        if let Some(kind) = read_error {
            result["remote"] = json!(kind.as_str());
        }
    }
    if let Some(kind) = read_error {
        result["rebuild_read_error"] = json!(kind.as_str());
    }
    if !complete {
        result["tombstones_suppressed"] = json!("incomplete_scan");
    }
    result
}

pub fn notes_upload(dry_run: bool, verbose: bool, lock_wait: Duration) -> Result<Value, String> {
    // The whole scan and eligibility screen run before any store lock or network operation.
    let report = notes::scan(&claude_home()?);
    let opted_in = notes_upload::read_opt_in(&notes_upload::opt_in_path())?;
    let prepared = notes_upload::prepare(&report, Path::new("/"), &opted_in, notes::project_for);
    if dry_run && !config_path().exists() {
        let plan = notes_upload::plan(&prepared, &[]);
        let mut result = notes_upload_output(
            plan,
            report.complete,
            true,
            verbose,
            "none",
            false,
            0,
            "rebuilt",
            true,
            None,
        );
        result["remote"] = json!("not_configured");
        return Ok(result);
    }
    with_notes_lock(lock_wait, |config| {
        let db = notes_db(&config)?;
        // An old replica directory means an older cort on this machine may still write these rows
        // through the replica without updating the state file, so the state cannot be trusted alone
        // until §8.4 removes that directory.
        let legacy_replica = old_lock_dir(&crate::data_dir::data_dir(), SUBDIR, &config).is_dir();
        notes_upload_with(
            &prepared,
            report.complete,
            dry_run,
            verbose,
            &config.url,
            usage::machine_id(),
            &crate::measure::host_name(),
            &db,
            &notes_state_path(),
            legacy_replica,
        )
    })
}

/// Test seam: the scan and transport are supplied without touching a real memory directory.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn notes_upload_with(
    prepared: &notes_upload::Prepared,
    complete: bool,
    dry_run: bool,
    verbose: bool,
    url: &str,
    machine: &str,
    host: &str,
    db: &RemoteDb,
    state_path: &Path,
    force_rebuild: bool,
) -> Result<Value, String> {
    let config = Config {
        url: url.into(),
        token: String::new(),
    };
    let loaded = load_notes_state(state_path, &config, machine);
    let valid = loaded.is_some();
    let mut state = loaded.unwrap_or_else(|| NotesUploaded::empty(&config, machine));
    let mut read_error = None;
    let mut planned_from = "local_state";
    let mut state_label = "loaded";
    let existing = if valid && !state.tombstones_unknown && !force_rebuild {
        state.existing()
    } else {
        match remote_existing(db, machine, !dry_run) {
            Ok(rows) => {
                state.notes = rows
                    .iter()
                    .filter(|r| !r.deleted)
                    .map(|r| UploadedNote {
                        project: r.project.clone(),
                        harness: NOTES_HARNESS.into(),
                        source_dir: r.source_dir.clone(),
                        file_name: r.file_name.clone(),
                        sha256: r.sha256.clone(),
                    })
                    .collect();
                state.tombstones_unknown = false;
                planned_from = "remote";
                state_label = if force_rebuild && valid {
                    "rebuilt_legacy_replica"
                } else if valid {
                    "rebuilt_tombstones_unknown"
                } else {
                    "rebuilt"
                };
                rows
            }
            Err(kind) => {
                read_error = Some(kind);
                state.tombstones_unknown = true;
                planned_from = if valid { "local_state" } else { "none" };
                state_label = if valid { "loaded" } else { "rebuilt" };
                state.existing()
            }
        }
    };
    let plan = notes_upload::plan(prepared, &existing);
    if dry_run {
        return Ok(notes_upload_output(
            plan,
            complete,
            true,
            verbose,
            planned_from,
            false,
            0,
            state_label,
            state.tombstones_unknown,
            read_error,
        ));
    }
    let now = notes_now()?;
    let host = notes::sanitize(host, Some(64));
    let host_changed = state.host != host;
    let hashes: BTreeMap<_, _> = existing
        .iter()
        .map(|r| ((&r.project, &r.source_dir, &r.file_name), &r.sha256))
        .collect();
    enum Change<'a> {
        Upsert(&'a notes_upload::Upload),
        Tombstone(&'a notes_upload::Tombstone, String),
    }
    let mut changes: Vec<_> = plan.upserts.iter().map(Change::Upsert).collect();
    for tombstone in &plan.tombstones {
        let hash = hashes
            .get(&(
                &tombstone.project,
                &tombstone.source_dir,
                &tombstone.file_name,
            ))
            .ok_or_else(|| {
                format!(
                    "missing tombstone hash: {}/{}/{}",
                    tombstone.project, tombstone.source_dir, tombstone.file_name
                )
            })?;
        changes.push(Change::Tombstone(tombstone, (*hash).clone()));
    }
    let mut chunks: Vec<Vec<Change<'_>>> = Vec::new();
    let mut chunk = Vec::new();
    let mut bytes = 0usize;
    for change in changes {
        let size = match &change {
            Change::Upsert(n) => n.body.len(),
            Change::Tombstone(_, _) => 0,
        };
        let limit = if chunks.is_empty() {
            50 - NOTES_SCHEMA.len() - usize::from(host_changed)
        } else {
            50
        };
        if !chunk.is_empty() && (chunk.len() == limit || bytes + size > 256 * 1024) {
            chunks.push(std::mem::take(&mut chunk));
            bytes = 0;
        }
        bytes += size;
        chunk.push(change);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    if chunks.is_empty() && host_changed {
        chunks.push(Vec::new());
    }
    let mut host_written = false;
    let mut tombstone_skipped = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        let mut stmts = Vec::new();
        if index == 0 {
            stmts.extend(NOTES_SCHEMA.iter().map(|sql| Stmt::new(*sql, vec![])));
        }
        if index == 0 && host_changed {
            stmts.push(Stmt::new(
                NOTES_HOST,
                vec![
                    Cell::Text(machine.into()),
                    Cell::Text(host.clone()),
                    Cell::Text(now.clone()),
                ],
            ));
        }
        for change in chunk {
            match change {
                Change::Upsert(n) => stmts.push(Stmt::new(
                    NOTES_UPSERT,
                    vec![
                        Cell::Text(n.project.clone()),
                        Cell::Text(machine.into()),
                        Cell::Text(NOTES_HARNESS.into()),
                        Cell::Text(n.source_dir.clone()),
                        Cell::Text(n.file_name.clone()),
                        Cell::Text(notes_upload::note_id(
                            &n.project,
                            machine,
                            NOTES_HARNESS,
                            &n.source_dir,
                            &n.file_name,
                        )),
                        Cell::Text(n.sha256.clone()),
                        opt_cell(&n.title),
                        opt_cell(&n.description),
                        opt_cell(&n.kind),
                        Cell::Text(n.body.clone()),
                        Cell::Text(now.clone()),
                    ],
                )),
                Change::Tombstone(t, hash) => stmts.push(Stmt::new(
                    NOTES_TOMBSTONE,
                    vec![
                        Cell::Text(now.clone()),
                        Cell::Text(t.project.clone()),
                        Cell::Text(machine.into()),
                        Cell::Text(NOTES_HARNESS.into()),
                        Cell::Text(t.source_dir.clone()),
                        Cell::Text(t.file_name.clone()),
                        Cell::Text(hash.clone()),
                    ],
                )),
            }
        }
        let results = db
            .transaction(&stmts)
            .map_err(|e| format!("remote: {}", e.kind.as_str()))?;
        let schema = if index == 0 { NOTES_SCHEMA.len() } else { 0 };
        let offset = schema + usize::from(index == 0 && host_changed);
        if index == 0 && host_changed {
            host_written = results[schema].affected_row_count == 1;
            state.host = host.clone();
        }
        for (change, result) in chunk.iter().zip(results.iter().skip(offset)) {
            match change {
                Change::Upsert(n) => {
                    if result.affected_row_count != 1 {
                        return Err(format!(
                            "upsert affected 0 rows: {}/{}/{}",
                            n.project, n.source_dir, n.file_name
                        ));
                    }
                    state.remove(&n.project, &n.source_dir, &n.file_name);
                    state.notes.push(UploadedNote {
                        project: n.project.clone(),
                        harness: NOTES_HARNESS.into(),
                        source_dir: n.source_dir.clone(),
                        file_name: n.file_name.clone(),
                        sha256: n.sha256.clone(),
                    });
                }
                Change::Tombstone(t, _) => {
                    // The remote row holds other content or is gone: not ours to bury.
                    if result.affected_row_count == 0 {
                        tombstone_skipped += 1;
                    }
                    state.remove(&t.project, &t.source_dir, &t.file_name);
                }
            }
        }
        save_notes_state(state_path, &state)?;
    }
    if chunks.is_empty() {
        save_notes_state(state_path, &state)?;
    }
    Ok(notes_upload_output(
        plan,
        complete,
        false,
        verbose,
        planned_from,
        host_written,
        tombstone_skipped,
        state_label,
        state.tombstones_unknown,
        read_error,
    ))
}

fn opt_cell(value: &Option<String>) -> Cell {
    value.as_ref().map_or(Cell::Null, |s| Cell::Text(s.clone()))
}

fn notes_read_db() -> Result<RemoteDb, LedgerError> {
    let config = config().map_err(LedgerError::Message)?;
    notes_db(&config).map_err(LedgerError::Message)
}

fn remote_text(row: &[Cell], index: usize) -> Result<String, LedgerError> {
    match row.get(index) {
        Some(Cell::Text(s)) => Ok(s.clone()),
        _ => Err(LedgerError::Message("invalid remote notes row".into())),
    }
}

fn remote_optional(row: &[Cell], index: usize) -> Result<Option<String>, LedgerError> {
    match row.get(index) {
        Some(Cell::Text(s)) => Ok(Some(s.clone())),
        Some(Cell::Null) => Ok(None),
        _ => Err(LedgerError::Message("invalid remote notes row".into())),
    }
}

pub fn notes_list(root: &Path) -> Result<Value, LedgerError> {
    let project = project_from_git_origin(root).map_err(LedgerError::Message)?;
    let db = notes_read_db()?;
    notes_list_with(&db, &project)
}

#[doc(hidden)]
pub fn notes_list_with(db: &RemoteDb, project: &str) -> Result<Value, LedgerError> {
    let result = db
        .execute(NOTES_LIST, vec![Cell::Text(project.into())])
        .map_err(|e| LedgerError::Remote(e.kind))?;
    let rows: Result<Vec<_>, LedgerError> = result.rows.iter().map(|row| {
        let machine = remote_text(row, 1)?;
        Ok(json!({"note_id":remote_text(row,5)?,"machine":machine,"host":remote_optional(row,11)?,
            "this_machine":machine==usage::machine_id(),"file_name":remote_text(row,4)?,
            "title":remote_optional(row,7)?,"description":remote_optional(row,8)?,
            "kind":remote_optional(row,9)?,"updated_at":remote_text(row,10)?}))
    }).collect();
    Ok(
        json!({"rows":rows?,"reading":"harness memory from other machines' agents, uploaded unreviewed; not checked against source"}),
    )
}

pub fn notes_show(note_id: &str) -> Result<Value, LedgerError> {
    let db = notes_read_db()?;
    notes_show_with(&db, note_id)
}

#[doc(hidden)]
pub fn notes_show_with(db: &RemoteDb, note_id: &str) -> Result<Value, LedgerError> {
    let result = db
        .execute(NOTES_SHOW, vec![Cell::Text(note_id.into())])
        .map_err(|e| LedgerError::Remote(e.kind))?;
    if result.rows.len() > 1 {
        return Err(LedgerError::Message(format!(
            "note id {note_id} matches more than one note"
        )));
    }
    let row = result
        .rows
        .first()
        .ok_or_else(|| LedgerError::Message(format!("unknown note id: {note_id}")))?;
    let body: String = remote_text(row, 12)?
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    Ok(
        json!({"note_id":remote_text(row,5)?,"project":remote_text(row,0)?,
        "machine":remote_text(row,1)?,"host":remote_optional(row,11)?,
        "harness":remote_text(row,2)?,"source_dir":remote_text(row,3)?,
        "file_name":remote_text(row,4)?,"title":remote_optional(row,7)?,
        "description":remote_optional(row,8)?,"kind":remote_optional(row,9)?,
        "updated_at":remote_text(row,10)?,"body":body}),
    )
}

/// Whether two stored bodies describe the same claim. A client that predates the machine stamp
/// drops it when it re-serializes a row, so the same id can legitimately arrive with and without
/// it. Only that absence is forgiven: two stamps that are both present must agree, and everything
/// else must match exactly.
fn same_claim(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    const STAMP: [&str; 2] = ["recorded_machine", "recorded_machine_source"];
    let parse = |body: &str| serde_json::from_str::<serde_json::Value>(body).ok();
    let (Some(mut x), Some(mut y)) = (parse(a), parse(b)) else {
        return false;
    };
    let (Some(x), Some(y)) = (x.as_object_mut(), y.as_object_mut()) else {
        return false;
    };
    for field in STAMP {
        match (x.remove(field), y.remove(field)) {
            (Some(p), Some(q)) if p != q => return false,
            _ => {}
        }
    }
    x == y
}

pub fn notes_lock_wait() -> Duration {
    std::env::var("CORT_MEMORY_LOCK_WAIT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| *seconds > 0.0)
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .unwrap_or(Duration::from_secs(30))
}

fn with_notes_lock<T>(
    wait: Duration,
    action: impl FnOnce(Config) -> Result<T, String>,
) -> Result<T, String> {
    let config = config()?;
    let _lock =
        lock_in_within(SUBDIR, &config, "memory.lock", wait).map_err(|error| match error {
            LockWait::TimedOut => format!("memory.lock busy after {wait:?}"),
            LockWait::Failed(message) => message,
        })?;
    action(config)
}
/// Hold the lock under `locks/`, plus its old location while that directory exists.
pub(crate) struct StoreLock {
    _old: Option<fs::File>,
    _new: fs::File,
}

fn old_lock_dir(base: &Path, subdir: &str, config: &Config) -> PathBuf {
    base.join(subdir).join(digest(&config.url))
}

pub(crate) fn lock_dir(subdir: &str, config: &Config) -> PathBuf {
    lock_dir_in(&crate::data_dir::data_dir(), subdir, config)
}

fn lock_dir_in(base: &Path, subdir: &str, config: &Config) -> PathBuf {
    base.join("locks").join(subdir).join(digest(&config.url))
}

fn open_lock(path: &Path) -> Result<fs::File, String> {
    fs::create_dir_all(path.parent().ok_or("lock path has no parent")?)
        .map_err(|e| e.to_string())?;
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())
}

fn open_old_lock(path: &Path) -> Result<fs::File, String> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())
}

pub(crate) fn lock_in(subdir: &str, config: &Config, name: &str) -> Result<StoreLock, String> {
    lock_in_at(&crate::data_dir::data_dir(), subdir, config, name)
}

fn lock_in_at(base: &Path, subdir: &str, config: &Config, name: &str) -> Result<StoreLock, String> {
    let old_dir = old_lock_dir(base, subdir, config);
    let old = if old_dir.is_dir() {
        let file = open_old_lock(&old_dir.join(name))?;
        file.lock().map_err(|e| e.to_string())?;
        Some(file)
    } else {
        None
    };
    let new = open_lock(&lock_dir_in(base, subdir, config).join(name))?;
    new.lock().map_err(|e| e.to_string())?;
    Ok(StoreLock {
        _old: old,
        _new: new,
    })
}

#[derive(Debug)]
pub(crate) enum LockWait {
    TimedOut,
    Failed(String),
}

/// Take the same lock as `lock_in`, giving up after `wait` if another holder keeps it.
pub(crate) fn lock_in_within(
    subdir: &str,
    config: &Config,
    name: &str,
    wait: Duration,
) -> Result<StoreLock, LockWait> {
    lock_in_within_at(&crate::data_dir::data_dir(), subdir, config, name, wait)
}

fn lock_in_within_at(
    base: &Path,
    subdir: &str,
    config: &Config,
    name: &str,
    wait: Duration,
) -> Result<StoreLock, LockWait> {
    let start = Instant::now();
    let acquire = |file: fs::File| -> Result<fs::File, LockWait> {
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(fs::TryLockError::WouldBlock) if start.elapsed() >= wait => {
                    return Err(LockWait::TimedOut)
                }
                Err(fs::TryLockError::WouldBlock) => {}
                Err(fs::TryLockError::Error(e)) => return Err(LockWait::Failed(e.to_string())),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let old_dir = old_lock_dir(base, subdir, config);
    let old = if old_dir.is_dir() {
        Some(acquire(
            open_old_lock(&old_dir.join(name)).map_err(LockWait::Failed)?,
        )?)
    } else {
        None
    };
    let new = acquire(
        open_lock(&lock_dir_in(base, subdir, config).join(name)).map_err(LockWait::Failed)?,
    )?;
    Ok(StoreLock {
        _old: old,
        _new: new,
    })
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn lock_transition_uses_new_path_and_holds_existing_old_path() {
        let data = tempfile::tempdir().unwrap();
        let base = data.path();
        let config = Config {
            url: "libsql://lock.invalid".into(),
            token: "test".into(),
        };
        let old_dir = old_lock_dir(base, "turso-measure", &config);
        let new_path = lock_dir_in(base, "turso-measure", &config).join("measure.lock");
        let guard = lock_in_at(base, "turso-measure", &config, "measure.lock").unwrap();
        assert!(new_path.exists());
        assert!(!old_dir.exists());
        drop(guard);

        fs::create_dir_all(&old_dir).unwrap();
        let guard = lock_in_at(base, "turso-measure", &config, "measure.lock").unwrap();
        let old = open_old_lock(&old_dir.join("measure.lock")).unwrap();
        let new = open_lock(&new_path).unwrap();
        assert!(matches!(old.try_lock(), Err(fs::TryLockError::WouldBlock)));
        assert!(matches!(new.try_lock(), Err(fs::TryLockError::WouldBlock)));
        drop(guard);
        // Released means retakable within a bound, not on the first try: a sibling test in this
        // binary that spawns a child forks with the just-dropped descriptors, and the flock stays
        // held until that child execs (same window as tests/upgrade.rs's release check).
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        for file in [&old, &new] {
            while let Err(error) = file.try_lock() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "release must free the lock: {error:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(matches!(
            lock_in_within_at(
                base,
                "turso-measure",
                &config,
                "measure.lock",
                Duration::from_millis(0)
            ),
            Err(LockWait::TimedOut)
        ));
    }
}

pub fn recall(root: &Path, query: &str) -> Result<MemoryRecall, LedgerError> {
    let ledger = Ledger::open()?;
    recall_with(root, query, &ledger)
}

pub fn recall_with(root: &Path, query: &str, ledger: &Ledger) -> Result<MemoryRecall, LedgerError> {
    let entries = ledger.read(&project(root)?)?;
    let present = !entries.entries.is_empty();
    Ok(memory::recall_ledger(root, query, &entries, present, None))
}

fn conflicts(ledger: &MemoryLedger) -> Vec<MemoryConflict> {
    let mut by_key = std::collections::BTreeMap::<String, Vec<String>>::new();
    for entry in memory::active_entries(ledger) {
        by_key
            .entry(entry.key.clone())
            .or_default()
            .push(entry.id.clone());
    }
    by_key
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(key, active_ids)| MemoryConflict { key, active_ids })
        .collect()
}

pub fn status(root: &Path) -> Result<MemoryStatus, LedgerError> {
    let config = config()?;
    let ledger = Ledger::open()?;
    status_with(root, &ledger, &config.url)
}

#[derive(Debug, Clone)]
pub enum AliasFrom {
    Remote(String),
    Project(String),
}

impl AliasFrom {
    fn id(self) -> String {
        match self {
            Self::Remote(remote) => digest(&normalize_remote(&remote)),
            Self::Project(id) => id,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AliasOutcome {
    pub outcome: &'static str,
    pub old_project: String,
    pub current_project: String,
    pub compatibility_note: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AliasRow {
    pub old_project: String,
    pub new_project: String,
    pub created_at: String,
    pub machine: String,
}

#[derive(Debug, Serialize)]
pub struct AliasProject {
    pub project_id: String,
    pub row_count: usize,
}

#[derive(Debug, Serialize)]
pub struct AliasList {
    pub aliases: Vec<AliasRow>,
    pub projects: Vec<AliasProject>,
}

struct AliasSnapshot {
    aliases: Vec<AliasRow>,
    bodies: Vec<(String, MemoryEntry)>,
    ids: Vec<String>,
}

impl Ledger {
    fn alias_snapshot(&self, project: &str) -> Result<AliasSnapshot, LedgerError> {
        let aliases_sql = format!("SELECT old_project, new_project, created_at, machine FROM project_aliases WHERE old_project IN ({COMPONENT})");
        let bodies_sql =
            format!("SELECT project, body FROM project_memory WHERE project IN ({COMPONENT})");
        let arg = || vec![Cell::Text(project.into())];
        let results = self
            .db
            .pipeline(&[
                Stmt::new(CREATE_TABLE, vec![]),
                Stmt::new(CREATE_ALIASES, vec![]),
                Stmt::new(COMPONENT, arg()),
                Stmt::new(aliases_sql, arg()),
                Stmt::new(bodies_sql, arg()),
            ])
            .map_err(|e| LedgerError::Remote(e.kind))?;
        let text = |row: &[Cell], index| match row.get(index) {
            Some(Cell::Text(value)) => Ok(value.clone()),
            _ => Err(LedgerError::Message("invalid remote alias row".into())),
        };
        let mut ids = results[2]
            .rows
            .iter()
            .map(|row| text(row, 0))
            .collect::<Result<Vec<_>, _>>()?;
        ids.sort();
        let mut aliases = results[3]
            .rows
            .iter()
            .map(|row| {
                Ok(AliasRow {
                    old_project: text(row, 0)?,
                    new_project: text(row, 1)?,
                    created_at: text(row, 2)?,
                    machine: text(row, 3)?,
                })
            })
            .collect::<Result<Vec<_>, LedgerError>>()?;
        aliases.sort_by(|a, b| {
            (&a.old_project, &a.new_project).cmp(&(&b.old_project, &b.new_project))
        });
        let bodies = results[4]
            .rows
            .iter()
            .map(|row| {
                let body = text(row, 1)?;
                let entry = serde_json::from_str(&body)
                    .map_err(|e| LedgerError::Message(format!("invalid remote memory row: {e}")))?;
                Ok((text(row, 0)?, entry))
            })
            .collect::<Result<Vec<_>, LedgerError>>()?;
        Ok(AliasSnapshot {
            aliases,
            bodies,
            ids,
        })
    }

    fn alias_write(
        &self,
        sql: &str,
        old: &str,
        current: &str,
        args: Vec<Cell>,
        present_after: bool,
        expected_count: u64,
    ) -> Result<bool, LedgerError> {
        let pair_args = || {
            vec![
                Cell::Text(old.into()),
                Cell::Text(current.into()),
                Cell::Text(current.into()),
                Cell::Text(old.into()),
            ]
        };
        let pair_sql = "SELECT 1 FROM project_aliases WHERE (old_project = ? AND new_project = ?) OR (old_project = ? AND new_project = ?)";
        match self.db.execute(sql, args) {
            Ok(result) if result.affected_row_count == expected_count => Ok(true),
            Ok(result) if present_after && result.affected_row_count == 0 => {
                match self.db.execute(pair_sql, pair_args()) {
                    Ok(result) if !result.rows.is_empty() => Ok(false),
                    Ok(_) => Err(LedgerError::Message(
                        "alias is not linked or changed concurrently".into(),
                    )),
                    Err(error) => Err(LedgerError::NotRecorded { kind: error.kind }),
                }
            }
            // A remove whose count differs from the snapshot raced another machine's add or remove
            // of one direction: the unlink succeeded if no row is left in either direction.
            Ok(_) if !present_after => match self.db.execute(pair_sql, pair_args()) {
                Ok(result) if result.rows.is_empty() => Ok(true),
                Ok(_) => Err(LedgerError::Message(
                    "alias is not linked or changed concurrently".into(),
                )),
                Err(error) => Err(LedgerError::OutcomeUnknown {
                    id: old.into(),
                    key: None,
                    kind: error.kind,
                }),
            },
            Ok(_) => Err(LedgerError::Message(
                "alias is not linked or changed concurrently".into(),
            )),
            Err(error) if outcome_unknown(error.kind) => {
                match self.db.execute(pair_sql, pair_args()) {
                    Ok(result) if result.rows.is_empty() != present_after => Ok(true),
                    Ok(_) => Err(LedgerError::NotRecorded { kind: error.kind }),
                    Err(_) => Err(LedgerError::OutcomeUnknown {
                        id: old.into(),
                        key: None,
                        kind: error.kind,
                    }),
                }
            }
            Err(error) => Err(LedgerError::NotRecorded { kind: error.kind }),
        }
    }
}

const ALIAS_INSERT: &str = "INSERT INTO project_aliases (old_project, new_project, created_at, machine) SELECT ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM project_aliases WHERE (old_project = ? AND new_project = ?) OR (old_project = ? AND new_project = ?))";
const ALIAS_EMPTY: &str = "old project component has no memory rows; the URL may be wrong. Use --from-project with the id cort memory status printed before the rename";
const ALIAS_COMPATIBILITY: &str =
    "Old binaries see only their own project id; cross-id corrections wait for them to upgrade.";

pub fn alias_add(root: &Path, from: AliasFrom) -> Result<AliasOutcome, LedgerError> {
    alias_add_with(root, from, &Ledger::open()?)
}

pub fn alias_list(root: &Path) -> Result<AliasList, LedgerError> {
    alias_list_with(root, &Ledger::open()?)
}

pub fn alias_remove(root: &Path, from: AliasFrom) -> Result<AliasOutcome, LedgerError> {
    alias_remove_with(root, from, &Ledger::open()?)
}

pub fn alias_add_with(
    root: &Path,
    from: AliasFrom,
    ledger: &Ledger,
) -> Result<AliasOutcome, LedgerError> {
    let current = project(root)?;
    let old = from.id();
    if old == current {
        return Err("cannot alias a project to itself".into());
    }
    let snapshot = ledger.alias_snapshot(&old).map_err(not_recorded_read)?;
    if snapshot.ids.contains(&current) {
        return Ok(AliasOutcome {
            outcome: "already_linked",
            old_project: old,
            current_project: current,
            compatibility_note: None,
        });
    }
    if snapshot.bodies.is_empty() {
        return Err(ALIAS_EMPTY.into());
    }
    let args = vec![
        Cell::Text(old.clone()),
        Cell::Text(current.clone()),
        Cell::Text(notes_now()?),
        Cell::Text(usage::machine_id().into()),
        Cell::Text(old.clone()),
        Cell::Text(current.clone()),
        Cell::Text(current.clone()),
        Cell::Text(old.clone()),
    ];
    let inserted = ledger.alias_write(ALIAS_INSERT, &old, &current, args, true, 1)?;
    Ok(AliasOutcome {
        outcome: if inserted { "linked" } else { "already_linked" },
        old_project: old,
        current_project: current,
        compatibility_note: inserted.then_some(ALIAS_COMPATIBILITY),
    })
}

pub fn alias_list_with(root: &Path, ledger: &Ledger) -> Result<AliasList, LedgerError> {
    let snapshot = ledger.alias_snapshot(&project(root)?)?;
    let projects = snapshot
        .ids
        .into_iter()
        .map(|id| {
            let row_count = snapshot
                .bodies
                .iter()
                .filter(|(project, _)| project == &id)
                .count();
            AliasProject {
                project_id: id,
                row_count,
            }
        })
        .collect();
    Ok(AliasList {
        aliases: snapshot.aliases,
        projects,
    })
}

pub fn alias_remove_with(
    root: &Path,
    from: AliasFrom,
    ledger: &Ledger,
) -> Result<AliasOutcome, LedgerError> {
    let current = project(root)?;
    let old = from.id();
    let snapshot = ledger.alias_snapshot(&current).map_err(not_recorded_read)?;
    let is_edge = |edge: &AliasRow| {
        (edge.old_project == old && edge.new_project == current)
            || (edge.old_project == current && edge.new_project == old)
    };
    let edge_count = snapshot.aliases.iter().filter(|edge| is_edge(edge)).count();
    if edge_count == 0 {
        return Err(LedgerError::Message(
            "alias is not linked directly to the current project".into(),
        ));
    }
    let mut components = BTreeMap::new();
    for id in &snapshot.ids {
        if components.contains_key(id) {
            continue;
        }
        let mut reached = BTreeSet::from([id.clone()]);
        loop {
            let before = reached.len();
            for other in &snapshot.aliases {
                if is_edge(other) {
                    continue;
                }
                if reached.contains(&other.old_project) || reached.contains(&other.new_project) {
                    reached.insert(other.old_project.clone());
                    reached.insert(other.new_project.clone());
                }
            }
            if reached.len() == before {
                break;
            }
        }
        for member in reached {
            components.insert(member, id.clone());
        }
    }
    let mut locations: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (project, entry) in &snapshot.bodies {
        let component = components
            .get(project)
            .ok_or_else(|| LedgerError::Message("invalid alias component".into()))?;
        locations.entry(&entry.id).or_default().insert(component);
    }
    let mut crossing = BTreeSet::new();
    for (project, entry) in &snapshot.bodies {
        let component = components
            .get(project)
            .ok_or_else(|| LedgerError::Message("invalid alias component".into()))?;
        for id in entry.supersedes.iter().chain(entry.resolves.iter()) {
            if !locations
                .get(id.as_str())
                .is_some_and(|places| places.contains(component.as_str()))
            {
                crossing.insert(id.clone());
            }
        }
    }
    if !crossing.is_empty() {
        return Err(LedgerError::Message(format!(
            "alias_in_use: removing the alias would separate referenced ids: {}",
            crossing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    ledger.alias_write(
        "DELETE FROM project_aliases WHERE (old_project = ? AND new_project = ?) OR (old_project = ? AND new_project = ?)",
        &old,
        &current,
        vec![
            Cell::Text(old.clone()),
            Cell::Text(current.clone()),
            Cell::Text(current.clone()),
            Cell::Text(old.clone()),
        ],
        false,
        edge_count as u64,
    )?;
    Ok(AliasOutcome {
        outcome: "removed",
        old_project: old,
        current_project: current,
        compatibility_note: None,
    })
}

#[doc(hidden)]
pub fn status_with(
    root: &Path,
    ledger: &Ledger,
    database_url: &str,
) -> Result<MemoryStatus, LedgerError> {
    let project = project(root)?;
    let entries = ledger.read(&project)?;
    let project_ids = ledger.project_ids(&project)?;
    Ok(MemoryStatus {
        project_id: project,
        project_ids,
        project_id_source: project_source(),
        database_url: database_url.into(),
        remote: "ok",
        total_entries: entries.entries.len(),
        active_entries: memory::active_entries(&entries).len(),
        conflicts: conflicts(&entries),
    })
}

pub fn check(root: &Path) -> Result<Value, LedgerError> {
    let ledger = Ledger::open()?;
    let project = project(root)?;
    let entries = ledger.read(&project)?;
    Ok(
        json!({"remote":"ok","project_id":project,"total_entries":entries.entries.len(),
        "active_entries":memory::active_entries(&entries).len(),"conflicts":conflicts(&entries)}),
    )
}

pub fn resolve(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: &[String],
) -> Result<MemoryEntry, LedgerError> {
    resolve_with(root, key, claim, source, line, supersedes, &Ledger::open()?)
}

pub fn resolve_with(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: &[String],
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    if supersedes.len() < 2 {
        return Err("resolution needs at least two active ids".into());
    }
    let entry = make_entry(root, key, claim, Some((source, line)))?;
    resolve_entry(root, entry, supersedes, ledger)
}

fn make_entry(
    root: &Path,
    key: &str,
    claim: &str,
    source: Option<(&Path, usize)>,
) -> Result<MemoryEntry, LedgerError> {
    if let Some((source, line)) = source {
        let (source_root, source) = memory::record_source(root, source)?;
        let entry = memory::new_entry(&source_root, key, claim, &source, line, None)?;
        require_committed_source(&source_root, &source)?;
        Ok(entry)
    } else {
        Ok(memory::new_unsourced_entry(key, claim)?)
    }
}

pub fn resolve_optional(
    root: &Path,
    key: &str,
    claim: &str,
    source: Option<(&Path, usize)>,
    supersedes: &[String],
) -> Result<MemoryEntry, LedgerError> {
    if supersedes.len() < 2 {
        return Err("resolution needs at least two active ids".into());
    }
    let entry = make_entry(root, key, claim, source)?;
    resolve_entry(root, entry, supersedes, &Ledger::open()?)
}

fn resolve_entry(
    root: &Path,
    mut entry: MemoryEntry,
    supersedes: &[String],
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    let project = project(root)?;
    let before = ledger.read(&project).map_err(not_recorded_read)?;
    let supersedes = resolve_ids(&before, supersedes)?;
    entry.resolves = supersedes.clone();
    let active_ids: std::collections::HashSet<_> = memory::active_entries(&before)
        .into_iter()
        .filter(|item| item.key == entry.key)
        .map(|item| item.id.as_str())
        .collect();
    let requested: std::collections::HashSet<_> = supersedes.iter().map(String::as_str).collect();
    if active_ids.len() < 2 || requested.len() != supersedes.len() || requested != active_ids {
        return Err(LedgerError::Message("--supersedes must name every active id for this key exactly once; inspect memory status".into()));
    }
    let placeholders = vec!["?"; supersedes.len()].join(", ");
    let sql = format!("INSERT OR IGNORE INTO project_memory (project, id, body) SELECT ?, ?, ? WHERE (SELECT COUNT(DISTINCT id) FROM project_memory WHERE project IN ({COMPONENT}) AND id IN ({placeholders})) = {}", supersedes.len());
    let mut args = entry_args(&project, &entry)?;
    args.push(Cell::Text(project.clone()));
    args.extend(supersedes.iter().cloned().map(Cell::Text));
    ledger.write_checked(&project, &entry, &sql, args, &supersedes, true)?;
    Ok(entry)
}

fn resolve_ids(before: &MemoryLedger, ids: &[String]) -> Result<Vec<String>, LedgerError> {
    ids.iter()
        .map(|id| {
            // An exact id always names itself, whatever it looks like (imported ids need not be
            // hex); anything else is a prefix.
            if before.entries.iter().any(|entry| entry.id == *id) {
                return Ok(id.clone());
            }
            if !(8..=64).contains(&id.len())
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(LedgerError::Message(
                    "--supersedes requires a full id or at least 8 lowercase hex characters".into(),
                ));
            }
            let matches: BTreeSet<_> = before
                .entries
                .iter()
                .filter(|entry| entry.id.starts_with(id))
                .map(|entry| entry.id.as_str())
                .collect();
            match matches.len() {
                0 => Err(LedgerError::SupersedesMissing {
                    superseded: vec![id.clone()],
                }),
                1 => Ok(matches.into_iter().next().unwrap_or_default().to_string()),
                _ => Err(LedgerError::Message(format!(
                    "ambiguous --supersedes {id}: {}",
                    matches.into_iter().collect::<Vec<_>>().join(", ")
                ))),
            }
        })
        .collect()
}

pub fn record(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: Option<&str>,
) -> Result<MemoryEntry, LedgerError> {
    record_with(root, key, claim, source, line, supersedes, &Ledger::open()?)
}

pub fn record_with(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: Option<&str>,
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    let ids: Vec<String> = supersedes.into_iter().map(str::to_string).collect();
    record_options_with(root, key, claim, source, line, &ids, false, ledger)
}

pub fn record_options(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: &[String],
    replace: bool,
) -> Result<MemoryEntry, LedgerError> {
    record_options_with(
        root,
        key,
        claim,
        source,
        line,
        supersedes,
        replace,
        &Ledger::open()?,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn record_options_with(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: &[String],
    replace: bool,
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    record_optional_with(
        root,
        key,
        claim,
        Some((source, line)),
        supersedes,
        replace,
        ledger,
    )
}

pub fn record_optional(
    root: &Path,
    key: &str,
    claim: &str,
    source: Option<(&Path, usize)>,
    supersedes: &[String],
    replace: bool,
) -> Result<MemoryEntry, LedgerError> {
    record_optional_with(
        root,
        key,
        claim,
        source,
        supersedes,
        replace,
        &Ledger::open()?,
    )
}

pub fn record_optional_with(
    root: &Path,
    key: &str,
    claim: &str,
    source: Option<(&Path, usize)>,
    supersedes: &[String],
    replace: bool,
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    if replace && !supersedes.is_empty() {
        return Err("--replace conflicts with --supersedes".into());
    }
    let entry = make_entry(root, key, claim, source)?;
    if supersedes.len() >= 2 {
        return resolve_entry(root, entry, supersedes, ledger);
    }
    record_entry(
        root,
        entry,
        supersedes.first().map(String::as_str),
        replace,
        ledger,
    )
}

fn record_entry(
    root: &Path,
    mut entry: MemoryEntry,
    supersedes: Option<&str>,
    replace: bool,
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    let project = project(root)?;
    let before = ledger.read(&project).map_err(not_recorded_read)?;
    let active = memory::active_entries(&before);
    let same_key: Vec<_> = active.iter().filter(|e| e.key == entry.key).collect();
    if replace && same_key.is_empty() {
        return Err("no active entry has this key; drop --replace".into());
    }
    if replace && same_key.len() > 1 {
        return Err(LedgerError::Message(format!(
            "conflicting active entries: {}; repeat --supersedes for every active id",
            same_key
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let resolved = if replace {
        Some(same_key[0].id.clone())
    } else if let Some(id) = supersedes {
        Some(resolve_ids(&before, &[id.to_string()])?.remove(0))
    } else {
        None
    };
    if same_key.len() > 1 {
        return Err(LedgerError::Message(
            "conflicting active entries for this key".into(),
        ));
    }
    let supersedes = resolved.as_deref();
    match supersedes {
        Some(id) if !active.iter().any(|e| e.id == id && e.key == entry.key) => {
            return Err(LedgerError::Message(
                "superseded id must be the active entry for the same key".into(),
            ))
        }
        None if !same_key.is_empty() => {
            return Err(LedgerError::Message(format!(
                "an active entry has this key: {}; pass --supersedes or --replace",
                same_key[0].id
            )))
        }
        _ => {}
    }
    entry.supersedes = resolved;
    let supersedes = entry.supersedes.as_deref();
    let mut args = entry_args(&project, &entry)?;
    let (sql, old) = match supersedes {
        Some(id) => {
            args.push(Cell::Text(project.clone()));
            args.push(Cell::Text(id.into()));
            (format!("INSERT OR IGNORE INTO project_memory (project, id, body) SELECT ?, ?, ? WHERE (SELECT COUNT(DISTINCT id) FROM project_memory WHERE project IN ({COMPONENT}) AND id = ?) = 1"), vec![id.to_string()])
        }
        None => (
            "INSERT OR IGNORE INTO project_memory (project, id, body) VALUES (?, ?, ?)".to_string(),
            Vec::new(),
        ),
    };
    ledger.write_checked(&project, &entry, &sql, args, &old, false)?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_matches_common_git_remote_spellings() {
        let expected = normalize_remote("https://github.com/yanggf8/claudecat.git");
        assert_eq!(
            normalize_remote("git@github.com:yanggf8/claudecat.git"),
            expected
        );
        assert_eq!(
            normalize_remote("ssh://git@github.com/yanggf8/claudecat"),
            expected
        );
    }

    #[test]
    fn remote_rows_isolate_projects_and_preserve_supersession() {
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(crate::remote::SqliteTransport::new().unwrap()),
        )
        .unwrap();
        let insert = |project: &str, entry: &MemoryEntry| -> Result<(), String> {
            let body = serde_json::to_string(entry).map_err(|e| e.to_string())?;
            ledger
                .db
                .execute(
                    "INSERT OR IGNORE INTO project_memory (project, id, body) VALUES (?, ?, ?)",
                    entry_args(project, entry).map_err(|e| format!("{e:?}"))?,
                )
                .map_err(|e| e.kind.as_str().to_string())?;
            let rows = ledger.read_bodies(project).map_err(|e| format!("{e:?}"))?;
            let stored = rows
                .into_iter()
                .find(|row| {
                    serde_json::from_str::<MemoryEntry>(row).is_ok_and(|e| e.id == entry.id)
                })
                .ok_or("memory id conflict without an existing row")?;
            if !same_claim(&stored, &body) {
                return Err("memory id collision with different content".into());
            }
            Ok(())
        };
        ledger.read("project-a").unwrap();
        let first = MemoryEntry {
            id: "first".into(),
            key: "choice".into(),
            claim: "A".into(),
            source: "decision.md".into(),
            source_line: 1,
            source_sha256: "hash".into(),
            unsourced: false,
            source_line_sha256: None,
            recorded_at: "1".into(),
            supersedes: None,
            resolves: Vec::new(),
            recorded_machine: None,
            recorded_machine_source: None,
        };
        let mut second = first.clone();
        second.id = "second".into();
        second.claim = "B".into();
        second.recorded_at = "0".into(); // a different machine may have a slow clock
        second.supersedes = Some("first".into());
        insert("project-a", &first).unwrap();
        insert("project-a", &first).unwrap();
        let mut collision = first.clone();
        collision.claim = "tampered".into();
        assert!(insert("project-a", &collision).is_err());
        // An older client re-serializes a row without the stamp: same claim, same id, both
        // directions.
        let mut stamped = first.clone();
        stamped.recorded_machine = Some("machine-a".into());
        stamped.recorded_machine_source = Some("etc-machine-id".into());
        insert("project-a", &stamped).unwrap();
        stamped.id = "stamped".into();
        insert("project-c", &stamped).unwrap();
        let mut unstamped = stamped.clone();
        unstamped.recorded_machine = None;
        unstamped.recorded_machine_source = None;
        insert("project-c", &unstamped).unwrap();
        // Two present stamps that disagree are two writers claiming one id: a replay or a
        // collision, never an older client. Same claim or not, it is refused.
        let mut restamped = stamped.clone();
        restamped.recorded_machine = Some("machine-b".into());
        assert!(insert("project-c", &restamped).is_err());
        restamped.claim = "tampered".into();
        assert!(insert("project-c", &restamped).is_err());
        insert("project-a", &second).unwrap();
        insert("project-b", &first).unwrap();
        let a = ledger.read("project-a").unwrap();
        let b = ledger.read("project-b").unwrap();
        assert_eq!(a.entries.len(), 2);
        assert_eq!(memory::active_entries(&a)[0].claim, "B");
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].claim, "A");
    }
}
