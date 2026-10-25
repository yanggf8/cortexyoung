//! Index-health observation: one project's index, read-only, and the labels its row carries.
//!
//! `cort measure health` (design §3) observes one project's index at a time, read-only, and
//! records the result in the local `index_health_log` table. [`observe`] is that observation;
//! [`fresh_label`] is the vocabulary its `fresh` cell speaks, defined here so the label rule can be
//! tested without building an index.
//!
//! The `fresh` cell has two readers and one definition. It is what claudecat's long-term metric
//! table has always printed (`row_md`, `claudecat src/cort_audit.rs:562` at `86ed339^`, the commit
//! that deleted it), and it is what the observation rows carry now, so the series stays continuous
//! across the handover.
//!
//! The metric cells are ported from claudecat `src/cort.rs` at that same commit, with one rule the
//! source states but does not always keep: **a failed read is NULL, never zero.** NULL means "could
//! not read it", the reason is in `status`/`error`, and `0` is a fact.

use crate::collect::CollectConfig;
use crate::db::Db;
use rusqlite::OptionalExtension;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// Design §3's four statuses; the same set `index_health_log`'s CHECK constraint allows.
const OK: &str = "ok";
const UNREADABLE: &str = "unreadable";
const MISSING_DIR: &str = "missing_dir";
const NO_INDEX: &str = "no_index";

/// One index's health, as the observation saw it.
///
/// `error` is not only a failure channel: an observation can be `ok` and still owe a reason, when
/// one phase was skipped (`staleness_skipped_large_tree`) or could not run (ast-grep missing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub status: &'static str,
    pub error: Option<String>,
    pub fresh: Option<String>,
    pub chunks: Option<i64>,
    pub relationships: Option<i64>,
    pub not_chunked: Option<i64>,
    pub real_gap: Option<i64>,
    pub fts: Option<String>,
}

impl Observation {
    fn blank(status: &'static str) -> Self {
        Self {
            status,
            error: None,
            fresh: None,
            chunks: None,
            relationships: None,
            not_chunked: None,
            real_gap: None,
            fts: None,
        }
    }

    fn unreadable(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::blank(UNREADABLE)
        }
    }
}

/// Observe one index, read-only.
///
/// **The index is never created, migrated, healed or written.** It is opened with
/// `db::open_db_readonly`, which creates nothing and asks for no write; `immutable=1` is not used
/// even as a fallback, because it promises the file never changes, skips locking and skips the WAL
/// — exactly the reads that would be wrong while a hook is mid-write (design §3).
///
/// Which status when:
///
/// - **`missing_dir` first, then `no_index`.** A project whose directory is gone is `missing_dir`
///   even though its index went with it: the directory's absence is the fact that explains the
///   other one, and design §3 orders it the same way ("目錄也不在就 missing_dir"). Only a live
///   directory with no index file is `no_index`. A path that exists but cannot be stat'ed
///   (permission denied, an I/O error) is neither: "could not look" is not "not there", so it is
///   `unreadable`.
/// - **`unreadable`** when the file is there but cannot be read: a garbage file opens fine (SQLite
///   reads lazily) and fails at the first statement, which is where this reports it; a valid
///   SQLite file with no `chunks` table is not a cort index either. All cells are NULL.
/// - **The index moving under the observation** is checked after the chunks and again before
///   answering. Deleted → `no_index` with `error = index_deleted_during_observation`; replaced (the
///   path names a different file than the one the handle reads) → `unreadable` with
///   `index_replaced_during_observation`. Either way nothing read is carried: it describes a
///   database nobody uses any more. A rebuild or refresh that commits into the *same* file while
///   the observation reads -- `cort index` and the edit hook both do -- is `unreadable` with
///   `index_written_during_observation`: the cells are one snapshot, but the staleness verdict
///   judged that snapshot against a tree the index has since moved with.
/// - **`unreadable`** when the staleness phase fails while walking the tree (`staleness_failed:
///   …`). The cells read before it describe the index this observation is about, so they stay.
/// - **`ok`** otherwise. A metric query that fails leaves its cell NULL and is named in `error`
///   (`cells_unreadable: …`); a staleness phase that was skipped or could not start leaves `fresh`
///   NULL and says why. Several reasons are joined with `; `, so a NULL always has one.
///
/// `cfg` is accepted because design §3's observation has exactly one setting — its wall-clock
/// budget — and that budget is applied by [`observe_bounded`], which owns the thread. The
/// observation itself reads no configuration: it must not become a second place where the timeout
/// is decided.
pub fn observe(root: &Path, local_project_id: &str, cfg: &CollectConfig) -> Observation {
    observe_segmented(root, local_project_id, cfg, |_| {})
}

