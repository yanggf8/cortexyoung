# Two-tier memory (#11) Phase 0 -- read-only evidence -- Implementation Plan

> **For agentic workers:** one task per dispatch; do not commit; one cargo process at a time with
> `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15`.

**Goal:** the numbers the user needs before phase 1: how the existing ledger would be placed into
tiers, and how many hits real `cort memory recall` calls return.

**Spec:** `docs/superpowers/specs/2026-11-14-memory-tiers-design.md` (Phases → 0, and "Tiers are
derived"). Phase 0 writes no rows to the shared database and adds no table; like every read, it may create
missing tables (idempotent `CREATE TABLE IF NOT EXISTS`).

## Global Constraints

- Read-only against the shared database: no INSERT, no new table (the shared read path may send idempotent `CREATE TABLE IF NOT EXISTS`).
- Pure Rust, no new crate, no `.unwrap()`/`.expect()` outside tests; fmt + clippy `-D warnings` clean.
- Agent-first output: JSON by default like `memory status`, `--lean` TSV with no empty field.

## Review Focus

1. An entry exactly 7 days old (boundary) is placed consistently with the spec's rule.
2. Claims with tabs/newlines do not break the lean TSV.
3. A transcript whose recall output is truncated or not JSON is counted as `unparsed`, not dropped.

---

### Task A: `cort memory list`

**Files:** `rust/src/memory_store.rs` (or a new `rust/src/memory_tiers.rs` for the pure placement),
`rust/src/main.rs` (verb, usage text, help), tests `rust/tests/memory_list.rs` (fake server pattern
of `rust/tests/memory_ledger.rs`).

**Interface:** `cort memory list [--root <ROOT>] [--epoch <YYYY-MM-DD>] [--lean]`.
- Reads the active entries of the alias component exactly as `recall` does (same read, same
  evidence/source status computation -- reuse, don't copy).
- Pure placement `fn place(recorded_at_ns: i128, epoch_ns: i128, now_ns: i128) -> Placement` with
  `tier: short|long`, `expires_at` (short only), `short_term_expired: bool`, and
  `forgetting_due_at = recorded_at + 60d` for long. `--epoch` defaults to today (local midnight, the
  `remind::local_midnight` helper) so the report shows "if phase 1a shipped today".
- `mentions_user`: claim or key contains `user`, `用戶` or `使用者` (case-insensitive ASCII).
- JSON: `{"project_id", "epoch", "summary": {"active", "short", "long", "short_term_expired",
  "mentions_user", "forgetting_due_by_week": {"2026-12-21": n, ...}, "by_evidence": {...}},
  "entries": [{"id","key","recorded_at_utc","age_days","tier","expires_at_utc"?,
  "forgetting_due_at_utc"?, "mentions_user","evidence_status","claim"}]}`, entries oldest first.
- Lean: first line `active=<n> short=<n> long=<n> mentions_user=<n> epoch=<date>`, then one row per
  entry `id8 \t recorded_date \t age_days \t tier \t evidence \t mentions_user(y|n) \t key \t claim`
  with tabs/newlines in key/claim replaced by spaces and claim cut to 160 chars with `…`.

**Tests:** `place_boundaries` (6d23h → short; 7d+1s before epoch → long; short expired after
epoch+7d); `list_json_summary_counts` (fake server with 4 entries incl. one superseded → active 3);
`list_lean_has_no_empty_field_and_escapes_tabs`; `list_reads_alias_component`.

### Task B: `cort-evals recall-hits`

**Files:** `evals/src/recall_hits.rs`, `evals/src/main.rs` (subcommand + help), tests in the module
or `evals/tests/`. Template: `evals/src/capture_gap.rs` (transcript walk, `--project` validation).

**Interface:** `cort-evals recall-hits [--dir <~/.claude/projects>] [--project <dirname>] [--days N] [--ledger FILE]`
- `--ledger FILE` (the JSON of `cort memory list`) adds a `replay` block: each recall invocation's
  query is run against that ledger snapshot with the product's own `memory::recall_selection` and
  bucketed like the observed side (`calls`, `distinct_queries`, `query_unparsed`, `full`, `partial`,
  `share_full_le_5`, `reading`). Only invocations at shell command position count (not text inside
  quotes, backticks or heredocs; those go to `not_invoked`); queries containing `$` or a backtick
  cannot be replayed. The ledger belongs to one project, so use it with `--project`. Subagent
  transcripts (`<project>/<session>/subagents/*.jsonl`) are read too.
- For each Claude Code transcript line with a `Bash` `tool_use` whose command contains
  `cort memory recall`, find the matching `tool_result` (by `tool_use_id`), parse its text as JSON;
  count `hits` length and `match` (`partial` or full). Unparsable → `unparsed`.
- Output JSON: `{"recalls", "unparsed", "full": {"0","1","2-5","6-20","21+"}, "partial": {...},
  "share_full_le_5", "by_project": {dir: {"recalls", "median_hits"}}, "window": {...}}`.

**Tests:** fixture transcripts with a full match of 3 hits, a partial of 5, a 40-hit result, a
non-JSON result, and a `cort memory record` call that must not count.
