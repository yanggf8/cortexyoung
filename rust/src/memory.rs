//! Reviewed memory entries and source checks.
//! `memory_store` reads and writes the shared database.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: u32 = 1;
/// English function words left out of partial-match scoring (see `recall_ledger_inner`).
const PARTIAL_IGNORED: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from",
    "how", "in", "into", "is", "it", "of", "on", "or", "should", "that", "the", "this", "to",
    "was", "we", "what", "when", "where", "which", "who", "why", "with",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub claim: String,
    pub source: String,
    pub source_line: usize,
    pub source_sha256: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unsourced: bool,
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

/// JSON presented to an agent; storage continues to serialize `MemoryEntry` directly.
pub fn entry_view(entry: &MemoryEntry) -> serde_json::Value {
    serde_json::Value::Object(entry_fields(entry))
}

fn entry_fields(entry: &MemoryEntry) -> serde_json::Map<String, serde_json::Value> {
    // A struct of strings and integers always serializes to an object; an empty map is the
    // never-taken fallback, so printing an entry can never panic.
    let mut fields = match serde_json::to_value(entry) {
        Ok(serde_json::Value::Object(fields)) => fields,
        _ => serde_json::Map::new(),
    };
    if entry.unsourced {
        fields.remove("source");
        fields.remove("source_line");
        fields.remove("source_sha256");
    }
    if let Ok(ns) = entry.recorded_at.parse::<i128>() {
        let seconds = ns.div_euclid(1_000_000_000);
        let days = seconds.div_euclid(86_400);
        // The requested four-digit UTC year has a finite representable range.
        if (-719_528..=2_932_896).contains(&days) {
            let (year, month, day) = crate::measure::civil_from_days(days as i64);
            let time = seconds.rem_euclid(86_400);
            fields.insert(
                "recorded_at_utc".into(),
                format!(
                    "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
                    time / 3600,
                    time % 3600 / 60,
                    time % 60
                )
                .into(),
            );
        }
    }
    fields
}

#[derive(Clone, Debug)]
pub struct MemoryHit {
    pub entry: MemoryEntry,
    pub matched_terms: Option<Vec<String>>,
    /// Evidence checks source bytes, not whether the claim follows from its context.
    pub evidence_status: String,
    pub source_file_changed: Option<bool>,
    /// `same_line`, `moved` (the cited line's exact text now sits only at `source_line_now`),
    /// `moved_ambiguous` (it sits at several lines), `changed` (it sits nowhere) or
    /// `source_missing`. Absent for rows recorded without a line hash.
    pub source_line_status: Option<String>,
    pub source_line_now: Option<usize>,
}

impl Serialize for MemoryHit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = entry_fields(&self.entry);
        fields.insert(
            "evidence_status".into(),
            self.evidence_status.clone().into(),
        );
        if let Some(changed) = self.source_file_changed {
            fields.insert("source_file_changed".into(), changed.into());
        }
        if let Some(status) = &self.source_line_status {
            fields.insert("source_line_status".into(), status.clone().into());
        }
        if let Some(line) = self.source_line_now {
            fields.insert("source_line_now".into(), line.into());
        }
        if let Some(terms) = &self.matched_terms {
            fields.insert("matched_terms".into(), terms.clone().into());
        }
        fields.serialize(serializer)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryRecall {
    /// An absent ledger is different from a present ledger with no matching entries.
    pub ledger_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_status: Option<String>,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<String>,
    pub hits: Vec<MemoryHit>,
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

pub(crate) fn source_root(root: &Path) -> PathBuf {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| PathBuf::from(path.trim_end_matches(['\n', '\r'])))
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| root.to_path_buf())
}

