//! Local snapshot of a project's reminders and the per-session set of reminders already shown.
//!
//! The prompt hook reads only these files and the clock, never the network. Writes are a temp
//! file plus rename without fsync (a torn file reads as absent, which costs one missed prompt),
//! and the shown set is appended with `O_APPEND` whole lines. Every function takes the base
//! directory (callers pass `data_dir::data_dir()`).

use crate::remind::{self, Reminder, ReminderEvent, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A snapshot older than this is stale: the hook says so once instead of replaying it.
pub const STALE_AFTER_NS: i128 = 3 * 86_400_000_000_000;

const SNAPSHOT_DIR: &str = "remind-snapshot";
const SESSIONS_DIR: &str = "remind-sessions";
const SNAPSHOT_VERSION: u32 = 1;
const PRUNE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub project: String,
    /// Clock reading (ns since the epoch, decimal) taken just before the read was sent.
    pub read_at: String,
    pub events: Vec<ReminderEvent>,
}

impl Snapshot {
    /// A version-1 snapshot.
    pub fn new(project: &str, read_at_ns: i128, events: Vec<ReminderEvent>) -> Self {
        Snapshot {
            version: SNAPSHOT_VERSION,
            project: project.to_string(),
            read_at: read_at_ns.to_string(),
            events,
        }
    }

    /// `read_at` as ns, 0 when it does not parse.
    pub fn read_at_ns(&self) -> i128 {
        self.read_at.parse().unwrap_or(0)
    }
}

/// (reminder id, `upcoming|due|overdue|stale`... see [`stale_key`], due_at or read_at as a
/// decimal string).
pub type ShownKey = (String, &'static str, String);

/// The key of the stale-snapshot notice: shown once per session per snapshot read.
pub fn stale_key(read_at_ns: i128) -> ShownKey {
    ("stale".to_string(), "-", read_at_ns.to_string())
}

/// The key of a reminder line in its current state; `None` for scheduled and closed ones.
pub fn shown_key(r: &Reminder) -> Option<ShownKey> {
    let state = match r.state {
        State::Upcoming => "upcoming",
        State::Due => "due",
        State::Overdue => "overdue",
        State::Scheduled | State::Closed => return None,
    };
    Some((r.add.id.clone(), state, r.effective.due_at.to_string()))
}

/// The add and every later event of each reminder that is not closed.
pub fn snapshot_events(reminders: &[Reminder]) -> Vec<ReminderEvent> {
    let mut out = Vec::new();
    for r in reminders.iter().filter(|r| r.state != State::Closed) {
        out.push(r.add.clone());
        out.extend(r.events.iter().cloned());
    }
    out
}

fn safe_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name == "." || name == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe file name",
        ));
    }
    Ok(())
}

fn snapshot_path(dir: &Path, project: &str) -> io::Result<PathBuf> {
    safe_name(project)?;
    Ok(dir.join(SNAPSHOT_DIR).join(format!("{project}.json")))
}

fn session_path(dir: &Path, session_id: &str) -> PathBuf {
    let name: String = Sha256::digest(session_id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    dir.join(SESSIONS_DIR).join(name)
}

/// Write `content` to `path` through a temp file in the same directory and a rename, no fsync.
fn write_rename(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent directory"))?;
    fs::create_dir_all(parent)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".tmp.{}.{}", std::process::id(), stamp));
    let result = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .and_then(|mut file| file.write_all(content))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Write the snapshot unless the one on disk was read later. `Ok(false)`: nothing written.
pub fn write_snapshot(dir: &Path, snap: &Snapshot) -> io::Result<bool> {
    let path = snapshot_path(dir, &snap.project)?;
    if let Some(existing) = read_snapshot(dir, &snap.project) {
        if existing.read_at_ns() > snap.read_at_ns() {
            return Ok(false);
        }
    }
    let body = serde_json::to_vec(snap).map_err(io::Error::other)?;
    write_rename(&path, &body)?;
    Ok(true)
}

