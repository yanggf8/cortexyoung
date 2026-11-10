# `cort question` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A per-repo, cross-machine bag of questions for the human (`cort question add|list|answer|close`), delivered at session start.

**Architecture:** A pure core (`question.rs`: event types, fold, matching, validation) over an append-only `project_questions` table in the ledger's Turso database, read through the ledger's alias component and written through a checked-insert path like the ledger's. `SessionStart` recall appends one statement to its existing pipeline and renders an open-questions block.

**Tech Stack:** Rust (clap, serde_json, the in-repo `remote::RemoteDb` Hrana client; tests use `remote::SqliteTransport`).

**Spec:** `docs/superpowers/specs/2026-11-10-cort-question-design.md` -- read it first; this plan does not restate its behaviour, it fixes names, files and tests.

## Global Constraints

- Pure Rust; no scripts. No absolute developer paths in code or fixtures.
- Storage failures are returned, never panicked on (no `.unwrap()`/`.expect()` outside tests).
- Table: `project_questions (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))`; no other constraint or index.
- Bounds at write: text 600 chars, each option text 200, option key / `blocks` / `by` 64; control characters refused.
- `CORT_QUESTION_STALE_DAYS` default 14. Session block: at most 8 questions, 1500 bytes.
- `harness` stored only when `--harness` is given (same value list as `memory recall --harness`).
- Lean output never has an empty field (TSV rule): print `-` for absent values.
- Before each commit: `cargo fmt --all` in `rust/` and `evals/`; before the last commit run `CARGO_BUILD_JOBS=2 cargo run -q --locked --manifest-path evals/Cargo.toml -- gate` and require its final JSON line to report success.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A question filed under an old project id before a rename, answered under the new id after `alias add` -- must list as answered, never as an orphan. (Task 3 test `answer_across_alias_folds_into_the_ask`.)
2. A first-ever session start on a database where nobody ran a question command -- must still inject the ledger and notes, record `questions: dropped`, and not exceed the deadline. (Task 5 test `missing_questions_table_keeps_ledger_and_notes`.)
3. A Traditional Chinese question with no spaces filed twice with one character changed -- must report `similar`, and filed identically must report `exists`. (Task 1 tests.)
4. An `add` retried after a lost response -- the first insert's id is reported as `outcome_unknown`/found, not a silent second question. (Task 2 test `add_after_lost_response_reports_the_landed_id`.)
5. Question text containing a newline or an instruction-like line -- refused at write (control chars), and rendered under the "unreviewed" header. (Task 1 bound test, Task 5 header test.)

---

### Task 1: Pure core -- events, fold, matching, validation

**Files:**
- Create: `rust/src/question.rs`; Modify: `rust/src/lib.rs` (add `pub mod question;`)
- Test: `rust/tests/question.rs`

**Interfaces:**
- Produces:
  - `pub enum EventKind { Ask, Answer, Close }` (serde `rename_all = "lowercase"`)
  - `pub struct QuestionOption { pub key: String, pub text: String }`
  - `pub struct QuestionEvent { id, kind, question: Option<String>, text: Option<String>, options: Vec<QuestionOption>, recommend, blocks, by: Option<String>, reasks: Vec<String>, answer, quote: Option<String>, quoted: Option<bool>, option: Option<String>, replaces: Vec<String>, reason, of: Option<String>, recorded_at: String, recorded_machine, recorded_machine_source, harness: Option<String> }` -- serde with `skip_serializing_if` on every optional/empty field, `#[serde(default)]` on all but `id`, `kind`, `recorded_at`.
  - `pub enum State { Open, Answered, Closed }` (serialize lowercase)
  - `pub struct Question { pub ask: QuestionEvent, pub events: Vec<QuestionEvent>, pub state: State, pub current_answers: Vec<QuestionEvent>, pub answer_conflict: bool, pub close: Option<QuestionEvent>, pub last_activity: String, pub stale: bool }`
  - `pub struct Folded { pub questions: Vec<Question>, pub orphan_events: Vec<QuestionEvent> }`
  - `pub fn fold(events: Vec<QuestionEvent>, now_ns: i128, stale_days: u64) -> Folded`
  - `pub fn terms(text: &str) -> Vec<String>` -- lowercase; ASCII words split on non-alphanumerics, dropping `memory::PARTIAL_IGNORED` words and single ASCII chars; each maximal non-ASCII run split into overlapping char bigrams (a 1-char run is kept as itself); deduplicated, order kept.
  - `pub enum Dedup { Exists(Vec<String>), Similar(Vec<String>), New }` (ids)
  - `pub fn classify(new_text: &str, questions: &[Question]) -> Dedup` -- `Exists` when every term of `new_text` occurs in an earlier question's `ask.text` terms (any state); else `Similar` for questions whose ask text + option texts + current answer texts contain at least half the terms (ASCII by word start, as `memory.rs` partial matching); else `New`.
  - `pub fn validate_ask(text: &str, options: &[QuestionOption], recommend: Option<&str>, blocks: Option<&str>, by: Option<&str>) -> Result<(), String>`
  - `pub fn resolve_prefix<'a>(ids: impl Iterator<Item = &'a str>, prefix: &str) -> Result<String, String>` -- <8 chars, non-lowercase-hex, none, or ambiguous → Err with the code word (`question_id_too_short`, `question_not_found`, `question_id_ambiguous`).