pub(crate) fn record_source(root: &Path, source: &Path) -> Result<(PathBuf, PathBuf), String> {
    let error = "source must be a relative path inside the repository";
    if source.is_absolute() || source.as_os_str().is_empty() {
        return Err(error.into());
    }
    let top = fs::canonicalize(source_root(root))
        .map_err(|e| format!("cannot open project root: {e}"))?;
    // Resolve through the filesystem, not lexically: `link/../x` follows `link` first, exactly
    // as reading the file will, so the path stored is the file that gets hashed.
    let base = fs::canonicalize(root).map_err(|e| format!("cannot open project root: {e}"))?;
    // Relative to the invoking directory first, as git reads a path; when nothing is there, relative
    // to the top level, because that is how recall prints every source and an agent copies it back.
    // Only when nothing at all is there: a dangling symlink or an unreadable entry at the invoking
    // path is the caller's file and its error is theirs, never a reason to cite a different file.
    let here = base.join(source);
    let nothing_here = nothing_at(&base, source);
    let candidate = if nothing_here { top.join(source) } else { here };
    let real = fs::canonicalize(&candidate).map_err(|e| {
        if nothing_here {
            format!(
                "cannot open source {} (not in the current directory or at the repository top level): {e}",
                source.display()
            )
        } else {
            format!("cannot open source {}: {e}", source.display())
        }
    })?;
    let relative = real
        .strip_prefix(&top)
        .map_err(|_| error.to_string())?
        .to_path_buf();
    if !safe_relative(&relative) {
        return Err(error.into());
    }
    Ok((top, relative))
}

/// True only when walking `relative` from `base` reaches a component that does not exist, with
/// every component before it present and resolvable. A dangling symlink anywhere on the way, or any
/// error other than not-found, means something is there that the caller named.
fn nothing_at(base: &Path, relative: &Path) -> bool {
    let mut path = base.to_path_buf();
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() && fs::metadata(&path).is_err() => {
                return false
            }
            Ok(_) => {}
            Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
        }
    }
    false
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

pub(crate) fn recall_ledger(
    root: &Path,
    query: &str,
    ledger: &MemoryLedger,
    present: bool,
    sync_status: Option<String>,
) -> MemoryRecall {
    recall_ledger_inner(
        &source_root(root),
        query,
        ledger,
        present,
        sync_status,
        None,
    )
    .0
}

/// SessionStart evaluates evidence only for entries it can display. `top` is the work-tree top
/// level, resolved by the caller before its network wait so no subprocess runs after it.
pub(crate) fn recall_ledger_for_hook(top: &Path, ledger: &MemoryLedger) -> (MemoryRecall, usize) {
    recall_ledger_inner(
        top,
        "",
        ledger,
        !ledger.entries.is_empty(),
        Some("shared_database".into()),
        Some(20),
    )
}

