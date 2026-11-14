# Memory tiers phase 1a Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A shared lifecycle table over the ledger: promote/archive/drop/restore, a review list, and recall and SessionStart that hide archived entries and put entries due for review last, behind a per-project switch.

**Architecture:** One pure module (`memory_lifecycle.rs`) folds events + ledger + clock into a `LifecycleView`. Storage adds one table read beside the ledger read in every pipeline that already reads the ledger. Recall and SessionStart take an `Option<&LifecycleView>`; `None` (or a disabled view) is today's behaviour byte for byte.

**Tech Stack:** Rust (existing `cort` crate), serde_json, the existing `RemoteDb` pipeline and fake transport in tests.

**Spec:** `docs/superpowers/specs/2026-11-14-memory-tiers-phase1a-design.md` (parent: `2026-11-14-memory-tiers-design.md`). Calls C1-C4 settled 2026-11-14 (ledger key `memory-tiers-phase1a-decisions`).

## Global Constraints

- Disabled project (no `on`, or latest switch `off`), dropped lifecycle read, or unaliased-retry answer: output byte-identical to today's everywhere.
- Table: `CREATE TABLE IF NOT EXISTS project_memory_lifecycle (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))`; writes `INSERT OR IGNORE`; never update/delete.
- Short-term 7 days, forgetting 60 days (reuse `memory_tiers::place`, `SHORT_TERM_NS`, `FORGETTING_NS`).
- `quote` 1-600 bytes, not blank; `note` ≤ 300 bytes.
- Storage failures are returned, never panicked on; no `.unwrap()`/`.expect()` outside tests; no new crate.
- SessionStart: no write, no lock, no extra round trip; deadline/watchdog unchanged; 9,500-byte total in `hook_recall.rs` stands.
- Never write a probe row to the shared database; every test uses the fake transport.
- Build hygiene: `uptime` first; `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15 cargo ...`, one cargo at a time; implementers never commit (the integrator commits named files after review).

## Review Focus

1. An entry whose `recorded_at` does not parse (phase 0 met these): it must never be hidden or demoted by derivation -- state `long_term`, `due_at` none, explicit events still apply. Test in Task 1.
2. An event naming an entry by an id that later became superseded while the entry it superseded is archived: `record --replace` on an archived key must still work (archived entries keep the key) and the successor starts short-term. Test in Task 3.
3. Archive with `--superseded-by` pointing at an entry that is itself due for review (allowed: it is active and not hidden). Test in Task 4.
4. SessionStart for a project whose every entry is hidden: no ledger block, no "left out" line, and the weekend line still prints when review has items. Test in Task 6.
5. Recall `--all` with a query that matches only hidden entries on an enabled project: hits carry `lifecycle: archived|dropped`; without `--all` the answer is the same as "no match" today (`ledger_status` unchanged). Test in Task 3.

---

### Task 1: Pure fold (`memory_lifecycle.rs`)

**Files:**
- Create: `rust/src/memory_lifecycle.rs`
- Modify: `rust/src/lib.rs` (add `pub mod memory_lifecycle;`)

