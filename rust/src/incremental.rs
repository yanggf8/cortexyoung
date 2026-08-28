//! Incremental index + git candidates. JS `src/incremental.js`.
//!
//! Git argv (spec §2):
//!   git -C root diff --name-status -M HEAD
//!   git -C root ls-files --others --exclude-standard

use crate::db::{get_meta, set_meta, Db};
use crate::indexer::{
    canonicalize_root, extract_one, full_index, git_head_of, insert_chunk, insert_rel, now_ms,
    FullIndexStats, IndexError, IGNORE_DIRS, SOURCE_EXT,
};
use rusqlite::{params, OptionalExtension};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCandidates {
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    pub git_available: bool,
}

fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let r = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !r.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&r.stdout).into_owned())
}

fn ext_of(rel: &str) -> String {
    Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default()
}

fn is_indexable(rel: &str) -> bool {
    if !SOURCE_EXT.contains(&ext_of(rel).as_str()) {
        return false;
    }
    !rel.split('/').any(|seg| IGNORE_DIRS.contains(&seg))
}

pub fn git_candidates(root: impl AsRef<Path>) -> GitCandidates {
    let root = root.as_ref();
    let Some(diff) = git_stdout(root, &["diff", "--name-status", "-M", "HEAD"]) else {
        return GitCandidates {
            changed: Vec::new(),
            deleted: Vec::new(),
            git_available: false,
        };
    };
    let mut changed = BTreeSet::new();
    let mut deleted = BTreeSet::new();
    for line in diff.lines().filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }
        let status = parts[0];
        if status.starts_with('R') {
            // rename: old path dies, new path is rebuilt
            if parts.len() >= 2 && is_indexable(parts[1]) {
                deleted.insert(parts[1].to_string());
            }
            if parts.len() >= 3 && is_indexable(parts[2]) {
                changed.insert(parts[2].to_string());
            }
        } else if status.starts_with('D') {
            if parts.len() >= 2 && is_indexable(parts[1]) {
                deleted.insert(parts[1].to_string());
            }
        } else if parts.len() >= 2 && is_indexable(parts[1]) {
            changed.insert(parts[1].to_string());
        }
    }
    let others =
        git_stdout(root, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for rel in others.lines().filter(|l| !l.is_empty()) {
        if is_indexable(rel) {
            changed.insert(rel.to_string());
        }
    }
    GitCandidates {
        changed: changed.into_iter().collect(),
        deleted: deleted.into_iter().collect(),
        git_available: true,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReindexOneResult {
    pub chunks: i64,
    pub unparsed: i64,
    pub relationships: i64,
    pub skipped: bool,
    pub removed: bool,
}

pub fn remove_file(db: &mut Db, project_id: &str, file_path: &str) -> Result<(), IndexError> {
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM chunks WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )?;
    tx.execute(
        "DELETE FROM file_state WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn reindex_one_file(
    db: &mut Db,
    bin: &str,
    root: impl AsRef<Path>,
    project_id: &str,
    file_path: &str,
) -> Result<ReindexOneResult, IndexError> {
    let abs: PathBuf = root.as_ref().join(file_path);
    if !abs.exists() {
        remove_file(db, project_id, file_path)?;
        return Ok(ReindexOneResult {
            chunks: 0,
            unparsed: 0,
            relationships: 0,
            skipped: false,
            removed: true,
        });
    }

    let source = fs::read_to_string(&abs)?;
    let result = extract_one(bin, project_id, file_path, &abs, &source)?;

    let prior: Option<String> = db
        .query_row(
            "SELECT file_content_hash FROM file_state WHERE project_id = ?1 AND file_path = ?2",
            params![project_id, file_path],
            |r| r.get(0),
        )
        .optional()?;
    if prior.as_deref() == Some(result.file_content_hash.as_str()) {
        return Ok(ReindexOneResult {
            chunks: 0,
            unparsed: 0,
            relationships: 0,
            skipped: true,
            removed: false,
        });
    }

    let mut relationships = 0i64;
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM chunks WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )?;
    for c in &result.chunks {
        insert_chunk(&tx, c)?;
    }
    tx.execute(
        "INSERT INTO file_state (project_id, file_path, file_content_hash) VALUES (?1, ?2, ?3)
         ON CONFLICT(project_id, file_path) DO UPDATE SET
           file_content_hash = excluded.file_content_hash, updated_at = datetime('now')",
        params![project_id, file_path, result.file_content_hash],
    )?;
    let rows = crate::graph::relationship_rows_for_file(
        &tx,
        project_id,
        file_path,
        &result.chunks,
        &result.edges,
    )?;
    for row in rows {
        insert_rel(&tx, &row)?;
        relationships += 1;
    }
    tx.commit()?;

    Ok(ReindexOneResult {
        chunks: result.chunks.len() as i64,
        unparsed: i64::from(result.unparsed),
        relationships,
        skipped: false,
        removed: false,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalIndexResult {
    pub mode: String,
    pub files: i64,
    pub chunks: i64,
    pub unparsed: i64,
    pub files_examined: i64,
    pub files_reindexed: i64,
    pub files_skipped: i64,
    pub files_removed: i64,
    pub relationships: i64,
    pub elapsed_ms: i64,
}

fn from_full(full: FullIndexStats, started: Instant) -> IncrementalIndexResult {
    IncrementalIndexResult {
        mode: "full".to_string(),
        files: full.files,
        chunks: full.chunks,
        unparsed: full.unparsed,
        files_examined: 0,
        files_reindexed: 0,
        files_skipped: 0,
        files_removed: 0,
        relationships: full.relationships,
        elapsed_ms: started.elapsed().as_millis() as i64,
    }
}

/// Entry: canonicalize root, then `project_id_for` (via `full_index` / `canonicalize_root`).
pub fn incremental_index(
    db: &mut Db,
    bin: &str,
    root: impl AsRef<Path>,
) -> Result<IncrementalIndexResult, IndexError> {
    let started = Instant::now();
    let canon = canonicalize_root(root)?;
    let version = crate::pack::extractor_version();
    let stored = get_meta(db, "extractor_version")?;

    if let Some(stored) = stored.as_deref() {
        if stored != version.as_str() {
            eprintln!("extractor_version mismatch: {stored} -> {version}, full reindex required");
            let full = full_index(db, bin, &canon.path)?;
            return Ok(from_full(full, started));
        }
    }

    let cands = git_candidates(&canon.path);
    if !cands.git_available {
        let full = full_index(db, bin, &canon.path)?;
        return Ok(from_full(full, started));
    }

    let mut reindexed = 0i64;
    let mut skipped = 0i64;
    let mut removed = 0i64;
    let mut relationships = 0i64;

    for file_path in &cands.deleted {
        remove_file(db, &canon.project_id, file_path)?;
        removed += 1;
    }
    for file_path in &cands.changed {
        let r = reindex_one_file(db, bin, &canon.path, &canon.project_id, file_path)?;
        if r.removed {
            removed += 1;
        } else if r.skipped {
            skipped += 1;
        } else {
            reindexed += 1;
            relationships += r.relationships;
        }
    }

    // Final, separate transaction: freshness markers advance only once every file landed.
    let tx = db.transaction()?;
    tx.execute(
        "UPDATE projects SET git_head = ?1, last_indexed_at = ?2, extractor_version = ?3 WHERE project_id = ?4",
        params![git_head_of(&canon.path), now_ms(), version, canon.project_id],
    )?;
    set_meta(&tx, "extractor_version", &version)?;
    tx.commit()?;

    Ok(IncrementalIndexResult {
        mode: "incremental".to_string(),
        files: 0,
        chunks: 0,
        unparsed: 0,
        files_examined: (cands.changed.len() + cands.deleted.len()) as i64,
        files_reindexed: reindexed,
        files_skipped: skipped,
        files_removed: removed,
        relationships,
        elapsed_ms: started.elapsed().as_millis() as i64,
    })
}