fn recall_ledger_inner(
    root: &Path,
    query: &str,
    ledger: &MemoryLedger,
    present: bool,
    sync_status: Option<String>,
    limit: Option<usize>,
) -> (MemoryRecall, usize) {
    let terms: Vec<_> = query.split_whitespace().map(|s| s.to_lowercase()).collect();
    let mut active = active_entries(ledger);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for entry in &active {
        *counts.entry(&entry.key).or_default() += 1;
    }
    if limit.is_some() {
        active.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at).then(b.id.cmp(&a.id)));
    }
    let omitted = limit.map_or(0, |n| active.len().saturating_sub(n));
    if let Some(n) = limit {
        active.truncate(n);
    }
    let mut selected: Vec<_> = active
        .iter()
        .filter(|entry| {
            let haystack = format!("{} {}", entry.key, entry.claim).to_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .map(|entry| (*entry, None))
        .collect();
    // Partial scoring is substring counting, so words that sit inside almost every claim ("to",
    // "the", a lone digit) would outrank the entry the query is about. They still count for the
    // all-terms match above; a single CJK character is a word and is kept.
    let mut distinct = Vec::new();
    for term in &terms {
        let noise =
            PARTIAL_IGNORED.contains(&term.as_str()) || (term.len() == 1 && term.is_ascii());
        if !noise && !distinct.contains(term) {
            distinct.push(term.clone());
        }
    }
    let partial =
        selected.is_empty() && terms.len() >= 2 && limit.is_none() && !distinct.is_empty();
    if partial {
        selected = active
            .iter()
            .filter_map(|entry| {
                let haystack = format!("{} {}", entry.key, entry.claim).to_lowercase();
                let matched: Vec<_> = distinct
                    .iter()
                    .filter(|term| haystack.contains(term.as_str()))
                    .cloned()
                    .collect();
                (matched.len() * 2 >= distinct.len()).then_some((*entry, Some(matched)))
            })
            .collect();
        selected.sort_by(|(a, a_terms), (b, b_terms)| {
            b_terms
                .as_ref()
                .unwrap()
                .len()
                .cmp(&a_terms.as_ref().unwrap().len())
                .then(b.recorded_at.cmp(&a.recorded_at))
                .then(a.id.cmp(&b.id))
        });
        selected.truncate(5);
    }
    let mut hits = Vec::new();
    for (entry, matched_terms) in selected {
        let source = if entry.unsourced {
            Err("unsourced".into())
        } else {
            source_bytes(root, Path::new(&entry.source))
        };
        let (source_line_status, source_line_now) = match (
            if entry.unsourced {
                None
            } else {
                entry.source_line_sha256.as_ref()
            },
            &source,
        ) {
            (None, _) => (None, None),
            (Some(expected), Ok(bytes)) => {
                let (status, now) = locate_line(bytes, entry.source_line, expected);
                (Some(status.to_string()), now)
            }
            (Some(_), Err(_)) => (Some("source_missing".to_string()), None),
        };
        let source_file_changed = source
            .as_ref()
            .ok()
            .map(|bytes| hash(bytes) != entry.source_sha256);
        let evidence_status = if counts[entry.key.as_str()] > 1 {
            "conflict"
        } else if entry.unsourced {
            "unsourced"
        } else {
            match (source_file_changed, source_line_status.as_deref()) {
                (None, _) => "source_missing",
                (Some(_), Some("same_line")) => "source_unchanged",
                (Some(_), Some("moved")) => "source_moved",
                (Some(_), Some(_)) => "source_changed",
                (Some(false), None) => "source_unchanged",
                (Some(true), None) => "source_changed",
            }
        };
        hits.push(MemoryHit {
            entry: entry.clone(),
            matched_terms,
            evidence_status: evidence_status.to_string(),
            source_file_changed,
            source_line_status,
            source_line_now,
        });
    }
    if !partial {
        hits.sort_by(|a, b| {
            a.entry
                .key
                .cmp(&b.entry.key)
                .then(a.entry.id.cmp(&b.entry.id))
        });
    }
    (
        MemoryRecall {
            ledger_status: if present { "present" } else { "absent" }.into(),
            sync_status,
            match_kind: partial.then(|| "partial".into()),
            hits,
        },
        omitted,
    )
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
        unsourced: false,
        source_line_sha256: line_hash(&bytes, source_line),
        recorded_at: timestamp.to_string(),
        supersedes: supersedes.map(str::to_string),
        resolves: Vec::new(),
        recorded_machine: Some(crate::usage::machine_id().to_string()),
        recorded_machine_source: Some(crate::usage::machine_id_source().to_string()),
    })
}