**Interfaces:**
- Consumes: `memory_tiers::{place, Placement, Tier}`, `memory::{MemoryLedger, MemoryEntry, active_entries}`.
- Produces:
  ```rust
  pub enum Kind { On, Off, Promote, Archive, Drop, Restore, #[serde(other)] Unknown } // serde lowercase
  pub struct LifecycleEvent { pub id: String, pub kind: Kind, pub entry: Option<String>,
      pub superseded_by: Option<String>, pub quote: Option<String>, pub note: Option<String>,
      pub recorded_at: String, pub recorded_machine: Option<String>,
      pub recorded_machine_source: Option<String>, pub harness: Option<String> } // Serialize+Deserialize, Option fields skip when None
  pub fn kind_name(kind: Kind) -> &'static str;
  pub fn decode_rows(bodies: &[String]) -> (Vec<LifecycleEvent>, usize); // (decoded, unreadable)
  pub enum State { ShortTerm, ShortTermExpired, LongTerm, ForgettingCandidate, Archived, Dropped }
  impl State { pub fn as_str(self) -> &'static str; pub fn is_hidden(self) -> bool; pub fn is_due(self) -> bool }
  pub struct EntryState { pub state: State, pub due_at_ns: Option<i128>, pub history: Vec<LifecycleEvent> }
  pub struct LifecycleView { pub enabled: bool, pub epoch_ns: Option<i128>,
      pub entries: HashMap<String, EntryState>, pub superseded: Vec<(String, Vec<LifecycleEvent>)>,
      pub orphans: Vec<LifecycleEvent>, pub unreadable: usize }
  impl LifecycleView { pub fn state_of(&self, id: &str) -> Option<State> } // None when !enabled
  pub fn fold(ledger: &MemoryLedger, events: Vec<LifecycleEvent>, unreadable: usize, now_ns: i128, epoch_override: Option<i128>) -> LifecycleView;
  pub fn stamp(now_ns: i128, events: &[LifecycleEvent], entry: Option<&str>) -> i128; // entry None = switch events
  ```
  `as_str`: `short_term`, `short_term_expired`, `long_term`, `forgetting_candidate`, `archived`, `dropped`.
  `fold` with `epoch_override = Some(e)` uses `e` as the epoch and computes states even when the project is disabled (used by `review` what-if and `list --epoch`); `enabled` still reports the real switch.

- [ ] **Step 1: Write failing unit tests** (`#[cfg(test)] mod tests` in the new file), one per spec line:
  `no_switch_is_disabled` (`enabled == false`, `state_of` → `None`); `off_after_on_disables`;
  `epoch_is_on_that_began_current_run` (on@T1, on@T2 → epoch T1; on@T1, off@T2, on@T3 → epoch T3);
  `earlier_run_events_still_apply` (archive in run 1 stays archived after off/on);
  one test per transition row: `promote_short_to_long` (last activity = event time, so `due_at = event + 60d`), `drop_short_hides`, `archive_long_hides`, `restore_archived_long_refreshes_activity`, `restore_dropped_short_expires_seven_days_later`;
  `not_allowed_event_is_inert` (promote on long-term: state unchanged, event in `history`);
  `equal_recorded_at_ordered_by_id`; `unknown_kind_changes_nothing`; `orphan_event_listed`;
  `event_on_superseded_entry_listed_as_superseded`;
  `decode_skips_and_counts_bad_rows` (one bad JSON + one `archive` without `entry` → unreadable 2, the rest apply);
  `unparsed_recorded_at_is_long_term_never_due` (Review Focus 1);
  `placement_boundaries` (recorded exactly `epoch - 7d` → short-term; expiry exactly `now` → `short_term_expired`);
  `forgetting_candidate_at_sixty_days`;
  `stamp_lands_after_existing_later_event` (`stamp(100, [event on X at 500], Some(X)) == 501`; unrelated entry ignored; switch events for `None`).
- [ ] **Step 2: Run, see them fail.** `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15 cargo test --manifest-path rust/Cargo.toml --lib memory_lifecycle` → compile errors/failures.
- [ ] **Step 3: Implement.** Order events by (`recorded_at` parsed as i128, `id`); a non-integer `recorded_at` makes the row unreadable in `decode_rows`. A known kind missing `entry` (all but `on`/`off`) is unreadable. Transition table exactly as the spec's.
- [ ] **Step 4: Run, see them pass;** then `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` in `rust/`.

### Task 2: Storage read and alias tables

**Files:**
- Modify: `rust/src/memory_store.rs` (`CREATE_LIFECYCLE` const near line 21; `Ledger::read_bodies` ~117 to named indexes; new `Ledger::read_with_lifecycle`; `AliasSnapshot` ~1799 and the alias add/remove checks ~2007, ~2121, ~2166)
- Test: existing fake-transport tests in `rust/src/memory_store.rs` / `rust/tests/` that cover `read_bodies` and alias add/remove.