- Also: make `PARTIAL_IGNORED` in `rust/src/memory.rs` `pub(crate)`.

Fold rules (from spec): current answers = answer events no later answer's `replaces` names; `answer_conflict = current_answers.len() > 1`; state Closed if any close, else Answered if any current answer, else Open; `stale` only when Open and `now - last_activity > stale_days`; events whose `question` matches no ask go to `orphan_events`; questions sorted newest `last_activity` first.

- [ ] **Step 1: Write failing tests** in `rust/tests/question.rs`:
  - `fold_ask_is_open`, `fold_answer_makes_answered`, `fold_close_wins_over_answer`
  - `two_unreplaced_answers_conflict` (asserts `answer_conflict` and 2 current)
  - `replace_naming_both_leaves_one_current`
  - `two_replaces_of_one_answer_conflict_again`
  - `orphan_answer_is_reported_not_dropped`
  - `stale_only_when_open_past_window` (open 15 days → stale; answered 30 days → not; open 13 days → not)
  - `cjk_terms_are_bigrams`: `terms("保留期限")` == `["保留","留期","期限"]`
  - `classify_exists_when_all_terms_in_earlier_text` (ASCII and CJK, incl. a closed earlier question)
  - `classify_terms_only_in_an_answer_are_not_exists` (→ `Similar` or `New`, never `Exists`)
  - `classify_partial_is_similar`, and one-character change in a CJK question → `Similar`
  - `validate_refuses_control_chars_and_overlength` (601-char text, `"a\nb"`, option 201 chars, `recommend` naming no option)
  - `prefix_rules` (7 chars, ambiguous, unique, uppercase)
- [ ] **Step 2:** `cargo test --locked --test question` → fails to compile (module missing).
- [ ] **Step 3:** Implement the interfaces above in `rust/src/question.rs`.
- [ ] **Step 4:** `cargo test --locked --test question` → all pass; `cargo clippy --all-targets -- -D warnings` clean.
- [ ] **Step 5:** Commit `feat(question): event fold, CJK-aware term matching and write bounds`.

### Task 2: Store -- table, checked writes, add/list/answer/close

**Files:**
- Create: `rust/src/question_store.rs`; Modify: `rust/src/lib.rs`, `rust/src/memory_store.rs` (DDL batches, `Ledger` accessors)
- Test: `rust/tests/question_store.rs`

**Interfaces:**
- Consumes: Task 1 types and functions; `memory_store::{Ledger, LedgerError, project, COMPONENT}`.
- Produces in `memory_store.rs`:
  - `pub(crate) const CREATE_QUESTIONS: &str` (the table DDL); add `Stmt::new(CREATE_QUESTIONS, vec![])` to the DDL prefix of `Ledger::read_bodies` and `Ledger::alias_snapshot` (so any memory command provisions it; adjust the result indexes those functions read).
  - `pub(crate) fn db(&self) -> &RemoteDb` on `Ledger`; make `COMPONENT`, `project`, `outcome_unknown`, `notes_now` (or a shared timestamp helper) `pub(crate)` as needed.