/// Where [`observe_segmented`] calls its hook.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The chunks have been read; the first move check comes next.
    AfterChunks,
    /// Every cell and the staleness phase are done; the last move check comes next.
    BeforeAnswer,
}

/// [`observe`] with a hook at each [`Stage`].
///
/// Test-only in purpose, public because integration tests link this crate from outside it (a
/// `pub(crate)` hook would be unreachable there). It exists so the move tests can delete or replace
/// the index under a live reader, at either check, instead of simulating the state: what such a
/// reader does must be recorded, and "recorded" is a property of this function, not of a mock.
#[doc(hidden)]
pub fn observe_segmented<F: FnMut(Stage)>(
    root: &Path,
    local_project_id: &str,
    _cfg: &CollectConfig,
    mut at: F,
) -> Observation {
    match presence(root, Want::Dir) {
        Presence::Present => {}
        Presence::Absent => return Observation::blank(MISSING_DIR),
        Presence::Unknown(e) => return Observation::unreadable(format!("root_unstatable: {e}")),
    }
    let index = crate::db::cache_dir().join(format!("{local_project_id}.db"));
    match presence(&index, Want::File) {
        Presence::Present => {}
        Presence::Absent => return Observation::blank(NO_INDEX),
        Presence::Unknown(e) => return Observation::unreadable(format!("index_unstatable: {e}")),
    }
    // The file the handle reads, pinned on both sides of the open: a replacement between the stat
    // and the open shows up as two different identities, so the identity kept is never the new
    // file's while the handle reads the old one.
    let identity = file_identity(&index);
    let db = match crate::db::open_db_readonly(&index) {
        Ok(db) => db,
        Err(e) => return Observation::unreadable(e.to_string()),
    };
    if file_identity(&index) != identity {
        return Observation::unreadable("index_replaced_during_observation");
    }
    // SQLite opens a file it cannot parse and only complains when a statement runs, so this probe
    // — deliberately not a metric — is where `SQLITE_NOTADB` is turned into `unreadable` with all
    // eight cells NULL, rather than into eight unrelated NULLs under an `ok` status.
    if let Err(e) = db.query_row(
        "SELECT count(*) FROM sqlite_master",
        rusqlite::params![],
        |r| r.get::<_, i64>(0),
    ) {
        return Observation::unreadable(e.to_string());
    }

    // `cort index` rebuilds in place -- it deletes and reinserts inside one transaction on the same
    // file -- so no identity check can see it. Two guards instead. Every read from here on runs in
    // one read transaction, so the cells are one snapshot and never half old, half new. And
    // `PRAGMA data_version`, which moves when another connection commits, is compared across that
    // transaction: a commit that landed while this was reading means the staleness verdict judged
    // a snapshot against a tree the index has since caught up with (or fallen behind), so the
    // observation is recorded as such instead of answered.
    let version_before = match data_version(&db) {
        Ok(v) => v,
        Err(e) => return Observation::unreadable(format!("data_version_unreadable: {e}")),
    };
    let tx = match db.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => return Observation::unreadable(format!("read_transaction_unavailable: {e}")),
    };

    // claudecat src/cort.rs:568 (`chunk_count`). Unlike the cells below, this one may not fail
    // quietly: every cort index has a `chunks` table, so a database that cannot answer it -- an
    // empty file, someone else's SQLite file -- is not an index this binary can read, and eight
    // NULLs under `ok` would hide that.
    let chunks = match tx.query_row(
        "SELECT COUNT(*) FROM chunks WHERE project_id = ?1",
        [local_project_id],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(n) => Some(n),
        Err(e) => return Observation::unreadable(format!("chunks_unreadable: {e}")),
    };

    at(Stage::AfterChunks);

    // Checked here to stop early, and again before answering, because the staleness walk below is
    // the longest stretch of the observation and the likeliest place for a rebuild to land.
    if let Some(moved) = moved_under_us(&index, identity) {
        return moved;
    }

    // claudecat src/cort.rs:573. The source answers `.unwrap_or(0)` here; this port keeps the rule
    // the same function states two lines below ("絕不 unwrap_or(0) 假裝健康") instead, because a
    // zero relationship count is a healthy-looking claim and a failed read is not one.
    let relationships = readable_count(
        &tx,
        "SELECT COUNT(*) FROM chunks c JOIN relationships r \
         ON r.source_chunk_id = c.chunk_id WHERE c.project_id = ?1",
        [local_project_id],
    );

    // claudecat src/cort.rs:599 (`not_chunked_total`): files the index knows about but stored no
    // chunk for. The two causes of that are split below, as they are in the source.
    const NOT_CHUNKED: &str = "SELECT COUNT(*) FROM file_state WHERE project_id = ?1 \
         AND file_path NOT IN (SELECT DISTINCT file_path FROM chunks WHERE project_id = ?1)";
    let not_chunked = readable_count(&tx, NOT_CHUNKED, [local_project_id]);

    // claudecat src/cort.rs:624: "scanned, nothing to say" (`chunk_count = 0`) vs "written before
    // v7, never rewritten" (`-1`). SUM returns NULL over zero rows, which is a successful read of
    // "none" -- so those are flattened to 0, while a failed query leaves both unknown.
    const GAP_SPLIT: &str = "SELECT SUM(CASE WHEN chunk_count = 0 THEN 1 ELSE 0 END), \
         SUM(CASE WHEN chunk_count = -1 THEN 1 ELSE 0 END) \
         FROM file_state WHERE project_id = ?1 AND file_path NOT IN \
         (SELECT DISTINCT file_path FROM chunks WHERE project_id = ?1)";
    let split: Option<(i64, i64)> = tx
        .query_row(GAP_SPLIT, [local_project_id], |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            ))
        })
        .ok();

    // claudecat src/cort.rs:441 (`real_gap`): any one of the three facts missing means no verdict,
    // and a negative gap is arithmetic, not information, so it clamps at zero.
    let real_gap = match (not_chunked, split) {
        (Some(total), Some((scanned_empty, unknown))) => {
            Some((total - scanned_empty - unknown).max(0))
        }
        _ => None,
    };

    // claudecat src/cort.rs:661. Row-id drift in both directions: equal counts can hide one
    // missing and one stale document, which is why the source compares rowids rather than totals.
    const FTS_DRIFT: &str = "SELECT \
           (SELECT COUNT(*) FROM chunks c LEFT JOIN chunks_fts f ON f.rowid = c.rowid \
             WHERE f.rowid IS NULL AND c.project_id = ?1) + \
           (SELECT COUNT(*) FROM chunks_fts f LEFT JOIN chunks c ON c.rowid = f.rowid \
             WHERE c.rowid IS NULL)";
    let fts = readable_count(&tx, FTS_DRIFT, [local_project_id]).map(|drift| {
        if drift == 0 {
            "synced".to_string()
        } else {
            format!("drift={drift}")
        }
    });

    // Design §3: a NULL is "could not read it" and the reason is in `status`/`error`, so a cell
    // that failed on its own is named rather than left to look like the other NULLs.
    let mut notes: Vec<String> = Vec::new();
    let cells: Vec<&str> = [
        ("relationships", relationships.is_none()),
        ("not_chunked", not_chunked.is_none()),
        ("real_gap", real_gap.is_none()),
        ("fts", fts.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, missing)| missing.then_some(name))
    .collect();
    if !cells.is_empty() {
        notes.push(format!("cells_unreadable: {}", cells.join(",")));
    }

    // A staleness phase that ran and failed is a read error on a tree that moved under it (design
    // §3: `unreadable` with the reason). The cells above were really read, so they stay.
    let mut skipped: Option<String> = None;
    let (status, fresh) = match staleness_label(&tx, root, local_project_id, &mut skipped) {
        Ok(fresh) => (OK, fresh),
        Err(e) => {
            notes.push(e);
            (UNREADABLE, None)
        }
    };
    notes.extend(skipped);

    at(Stage::BeforeAnswer);
    // Ending the read transaction releases the snapshot; nothing was written, so this is a
    // rollback of nothing.
    drop(tx);
    if let Some(moved) = moved_under_us(&index, identity) {
        return moved;
    }
    match data_version(&db) {
        Ok(v) if v == version_before => {}
        Ok(_) => return Observation::unreadable("index_written_during_observation"),
        Err(e) => return Observation::unreadable(format!("data_version_unreadable: {e}")),
    }

    Observation {
        status,
        error: (!notes.is_empty()).then(|| notes.join("; ")),
        fresh,
        chunks,
        relationships,
        not_chunked,
        real_gap,
        fts,
    }
}