**Interfaces:**
- Consumes: Task 1 `decode_rows`.
- Produces:
  ```rust
  pub(crate) const CREATE_LIFECYCLE: &str; // the Global Constraints DDL
  pub struct LifecycleRows { pub events: Vec<memory_lifecycle::LifecycleEvent>, pub unreadable: usize }
  impl Ledger { pub fn read_with_lifecycle(&self, project: &str) -> Result<(MemoryLedger, LifecycleRows), LedgerError> }
  ```
  `Ledger::read` keeps its signature and result. Both send `CREATE_LIFECYCLE` in the same create batch; `read_with_lifecycle` adds `SELECT body FROM project_memory_lifecycle WHERE project IN (COMPONENT)` in the same pipeline (one round trip).

- [ ] **Step 1: Failing tests:** `read_with_lifecycle_one_round_trip` (fake transport sees one pipeline containing both SELECTs and the CREATE); `read_with_lifecycle_counts_bad_rows`; `alias_remove_refuses_when_lifecycle_rows_cross` and `alias_add_accepts_old_id_with_only_lifecycle_rows`, mirroring the existing reminders tests for those checks.
- [ ] **Step 2: Run, see them fail** (`cargo test --lib memory_store` and the alias test file).
- [ ] **Step 3: Implement.** Replace `results[4]` with indexes captured at push time; add `lifecycle` to `AliasSnapshot` and every place the snapshot's `reminders` is consulted.
- [ ] **Step 4: Run, pass; fmt + clippy.**

### Task 3: Recall hides and labels

**Files:**
- Modify: `rust/src/memory.rs` (`MemoryHit` + its `Serialize`, `MemoryRecall`, `recall_ledger_inner`, `recall_ledger_for_hook` signature only as in Task 6 -- here pass `None`), `rust/src/memory_store.rs` (`recall_with`), `rust/src/main.rs` (`memory recall --all`)

**Interfaces:**
- Consumes: Task 1 `LifecycleView`, `State`; Task 2 `read_with_lifecycle`.
- Produces:
  - `MemoryHit.lifecycle: Option<String>` -- serialized as `"lifecycle"` only when `Some`; set only for `is_due() || is_hidden()` states.
  - `MemoryRecall.lifecycle_unreadable: Option<usize>` (`skip_serializing_if = "Option::is_none"`, set when > 0) and `lifecycle_hint: Option<String>` = `"some lifecycle rows could not be read; cort memory review --all lists what was applied, cort memory recall --all finds anything hidden"` when `lifecycle_unreadable` is set.
  - `pub(crate) fn recall_ledger_view(root: &Path, query: &str, ledger: &MemoryLedger, present: bool, sync_status: Option<String>, view: Option<&LifecycleView>, include_hidden: bool) -> MemoryRecall`; `recall_ledger` calls it with `None, false`.
  - `memory_store::recall_with(root, query, ledger)` → keeps signature; new `recall_all_with(root, query, ledger, include_hidden: bool)`; `recall_with` = `include_hidden: false`.
  - `recall_selection` (evals) is unchanged.

- [ ] **Step 1: Failing tests** in `memory.rs` tests: `hidden_entries_do_not_change_partial_fallback` (an archived entry that would have matched a term is gone, and the partial pick equals the pick on a ledger without it); `include_hidden_labels_archived`; `healthy_hits_serialize_without_lifecycle` (JSON identical to `recall_ledger` output); `disabled_view_is_byte_identical`; `conflict_counts_hidden_partner` (visible hit keeps `evidence_status: "conflict"`); `only_hidden_matches_reads_as_no_match` (Review Focus 5). In `rust/tests/` (fake transport): `record_replace_on_archived_key_starts_short_term` (Review Focus 2).
- [ ] **Step 2: Run, fail.**
- [ ] **Step 3: Implement.** Filter hidden ids out of `active` before `select_terms` unless `include_hidden`; the `counts` map for `conflict` is built before the filter.
- [ ] **Step 4: Run, pass; fmt + clippy; `cargo test --test cli`** (help text lists `--all`).

