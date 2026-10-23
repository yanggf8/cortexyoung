//! Scan Claude Code transcripts into the measurement store, and print one compaction's detail
//! locally. Message text and paths only ever reach stdout through [`compaction_details`]; the
//! store receives counts and nothing else.

use crate::data_dir::{data_dir, write_atomic};
use crate::measure_store::{CompactionRow, MeasureStore};
use crate::memory_store::project_from_git_origin;
use crate::transcript::{compactions, parse_transcript, Compaction, Step, WindowDetails};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-file `(size, mtime_ms)` seen at the last successful scan, keyed by file path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watermarks {
    files: BTreeMap<String, (u64, i64)>,
}

impl Watermarks {
    pub fn default_path() -> PathBuf {
        data_dir().join("measure-transcripts.json")
    }

    /// A missing or unreadable file is an empty set: the worst case is a full rescan.
    pub fn load(path: &Path) -> Self {
        let files = fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Watermarks { files }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(&self.files).map_err(|e| e.to_string())?;
        write_atomic(path, &text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub files_seen: usize,
    pub files_scanned: usize,
    pub compactions: usize,
    pub skipped_lines: usize,
}

pub fn claude_projects_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(dir).join("projects");
    }
    std::env::var_os("HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude/projects")
}

struct TranscriptFile {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}

impl TranscriptFile {
    /// Millisecond form, kept only for the watermark format; ordering uses `mtime`.
    fn mtime_ms(&self) -> i64 {
        self.mtime
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// `projects_dir/*/*.jsonl` only; `<session>/subagents/` is a directory and never matches.
fn transcript_files(projects_dir: &Path) -> Result<Vec<TranscriptFile>, String> {
    collect_files(projects_dir, true).map(|(files, _)| files)
}

/// Strict (`scan`): any traversal error is returned. Lenient (`show`): unreadable entries are
/// skipped and counted, so one bad sibling cannot hide a readable transcript.
fn collect_files(
    projects_dir: &Path,
    strict: bool,
) -> Result<(Vec<TranscriptFile>, usize), String> {
    let mut out = Vec::new();
    let mut unreadable = 0usize;
    // A missing projects dir is a confirmed absence; any other failure is not.
    let projects = match fs::read_dir(projects_dir) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, 0)),
        Err(e) => return Err(format!("cannot read {}: {e}", projects_dir.display())),
    };
    let io = |p: &Path, e: std::io::Error| format!("cannot read {}: {e}", p.display());
    macro_rules! tolerate {
        ($res:expr, $path:expr) => {
            match $res {
                Ok(v) => v,
                Err(e) if strict => return Err(io($path, e)),
                Err(_) => {
                    unreadable += 1;
                    continue;
                }
            }
        };
    }
    for project in projects {
        let project = tolerate!(project, projects_dir);
        let ftype = tolerate!(project.file_type(), &project.path());
        if !ftype.is_dir() {
            continue;
        }
        let entries = tolerate!(fs::read_dir(project.path()), &project.path());
        for entry in entries {
            let entry = tolerate!(entry, &project.path());
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let meta = tolerate!(entry.metadata(), &path);
            if !meta.is_file() {
                continue;
            }
            let mtime = tolerate!(meta.modified(), &path);
            out.push(TranscriptFile {
                path,
                size: meta.len(),
                mtime,
            });
        }
    }
    out.sort_by(|a, b| (a.mtime, &a.path).cmp(&(b.mtime, &b.path)));
    Ok((out, unreadable))
}

fn read_lossy(path: &Path) -> std::io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