pub(crate) fn new_unsourced_entry(key: &str, claim: &str) -> Result<MemoryEntry, String> {
    let key = key.trim();
    let claim = claim.trim();
    if key.is_empty() || claim.is_empty() {
        return Err("key and claim must be nonempty".into());
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("clock error: {e}"))?
        .as_nanos();
    let id = hash(format!("{key}\0{claim}\0{timestamp}").as_bytes());
    Ok(MemoryEntry {
        source: format!(".cort-unsourced/{id}"),
        source_line: 1,
        source_sha256: String::new(),
        source_line_sha256: None,
        unsourced: true,
        id,
        key: key.into(),
        claim: claim.into(),
        recorded_at: timestamp.to_string(),
        supersedes: None,
        resolves: Vec::new(),
        recorded_machine: Some(crate::usage::machine_id().to_string()),
        recorded_machine_source: Some(crate::usage::machine_id_source().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recall_fixture(id: &str, key: &str, claim: &str, recorded_at: &str) -> MemoryEntry {
        serde_json::from_value(serde_json::json!({
            "id": id, "key": key, "claim": claim, "recorded_at": recorded_at,
            "source": "unused", "source_line": 1, "source_sha256": "", "unsourced": true
        }))
        .unwrap()
    }

    #[test]
    fn recall_all_terms_keeps_normal_output_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![
                recall_fixture("2", "z", "alpha beta", "2"),
                recall_fixture("1", "a", "alpha beta", "1"),
                recall_fixture("3", "partial", "alpha", "3"),
            ],
        };
        let json =
            serde_json::to_value(recall_ledger(dir.path(), "ALPHA beta", &ledger, true, None))
                .unwrap();
        assert!(json.get("match").is_none());
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["key"], "a");
        assert_eq!(hits[1]["key"], "z");
        assert!(hits.iter().all(|hit| hit.get("matched_terms").is_none()));
    }

    #[test]
    fn recall_partial_matches_mixed_language_query() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![
                recall_fixture(
                    "1",
                    "time budgets measured first",
                    "hook timeout on CI",
                    "1",
                ),
                recall_fixture("2", "unrelated", "another subject", "2"),
            ],
        };
        let json = serde_json::to_value(recall_ledger(
            dir.path(),
            "hook timeout 5 秒 CI 時間預算",
            &ledger,
            true,
            None,
        ))
        .unwrap();
        assert_eq!(json["match"], "partial");
        assert_eq!(json["hits"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["hits"][0]["matched_terms"],
            serde_json::json!(["hook", "timeout", "ci"])
        );
        assert_eq!(json["hits"][0]["evidence_status"], "unsourced");
    }

    #[test]
    fn recall_partial_excludes_below_half_and_counts_distinct_terms() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![
                recall_fixture("1", "one", "alpha", "1"),
                recall_fixture("2", "two", "alpha beta", "2"),
            ],
        };
        let json = serde_json::to_value(recall_ledger(
            dir.path(),
            "alpha alpha beta gamma delta",
            &ledger,
            true,
            None,
        ))
        .unwrap();
        assert_eq!(json["hits"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["hits"][0]["matched_terms"],
            serde_json::json!(["alpha", "beta"])
        );
    }

    #[test]
    fn recall_partial_caps_and_orders_by_score_recency_and_id() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![
                recall_fixture("z", "z", "alpha beta gamma", "01"),
                recall_fixture("b", "b", "alpha beta", "05"),
                recall_fixture("a", "a", "alpha beta", "05"),
                recall_fixture("c", "c", "alpha beta", "04"),
                recall_fixture("d", "d", "alpha beta", "03"),
                recall_fixture("e", "e", "alpha beta", "02"),
            ],
        };
        let recall = recall_ledger(dir.path(), "alpha beta gamma delta", &ledger, true, None);
        assert_eq!(
            recall
                .hits
                .iter()
                .map(|hit| hit.entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["z", "a", "b", "c", "d"]
        );
    }

    #[test]
    fn recall_partial_ignores_function_words_and_lone_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![
                recall_fixture("1", "deployed version", "machines run the build", "1"),
                recall_fixture("2", "noise", "nothing to do with the 5 others", "2"),
            ],
        };
        let json = serde_json::to_value(recall_ledger(
            dir.path(),
            "deploy to the 5 server",
            &ledger,
            true,
            None,
        ))
        .unwrap();
        assert_eq!(json["match"], "partial");
        assert_eq!(json["hits"].as_array().unwrap().len(), 1);
        assert_eq!(json["hits"][0]["key"], "deployed version");
        assert_eq!(
            json["hits"][0]["matched_terms"],
            serde_json::json!(["deploy"])
        );
        let only_noise =
            serde_json::to_value(recall_ledger(dir.path(), "to the 7", &ledger, true, None))
                .unwrap();
        assert!(only_noise.get("match").is_none());
        assert!(only_noise["hits"].as_array().unwrap().is_empty());
    }

    #[test]
    fn recall_partial_counts_cjk_terms() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![recall_fixture("1", "budget", "時間預算 以 秒 計", "1")],
        };
        // Only the two CJK terms match; dropping them as noise would leave 0 of 2.
        let json = serde_json::to_value(recall_ledger(
            dir.path(),
            "秒 時間預算 hook alpha",
            &ledger,
            true,
            None,
        ))
        .unwrap();
        assert_eq!(json["match"], "partial");
        assert_eq!(
            json["hits"][0]["matched_terms"],
            serde_json::json!(["秒", "時間預算"])
        );
    }

    #[test]
    fn recall_single_missing_term_stays_empty() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![recall_fixture("1", "key", "claim", "1")],
        };
        let json = serde_json::to_value(recall_ledger(dir.path(), "missing", &ledger, true, None))
            .unwrap();
        assert!(json.get("match").is_none());
        assert!(json["hits"].as_array().unwrap().is_empty());
    }

    #[test]
    fn recall_empty_query_and_hook_keep_normal_output() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![
                recall_fixture("2", "z", "claim", "2"),
                recall_fixture("1", "a", "claim", "1"),
            ],
        };
        let normal = serde_json::to_value(recall_ledger(
            dir.path(),
            "",
            &ledger,
            true,
            Some("shared_database".into()),
        ))
        .unwrap();
        let (hook, omitted) = recall_ledger_for_hook(dir.path(), &ledger);
        assert_eq!(omitted, 0);
        assert_eq!(serde_json::to_value(hook).unwrap(), normal);
        assert!(normal.get("match").is_none());
        assert_eq!(normal["hits"][0]["key"], "a");
        assert!(normal["hits"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit.get("matched_terms").is_none()));
    }

    #[test]
    fn unsourced_body_is_readable_by_old_field_set() {
        #[derive(Deserialize)]
        struct OldEntry {
            id: String,
            key: String,
            claim: String,
            source: String,
            source_line: usize,
            source_sha256: String,
            recorded_at: String,
        }
        let entry = new_unsourced_entry("key", "claim").unwrap();
        let body = serde_json::to_string(&entry).unwrap();
        let old: OldEntry = serde_json::from_str(&body).unwrap();
        assert!(
            !old.id.is_empty()
                && !old.key.is_empty()
                && !old.claim.is_empty()
                && !old.recorded_at.is_empty()
        );
        assert!(
            safe_relative(Path::new(&old.source))
                && old.source_line > 0
                && old.source_sha256.is_empty()
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(source_bytes(dir.path(), Path::new(&old.source)).is_err());
        validate(&MemoryLedger {
            schema: SCHEMA,
            entries: vec![entry],
        })
        .unwrap();
    }

    #[test]
    fn line_evidence_and_file_change_are_reported_independently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("decision.md");
        fs::write(&path, "Heading\nDecision\nOther\n").unwrap();
        let entry = new_entry(dir.path(), "k", "claim", Path::new("decision.md"), 2, None).unwrap();
        let mut ledger = MemoryLedger {
            schema: SCHEMA,
            entries: vec![entry.clone()],
        };
        let check = |ledger: &MemoryLedger, status: &str, file_changed: Option<bool>| {
            let hits = recall_ledger(dir.path(), "k", ledger, true, None).hits;
            assert_eq!(hits[0].evidence_status, status);
            assert_eq!(
                serde_json::to_value(&hits[0])
                    .unwrap()
                    .get("source_file_changed")
                    .and_then(|v| v.as_bool()),
                file_changed
            );
        };
        check(&ledger, "source_unchanged", Some(false));
        fs::write(&path, "Heading\nDecision\nOther edited\n").unwrap();
        check(&ledger, "source_unchanged", Some(true));
        fs::write(&path, "Intro\nHeading\nDecision\nOther\n").unwrap();
        check(&ledger, "source_moved", Some(true));
        assert_eq!(
            recall_ledger(dir.path(), "k", &ledger, true, None).hits[0].source_line_now,
            Some(3)
        );
        fs::write(&path, "Heading\nEdited\nOther\n").unwrap();
        check(&ledger, "source_changed", Some(true));
        ledger.entries[0].source_line_sha256 = None;
        fs::write(&path, "Heading\nDecision\nOther edited\n").unwrap();
        check(&ledger, "source_changed", Some(true));
        fs::write(&path, "Heading\nDecision\nOther\n").unwrap();
        check(&ledger, "source_unchanged", Some(false));
        let mut other = entry;
        other.id = "different".into();
        ledger.entries.push(other);
        check(&ledger, "conflict", Some(false));
        fs::remove_file(&path).unwrap();
        check(&ledger, "conflict", None);
        ledger.entries.pop();
        check(&ledger, "source_missing", None);
    }

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
        assert_eq!(hit.evidence_status, "source_moved");
        assert_eq!(hit.source_line_status.as_deref(), Some("moved"));
        assert_eq!(hit.source_line_now, Some(5));

        fs::write(
            &path,
            "The database is harnessuse.\n# Title\nThe database is harnessuse.\n",
        )
        .unwrap();
        let hit = &recall(dir.path())[0];
        assert_eq!(hit.evidence_status, "source_changed");
        assert_eq!(hit.source_line_status.as_deref(), Some("moved_ambiguous"));
        assert_eq!(hit.source_line_now, None);

        fs::write(&path, "# Title\nThe database is cortex-v5.\n").unwrap();
        let hit = &recall(dir.path())[0];
        assert_eq!(hit.evidence_status, "source_changed");
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
