//! The outcome vocabularies of the two hook events `cort` wires.
//!
//! `PreToolUse` (`hook-suggest`) is a thin trigger since 2026-10-25: it records `no_payload` when
//! stdin was not a payload it could read and `fired` when it was, then kicks collection. The judge
//! that used to decide whether a search deserved a `cort impact` answer was removed the same day;
//! its vocabulary stays in [`LEGACY_SUGGEST_OUTCOMES`] and [`SUGGEST_DECLINES`] so a census can
//! still attribute the rows it left in `usage.db` to the rule that refused them.
//!
//! `PostToolUse` (`hook-refresh`) keeps its own vocabulary in [`REFRESH_OUTCOMES`].

/// The closed outcome vocabulary `hook-suggest` can record, one entry per writer path. It exists
/// so a funnel over `usage.db` can be checked for closure without re-reading the dispatcher: a
/// census of real rows that surfaces a value not listed here means a writer path grew without the
/// vocabulary growing with it (the test that drives representative branches through the binary
/// holds these against what is actually emitted). Each string's single home:
/// - `no_payload` — the default, installed before stdin is read; also the answer when stdin was
///   not JSON or carried no (or a null) `tool_input`, because the harness has not said what is
///   about to run
/// - `fired` — a payload was read, the row was written, and collection was kicked
///
/// Since the thin trigger (2026-10-25) the hook no longer judges anything, so the vocabulary is
/// two values. Rows written by the judging hook are still in `usage.db` inside its 90-day
/// retention -- and forever on the remote -- and [`LEGACY_SUGGEST_OUTCOMES`] is what a census has
/// to keep recognising so those rows do not fall into `unknown/`.
pub const SUGGEST_OUTCOMES: [&str; 2] = ["no_payload", "fired"];

/// The `hook-suggest` outcomes that only v≤5 rows can carry: the judging hook's vocabulary minus
/// `no_payload`, which the thin trigger still writes. They are not written any more, but the local
/// database keeps 90 days and the remote keeps everything, so a census that did not know them would
/// report live history as `unknown/<value>`. One entry per value, never a prefix rule: a bucket is
/// a name someone can quote.
pub const LEGACY_SUGGEST_OUTCOMES: [&str; 9] = [
    "no_shape",
    "upgrade_stood_down",
    "no_index",
    "no_index_hinted",
    "no_evidence",
    "no_evidence_hinted",
    "hit",
    "hit_stale",
    "hit_yielded",
];

/// The stable identifiers a legacy `no_shape` row carries as its decline tag, split across the two
/// paths that produced it: the payload/parser path (`grep_fields_no_pattern`,
/// `unsupported_tool_surface`, `not_a_search_tool`, `unparseable_command`) and the judging path
/// removed on 2026-10-25 (`pattern_not_symbol`, `non_source_target`, `unindexed_extension`,
/// `concrete_file_read`, `target_not_source`, `context_lines_requested`).
///
/// Identifiers, never prose: mining groups on them, so renaming one is a breaking change to the
/// log's vocabulary. A `no_shape` row whose summary carries no tag at all is the pre-issue-#3
/// shape, a deployment state rather than one more cause — the census names it `decline_absent`
/// instead of guessing.
///
/// Only used to split legacy rows: the thin trigger writes no `no_shape` at all, and these tags
/// exist here so a reader can still break the old ones down by the rule that refused them.
pub const SUGGEST_DECLINES: [&str; 10] = [
    "grep_fields_no_pattern",
    "unsupported_tool_surface",
    "not_a_search_tool",
    "unparseable_command",
    "pattern_not_symbol",
    "non_source_target",
    "unindexed_extension",
    "concrete_file_read",
    "target_not_source",
    "context_lines_requested",
];

/// The closed outcome vocabulary `hook-refresh` can record, one entry per writer path — the
/// `PostToolUse` twin of [`SUGGEST_OUTCOMES`], with the same contract: a census of real rows that
/// surfaces a value not listed here means a writer path grew without the vocabulary growing with
/// it. Unlike the suggest funnel there is no decline layer: every refresh outcome is attributed
/// directly, so the census buckets these verbatim. Each string's single home is `cmd_hook_refresh`
/// (`main.rs`), in the order the function reaches them:
/// - `upgrade_stood_down` — the protected-entry guard was held, before any pack read
/// - `no_index` — the edited path (or the payload's `cwd`) resolves to no indexed project
/// - `db_unavailable` — the resolved project's database would not answer
/// - `no_ast_grep` — the pinned `ast-grep` binary could not be located
/// - `refreshed` — `incremental_index` reindexed or removed at least one file
/// - `already_current` — `incremental_index` ran and changed nothing
/// - `rebuild_required` — the index demands a full rebuild under `RebuildPolicy::Forbid`
/// - `busy_or_failed` — any other `incremental_index` failure (lock contention included)
pub const REFRESH_OUTCOMES: [&str; 8] = [
    "upgrade_stood_down",
    "no_index",
    "db_unavailable",
    "no_ast_grep",
    "refreshed",
    "already_current",
    "rebuild_required",
    "busy_or_failed",
];
