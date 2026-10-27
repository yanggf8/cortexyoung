//! Reviewable synchronization of scanned harness notes into the local Turso replica.

use crate::notes::{self, Decoded, ScanReport, ScanStatus};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS harness_notes (project TEXT NOT NULL, machine TEXT NOT NULL, harness TEXT NOT NULL, source_dir TEXT NOT NULL, file_name TEXT NOT NULL, note_id TEXT NOT NULL, sha256 TEXT NOT NULL, title TEXT, description TEXT, kind TEXT, body TEXT NOT NULL, deleted INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL, PRIMARY KEY (project, machine, harness, source_dir, file_name))";
pub const CREATE_INDEX: &str = "CREATE INDEX IF NOT EXISTS harness_notes_by_project ON harness_notes (project, machine, deleted, updated_at)";

pub async fn ensure_notes_schema(conn: &turso::Connection) -> Result<(), String> {
    conn.execute(CREATE_TABLE, ())
        .await
        .map_err(|e| e.to_string())?;
    conn.execute(CREATE_INDEX, ())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

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

pub async fn existing_rows(
    conn: &turso::Connection,
    machine: &str,
    harness: &str,
) -> Result<Vec<ExistingRow>, String> {
    if !table_exists(conn).await? {
        return Ok(Vec::new());
    }
    let mut rows = conn.query("SELECT project, source_dir, file_name, sha256, deleted FROM harness_notes WHERE machine = ? AND harness = ?", (machine, harness)).await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
        out.push(ExistingRow {
            project: row.get(0).map_err(|e| e.to_string())?,
            source_dir: row.get(1).map_err(|e| e.to_string())?,
            file_name: row.get(2).map_err(|e| e.to_string())?,
            sha256: row.get(3).map_err(|e| e.to_string())?,
            deleted: row.get::<i64>(4).map_err(|e| e.to_string())? != 0,
        });
    }
    Ok(out)
}

pub async fn apply(
    conn: &turso::Connection,
    machine: &str,
    harness: &str,
    now_rfc3339: &str,
    plan: &Plan,
) -> Result<(), String> {
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        for note in &plan.upserts {
            let id = note_id(&note.project, machine, harness, &note.source_dir, &note.file_name);
            conn.execute("INSERT INTO harness_notes (project, machine, harness, source_dir, file_name, note_id, sha256, title, description, kind, body, deleted, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?) ON CONFLICT(project, machine, harness, source_dir, file_name) DO UPDATE SET note_id=excluded.note_id, sha256=excluded.sha256, title=excluded.title, description=excluded.description, kind=excluded.kind, body=excluded.body, deleted=0, updated_at=excluded.updated_at", turso::params![note.project.as_str(), machine, harness, note.source_dir.as_str(), note.file_name.as_str(), id.as_str(), note.sha256.as_str(), note.title.as_deref(), note.description.as_deref(), note.kind.as_deref(), note.body.as_str(), now_rfc3339]).await.map_err(|e| e.to_string())?;
        }
        for row in &plan.tombstones {
            conn.execute("UPDATE harness_notes SET deleted=1, body='', sha256='', title=NULL, description=NULL, kind=NULL, updated_at=? WHERE project=? AND machine=? AND harness=? AND source_dir=? AND file_name=? AND deleted=0", (now_rfc3339, row.project.as_str(), machine, harness, row.source_dir.as_str(), row.file_name.as_str())).await.map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    }.await;
    if let Err(error) = result {
        let _ = conn.execute("ROLLBACK", ()).await;
        return Err(error);
    }
    if let Err(error) = conn.execute("COMMIT", ()).await {
        let _ = conn.execute("ROLLBACK", ()).await;
        return Err(error.to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRow {
    pub project: String,
    pub machine: String,
    pub harness: String,
    pub source_dir: String,
    pub file_name: String,
    pub note_id: String,
    pub sha256: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteWithBody {
    pub note: NoteRow,
    pub body: String,
}

async fn table_exists(conn: &turso::Connection) -> Result<bool, String> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='harness_notes'",
            (),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.next().await.map_err(|e| e.to_string())?.is_some())
}

fn read_note(row: &turso::Row) -> Result<NoteRow, String> {
    Ok(NoteRow {
        project: row.get(0).map_err(|e| e.to_string())?,
        machine: row.get(1).map_err(|e| e.to_string())?,
        harness: row.get(2).map_err(|e| e.to_string())?,
        source_dir: row.get(3).map_err(|e| e.to_string())?,
        file_name: row.get(4).map_err(|e| e.to_string())?,
        note_id: row.get(5).map_err(|e| e.to_string())?,
        sha256: row.get(6).map_err(|e| e.to_string())?,
        title: row.get(7).map_err(|e| e.to_string())?,
        description: row.get(8).map_err(|e| e.to_string())?,
        kind: row.get(9).map_err(|e| e.to_string())?,
        updated_at: row.get(10).map_err(|e| e.to_string())?,
    })
}

pub async fn list(conn: &turso::Connection, project: &str) -> Result<Vec<NoteRow>, String> {
    if !table_exists(conn).await? {
        return Ok(Vec::new());
    }
    let mut rows = conn.query("SELECT project, machine, harness, source_dir, file_name, note_id, sha256, title, description, kind, updated_at FROM harness_notes WHERE project=? AND deleted=0 ORDER BY updated_at DESC, machine, harness, source_dir, file_name", (project,)).await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
        out.push(read_note(&row)?);
    }
    Ok(out)
}

pub async fn show(conn: &turso::Connection, id: &str) -> Result<Option<NoteWithBody>, String> {
    if !table_exists(conn).await? {
        return Ok(None);
    }
    let mut rows = conn.query("SELECT project, machine, harness, source_dir, file_name, note_id, sha256, title, description, kind, updated_at, body FROM harness_notes WHERE note_id=? AND deleted=0", (id,)).await.map_err(|e| e.to_string())?;
    let Some(row) = rows.next().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let found = NoteWithBody {
        note: read_note(&row)?,
        body: row.get(11).map_err(|e| e.to_string())?,
    };
    // note_id is a 48-bit prefix: refuse a collision rather than show the wrong note.
    if rows.next().await.map_err(|e| e.to_string())?.is_some() {
        return Err(format!("note id {id} matches more than one note"));
    }
    Ok(Some(found))
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
