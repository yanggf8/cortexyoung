//! Cortexyoung-owned cross-machine project memory backed by the Turso Database sync SDK.
//! The local file is a sync replica and must never be opened with rusqlite or the Turso CLI.

use crate::memory::{self, MemoryEntry, MemoryLedger, MemoryRecall};
use crate::network_timeout::NetworkGuard;
use crate::remote::{Cell, ErrorKind, RemoteDb, Stmt, Transport, UreqTransport};
use crate::{notes, notes_upload, usage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))";
#[derive(Debug, Serialize)]
pub struct MemoryConflict {
    pub key: String,
    pub active_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryStatus {
    pub project_id: String,
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
        id: Option<String>,
        kind: ErrorKind,
    },
    OutcomeUnknown {
        id: Option<String>,
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
        let results = self
            .db
            .pipeline(&[
                Stmt::new(CREATE_TABLE, vec![]),
                Stmt::new(
                    "SELECT body FROM project_memory WHERE project = ?",
                    vec![Cell::Text(project.into())],
                ),
            ])
            .map_err(|e| LedgerError::Remote(e.kind))?;
        results[1]
            .rows
            .iter()
            .map(|row| match row.first() {
                Some(Cell::Text(body)) => Ok(body.clone()),
                _ => Err(LedgerError::Message("invalid remote memory row".into())),
            })
            .collect()
    }