### Task 4: Write verbs and the switch

**Files:**
- Create: `rust/src/lifecycle_store.rs` (pattern: `remind_store.rs`)
- Modify: `rust/src/lib.rs`, `rust/src/main.rs` (`MemoryAction::{Promote, Archive, Drop, Restore, Lifecycle}`, usage string, lean rendering), `rust/tests/cli.rs` (verb list)
- Test: `rust/tests/memory_lifecycle.rs` (fake transport)

**Interfaces:**
- Consumes: Tasks 1-2; `memory_tiers::mentions_user`; the id rule of `memory_store::resolve_ids` (make a shared `pub(crate) fn resolve_one(ledger: &MemoryLedger, id: &str) -> Result<String, LedgerError>` and have `resolve_ids` use it).
- Produces:
  ```rust
  pub enum Verb { Promote, Archive, Drop, Restore }
  pub struct Release { pub superseded_by: Option<String>, pub quote: Option<String> }
  pub fn act_with(root: &Path, ledger: &Ledger, verb: Verb, id: &str, release: Release, note: Option<String>) -> Result<Value, LedgerError>;
  pub fn switch_with(root: &Path, ledger: &Ledger, on: bool, note: Option<String>) -> Result<Value, LedgerError>;
  pub fn render_lean(verb: &str, payload: &Value) -> String;
  ```
  Error codes (spec): `memory_lifecycle_off`, `memory_id_unknown`, `memory_id_ambiguous`, `memory_entry_superseded` (names the successor id), `memory_lifecycle_not_allowed` (payload has `state` and `allowed` verbs), `memory_archive_needs_release`, `memory_quote_invalid`; unknown write outcome as `remind_store` reports it. Event id: `memory::hash(kind\0entry\0machine\0stamp)[..16]` like `remind_store::new_event`; `recorded_at = memory_lifecycle::stamp(...)`.

- [ ] **Step 1: Failing tests:** `each_verb_writes_one_insert_or_ignore`; `refusals_write_nothing` (table-driven over every error code); `disabled_project_writes_nothing_but_on`; `guard_user_in_key`, `guard_user_in_claim`, `guard_cjk_terms`; `superseded_by_self_hidden_superseded_unknown_refused`; `superseded_by_due_entry_allowed` (Review Focus 3); `quote_empty_blank_601_bytes_refused`; `prefix_exact_non_hex_id`, `prefix_ambiguous`, `prefix_of_superseded_entry_refused_with_successor`; `write_stamps_after_later_event`.
- [ ] **Step 2: Run, fail.**
- [ ] **Step 3: Implement** reading with `read_with_lifecycle`, folding at `now`, checking, writing with the read-back confirmation `remind_store::write_event` uses.
- [ ] **Step 4: Run, pass; fmt + clippy; `--test cli --test memory_lifecycle`.**

### Task 5: `cort memory review` and `list`

**Files:**
- Modify: `rust/src/lifecycle_store.rs` (review), `rust/src/memory_tiers.rs` (`build` takes the view), `rust/src/main.rs` (`MemoryAction::Review { root, all, lean }`; `List` reads lifecycle), `docs/memory.md` (review, verbs, switch, guard -- one section)
- Test: `rust/tests/memory_lifecycle.rs`, `rust/tests/memory_list.rs`

**Interfaces:**
- Consumes: Tasks 1, 2, 4.
- Produces: `pub fn review_with(root: &Path, ledger: &Ledger, all: bool, now_ns: i128) -> Result<Value, LedgerError>`; JSON: `lifecycle` (`on`/`off`), `epoch_utc`, `counts` per state, `items` oldest `due_at` first, each `{id, key, claim, state, recorded_at_utc, due_at_utc, mentions_user, commands: [..]}`; with `all`: `hidden`, `superseded`, `orphans`; `lifecycle_unreadable` + hint as Task 3. Commands are complete strings, e.g. `cort memory archive <id8> --superseded-by <id>` when `mentions_user`.
  `memory_tiers::build(.., view: Option<&LifecycleView>)`: with a view, the `tier` column is `State::as_str`; without, phase 0 output unchanged.

