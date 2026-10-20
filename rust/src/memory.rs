//! Reviewed memory entries, source checks, and explicit import of the older Git JSON ledger.
//! The active database and sync replica are owned by `memory_store`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: u32 = 1;
const LEDGER_PATH: &str = ".claudecat/memory.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub claim: String,
    pub source: String,
    pub source_line: usize,
    pub source_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_line_sha256: Option<String>,
    pub recorded_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolves: Vec<String>,
    /// The writer's `usage::machine_id`. Without it a reader cannot tell a claim another machine
    /// wrote from one its own machine wrote, so a cross-machine read is unverifiable from the data
    /// alone. Rows written before this field existed carry none and say nothing about their origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_machine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_machine_source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryLedger {
    pub schema: u32,
    pub entries: Vec<MemoryEntry>,
}

impl Default for MemoryLedger {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryHit {
    #[serde(flatten)]
    pub entry: MemoryEntry,
    /// `source_unchanged` checks bytes only. It does not prove the claim follows from the source.
    pub evidence_status: String,
    /// `same_line`, `moved` (the cited line's exact text now sits only at `source_line_now`),
    /// `moved_ambiguous` (it sits at several lines), `changed` (it sits nowhere) or
    /// `source_missing`. Absent for rows recorded without a line hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_line_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_line_now: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryRecall {
    /// An absent ledger is different from a present ledger with no matching entries.
    pub ledger_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_status: Option<String>,
    pub hits: Vec<MemoryHit>,
}

pub fn ledger_path(root: &Path) -> PathBuf {
    root.join(LEDGER_PATH)
}

pub fn load(root: &Path) -> Result<MemoryLedger, String> {
    let path = ledger_path(root);
    reject_ledger_symlinks(root)?;
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(MemoryLedger::default()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let ledger: MemoryLedger =
        serde_json::from_str(&body).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    validate(&ledger)?;
    Ok(ledger)
}

fn reject_ledger_symlinks(root: &Path) -> Result<(), String> {
    for path in [root.join(".claudecat"), ledger_path(root)] {
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "memory path must not be a symlink: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate(ledger: &MemoryLedger) -> Result<(), String> {
    if ledger.schema != SCHEMA {
        return Err(format!("unsupported memory schema {}", ledger.schema));
    }
    let mut ids = HashSet::new();
    let by_id: HashMap<_, _> = ledger.entries.iter().map(|e| (e.id.as_str(), e)).collect();
    for entry in &ledger.entries {
        if entry.id.is_empty() || !ids.insert(entry.id.as_str()) {
            return Err(format!("empty or duplicate memory id: {}", entry.id));
        }
        if entry.key.trim().is_empty() || entry.claim.trim().is_empty() {
            return Err(format!("empty key or claim in memory {}", entry.id));
        }
        if entry.source_line == 0 || !safe_relative(Path::new(&entry.source)) {
            return Err(format!("invalid source in memory {}", entry.id));
        }
        if (!entry.resolves.is_empty() && entry.resolves.len() < 2)
            || (entry.supersedes.is_some() && !entry.resolves.is_empty())
        {
            return Err(format!("invalid resolution in memory {}", entry.id));
        }
        let predecessors = entry.supersedes.iter().chain(entry.resolves.iter());
        let mut unique = HashSet::new();
        for old in predecessors {
            if !unique.insert(old) {
                return Err(format!("memory {} repeats a superseded id", entry.id));
            }
            let Some(prior) = by_id.get(old.as_str()) else {
                return Err(format!("memory {} supersedes missing id {old}", entry.id));
            };
            // Supersession is append-only: referring to a later record would allow cycles and
            // could silently leave a key with no active answer after a hand-edited Git merge.
            if prior.key != entry.key || old == &entry.id || !ids.contains(old.as_str()) {
                return Err(format!("memory {} has invalid supersession", entry.id));
            }
        }
    }
    Ok(())
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn source_bytes(root: &Path, relative: &Path) -> Result<Vec<u8>, String> {
    if !safe_relative(relative) {
        return Err("source must be a relative path inside the repository".into());
    }
    let real_root = fs::canonicalize(root).map_err(|e| format!("cannot open project root: {e}"))?;
    let real_source = fs::canonicalize(real_root.join(relative))
        .map_err(|e| format!("cannot open source {}: {e}", relative.display()))?;
    if !real_source.starts_with(&real_root) || !real_source.is_file() {
        return Err("source resolves outside the repository or is not a file".into());
    }
    fs::read(&real_source).map_err(|e| format!("cannot read source {}: {e}", relative.display()))
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn line_hash(bytes: &[u8], line: usize) -> Option<String> {
    std::str::from_utf8(bytes)
        .ok()?
        .lines()
        .nth(line.checked_sub(1)?)
        .map(|text| hash(text.as_bytes()))
}

/// Where the cited line's exact text sits now. This locates the line; it does not vouch for the
/// claim. A line can keep its text and move into a section that reverses its meaning ("Current"
/// to "Deprecated"), so `moved` tells the reader where to look, and the reader still judges the
/// surrounding context before treating the claim as supported.
fn locate_line(bytes: &[u8], line: usize, expected: &str) -> (&'static str, Option<usize>) {
    if line_hash(bytes, line).as_deref() == Some(expected) {
        return ("same_line", None);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return ("changed", None);
    };
    let found: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| hash(l.as_bytes()) == expected)
        .map(|(i, _)| i + 1)
        .collect();
    match found.as_slice() {
        [] => ("changed", None),
        [only] => ("moved", Some(*only)),
        _ => ("moved_ambiguous", None),
    }
}

pub(crate) fn active_entries(ledger: &MemoryLedger) -> Vec<&MemoryEntry> {
    let replaced: HashSet<_> = ledger
        .entries
        .iter()
        .flat_map(|entry| {
            entry
                .supersedes
                .iter()
                .chain(entry.resolves.iter())
                .map(String::as_str)
        })
        .collect();
    ledger
        .entries
        .iter()
        .filter(|entry| !replaced.contains(entry.id.as_str()))
        .collect()
}

// The JSON ledger is read-only now (`load`, for `cort memory import-legacy`). Its writer and reader
// had no caller left once the Turso store took over, and a writer that stamps provenance fields
// the older ClaudeCat client drops is how one id could later import with two different bodies.

pub(crate) fn recall_ledger(
    root: &Path,
    query: &str,
    ledger: &MemoryLedger,
    present: bool,
    sync_status: Option<String>,
) -> MemoryRecall {
    let terms: Vec<_> = query.split_whitespace().map(|s| s.to_lowercase()).collect();
    let active = active_entries(ledger);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for entry in &active {
        *counts.entry(&entry.key).or_default() += 1;
    }
    let mut hits = Vec::new();
    for entry in active {
        let haystack = format!("{} {}", entry.key, entry.claim).to_lowercase();
        if !terms.iter().all(|term| haystack.contains(term)) {
            continue;
        }
        let source = source_bytes(root, Path::new(&entry.source));
        let (source_line_status, source_line_now) = match (&entry.source_line_sha256, &source) {
            (None, _) => (None, None),
            (Some(expected), Ok(bytes)) => {
                let (status, now) = locate_line(bytes, entry.source_line, expected);
                (Some(status.to_string()), now)
            }
            (Some(_), Err(_)) => (Some("source_missing".to_string()), None),
        };
        let evidence_status = if counts[entry.key.as_str()] > 1 {
            "conflict"
        } else {
            match source {
                Ok(bytes) if hash(&bytes) == entry.source_sha256 => "source_unchanged",
                Ok(_) => "source_changed",
                Err(_) => "source_missing",
            }
        };
        hits.push(MemoryHit {
            entry: entry.clone(),
            evidence_status: evidence_status.to_string(),
            source_line_status,
            source_line_now,
        });
    }
    hits.sort_by(|a, b| {
        a.entry
            .key
            .cmp(&b.entry.key)
            .then(a.entry.id.cmp(&b.entry.id))
    });
    MemoryRecall {
        ledger_status: if present { "present" } else { "absent" }.into(),
        sync_status,
        hits,
    }
}

pub(crate) fn new_entry(
    root: &Path,
    key: &str,
    claim: &str,
    source: &Path,
    source_line: usize,
    supersedes: Option<&str>,
) -> Result<MemoryEntry, String> {
    let key = key.trim();
    let claim = claim.trim();
    if key.is_empty() || claim.is_empty() {
        return Err("key and claim must be nonempty".into());
    }
    let bytes = source_bytes(root, source)?;
    let source_text = std::str::from_utf8(&bytes).map_err(|_| "source is not UTF-8")?;
    if source_line == 0
        || source_text
            .lines()
            .nth(source_line - 1)
            .is_none_or(|line| line.trim().is_empty())
    {
        return Err("source line does not exist".into());
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("clock error: {e}"))?
        .as_nanos();
    let source_name = source.to_string_lossy().to_string();
    let source_sha256 = hash(&bytes);
    let id =
        hash(format!("{key}\0{claim}\0{source_name}\0{source_sha256}\0{timestamp}").as_bytes());
    Ok(MemoryEntry {
        id,
        key: key.to_string(),
        claim: claim.to_string(),
        source: source_name,
        source_line,
        source_sha256,
        source_line_sha256: line_hash(&bytes, source_line),
        recorded_at: timestamp.to_string(),
        supersedes: supersedes.map(str::to_string),
        resolves: Vec::new(),
        recorded_machine: Some(crate::usage::machine_id().to_string()),
        recorded_machine_source: Some(crate::usage::machine_id_source().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_source_is_disclosed_and_a_correction_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("decision.md"), "Use option A.\n").unwrap();
        let first = new_entry(
            dir.path(),
            "deployment",
            "Option A",
            Path::new("decision.md"),
            1,
            None,
        )
        .unwrap();
        let mut ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![first.clone()],
        };
        let hit = &recall_ledger(dir.path(), "deployment", &ledger, true, None).hits[0];
        assert_eq!(hit.evidence_status, "source_unchanged");
        fs::write(dir.path().join("decision.md"), "Use option B.\n").unwrap();
        let hit = &recall_ledger(dir.path(), "deployment", &ledger, true, None).hits[0];
        assert_eq!(hit.evidence_status, "source_changed");
        let second = new_entry(
            dir.path(),
            "deployment",
            "Option B",
            Path::new("decision.md"),
            1,
            Some(&first.id),
        )
        .unwrap();
        ledger.entries.push(second.clone());
        validate(&ledger).unwrap();
        let hits = recall_ledger(dir.path(), "deployment", &ledger, true, None).hits;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.id, second.id);
        assert_eq!(hits[0].evidence_status, "source_unchanged");
    }

    #[test]
    fn a_line_that_only_moved_is_located_and_a_rewritten_line_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("README.md");
        fs::write(&path, "# Title\nThe database is harnessuse.\n").unwrap();
        let entry = new_entry(
            dir.path(),
            "memory database",
            "harnessuse",
            Path::new("README.md"),
            2,
            None,
        )
        .unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![entry],
        };
        let recall = |dir: &Path| recall_ledger(dir, "database", &ledger, true, None).hits;

        fs::write(&path, "# Title\n\nIntro.\n\nThe database is harnessuse.\n").unwrap();
        let hit = &recall(dir.path())[0];
        assert_eq!(hit.evidence_status, "source_changed");
        assert_eq!(hit.source_line_status.as_deref(), Some("moved"));
        assert_eq!(hit.source_line_now, Some(5));

        fs::write(
            &path,
            "The database is harnessuse.\n# Title\nThe database is harnessuse.\n",
        )
        .unwrap();
        let hit = &recall(dir.path())[0];
        assert_eq!(hit.source_line_status.as_deref(), Some("moved_ambiguous"));
        assert_eq!(hit.source_line_now, None);

        fs::write(&path, "# Title\nThe database is cortex-v5.\n").unwrap();
        let hit = &recall(dir.path())[0];
        assert_eq!(hit.source_line_status.as_deref(), Some("changed"));
        assert_eq!(hit.source_line_now, None);
    }

    #[test]
    fn new_rows_name_their_machine_and_old_rows_still_parse() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("d.md"), "Use A.\n").unwrap();
        let entry = new_entry(dir.path(), "k", "A", Path::new("d.md"), 1, None).unwrap();
        assert_eq!(
            entry.recorded_machine.as_deref(),
            Some(crate::usage::machine_id())
        );
        let mut body = serde_json::to_value(&entry).unwrap();
        let fields = body.as_object_mut().unwrap();
        fields.remove("recorded_machine");
        fields.remove("recorded_machine_source");
        let old: MemoryEntry = serde_json::from_value(body).unwrap();
        assert_eq!(old.recorded_machine, None);
    }
}
