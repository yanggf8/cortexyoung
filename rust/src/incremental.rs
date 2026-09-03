//! Incremental index + git candidates.
//!
//! Git argv (spec §2):
//!   git -C root diff --name-status -M HEAD
//!   git -C root diff --name-status -M <indexed-head> HEAD
//!   git -C root ls-files --others --exclude-standard

use crate::db::{get_meta, set_meta, Db};
use crate::graph::rebuild_relationships;
use crate::indexer::{
    canonicalize_root, extract_one, full_index, git_head_of, insert_chunk, now_ms,
    replace_file_raw_edges, FullIndexStats, IndexError, IGNORE_DIRS, SOURCE_EXT,
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
    /// Whether this list may be trusted as a *narrowing* of what changed. False is not "git is
    /// missing" -- it is "git would not tell me", which for a caller means: examine everything.
    pub narrowed: bool,
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

fn absorb_name_status(diff: &str, changed: &mut BTreeSet<String>, deleted: &mut BTreeSet<String>) {
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
}

fn cannot_narrow() -> GitCandidates {
    GitCandidates {
        changed: Vec::new(),
        deleted: Vec::new(),
        narrowed: false,
    }
}

/// What might have changed since the index was built.
///
/// `indexed_head` is the head the index was built from, and passing it is not optional bookkeeping:
/// `git diff HEAD` compares the tree to wherever HEAD points *now*, so every commit that arrives
/// without dirtying the tree -- `pull`, `checkout`, `rebase`, `reset`, a teammate's or another
/// agent's commit -- produces an empty diff over a tree the index has never seen. The second diff
/// below is the only thing that closes that window.
///
/// The two ways this can fail are both answered by refusing to narrow at all (`narrowed: false`),
/// because a diff git declined to compute is not evidence that nothing moved: no git in the
/// directory, and a stored head git cannot resolve (force-push, shallow clone, a rebased-away
/// commit, a database carried in from elsewhere).
pub fn git_candidates(root: impl AsRef<Path>, indexed_head: Option<&str>) -> GitCandidates {
    let root = root.as_ref();
    let Some(diff) = git_stdout(root, &["diff", "--name-status", "-M", "HEAD"]) else {
        return cannot_narrow();
    };
    let mut changed = BTreeSet::new();
    let mut deleted = BTreeSet::new();
    absorb_name_status(&diff, &mut changed, &mut deleted);

    if let Some(indexed) = indexed_head {
        // Only a definite agreement lets us skip the second diff -- an unreadable current head is
        // not the same fact as an unmoved one.
        if git_head_of(root).as_deref() != Some(indexed) {
            let Some(moved) = git_stdout(root, &["diff", "--name-status", "-M", indexed, "HEAD"])
            else {
                return cannot_narrow();
            };
            absorb_name_status(&moved, &mut changed, &mut deleted);
        }
    }

    let others =
        git_stdout(root, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for rel in others.lines().filter(|l| !l.is_empty()) {
        if is_indexable(rel) {
            changed.insert(rel.to_string());
        }
    }
    // A path can be deleted by the commits we moved past and restored in the working tree; the
    // caller removes before it reindexes, so listing it in both is the correct instruction.
    GitCandidates {
        changed: changed.into_iter().collect(),
        deleted: deleted.into_iter().collect(),
        narrowed: true,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReindexOneResult {
    pub chunks: i64,
    pub unparsed: i64,
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
    tx.execute(
        "DELETE FROM raw_edges WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )?;
    // The graph is derived from every file's raw edges, so it is recomputed once for the whole
    // project after the last candidate lands — not per file. Mark it until then.
    set_meta(&tx, "graph_pending", "1")?;
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
            skipped: true,
            removed: false,
        });
    }

    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM chunks WHERE project_id = ?1 AND file_path = ?2",
        params![project_id, file_path],
    )?;
    for c in &result.chunks {
        insert_chunk(&tx, c)?;
    }
    replace_file_raw_edges(&tx, project_id, file_path, &result.edges)?;
    tx.execute(
        "INSERT INTO file_state (project_id, file_path, file_content_hash) VALUES (?1, ?2, ?3)
         ON CONFLICT(project_id, file_path) DO UPDATE SET
           file_content_hash = excluded.file_content_hash, updated_at = datetime('now')",
        params![project_id, file_path, result.file_content_hash],
    )?;
    // Relationship resolution deliberately does NOT happen here: it needs the chunks of every
    // other file (an edge's target usually lives elsewhere), so a single file can only ever
    // rebuild its own outgoing edges and would drop everyone else's incoming ones. The project
    // graph is recomputed once, after the last candidate file has landed.
    set_meta(&tx, "graph_pending", "1")?;
    tx.commit()?;

    Ok(ReindexOneResult {
        chunks: result.chunks.len() as i64,
        unparsed: i64::from(result.unparsed),
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

    // A pending graph means a previous run changed chunks without rebuilding the derived
    // relationships (interrupt), or the database predates the raw-edge layer (schema upgrade).
    // Either way only a full re-extraction can be trusted to repopulate it.
    if get_meta(db, "graph_pending")?.as_deref() == Some("1") {
        let full = full_index(db, bin, &canon.path)?;
        return Ok(from_full(full, started));
    }

    let indexed_head = crate::db::indexed_head(db, &canon.project_id)?;
    let cands = git_candidates(&canon.path, indexed_head.as_deref());
    if !cands.narrowed {
        let full = full_index(db, bin, &canon.path)?;
        return Ok(from_full(full, started));
    }

    let mut reindexed = 0i64;
    let mut skipped = 0i64;
    let mut removed = 0i64;

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
        }
    }

    // Final, separate transaction: the derived graph is rebuilt from the raw-edge layer, and
    // freshness markers advance only once every file landed *and* the graph is whole again.
    //
    // The rebuild is skipped when no chunk or raw edge actually moved: a clean `index
    // --incremental` runs on almost every agent turn, and recomputing ~1.8k edges from ~17k raw
    // edges for nothing cost ~1s per call. Entering this function with the graph pending already
    // routed to a full index above, so "nothing changed" really does mean the graph is intact.
    let graph_changed = reindexed > 0 || removed > 0;
    let tx = db.transaction()?;
    let relationships = if graph_changed {
        rebuild_relationships(&tx, &canon.project_id)?
    } else {
        tx.query_row("SELECT COUNT(*) FROM relationships", [], |r| {
            r.get::<_, i64>(0)
        })?
    };
    set_meta(&tx, "graph_pending", "0")?;
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