- Produces in `question_store.rs` (each `_with` takes `&Ledger`; a thin wrapper opens `Ledger::open()`):
  - `pub struct AddArgs { text, options: Vec<QuestionOption>, recommend, blocks, by, harness: Option<String>, force: bool }`
  - `pub fn add_with(root: &Path, args: AddArgs, ledger: &Ledger) -> Result<Value, LedgerError>` → `{"outcome":"added","question":{..},"similar":[..]}` or `{"outcome":"exists","questions":[..]}`; with `force` and earlier matches, stores `reasks`.
  - `pub fn list_with(root: &Path, filter: ListFilter, id: Option<&str>, matching: Option<&str>, ledger: &Ledger) -> Result<Value, LedgerError>`; `pub enum ListFilter { Open, Answered, Closed, All }`. Output includes `orphan_events` when non-empty and `events_total` when > 2000.
  - `pub fn answer_with(root, id: &str, answer: &str, quote: Option<&str>, option: Option<&str>, replace: Option<Vec<String>>, harness: Option<String>, ledger) -> Result<Value, LedgerError>` -- `replace: None` = no flag, `Some(vec![])` = bare `--replace`. Refusals as `LedgerError::Message` starting with the spec's code word (`question_answered`, `question_answer_conflict`, `question_closed`), including the current answers and the retry command. Success output carries `next`.
  - `pub fn close_with(root, id: &str, reason: CloseReason, of: Option<&str>, harness, ledger) -> Result<Value, LedgerError>`; `pub enum CloseReason { Stale, Moot, Duplicate }`.
  - Events read with `SELECT project, body FROM project_questions WHERE project IN ({COMPONENT})`; writes `INSERT OR IGNORE INTO project_questions (project, id, body) VALUES (?, ?, ?)` under the current project id, then the ledger's pattern: `affected_row_count == 1` ok; on `outcome_unknown(kind)` read `SELECT 1 FROM project_questions WHERE project = ? AND id = ?` → found ok / not found `NotRecorded` / read failed `OutcomeUnknown { id, key: None, kind }`; other errors `NotRecorded`.
  - Event ids from the same generator `memory_store` uses for entries.

