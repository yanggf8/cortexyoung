//! Reviewable upload of scanned harness notes into the shared database.

use crate::notes::{self, Decoded, ScanReport, ScanStatus};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS harness_notes (project TEXT NOT NULL, machine TEXT NOT NULL, harness TEXT NOT NULL, source_dir TEXT NOT NULL, file_name TEXT NOT NULL, note_id TEXT NOT NULL, sha256 TEXT NOT NULL, title TEXT, description TEXT, kind TEXT, body TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL, PRIMARY KEY (project, machine, harness, source_dir, file_name))";
pub const CREATE_INDEX: &str = "CREATE INDEX IF NOT EXISTS harness_notes_by_project ON harness_notes (project, machine, deleted, updated_at)";
pub const CREATE_MACHINE_INDEX: &str = "CREATE INDEX IF NOT EXISTS harness_notes_by_machine ON harness_notes (machine, harness, project)";
pub const CREATE_ID_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS harness_notes_by_id ON harness_notes (note_id)";
pub const CREATE_PROJECT_TIME_INDEX: &str = "CREATE INDEX IF NOT EXISTS harness_notes_by_project_time ON harness_notes (project, deleted, updated_at)";
pub const CREATE_MACHINES: &str = "CREATE TABLE IF NOT EXISTS harness_machines (machine TEXT PRIMARY KEY, host TEXT NOT NULL, updated_at TEXT NOT NULL)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub project: String,
    pub source_dir: String,
    pub file_name: String,
    pub sha256: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    Upload(Upload),
    Unresolved,
    NotOptedIn,
    SkippedSize,
    SkippedOther,
    SkippedSecret(&'static str),
}