- [ ] **Step 1: Failing tests:** `review_lists_due_oldest_first_with_commands`; `review_all_adds_hidden_superseded_orphans`; `review_disabled_reports_off_and_what_if_with_events` (epoch = now, an existing archive applied); `list_disabled_unchanged` (phase 0 golden); `list_enabled_shows_state_and_hidden`; `list_epoch_override_keeps_events`.
- [ ] **Step 2: Run, fail. Step 3: Implement. Step 4: Run, pass; fmt + clippy; `--test memory_list --test memory_lifecycle --test cli`.**

### Task 6: SessionStart

**Files:**
- Modify: `rust/src/memory_store.rs` (`session_recall_with` ~637: optional lifecycle step; `SessionRecall` gains `lifecycle: Option<LifecycleView>`, `lifecycle_full_scope: bool`), `rust/src/memory.rs` (`recall_ledger_for_hook(top, ledger, view: Option<&LifecycleView>)`: hide before sort/cut, sort key), `rust/src/hook_recall.rs` (`render_with_omitted` sort key and ` [<state>]` suffix; weekend line; `lifecycle_usage` in `session_usage_fields`), `rust/src/remind_snapshot.rs` (`pub(crate) fn weekday(date: &str) -> Option<u8>`, 0 = Monday … 6 = Sunday), `docs/memory.md`, `CLAUDE.md` (one sentence in the SessionStart paragraph)
- Test: `rust/tests/hook_recall.rs`, unit tests in `remind_snapshot.rs`

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: sort key `(is_due, Reverse(recorded_at), Reverse(id))` used by both the cut and `render_with_omitted` (`is_due` read from `MemoryHit.lifecycle`); usage field `lifecycle`: `"off"`, `"dropped"`, or `{"due": n, "hidden": n, "unreadable": n}`; weekend line text exactly `N memory entries await review: cort memory review`; its bytes reserved from the ledger budget only when it prints.

- [ ] **Step 1: Failing tests:** `lifecycle_sql_error_drops_only_that_read_and_prints_nothing`; `notes_and_lifecycle_fail_together`; `unaliased_retry_ignores_lifecycle`; `hidden_removed_before_cut` (21 visible + 5 hidden → 20 shown, "1 left out"); `due_entries_after_rest_and_render_keeps_order`; `due_suffix`; `weekend_line_saturday_not_friday_not_without_local_parts`; `weekend_reserve_only_when_line_prints` (full ledger, Saturday, nothing due → cut identical to Friday); `all_hidden_project_prints_only_weekend_line` (Review Focus 4); `disabled_project_byte_identical`; `usage_row_carries_lifecycle_field`; `weekday_known_dates` (2026-11-14 → 5, 2026-11-16 → 0, 2000-02-29 → 1, bad input → `None`).
- [ ] **Step 2: Run, fail. Step 3: Implement** the step exactly like the reminders step (SQL error drops only it; decoded via `decode_rows`; full scope only when not `retried_alias`). **Step 4: Run, pass;** then the full gate: `CARGO_BUILD_JOBS=2 nice -n 15 cargo run -q --locked --manifest-path evals/Cargo.toml -- gate`.

### Integration (integrator, after each task)

- [ ] Review the diff line by line against the spec; Fable external review of the task's diff; fix; gate (`--crate rust`, full gate after Task 6); commit the named files only.
- [ ] After Task 6: push only with the user's words relayed by bsr; then install on NUC, `--check`, and ask (through bsr) before `cort memory lifecycle on` for agent-portal-tpm.
