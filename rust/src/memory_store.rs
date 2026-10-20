//! Cortexyoung-owned cross-machine project memory backed by the Turso Database sync SDK.
//! The local file is a sync replica and must never be opened with rusqlite or the Turso CLI.

use crate::memory::{self, MemoryEntry, MemoryLedger, MemoryRecall};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    pub sync_status: &'static str,
    pub total_entries: usize,
    pub active_entries: usize,
    pub conflicts: Vec<MemoryConflict>,
}

#[derive(Deserialize, Serialize)]
struct Config {
    url: String,
    token: String,
}

fn digest(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn config_path() -> PathBuf {
    crate::data_dir::data_dir().join("turso-memory.json")
}

pub fn configure(url: &str, token: &str) -> Result<PathBuf, String> {
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("turso://") || url.starts_with("libsql://"))
        || token.trim().is_empty()
    {
        return Err("provide a Turso database URL and a nonempty auth token".into());
    }
    let path = config_path();
    let body = serde_json::to_string(&Config {
        url: url.to_string(),
        token: token.trim().to_string(),
    })
    .map_err(|e| e.to_string())?;
    // A credential file is local to this machine and private to its owner.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Turso memory config must not be a symlink".into());
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
        fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    crate::data_dir::write_atomic(&path, &body).map_err(|e| e.to_string())?;
    Ok(path)
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
    let path = config_path();
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Turso memory config must not be a symlink".into());
    }
    let body = fs::read_to_string(&path).map_err(|_| {
        "Turso memory is not configured; run `cort memory configure --url URL --token-stdin`"
            .to_string()
    })?;
    serde_json::from_str(&body).map_err(|e| format!("invalid Turso memory config: {e}"))
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
pub(crate) fn project_from_git_origin(root: &Path) -> Result<String, String> {
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

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

async fn open(config: &Config) -> Result<turso::sync::Database, String> {
    open_replica(config, "memory.db").await
}

async fn open_replica(config: &Config, file: &str) -> Result<turso::sync::Database, String> {
    let dir = crate::data_dir::data_dir()
        .join("turso-memory")
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
        .build()
        .await
        .map_err(|e| format!("cannot open Turso sync replica: {e}"))
}

async fn read_ledger(conn: &turso::Connection, project: &str) -> Result<MemoryLedger, String> {
    ensure_schema(conn).await?;
    let mut rows = conn
        .query(
            "SELECT body FROM project_memory WHERE project = ?",
            [project],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut ledger = MemoryLedger::default();
    while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
        let body: String = row.get(0).map_err(|e| e.to_string())?;
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

async fn ensure_schema(conn: &turso::Connection) -> Result<(), String> {
    conn.execute(CREATE_TABLE, ())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn insert(
    conn: &turso::Connection,
    project: &str,
    entry: &MemoryEntry,
) -> Result<(), String> {
    let body = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO project_memory (project, id, body) VALUES (?, ?, ?)",
            (project, entry.id.as_str(), body.as_str()),
        )
        .await
        .map_err(|e| e.to_string())?;
    if inserted == 0 {
        let mut rows = conn
            .query(
                "SELECT body FROM project_memory WHERE project = ? AND id = ?",
                (project, entry.id.as_str()),
            )
            .await
            .map_err(|e| e.to_string())?;
        let old: String = rows
            .next()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("memory id conflict without an existing row")?
            .get(0)
            .map_err(|e| e.to_string())?;
        if old != body {
            return Err("memory id collision with different content".into());
        }
    }
    Ok(())
}

fn with_lock<T>(
    action: impl FnOnce(Config, String) -> Result<T, String>,
    root: &Path,
) -> Result<T, String> {
    with_store_lock(|config| {
        let project = project(root)?;
        action(config, project)
    })
}

fn with_store_lock<T>(action: impl FnOnce(Config) -> Result<T, String>) -> Result<T, String> {
    with_named_lock("memory.lock", action)
}

fn with_named_lock<T>(
    name: &str,
    action: impl FnOnce(Config) -> Result<T, String>,
) -> Result<T, String> {
    let config = config()?;
    let lock_path = crate::data_dir::data_dir()
        .join("turso-memory")
        .join(digest(&config.url))
        .join(name);
    fs::create_dir_all(lock_path.parent().unwrap()).map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    action(config)
}

pub fn recall(root: &Path, query: &str) -> Result<MemoryRecall, String> {
    with_lock(
        |config, project| {
            runtime()?.block_on(async {
                let db = open(&config).await?;
                let conn = db.connect().await.map_err(|e| e.to_string())?;
                ensure_schema(&conn).await?;
                let pushed = db.push().await.is_ok();
                let pulled = db.pull().await.is_ok();
                let ledger = read_ledger(&conn, &project).await?;
                let present = !ledger.entries.is_empty();
                let status = if pushed && pulled {
                    "synced"
                } else {
                    "offline_or_pending"
                };
                Ok(memory::recall_ledger(
                    root,
                    query,
                    &ledger,
                    present,
                    Some(status.into()),
                ))
            })
        },
        root,
    )
}

pub fn status(root: &Path) -> Result<MemoryStatus, String> {
    with_lock(
        |config, project| {
            runtime()?.block_on(async {
                let db = open(&config).await?;
                let conn = db.connect().await.map_err(|e| e.to_string())?;
                ensure_schema(&conn).await?;
                let pushed = db.push().await.is_ok();
                let pulled = db.pull().await.is_ok();
                let ledger = read_ledger(&conn, &project).await?;
                let active = memory::active_entries(&ledger);
                let mut by_key = std::collections::BTreeMap::<String, Vec<String>>::new();
                for entry in &active {
                    by_key
                        .entry(entry.key.clone())
                        .or_default()
                        .push(entry.id.clone());
                }
                let conflicts = by_key
                    .into_iter()
                    .filter(|(_, ids)| ids.len() > 1)
                    .map(|(key, active_ids)| MemoryConflict { key, active_ids })
                    .collect();
                Ok(MemoryStatus {
                    project_id: project,
                    project_id_source: project_source(),
                    database_url: config.url,
                    sync_status: if pushed && pulled {
                        "synced"
                    } else {
                        "offline_or_pending"
                    },
                    total_entries: ledger.entries.len(),
                    active_entries: active.len(),
                    conflicts,
                })
            })
        },
        root,
    )
}

pub fn resolve(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: &[String],
) -> Result<MemoryEntry, String> {
    if supersedes.len() < 2 {
        return Err("resolution needs at least two active ids".into());
    }
    let mut entry = memory::new_entry(root, key, claim, source, line, None)?;
    require_committed_source(root, source)?;
    entry.resolves = supersedes.to_vec();
    with_lock(
        |config, project| {
            runtime()?.block_on(async {
                let db = open(&config).await?;
                db.push().await.map_err(|e| e.to_string())?;
                db.pull().await.map_err(|e| e.to_string())?;
                let conn = db.connect().await.map_err(|e| e.to_string())?;
                let ledger = read_ledger(&conn, &project).await?;
                let active_ids: std::collections::HashSet<_> = memory::active_entries(&ledger)
                    .into_iter()
                    .filter(|item| item.key == entry.key)
                    .map(|item| item.id.as_str())
                    .collect();
                let requested: std::collections::HashSet<_> =
                    supersedes.iter().map(String::as_str).collect();
                if active_ids.len() < 2 || requested.len() != supersedes.len() || requested != active_ids {
                    return Err("--supersedes must name every active id for this key exactly once; inspect memory status".into());
                }
                insert(&conn, &project, &entry).await?;
                db.push().await.map_err(|e| format!("resolution saved locally but push failed; retry sync: {e}"))?;
                db.pull().await.map_err(|e| format!("resolution pushed but pull failed: {e}"))?;
                let after = read_ledger(&conn, &project).await?;
                if memory::active_entries(&after)
                    .iter()
                    .filter(|item| item.key == entry.key)
                    .count() != 1
                {
                    return Err("resolution pushed but a concurrent conflict remains; inspect memory status".into());
                }
                Ok(entry)
            })
        },
        root,
    )
}

pub fn record(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    line: usize,
    supersedes: Option<&str>,
) -> Result<MemoryEntry, String> {
    let entry = memory::new_entry(root, key, claim, source, line, supersedes)?;
    require_committed_source(root, source)?;
    with_lock(
        |config, project| {
            runtime()?.block_on(async {
                let db = open(&config).await?;
                db.push()
                    .await
                    .map_err(|e| format!("pending local writes could not sync: {e}"))?;
                db.pull()
                    .await
                    .map_err(|e| format!("cannot pull latest memory before recording: {e}"))?;
                let conn = db.connect().await.map_err(|e| e.to_string())?;
                let ledger = read_ledger(&conn, &project).await?;
                let active = memory::active_entries(&ledger);
                if active.iter().filter(|e| e.key == entry.key).count() > 1 {
                    return Err("conflicting active entries for this key".into());
                }
                match supersedes {
                    Some(id) if !active.iter().any(|e| e.id == id && e.key == entry.key) => {
                        return Err("superseded id must be the active entry for the same key".into())
                    }
                    None if active.iter().any(|e| e.key == entry.key) => {
                        return Err("an active entry has this key; pass --supersedes".into())
                    }
                    _ => {}
                }
                insert(&conn, &project, &entry).await?;
                db.push().await.map_err(|e| {
                    format!("record saved locally but Turso push failed; retry sync: {e}")
                })?;
                db.pull()
                    .await
                    .map_err(|e| format!("record pushed, but post-write pull failed: {e}"))?;
                let after = read_ledger(&conn, &project).await?;
                if memory::active_entries(&after)
                    .iter()
                    .filter(|item| item.key == entry.key)
                    .count()
                    > 1
                {
                    return Err(
                        "record pushed, but concurrent active claims conflict; inspect recall"
                            .into(),
                    );
                }
                Ok(entry)
            })
        },
        root,
    )
}

pub fn sync(root: &Path) -> Result<String, String> {
    with_lock(
        |config, project| {
            runtime()?.block_on(async {
                let db = open(&config).await?;
                let conn = db.connect().await.map_err(|e| e.to_string())?;
                ensure_schema(&conn).await?;
                db.push().await.map_err(|e| e.to_string())?;
                db.pull().await.map_err(|e| e.to_string())?;
                let ledger = read_ledger(&conn, &project).await?;
                Ok(format!(
                    "synced {} project memory entries",
                    ledger.entries.len()
                ))
            })
        },
        root,
    )
}

pub fn import_legacy(root: &Path) -> Result<usize, String> {
    if !memory::ledger_path(root).exists() {
        return Err("no legacy .claudecat/memory.json to import".into());
    }
    let legacy = memory::load(root)?;
    with_lock(
        |config, project| {
            runtime()?.block_on(async {
                let db = open(&config).await?;
                db.push().await.map_err(|e| e.to_string())?;
                db.pull().await.map_err(|e| e.to_string())?;
                let conn = db.connect().await.map_err(|e| e.to_string())?;
                let current = read_ledger(&conn, &project).await?;
                let existing: std::collections::HashSet<_> =
                    current.entries.iter().map(|e| e.id.as_str()).collect();
                let mut count = 0;
                for entry in &legacy.entries {
                    insert(&conn, &project, entry).await?;
                    if !existing.contains(entry.id.as_str()) {
                        count += 1;
                    }
                }
                db.push()
                    .await
                    .map_err(|e| format!("import saved locally but Turso push failed: {e}"))?;
                Ok(count)
            })
        },
        root,
    )
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
    fn turso_rows_isolate_projects_and_preserve_supersession() {
        runtime().unwrap().block_on(async {
            let db = turso::Builder::new_local(":memory:").build().await.unwrap();
            let conn = db.connect().unwrap();
            read_ledger(&conn, "project-a").await.unwrap();
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
            insert(&conn, "project-a", &first).await.unwrap();
            insert(&conn, "project-a", &first).await.unwrap();
            let mut collision = first.clone();
            collision.claim = "tampered".into();
            assert!(insert(&conn, "project-a", &collision).await.is_err());
            insert(&conn, "project-a", &second).await.unwrap();
            insert(&conn, "project-b", &first).await.unwrap();
            let a = read_ledger(&conn, "project-a").await.unwrap();
            let b = read_ledger(&conn, "project-b").await.unwrap();
            assert_eq!(a.entries.len(), 2);
            assert_eq!(memory::active_entries(&a)[0].claim, "B");
            assert_eq!(b.entries.len(), 1);
            assert_eq!(b.entries[0].claim, "A");
        });
    }
}