impl Disposition {
    fn label(&self) -> &'static str {
        match self {
            Self::Upload(_) => "upload",
            Self::Unresolved => "unresolved",
            Self::NotOptedIn => "not_opted_in",
            Self::SkippedSize => "skipped_size",
            Self::SkippedOther => "skipped_other",
            Self::SkippedSecret(_) => "skipped_secret",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedNote {
    pub project: Option<String>,
    pub source_dir: String,
    pub file_name: String,
    pub disposition: Disposition,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub scanned: usize,
    pub upload: usize,
    pub unresolved: usize,
    pub not_opted_in: usize,
    pub skipped_size: usize,
    pub skipped_other: usize,
    pub skipped_secret: usize,
    pub unchanged: usize,
    pub upserts: usize,
    pub tombstones: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub notes: Vec<PreparedNote>,
    pub tombstone_scope: BTreeSet<(String, String)>,
    pub counts: Counts,
}

pub fn prepare<'a>(
    report: &'a ScanReport,
    fs_root: &Path,
    opted_in: &BTreeSet<String>,
    resolve: impl Fn(&Path) -> Result<String, String>,
) -> Prepared {
    let mut cache: BTreeMap<&str, Option<String>> = BTreeMap::new();
    let mut project_of = |dir: &'a str| -> Option<String> {
        cache
            .entry(dir)
            .or_insert_with(|| match notes::decode_project_dir(dir, fs_root) {
                Decoded::Unique(path) => resolve(&path).ok(),
                _ => None,
            })
            .clone()
    };
    let mut notes = Vec::with_capacity(report.notes.len());
    let mut scope = BTreeSet::new();
    let mut counts = Counts {
        scanned: report.notes.len(),
        ..Counts::default()
    };
    for note in &report.notes {
        let project = project_of(&note.encoded_dir_name);
        let disposition = if project.is_none() {
            counts.unresolved += 1;
            Disposition::Unresolved
        } else if !opted_in.contains(project.as_ref().unwrap_or(&String::new())) {
            counts.not_opted_in += 1;
            Disposition::NotOptedIn
        } else if note.status == ScanStatus::SkippedSize {
            counts.skipped_size += 1;
            Disposition::SkippedSize
        } else if note.status != ScanStatus::Read {
            counts.skipped_other += 1;
            Disposition::SkippedOther
        } else if let (Some(bytes), Some(sha256)) = (&note.bytes, &note.sha256) {
            match std::str::from_utf8(bytes) {
                Ok(body) => match notes::looks_secret(body) {
                    Some(rule) => {
                        counts.skipped_secret += 1;
                        Disposition::SkippedSecret(rule)
                    }
                    None => {
                        counts.upload += 1;
                        let metadata = notes::frontmatter(body);
                        Disposition::Upload(Upload {
                            project: project.clone().unwrap_or_default(),
                            source_dir: note.encoded_dir_name.clone(),
                            file_name: note.file_name.clone(),
                            sha256: sha256.clone(),
                            title: metadata.name,
                            description: metadata.description,
                            kind: metadata.kind,
                            body: body.to_owned(),
                        })
                    }
                },
                Err(_) => {
                    counts.skipped_other += 1;
                    Disposition::SkippedOther
                }
            }
        } else {
            counts.skipped_other += 1;
            Disposition::SkippedOther
        };
        notes.push(PreparedNote {
            project,
            source_dir: note.encoded_dir_name.clone(),
            file_name: note.file_name.clone(),
            disposition,
        });
    }
    // Tombstones come only from a directory whose listing was read to the end, so one that is
    // now empty still proves its notes are gone, and one that was a symlink or failed proves
    // nothing. An incomplete scan proves nothing anywhere.
    if report.complete {
        for dir in &report.listed_dirs {
            if let Some(project) = project_of(dir).filter(|p| opted_in.contains(p)) {
                scope.insert((project, dir.clone()));
            }
        }
    }
    Prepared {
        notes,
        tombstone_scope: scope,
        counts,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingRow {
    pub project: String,
    pub source_dir: String,
    pub file_name: String,
    pub sha256: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub project: String,
    pub source_dir: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub upserts: Vec<Upload>,
    pub tombstones: Vec<Tombstone>,
    pub counts: Counts,
    pub listing: String,
}

pub fn plan(prepared: &Prepared, existing: &[ExistingRow]) -> Plan {
    let indexed: BTreeMap<_, _> = existing
        .iter()
        .map(|row| {
            (
                (
                    row.project.as_str(),
                    row.source_dir.as_str(),
                    row.file_name.as_str(),
                ),
                row,
            )
        })
        .collect();
    let seen: BTreeSet<_> = prepared
        .notes
        .iter()
        .map(|note| (note.source_dir.as_str(), note.file_name.as_str()))
        .collect();
    let mut counts = prepared.counts.clone();
    let mut upserts = Vec::new();
    let mut listing = Vec::new();
    for note in &prepared.notes {
        let mut label = note.disposition.label();
        if let Disposition::Upload(upload) = &note.disposition {
            match indexed.get(&(
                upload.project.as_str(),
                upload.source_dir.as_str(),
                upload.file_name.as_str(),
            )) {
                Some(row) if !row.deleted && row.sha256 == upload.sha256 => {
                    counts.unchanged += 1;
                    label = "unchanged";
                }
                _ => upserts.push(upload.clone()),
            }
        }
        let rule = match note.disposition {
            Disposition::SkippedSecret(rule) => format!(" rule={rule}"),
            _ => String::new(),
        };
        listing.push(format!(
            "{label}\t{}\t{}/{}{}",
            note.project.as_deref().unwrap_or("?"),
            note.source_dir,
            note.file_name,
            rule
        ));
    }
    let mut tombstones: Vec<_> = existing
        .iter()
        .filter(|row| {
            !row.deleted
                && prepared
                    .tombstone_scope
                    .contains(&(row.project.clone(), row.source_dir.clone()))
                && !seen.contains(&(row.source_dir.as_str(), row.file_name.as_str()))
        })
        .map(|row| Tombstone {
            project: row.project.clone(),
            source_dir: row.source_dir.clone(),
            file_name: row.file_name.clone(),
        })
        .collect();
    for row in &tombstones {
        listing.push(format!(
            "tombstone\t{}\t{}/{}",
            row.project, row.source_dir, row.file_name
        ));
    }
    upserts.sort_by(|a, b| {
        (&a.project, &a.source_dir, &a.file_name).cmp(&(&b.project, &b.source_dir, &b.file_name))
    });
    tombstones.sort_by(|a, b| {
        (&a.project, &a.source_dir, &a.file_name).cmp(&(&b.project, &b.source_dir, &b.file_name))
    });
    listing.sort();
    counts.upserts = upserts.len();
    counts.tombstones = tombstones.len();
    Plan {
        upserts,
        tombstones,
        counts,
        listing: listing.join("\n"),
    }
}

pub fn note_id(
    project: &str,
    machine: &str,
    harness: &str,
    source_dir: &str,
    file_name: &str,
) -> String {
    let joined = [project, machine, harness, source_dir, file_name].join("\0");
    Sha256::digest(joined.as_bytes())
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteBrief {
    pub note_id: String,
    pub machine: String,
    pub host: Option<String>,
    pub file_name: String,
    pub updated_at: String,
}

pub fn opt_in_path() -> PathBuf {
    crate::data_dir::data_dir().join("notes-opt-in")
}

pub fn read_opt_in(path: &Path) -> Result<BTreeSet<String>, String> {
    match fs::read_to_string(path) {
        Ok(body) => Ok(body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn write_opt_in(path: &Path, projects: &BTreeSet<String>) -> Result<(), String> {
    let body = projects
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    crate::data_dir::write_atomic(path, &format!("{body}\n")).map_err(|e| e.to_string())
}
