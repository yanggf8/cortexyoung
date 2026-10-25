//! Index-health observation: the labels an observation writes.
//!
//! `cort measure health` (design §3) observes one project's index at a time, read-only, and
//! records the result in the local `index_health_log` table. Task D2 owns the observation itself;
//! this module owns only the vocabulary its rows carry, so the label rule can be tested without
//! building an index.
//!
//! The `fresh` cell has two readers and one definition. It is what claudecat's long-term metric
//! table has always printed (`row_md`, `claudecat src/cort_audit.rs:562` at `86ed339^`, the commit
//! that deleted it), and it is what the observation rows carry now, so the series stays continuous
//! across the handover.

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