/// `None` when the file is missing, does not parse, or has another version.
pub fn read_snapshot(dir: &Path, project: &str) -> Option<Snapshot> {
    let path = snapshot_path(dir, project).ok()?;
    let snap: Snapshot = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (snap.version == SNAPSHOT_VERSION).then_some(snap)
}

fn prune_sessions(sessions: &Path) {
    let Ok(entries) = fs::read_dir(sessions) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|time| SystemTime::now().duration_since(time).ok())
            .is_some_and(|age| age > PRUNE_AFTER);
        if old && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn state_name(raw: &str) -> Option<&'static str> {
    ["upcoming", "due", "overdue", "stale", "-"]
        .into_iter()
        .find(|s| *s == raw)
}

fn lines_of(keys: &[ShownKey]) -> String {
    keys.iter()
        .map(|(id, state, at)| format!("{id}\t{state}\t{at}\n"))
        .collect()
}

/// The keys already shown in this session; empty when the file is missing or unreadable.
pub fn read_shown(dir: &Path, session_id: &str) -> HashSet<ShownKey> {
    let Ok(text) = fs::read_to_string(session_path(dir, session_id)) else {
        return HashSet::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (id, state, at) = (parts.next()?, state_name(parts.next()?)?, parts.next()?);
            Some((id.to_string(), state, at.to_string()))
        })
        .collect()
}

/// Make the session's shown set exactly `keys` (empty clears it).
pub fn replace_shown(dir: &Path, session_id: &str, keys: &[ShownKey]) -> io::Result<()> {
    let path = session_path(dir, session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        prune_sessions(parent);
    }
    write_rename(&path, lines_of(keys).as_bytes())
}

/// Add `keys` to the session's shown set with one `O_APPEND` write of whole lines.
pub fn append_shown(dir: &Path, session_id: &str, keys: &[ShownKey]) -> io::Result<()> {
    let path = session_path(dir, session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        prune_sessions(parent);
    }
    if keys.is_empty() {
        return Ok(());
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(lines_of(keys).as_bytes())
}

/// Upcoming, due and overdue reminders of the snapshot whose key is not in `shown`, oldest
/// effective `due_at` first.
pub fn select_unshown(
    snapshot: &Snapshot,
    shown: &HashSet<ShownKey>,
    now_ns: i128,
) -> Vec<Reminder> {
    let mut out: Vec<Reminder> = remind::fold(snapshot.events.clone(), now_ns)
        .reminders
        .into_iter()
        .filter(|r| shown_key(r).is_some_and(|key| !shown.contains(&key)))
        .collect();
    out.sort_by(|a, b| {
        a.effective
            .due_at
            .cmp(&b.effective.due_at)
            .then_with(|| a.add.id.cmp(&b.add.id))
    });
    out
}

/// `(YYYY-MM-DD, HH:MM)` of an instant in this machine's local zone; `None` when it cannot say.
/// Uses `remind.rs`'s `Tm` and its `localtime_r` declaration, one FFI home for both.
#[cfg(unix)]
fn local_parts(ns: i128) -> Option<(String, String)> {
    let secs = std::os::raw::c_long::try_from(ns.div_euclid(1_000_000_000)).ok()?;
    let mut tm = crate::remind::Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: both pointers refer to live, initialized values for the whole call;
    // `localtime_r` writes only through `out` and keeps no reference.
    let done = unsafe { crate::remind::localtime_r(&secs, &mut tm) };
    if done.is_null() {
        return None;
    }
    Some((
        format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        ),
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
    ))
}

#[cfg(not(unix))]
fn local_parts(_ns: i128) -> Option<(String, String)> {
    None
}

/// `YYYY-MM-DD HH:MM` in the local zone, or the raw ns when the zone cannot be resolved.
pub fn local_stamp(ns: i128) -> String {
    match local_parts(ns) {
        Some((date, time)) => format!("{date} {time}"),
        None => format!("{ns}ns"),
    }
}

/// `YYYY-MM-DD` in the local zone, or the raw ns when the zone cannot be resolved.
pub fn local_date(ns: i128) -> String {
    local_parts(ns).map_or_else(|| format!("{ns}ns"), |(date, _)| date)
}