/// A count, or the honest absence of one.
///
/// Every caller is a `COUNT`, which cannot be negative; the filter is the plan's "a `-1` is stored
/// as NULL" rule kept as a guard, not a path any query here reaches. The real `-1` --
/// `file_state.chunk_count`'s "written before v7, never rewritten" marker -- is consumed by the
/// gap split above, where it is counted as `unknown`.
fn readable_count(db: &Db, sql: &str, params: impl rusqlite::Params) -> Option<i64> {
    db.query_row(sql, params, |r| r.get::<_, i64>(0))
        .ok()
        .filter(|value| *value >= 0)
}

/// The `fresh` cell: `Ok(Some)` when the staleness phase ran, `Ok(None)` with `error` set when it
/// was skipped or could not start, `Err` when it ran and failed.
///
/// The split is the one design §3 draws. A skip (`staleness_skipped_large_tree`) and a missing
/// ast-grep are facts about this machine, not about the index, so the observation stays `ok` and
/// says why one cell is NULL. A failure while reading -- `file_state` that will not answer, a
/// candidate file that vanished or became unreadable mid-walk, an extractor that failed on it -- is
/// a read error on a tree that moved, which the design records as `unreadable`.
///
/// The large-tree gate is `heal::ensure_fresh`'s own count and limit (`SELECT COUNT(*) FROM
/// file_state`, `CORT_HEAL_MAX_FILES`), so a tree the heal would defer is a tree the observation
/// will not walk either — it measures, it does not spend a rebuild.
fn staleness_label(
    db: &Db,
    root: &Path,
    local_project_id: &str,
    error: &mut Option<String>,
) -> Result<Option<String>, String> {
    let indexed = readable_count(db, "SELECT COUNT(*) FROM file_state", rusqlite::params![])
        .ok_or_else(|| "staleness_file_state_unreadable".to_string())?;
    if indexed > crate::heal::heal_max_files() {
        *error = Some("staleness_skipped_large_tree".to_string());
        return Ok(None);
    }
    let bin = match crate::ast_grep::resolve_ast_grep_bin() {
        Ok(bin) => bin,
        Err(e) => {
            *error = Some(e.to_string());
            return Ok(None);
        }
    };
    let report = crate::staleness::compute_stale(db, &bin, root, local_project_id)
        .map_err(|e| format!("staleness_failed: {e}"))?;
    Ok(Some(
        fresh_label(
            Some(crate::staleness::repair_label(&report)),
            &report.rebuild_required,
            report.index_is_stale,
            read_graph_pending(db),
        )
        .to_string(),
    ))
}