pub fn scan_transcripts(
    store: &mut dyn MeasureStore,
    projects_dir: &Path,
    marks: &mut Watermarks,
    machine_id: &str,
) -> Result<ScanSummary, String> {
    if matches!(fs::metadata(projects_dir), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(ScanSummary::default());
    }
    let files = transcript_files(projects_dir)?;
    let mut summary = ScanSummary {
        files_seen: files.len(),
        ..ScanSummary::default()
    };
    let mut projects: HashMap<String, Option<String>> = HashMap::new();
    let mut next: BTreeMap<String, (u64, i64)> = BTreeMap::new();
    for file in &files {
        let key = file.path.to_string_lossy().into_owned();
        let stamp = (file.size, file.mtime_ms());
        if marks.files.get(&key) == Some(&stamp) {
            next.insert(key, stamp);
            continue;
        }
        // An unreadable file keeps its old watermark, so the next scan tries it again.
        let Ok(text) = read_lossy(&file.path) else {
            if let Some(old) = marks.files.get(&key) {
                next.insert(key, *old);
            }
            continue;
        };
        let parsed = parse_transcript(&text);
        summary.files_scanned += 1;
        summary.skipped_lines += parsed.skipped_lines;
        for c in compactions(&parsed.steps) {
            let owner = store.compaction_session(machine_id, &c.boundary.uuid)?;
            if matches!(&owner, Some(s) if *s != c.boundary.session_id) {
                continue;
            }
            let project_id = c.boundary.cwd.as_ref().and_then(|cwd| {
                projects
                    .entry(cwd.clone())
                    .or_insert_with(|| project_from_git_origin(Path::new(cwd)).ok())
                    .clone()
            });
            store.upsert_compaction(&CompactionRow {
                machine_id: machine_id.to_string(),
                boundary_uuid: c.boundary.uuid.clone(),
                session_id: c.boundary.session_id.clone(),
                time_ms: c.boundary.time_ms,
                project_id,
                trigger: c.boundary.trigger.clone(),
                pre_tokens: c.boundary.pre_tokens,
                post_tokens: c.boundary.post_tokens,
                pre: c.pre.counts(),
                post: c.post.counts(),
                post_complete: c.post_complete,
            })?;
            summary.compactions += 1;
        }
        next.insert(key, stamp);
    }
    // Files that disappeared drop out of the watermarks; store rows are never deleted.
    marks.files = next;
    Ok(summary)
}

fn details_json(d: &WindowDetails) -> Value {
    json!({
        "tool_calls": d.tool_calls,
        "rereads": d.rereads,
        "repeated_commands": d.repeated_commands,
        "transcript_lookups": d.transcript_lookups,
        "human_messages": d.human_messages,
        "corrections": d.corrections,
    })
}

fn compaction_json(file: &Path, c: &Compaction) -> Value {
    json!({
        "file": file.to_string_lossy(),
        "uuid": c.boundary.uuid,
        "session_id": c.boundary.session_id,
        "time_ms": c.boundary.time_ms,
        "cwd": c.boundary.cwd,
        "trigger": c.boundary.trigger,
        "pre_tokens": c.boundary.pre_tokens,
        "post_tokens": c.boundary.post_tokens,
        "post_complete": c.post_complete,
        "pre": details_json(&c.pre),
        "post": details_json(&c.post),
    })
}

/// Local detail for one transcript (`id` = file stem, every boundary in it) or one boundary
/// (`id` = boundary uuid). Reads local files only and never opens the store.
pub fn compaction_details(projects_dir: &Path, id: &str) -> Result<Value, String> {
    let mut found = Vec::new();
    let (files, mut unreadable) = collect_files(projects_dir, false)?;
    for file in files {
        let by_stem = file.path.file_stem().and_then(|s| s.to_str()) == Some(id);
        let Ok(text) = read_lossy(&file.path) else {
            unreadable += 1;
            continue;
        };
        if !by_stem && !text.contains(id) {
            continue;
        }
        let parsed = parse_transcript(&text);
        let has_boundary = parsed.steps.iter().any(|s| matches!(s, Step::Boundary(_)));
        if !has_boundary {
            continue;
        }
        for c in compactions(&parsed.steps) {
            if by_stem || c.boundary.uuid == id {
                found.push(compaction_json(&file.path, &c));
            }
        }
    }
    if found.is_empty() {
        return Err(format!(
            "no compaction found for {id} ({unreadable} unreadable entries skipped)"
        ));
    }
    Ok(json!({"id": id, "compactions": found}))
}