    pub fn read(&self, project: &str) -> Result<MemoryLedger, LedgerError> {
        ledger_from_bodies(self.read_bodies(project)?).map_err(LedgerError::Message)
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

/// The `cort memory` connection, for a second replica of the same database.
pub(crate) fn load_config() -> Result<Config, String> {
    config()
}

/// Directory name (under `data_dir()`) of the memory replica and its sibling replicas.
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

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

fn network_guard() -> NetworkGuard {
    NetworkGuard::new(Duration::from_millis(
        crate::collect::CollectConfig::from_env().net_timeout_ms,
    ))
}

async fn open_network(config: &Config) -> Result<(turso::sync::Database, NetworkGuard), String> {
    let mut net = network_guard();
    let db = net.run(open_replica(config, "memory.db")).await?;
    Ok((db, net))
}

async fn open_replica(config: &Config, file: &str) -> Result<turso::sync::Database, String> {
    open_replica_in(SUBDIR, config, file).await
}

/// Open (creating) a sync replica at `data_dir()/<subdir>/<digest(url)>/<file>`.
pub(crate) async fn open_replica_in(
    subdir: &str,
    config: &Config,
    file: &str,
) -> Result<turso::sync::Database, String> {
    let dir = crate::data_dir::data_dir()
        .join(subdir)
        .join(digest(&config.url));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let path = dir.join(file);
    turso::sync::Builder::new_remote(&path.to_string_lossy())
        .with_remote_url(&config.url)
        .with_auth_token(&config.token)
        // turso 0.9.0-pre.1 `sync::Builder::bootstrap_if_empty(bool)`: download schema and data
        // when the local replica is empty. It already defaults to true; stated so a fresh
        // machine's first open pulls the remote instead of starting a divergent database.
        .bootstrap_if_empty(true)
        .build()
        .await
        .map_err(|e| format!("cannot open Turso sync replica: {e}"))
}
async fn read_existing_ledger(
    conn: &turso::Connection,
    project: &str,
) -> Result<MemoryLedger, String> {
    let mut rows = conn
        .query(
            "SELECT body FROM project_memory WHERE project = ?",
            [project],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut bodies = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
        let body: String = row.get(0).map_err(|e| e.to_string())?;
        bodies.push(body);
    }
    ledger_from_bodies(bodies)
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
            Err(error) if outcome_unknown(error.kind) => match self.read(project) {
                Ok(after) if after.entries.iter().any(|item| item.id == entry.id) => {}
                Ok(_) => return Err(LedgerError::NotRecorded { kind: error.kind }),
                Err(_) => {
                    return Err(LedgerError::OutcomeUnknown {
                        id: Some(entry.id.clone()),
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
                    id: Some(entry.id.clone()),
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

/// A session hook reads only a locally synced replica. The metadata guard excludes the
/// legacy open path that pulls remotely even with bootstrap disabled.
pub fn recall_local(
    root: &Path,
) -> Result<(MemoryRecall, usize, Vec<notes_upload::NoteBrief>, usize), LocalRecallError> {
    let config = config().map_err(|_| {
        if fs::symlink_metadata(config_path()).is_ok() {
            LocalRecallError::Failed
        } else {
            LocalRecallError::NotConfigured
        }
    })?;
    let dir = replica_dir(&config);
    if !dir.join("memory.db").is_file() {
        return Err(LocalRecallError::NoReplica);
    }
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("memory.lock"))
        .map_err(|_| LocalRecallError::Failed)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Err(LocalRecallError::Busy),
        Err(fs::TryLockError::Error(_)) => return Err(LocalRecallError::Failed),
    }
    runtime()
        .map_err(|_| LocalRecallError::Failed)?
        .block_on(async {
            // The replica guards answer first, as before the open was factored out, so a
            // project that cannot be resolved does not mask an unsynced replica.
            let db = open_guarded_replica(&config).await?;
            let project = project(root).map_err(|_| LocalRecallError::Failed)?;
            let conn = db.connect().await.map_err(|_| LocalRecallError::Failed)?;
            let ledger = read_existing_ledger(&conn, &project)
                .await
                .map_err(|_| LocalRecallError::Failed)?;
            let (recall, omitted) = memory::recall_ledger_for_hook(root, &ledger);
            let (briefs, total) = match project_from_git_origin(root) {
                Ok(notes_project) => notes_upload::other_machine_briefs(
                    &conn,
                    &notes_project,
                    usage::machine_id(),
                    crate::hook_recall::MAX_NOTES,
                )
                .await
                .unwrap_or_default(),
                Err(_) => (Vec::new(), 0),
            };
            Ok((recall, omitted, briefs, total))
        })
}

fn replica_dir(config: &Config) -> PathBuf {
    crate::data_dir::data_dir()
        .join(SUBDIR)
        .join(digest(&config.url))
}

/// Caller holds memory.lock. Refuse any state in which Turso might write or pull at open.
async fn open_guarded_replica(config: &Config) -> Result<turso::sync::Database, LocalRecallError> {
    let dir = replica_dir(config);
    let path = dir.join("memory.db");
    if !path.is_file() {
        return Err(LocalRecallError::NoReplica);
    }
    // Read under the lock, so a writer cannot change the metadata between this check and the open.
    let info_path = dir.join("memory.db-info");
    let info = fs::read(&info_path)
        .ok()
        .and_then(|bytes| turso_sync_engine::types::DatabaseMetadata::load(&bytes).ok())
        .ok_or(LocalRecallError::UnsyncedReplica)?;
    // open_db pulls for Legacy { synced_frame_no: None }, regardless of bootstrap.
    let safe = matches!(
        info.synced_revision,
        Some(turso_sync_engine::types::DatabasePullRevision::V1 { .. })
            | Some(turso_sync_engine::types::DatabasePullRevision::Legacy {
                synced_frame_no: Some(_),
                ..
            })
    );
    if !safe {
        return Err(LocalRecallError::UnsyncedReplica);
    }
    // open_db creates missing auxiliaries; require them before it runs.
    if !dir.join("memory.db-changes").is_file() {
        return Err(LocalRecallError::UnsyncedReplica);
    }
    // An interrupted replace-base apply is recovered at open (restoring files, then possibly
    // pulling), and metadata without a saved configuration is rewritten at open: neither is a
    // read, so leave both to the next foreground `cort memory` command.
    if dir.join("memory.db-replace-base-apply").exists()
        || info
            .saved_configuration
            .as_ref()
            .and_then(|c| c.remote_url.as_ref())
            .is_none()
    {
        return Err(LocalRecallError::UnsyncedReplica);
    }
    turso::sync::Builder::new_remote(&path.to_string_lossy())
        .with_remote_url(&config.url)
        .with_auth_token(&config.token)
        .bootstrap_if_empty(false)
        .build()
        .await
        .map_err(|_| LocalRecallError::Failed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRecallError {
    NotConfigured,
    NoReplica,
    UnsyncedReplica,
    Busy,
    Failed,
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
    Ok(
        json!({"opted_in":opted_in,"memory_configured":config().is_ok(),"claude_home":claude_home()?,"machine":notes_machine()}),
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

fn notes_upload_output(
    plan: notes_upload::Plan,
    complete: bool,
    dry_run: bool,
    verbose: bool,
    replica: Option<&str>,
    host_written: bool,
) -> Value {
    let counts = &plan.counts;
    let mut result = json!({
        "dry_run":dry_run,
        "counts":{"scanned":counts.scanned,"upload":counts.upload,"unresolved":counts.unresolved,
            "not_opted_in":counts.not_opted_in,"skipped_size":counts.skipped_size,
            "skipped_other":counts.skipped_other,"skipped_secret":counts.skipped_secret,
            "unchanged":counts.unchanged,"upserts":counts.upserts,"tombstones":counts.tombstones},
        "listing":if dry_run || verbose {plan.listing.lines().map(str::to_owned).collect::<Vec<_>>()} else {Vec::new()},
        "complete":complete,"pushed":!dry_run,"pulled":!dry_run,"machine":notes_machine(),
        "host_written":host_written
    });
    if dry_run {
        result["planned_from"] = json!("local_replica");
    }
    if let Some(replica) = replica {
        result["replica"] = json!(replica);
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
        return Ok(notes_upload_output(
            plan,
            report.complete,
            true,
            verbose,
            Some("not_configured"),
            false,
        ));
    }
    let now = if dry_run { None } else { Some(notes_now()?) };
    with_notes_lock(lock_wait, |config| {
        runtime()?.block_on(async {
            let machine = usage::machine_id();
            let harness = "claude-code";
            if dry_run {
                // Only a replica the guards refuse to open read-only is "absent". A read that fails on
                // an open replica is an error, never a plan computed against nothing.
                let (existing, absent) = match open_guarded_replica(&config).await {
                    Ok(db) => {
                        let conn = db.connect().await.map_err(|e| e.to_string())?;
                        (
                            notes_upload::existing_rows(&conn, machine, harness).await?,
                            false,
                        )
                    }
                    Err(_) => (Vec::new(), true),
                };
                let plan = notes_upload::plan(&prepared, &existing);
                return Ok(notes_upload_output(
                    plan,
                    report.complete,
                    true,
                    verbose,
                    absent.then_some("absent"),
                    false,
                ));
            }
            let (db, mut net) = open_network(&config).await?;
            let conn = db.connect().await.map_err(|e| e.to_string())?;
            notes_upload::ensure_notes_schema(&conn).await?;
            net.run(db.push()).await?;
            net.run(db.pull()).await?;
            let existing = notes_upload::existing_rows(&conn, machine, harness).await?;
            let plan = notes_upload::plan(&prepared, &existing);
            let host_written = notes_upload::apply(
                &conn,
                machine,
                &crate::measure::host_name(),
                harness,
                now.as_deref().unwrap_or_default(),
                &plan,
            )
            .await?;
            net.run(db.push()).await?;
            Ok(notes_upload_output(
                plan,
                report.complete,
                false,
                verbose,
                None,
                host_written,
            ))
        })
    })
}

pub fn notes_list(root: &Path) -> Result<Value, String> {
    let project = project_from_git_origin(root)?;
    with_notes_lock(notes_lock_wait(), |config| {
        runtime()?.block_on(async {
        let (db, _net) = open_network(&config).await?;
        let conn = db.connect().await.map_err(|e| e.to_string())?;
        let rows = notes_upload::list(&conn, &project).await?;
        let rows: Vec<_> = rows.into_iter().map(|row| json!({
            "note_id":row.note_id,"machine":row.machine,"host":row.host,"this_machine":row.machine == usage::machine_id(),
            "file_name":row.file_name,"title":row.title,"description":row.description,
            "kind":row.kind,"updated_at":row.updated_at
        })).collect();
        Ok(json!({"rows":rows,"reading":"harness memory from other machines' agents, uploaded unreviewed; not checked against source"}))
    })
    })
}

pub fn notes_show(note_id: &str) -> Result<Value, String> {
    with_notes_lock(notes_lock_wait(), |config| {
        runtime()?.block_on(async {
            let (db, _net) = open_network(&config).await?;
            let conn = db.connect().await.map_err(|e| e.to_string())?;
            let found = notes_upload::show(&conn, note_id)
                .await?
                .ok_or_else(|| format!("unknown note id: {note_id}"))?;
            let row = found.note;
            let body: String = found
                .body
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
                json!({"note_id":row.note_id,"project":row.project,"machine":row.machine,"host":row.host,
            "harness":row.harness,"source_dir":row.source_dir,"file_name":row.file_name,
            "title":row.title,"description":row.description,"kind":row.kind,
            "updated_at":row.updated_at,"body":body}),
            )
        })
    })
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
        assert!(old.try_lock().is_ok());
        assert!(new.try_lock().is_ok());
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
    let project = project(root)?;
    let entries = ledger.read(&project)?;
    Ok(MemoryStatus {
        project_id: project,
        project_id_source: project_source(),
        database_url: config.url,
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
    let entry = memory::new_entry(root, key, claim, source, line, None)?;
    require_committed_source(root, source)?;
    resolve_entry(root, entry, supersedes, ledger)
}

fn resolve_entry(
    root: &Path,
    mut entry: MemoryEntry,
    supersedes: &[String],
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    entry.resolves = supersedes.to_vec();
    let project = project(root)?;
    let before = ledger.read(&project).map_err(not_recorded_read)?;
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
    let sql = format!("INSERT OR IGNORE INTO project_memory (project, id, body) SELECT ?, ?, ? WHERE (SELECT COUNT(*) FROM project_memory WHERE project = ? AND id IN ({placeholders})) = {}", supersedes.len());
    let mut args = entry_args(&project, &entry)?;
    args.push(Cell::Text(project.clone()));
    args.extend(supersedes.iter().cloned().map(Cell::Text));
    ledger.write_checked(&project, &entry, &sql, args, supersedes, true)?;
    Ok(entry)
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
    let entry = memory::new_entry(root, key, claim, source, line, supersedes)?;
    require_committed_source(root, source)?;
    record_entry(root, entry, supersedes, ledger)
}

fn record_entry(
    root: &Path,
    entry: MemoryEntry,
    supersedes: Option<&str>,
    ledger: &Ledger,
) -> Result<MemoryEntry, LedgerError> {
    let project = project(root)?;
    let before = ledger.read(&project).map_err(not_recorded_read)?;
    let active = memory::active_entries(&before);
    if active.iter().filter(|e| e.key == entry.key).count() > 1 {
        return Err(LedgerError::Message(
            "conflicting active entries for this key".into(),
        ));
    }
    match supersedes {
        Some(id) if !active.iter().any(|e| e.id == id && e.key == entry.key) => {
            return Err(LedgerError::Message(
                "superseded id must be the active entry for the same key".into(),
            ))
        }
        None if active.iter().any(|e| e.key == entry.key) => {
            return Err(LedgerError::Message(
                "an active entry has this key; pass --supersedes".into(),
            ))
        }
        _ => {}
    }
    let mut args = entry_args(&project, &entry)?;
    let (sql, old) = match supersedes {
        Some(id) => {
            args.push(Cell::Text(project.clone()));
            args.push(Cell::Text(id.into()));
            ("INSERT OR IGNORE INTO project_memory (project, id, body) SELECT ?, ?, ? WHERE EXISTS (SELECT 1 FROM project_memory WHERE project = ? AND id = ?)", vec![id.to_string()])
        }
        None => (
            "INSERT OR IGNORE INTO project_memory (project, id, body) VALUES (?, ?, ?)",
            Vec::new(),
        ),
    };
    ledger.write_checked(&project, &entry, sql, args, &old, false)?;
    Ok(entry)
}

pub fn import_legacy(root: &Path) -> Result<usize, LedgerError> {
    let ledger = Ledger::open()?;
    import_legacy_with(root, &ledger)
}

pub fn import_legacy_with(root: &Path, ledger: &Ledger) -> Result<usize, LedgerError> {
    if !memory::ledger_path(root).exists() {
        return Err("no legacy .claudecat/memory.json to import".into());
    }
    let legacy = memory::load(root)?;
    if legacy.entries.is_empty() {
        return Ok(0);
    }
    let project = project(root)?;
    let before = ledger.read(&project).map_err(not_recorded_read)?;
    let existing: std::collections::HashSet<_> =
        before.entries.iter().map(|e| e.id.as_str()).collect();
    let chunks: Vec<_> = legacy.entries.chunks(50).collect();
    // A chunk whose outcome is unknown (`outcome_unknown`) is settled by the reread below.
    let mut unknown = None;
    for (written, chunk) in chunks.iter().enumerate() {
        let statements = chunk
            .iter()
            .map(|entry| {
                Ok(Stmt::new(
                    "INSERT OR IGNORE INTO project_memory (project, id, body) VALUES (?, ?, ?)",
                    entry_args(&project, entry)?,
                ))
            })
            .collect::<Result<Vec<_>, LedgerError>>()?;
        match ledger.db.transaction(&statements) {
            Ok(_) => {}
            Err(error) if outcome_unknown(error.kind) => {
                unknown = Some(error.kind);
                break;
            }
            Err(error) if written == 0 => {
                return Err(LedgerError::NotRecorded { kind: error.kind })
            }
            Err(error) => {
                return Err(LedgerError::Message(format!(
                    "legacy import stopped after {written} of {} batches ({}); re-run import-legacy, it is idempotent",
                    chunks.len(),
                    error.kind.as_str()
                )))
            }
        }
    }
    let after_bodies = match ledger.read_bodies(&project) {
        Ok(bodies) => bodies,
        Err(LedgerError::Remote(kind)) => {
            return Err(match unknown {
                Some(kind) => LedgerError::OutcomeUnknown {
                    id: None,
                    key: None,
                    kind,
                },
                None => LedgerError::CommittedUnverified { id: None, kind },
            })
        }
        Err(other) => return Err(other),
    };
    ledger_from_bodies(after_bodies.clone()).map_err(LedgerError::Message)?;
    let stored: std::collections::HashMap<String, String> = after_bodies
        .into_iter()
        .map(|body| {
            let entry: MemoryEntry = serde_json::from_str(&body)
                .map_err(|e| LedgerError::Message(format!("invalid remote memory row: {e}")))?;
            Ok((entry.id, body))
        })
        .collect::<Result<_, LedgerError>>()?;
    let mut count = 0;
    for entry in &legacy.entries {
        let body = serde_json::to_string(entry).map_err(|e| LedgerError::Message(e.to_string()))?;
        match stored.get(entry.id.as_str()) {
            Some(old) if same_claim(old, &body) => {
                if !existing.contains(entry.id.as_str()) {
                    count += 1;
                }
            }
            Some(_) => {
                return Err(LedgerError::Message(
                    "memory id collision with different content".into(),
                ))
            }
            None => {
                return Err(LedgerError::Message(
                    "legacy import incomplete; re-run import-legacy".into(),
                ))
            }
        }
    }
    Ok(count)
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