/// SQLite's per-connection commit counter: it changes between two calls on the same connection if
/// any *other* connection committed in between, and never because of this one.
fn data_version(db: &Db) -> rusqlite::Result<i64> {
    db.query_row("PRAGMA data_version", [], |r| r.get(0))
}

/// What a stat said about a path the observation needs.
enum Presence {
    Present,
    /// Not there, or there as the wrong kind of entry (a file where the project directory was).
    Absent,
    /// Could not be looked at: permission denied, an I/O error. Not the same fact as absent.
    Unknown(std::io::Error),
}

#[derive(Clone, Copy)]
enum Want {
    Dir,
    File,
}

/// `Path::is_dir`/`is_file` answer `false` for "could not stat" too, which would record an
/// unreadable path as a missing one; this keeps the two apart.
fn presence(path: &Path, want: Want) -> Presence {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let right_kind = match want {
                Want::Dir => meta.is_dir(),
                Want::File => meta.is_file(),
            };
            if right_kind {
                Presence::Present
            } else {
                Presence::Absent
            }
        }
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Presence::Absent
        }
        Err(e) => Presence::Unknown(e),
    }
}

/// The index deleted, replaced or no longer stat-able since the handle was opened, as the
/// observation to record instead; `None` when the path still names the file being read.
fn moved_under_us(index: &Path, identity: Option<(u64, u64)>) -> Option<Observation> {
    match presence(index, Want::File) {
        Presence::Absent => Some(Observation {
            error: Some("index_deleted_during_observation".to_string()),
            ..Observation::blank(NO_INDEX)
        }),
        Presence::Unknown(e) => Some(Observation::unreadable(format!(
            "index_unstatable_during_observation: {e}"
        ))),
        Presence::Present if file_identity(index) != identity => {
            Some(Observation::unreadable("index_replaced_during_observation"))
        }
        Presence::Present => None,
    }
}

