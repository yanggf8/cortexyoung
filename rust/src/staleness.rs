//! Staleness: `index_is_stale` signalling. JS `src/staleness.js`.
//! Compared extraction hash, not git dirty and not raw file bytes (spec §7.5).

use crate::db::Db;
use crate::incremental::git_candidates;
use crate::indexer::{canonicalize_root, extract_one, walk_files, IndexError};
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleReport {
    pub index_is_stale: bool,
    pub deleted_files: Vec<String>,
    pub changed_files: Vec<String>,
}

/// `base` is always `projects.path` when a row exists, never cwd (C2-22).
pub fn compute_stale(
    db: &Db,
    bin: &str,
    root: impl AsRef<Path>,
    project_id: &str,
) -> Result<StaleReport, IndexError> {
    let proj: Option<String> = db
        .query_row(
            "SELECT path FROM projects WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .optional()?;
    let base: PathBuf = match proj {
        Some(p) => PathBuf::from(p),
        None => canonicalize_root(root)?.path,
    };

    let mut stored: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = db
            .prepare("SELECT file_path, file_content_hash FROM file_state WHERE project_id = ?1")?;
        let rows = stmt.query_map(params![project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (p, h) = row?;
            stored.insert(p, h);
        }
    }

    let disk_files: HashSet<String> = walk_files(&base).into_iter().collect();

    let mut deleted: Vec<String> = stored
        .keys()
        .filter(|f| !disk_files.contains(*f))
        .cloned()
        .collect();
    deleted.sort();

    let cands = git_candidates(&base);
    let candidates: Vec<String> = if cands.git_available {
        let mut set: HashSet<String> = cands.changed.into_iter().collect();
        for f in &disk_files {
            if !stored.contains_key(f) {
                set.insert(f.clone());
            }
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    } else {
        let mut v: Vec<String> = disk_files.into_iter().collect();
        v.sort();
        v
    };

    let mut changed_files = Vec::new();
    for rel in candidates {
        let abs = base.join(&rel);
        if !abs.exists() {
            continue;
        }
        let source = fs::read_to_string(&abs)?;
        let result = extract_one(bin, project_id, &rel, &abs, &source)?;
        if stored.get(&rel) != Some(&result.file_content_hash) {
            changed_files.push(rel);
        }
    }

    Ok(StaleReport {
        index_is_stale: !deleted.is_empty() || !changed_files.is_empty(),
        deleted_files: deleted,
        changed_files,
    })
}