- [ ] **Step 1: Write failing tests** in `rust/tests/question_store.rs` (fixture: git repo with an origin + `Ledger::with_transport(.., SqliteTransport)`, the `memory_ledger.rs` pattern):
  - `add_then_list_open`, `add_exists_writes_nothing` (row count unchanged), `add_similar_writes_and_lists_similar`, `add_force_stores_reasks`
  - `answer_without_quote_stores_quoted_false`, `answer_twice_is_question_answered_with_retry_command`, `bare_replace_with_two_current_is_refused`, `replace_subset_is_question_answer_conflict`, `answer_closed_is_question_closed`, `answer_output_has_next_record_command`
  - `close_duplicate_requires_existing_of`, `close_records_machine`
  - `add_after_lost_response_reports_the_landed_id` (inject forward-then-timeout; assert ok and exactly one row)
  - `memory_recall_never_returns_question_events` (add a question with the recall query's words; `memory_store::recall_with` returns no hit)
  - `memory_status_provisions_questions_table` (fresh db, `status_with`, then `SELECT 1 FROM project_questions` succeeds)
- [ ] **Step 2:** `cargo test --locked --test question_store` → fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test --locked --test question_store --test memory_ledger --test memory` → pass.
- [ ] **Step 5:** Commit `feat(question): append-only question events in the ledger database`.

### Task 3: Aliases know about questions

**Files:**
- Modify: `rust/src/memory_store.rs` (`alias_snapshot`, `alias_add_with`, `alias_remove_with`)
- Test: `rust/tests/question_store.rs`

**Interfaces:**
- Consumes: Task 1 `QuestionEvent`; Task 2 table.
- Produces: `AliasSnapshot` gains `questions: Vec<(String, QuestionEvent)>` read in the same pipeline (`SELECT project, body FROM project_questions WHERE project IN ({COMPONENT})`). `alias_add_with` treats the old component as non-empty when `bodies` or `questions` is non-empty. `alias_remove_with` adds each question event's `question`, `replaces` and `of` ids to the crossing check alongside `supersedes`/`resolves`. `AliasProject.row_count` counts both.

- [ ] **Step 1: Write failing tests:** `alias_add_accepts_questions_only_old_project`, `answer_across_alias_folds_into_the_ask` (ask under old id, alias, answer under new id → `list` shows answered, no orphans), `alias_remove_refuses_when_answer_would_split_from_ask` (`alias_in_use`).
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** `cargo test --locked --test question_store --test memory_ledger` → pass.
- [ ] **Step 5:** Commit `fix(alias): a questions-only project can be aliased and an answer cannot be split from its ask`.

### Task 4: CLI `cort question`

**Files:**
- Modify: `rust/src/main.rs` (`KNOWN_COMMANDS` → 17 entries incl. `"question"`, `usage_value`, dispatch `Some("question") => cmd_question(..)`, clap `QuestionArgs`/`QuestionAction`, lean renderer, `--help` narrowing like `memory`)
- Test: `rust/tests/question_cli.rs`

**Interfaces:**
- Consumes: Task 2 `add_with`/`list_with`/`answer_with`/`close_with` wrappers (`add`, `list`, `answer`, `close` opening `Ledger::open()`).
- Produces: the command surface exactly as the spec's Commands block. `--option` on `add` parses `K=text` (split at first `=`; empty key or text refused before any network). `--replace` on `answer` is `Option<Vec<String>>` with `num_args = 0..`. `--harness` value list identical to `memory recall`'s. Lean: `add` → `added <id>` / `exists <id> <state>` (one line per id); `list` → one line per question: `<id8>\t<state>\t<stale|->\t<age_days>d\t<text>` plus `<answer|(no quote)|->`; `answer`/`close` → `<outcome> <id8>`. Errors use the existing memory error mapping (`memory_alias_error`-style) so `remote: <kind>` and the code words survive.

- [ ] **Step 1: Write failing tests** (CLI, no network -- the `notes_cli.rs` fixture with an unconfigured `CORT_MEMORY_DIR`): `question_help_lists_four_verbs`, `top_level_usage_lists_question`, `option_without_equals_is_refused_before_network`, `unconfigured_add_reports_memory_config_error`, `replace_accepts_zero_or_more_ids` (parse-level via `--help`-free dry error), `lean_list_has_no_empty_field` (unit test on the lean renderer with a folded fixture), `usage_row_omits_question_text` (the local `usage.db` row for `question add --text <secret words>` records the verb but not the words -- check how `usage_from_args` summarises args and redact `--text`, `--answer`, `--quote`, `--option`, `--match` values as `memory recall` redacts its query).
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** `cargo test --locked --test question_cli --test cli` → pass.
- [ ] **Step 5:** Commit `feat(question): cort question add|list|answer|close`.

### Task 5: Session start block

**Files:**
- Modify: `rust/src/memory_store.rs` (`session_recall_with`), `rust/src/hook_recall.rs` (render), `rust/src/main.rs` (hook-recall caller, usage row)
- Test: `rust/tests/hook_recall.rs`

**Interfaces:**
- Consumes: Task 1 `fold`, `Question`.
- Produces:
  - `session_recall_with` returns `SessionRecall { recall: MemoryRecall, omitted: usize, notes: Vec<NoteBrief>, notes_total: usize, questions: Option<Vec<Question>> }` (`None` = statement dropped). Pipeline order: ledger, notes, notes count, questions (`SELECT body FROM project_questions WHERE project IN ({COMPONENT})`, or `= ?` after the unaliased retry). Retry rules: SQL error at step 0 → existing unaliased retry; at a notes step (1 or 2) → drop notes, keep questions; at the questions step → drop questions, keep notes; each optional group dropped at most once; every retry still guarded by the existing 200 ms remaining rule. Update all existing callers/tests to the struct.
  - `hook_recall::render_session(s: &SessionRecall, now_ns: i128, stale_days: u64) -> Option<(String, usize, usize, usize)>` (text, entries, notes shown, questions shown) replacing `render_with_notes` at the call site; returns `Some` when any section has content. Question block: header lines exactly as the spec's example, non-stale open questions oldest-first, `- <id8>  <text>  <K=text ...> rec=<K>  (blocks: <b>, <age>d)` with absent parts omitted, cap 8 / 1500 bytes, tail `… N more open, M stale: cort question list` when anything is cut or stale. All agent text through `crate::notes::sanitize`.
  - Usage row: `questions_open`, `questions_shown`, and `questions: "dropped"` when the statement was dropped.
- Before Step 3: confirm Claude Code's per-hook `additionalContext` limit in current docs (claude-code-guide agent). If below 9,500 bytes, lower `MAX_NOTE_BYTES` so ledger + questions + notes fit, and note the source in a comment.

- [ ] **Step 1: Write failing tests:** `questions_block_follows_ledger_and_precedes_notes`, `questions_only_project_still_injects`, `missing_questions_table_keeps_ledger_and_notes` (no CREATE in the db; assert ledger + notes present, `questions` is `None`, no table created), `notes_failure_keeps_questions`, `stale_questions_are_counted_not_listed`, `block_respects_eight_and_1500_bytes`, `header_marks_questions_unreviewed`.
- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** `cargo test --locked --test hook_recall --test memory_deadline` → pass.
- [ ] **Step 5:** Commit `feat(recall): open questions reach every session start`.

### Task 6: Skill, docs, gate

**Files:**
- Modify: `skills/cort/SKILL.md`, `docs/memory.md` (new "Questions" section incl. the deploy-time `cort question list`), `AGENTS.md` (one bullet in "Shared decision memory"), `README.md` only if it lists memory verbs.
- Test: `rust/tests/skill_format.rs` (existing gate)

- [ ] **Step 1:** Edit the three documents per the spec's "Surfaces to update"; SKILL.md frontmatter key set unchanged.
- [ ] **Step 2:** `cargo test --locked --test skill_format` → pass.
- [ ] **Step 3:** `cargo fmt --all` in both crates; `CARGO_BUILD_JOBS=2 cargo run -q --locked --manifest-path evals/Cargo.toml -- gate` → final JSON line reports success.
- [ ] **Step 4:** Commit `docs(question): skill, memory doc and agent rules for cort question`.