/// Which file a path names right now, so a reader can tell its own file from a replacement.
///
/// `(device, inode)` on Unix. Elsewhere there is no cheap equivalent and the replacement check is
/// skipped (`None == None`): the deletion check still runs, and cort ships for Linux and macOS.
#[cfg(unix)]
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn file_identity(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// The stored `graph_pending` fact (claudecat `src/cort.rs:104`, `read_meta`).
///
/// Three different answers, and the source keeps all three apart: the table is unreadable (`None`
/// — a label cannot say the graph is fine), the key is absent (`Some(false)` — a fact about an
/// index that was never marked), or the flag is set (`Some(true)`, cort's own test: `== "1"`).
fn read_graph_pending(db: &Db) -> Option<bool> {
    match db
        .query_row(
            "SELECT value FROM _cortex_meta WHERE key = ?1",
            ["graph_pending"],
            |r| r.get::<_, String>(0),
        )
        .optional()
    {
        Ok(value) => Some(value.as_deref() == Some("1")),
        Err(_) => None,
    }
}

/// Run `f` on its own thread and wait at most `timeout` for its answer.
///
/// A timeout is `unreadable`/`timeout`, not a guess: `compute_stale` walks files, git and the
/// extractor in one call with no place to stop, so the only lever is the thread (design §3). The
/// abandoned thread is **not** joined — it only reads, and the worker process outlives it.
pub fn bounded<F: FnOnce() -> Observation + Send + 'static>(
    f: F,
    timeout: Duration,
) -> Observation {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("cort-observe".to_string())
        .spawn(move || {
            // A receiver that has already gone away is the timeout path, not an error worth
            // reporting twice.
            let _ = tx.send(f());
        });
    if let Err(e) = spawned {
        return Observation::unreadable(format!("observation_thread_unavailable: {e}"));
    }
    match rx.recv_timeout(timeout) {
        Ok(observation) => observation,
        Err(mpsc::RecvTimeoutError::Timeout) => Observation::unreadable("timeout"),
        // The thread panicked between spawn and send. Storage failures are returned rather than
        // panicked on, but the observation runs user code (git, the extractor) on a tree that is
        // changing underneath it, so the case is named instead of assumed away.
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Observation::unreadable("observation_thread_failed")
        }
    }
}

/// [`observe`] under this run's per-root wall-clock budget (`CORT_HEALTH_ROOT_TIMEOUT_SECS`).
pub fn observe_bounded(root: &Path, local_project_id: &str, cfg: &CollectConfig) -> Observation {
    let root = root.to_path_buf();
    let project_id = local_project_id.to_string();
    let timeout = Duration::from_millis(cfg.root_timeout_ms);
    let cfg = cfg.clone();
    bounded(move || observe(&root, &project_id, &cfg), timeout)
}

/// The `fresh` label for one observation.
///
/// Ported from claudecat `src/cort_audit.rs:562` (`row_md`, `86ed339^`) with the one correction
/// design §3 makes: a rebuild is classified by its **reasons** before its repair verdict. A
/// `graph_pending` index always carries `graph_incomplete` among its rebuild reasons
/// (`indexer::rebuild_reasons_of`), and any rebuild reason is `rebuild_required` -- so reading the
/// verdict first would print `STALE/rebuild` for an index whose only problem is a half-built graph,
/// which is the one decision a reader can wait out rather than reindex.
///
/// `repair` is [`crate::staleness::repair_label`]'s vocabulary; `stale` is
/// `StaleReport::index_is_stale`; `graph_pending` is the stored meta fact, `None` when it could not
/// be read. There is **no age threshold**: an index nobody touched for any number of days is
/// `fresh` if its content hashes still match (design §3). `no-index` is not a label -- the row's
/// `status` says that, and a label would have to invent a `fresh` value for a database that is not
/// there.
pub fn fresh_label(
    repair: Option<&str>,
    rebuild_reasons: &[String],
    stale: bool,
    graph_pending: Option<bool>,
) -> &'static str {
    if !rebuild_reasons.is_empty() && rebuild_reasons.iter().all(|r| r == "graph_incomplete") {
        return "STALE/graph";
    }
    match repair {
        Some("rebuild_required") => "STALE/rebuild",
        Some("refreshable") => "STALE/refresh",
        _ => match (stale, graph_pending) {
            (_, Some(true)) => "STALE/graph",
            (true, _) => "STALE",
            (false, None) => "fresh?",
            (false, Some(false)) => "fresh",
        },
    }
}
