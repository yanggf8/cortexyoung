# Reminders in a live session (#10) Implementation Plan

> **For agentic workers:** Implement one task per dispatch. Do not commit. Do not run more than one
> cargo process; use `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15`.

**Goal:** a reminder that enters `upcoming`/`due`/`overdue` while a Claude Code (or Kimi) session is
alive is shown once at the next prompt, from a local snapshot, with no remote read.

**Spec:** `docs/superpowers/specs/2026-11-14-remind-live-session-design.md` -- read it whole; it is
the authority on behaviour, this plan only fixes names, files and tests.

## Global Constraints

- Pure Rust; no new crates. Storage failures are returned or ignored, never panicked on.
- The prompt hook makes no network call and never exits non-zero.
- Snapshot and shown-set writes: temp file + rename, no fsync; failures never change a command's
  output or exit code.
- `cargo fmt --all` and clippy `-D warnings` clean in `rust/`.
- Do not change the SessionStart block's text, the data model, or `add`/`defer`/`close`/`list` output.
- Codex is not wired (stage 2): `events_for("codex")` stays three events.

## Review Focus

1. Kimi: first prompt (full recall) records shown keys; second prompt does not replay them.
2. A compaction whose recall fails clears the shown set, so the next prompt re-shows due reminders.
3. A slow recall never overwrites a newer snapshot (`read_at` guard); the alias retry never writes.
4. A reminder deferred after being shown is shown again when its new date enters a window.
5. Install then uninstall returns `~/.claude/settings.json` byte for byte with the new event.

---

### Task 1: snapshot, shown set, selection and rendering (library only)

**Files:** create `rust/src/remind_snapshot.rs` (declare in `rust/src/lib.rs`); modify
`rust/src/hook_recall.rs`; test `rust/tests/remind_snapshot.rs`.

**Interfaces (produces):**
- `pub struct Snapshot { pub version: u32, pub project: String, pub read_at: String, pub events: Vec<ReminderEvent> }` (serde; `version` = 1).
- `pub fn snapshot_events(reminders: &[Reminder]) -> Vec<ReminderEvent>` -- add + events of every
  non-closed reminder.
- `pub fn write_snapshot(dir: &Path, snap: &Snapshot) -> std::io::Result<bool>` -- `false` when an
  existing snapshot has a newer `read_at` (nothing written). File `<dir>/remind-snapshot/<project>.json`.
- `pub fn read_snapshot(dir: &Path, project: &str) -> Option<Snapshot>` -- `None` when missing,
  unparsable, or `version != 1`.
- `pub type ShownKey = (String, &'static str, String)` -- (reminder id, `upcoming|due|overdue|stale`,
  due_at or read_at as decimal string). Implementer may use a small struct instead; keep it `Eq + Hash`.
- `pub fn read_shown(dir: &Path, session_id: &str) -> HashSet<ShownKey>`, `pub fn replace_shown(dir, session_id, keys: &[ShownKey]) -> io::Result<()>`,
  `pub fn append_shown(dir, session_id, keys: &[ShownKey]) -> io::Result<()>` (O_APPEND, whole lines).
  File `<dir>/remind-sessions/<sha256 hex of session_id>`; prune files older than 7 days on write,
  as `claim_prompt_session` does.
- `pub fn select_unshown(snapshot: &Snapshot, shown: &HashSet<ShownKey>, now_ns: i128) -> Vec<Reminder>`
  -- `remind::fold(snapshot.events.clone(), now_ns)`, keep `Upcoming|Due|Overdue` whose key is not in
  `shown`, oldest effective `due_at` first.
- `pub const STALE_AFTER_NS: i128 = 3 days`.
- In `hook_recall.rs`: change `render_reminders` to also return the `ShownKey`s of the lines it
  printed (skipped/cut-away lines excluded); `render_session`'s `SessionRender` gains
  `pub reminder_keys: Vec<ShownKey>`. Add
  `pub fn render_prompt_reminders(selected: &[Reminder], read_at_ns: i128, now_ns: i128) -> Option<(String, Vec<ShownKey>)>`
  producing the spec's header (local time of `read_at` as `YYYY-MM-DD HH:MM`, via the same `mktime`/
  `localtime` approach `remind.rs` uses or `localtime_r` through its existing `extern "C"`), the same
  lines/budget/tail code as `render_reminders` (shared helper, not a copy), `None` if no line fits;
  and `pub fn render_stale_snapshot(read_at_ns: i128) -> String` (the one-line notice).

**Tests (names and assertions):**
- `snapshot_round_trip_and_version_guard`: write then read equal; a file with `"version":2` or junk reads `None`.
- `older_read_never_overwrites_newer`: write read_at=200, then read_at=100 returns `Ok(false)` and file still says 200.
- `snapshot_events_drop_closed_reminders`.
- `select_each_state_once`: upcoming/due/overdue selected; scheduled not; with their keys in `shown` nothing selected.
- `deferred_reminder_is_selected_again`: key with old due_at in `shown`, snapshot has a defer → selected.
- `replace_then_append_shown`: replace with A, append B → read gives {A,B}; replace with [] → empty.
- `render_reminders_returns_printed_keys_only`: 6 due reminders (MAX_REMINDERS=4) → 4 keys, matching the printed ids.
- `prompt_block_header_and_budget`: header starts with `Reminders not yet shown in this session (local snapshot read `, total ≤ `MAX_REMINDER_BYTES` + header, tail `… N more due` when over 4.

### Task 2: writers -- hook-recall and the remind commands

**Files:** modify `rust/src/main.rs` (`cmd_hook_recall`, main's Ok branch), `rust/src/memory_store.rs`
(`SessionRecall`), `rust/src/remind_store.rs`; tests in `rust/tests/hook_recall.rs`,
`rust/tests/remind_store.rs`.

**Interfaces:**
- Consumes Task 1.
- `SessionRecall` gains `pub reminders_full_scope: bool` (false when the read was the `retried_alias` read).
- `hook-recall` reads `session_id` on every path. A process-global deferred action
  (`static AFTER_OUTPUT: Mutex<Option<Box<dyn FnOnce() + Send>>>` in main.rs) is set by
  `cmd_hook_recall` and run by `main` after `finish_record` in the `Ok` branch only; it writes the
  snapshot (when `reminders` is `Some` and `reminders_full_scope`) and **replaces** the shown set with
  `reminder_keys` (empty on any failed/skipped recall) whenever this run was a session start
  (SessionStart, or Kimi's first prompt).
- Kimi later prompt (`already_injected`): load snapshot + shown set, select, render with
  `render_prompt_reminders` (or `render_stale_snapshot` when `now - read_at > STALE_AFTER_NS` and the
  stale key is not shown), print `{"message": text}`, append keys in the deferred action, row field
  `reminders_shown`. `marker_unavailable`: unchanged (silent).
- `remind_store`: `list_with`, `add_with` (including `exists`), `defer_with`, `close_with` write the
  snapshot after their successful read (plus the confirmed new event for add/defer/close) keyed by
  `memory_store::project(root)`, `read_at` = clock before the read. Never on refusal-before-read or
  `outcome_unknown`. Errors ignored.

**Tests:**
- `session_start_writes_snapshot_and_replaces_shown_set` (fake server, as existing hook_recall tests do).
- `failed_recall_clears_shown_set_and_keeps_snapshot`.
- `alias_retry_does_not_write_snapshot`.
- `kimi_first_prompt_records_keys_second_prompt_silent` and `kimi_later_prompt_shows_newly_due`.
- `remind_list_and_close_refresh_snapshot`; `outcome_unknown_leaves_snapshot`.

### Task 3: `hook-remind`, install wiring, docs

**Files:** `rust/src/main.rs`, `rust/src/settings.rs`, `rust/src/settings_toml.rs`,
`rust/src/settings_kimi.rs`, `rust/src/usage.rs`, `rust/src/hook.rs`, `tests/install-smoke.sh`,
`AGENTS.md`, `docs/memory.md`, `skills/cort/SKILL.md` (one sentence at most); tests in
`rust/tests/hook_recall.rs` (or a new `rust/tests/hook_remind.rs`) and the existing settings tests.

**Interfaces:**
- `cort hook-remind --harness <h>`: stdin payload → `cwd`, `session_id`; outcomes and envelope per
  spec (`{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":text}}`);
  appends shown keys after writing stdout; usage row with `outcome` and `reminders_shown`; never
  touches memory config or the network; every failure → empty stdout, exit 0.
- `HookEvent::Remind` per spec's Install section; `EVENTS` = 4; `events_for`: claude-code → EVENTS,
  codex and kimi-code → `[Suggest, Refresh, Recall]`. All exhaustive matches extended (spec lists
  them). Usage census / `hook::` known outcome lists include the new outcomes.
- `tests/install-smoke.sh` §20: ten entries.
- Docs: AGENTS.md hook paragraphs (a fourth event for Claude Code; Kimi's later prompts; snapshot
  under the memory data dir; no remote read), `docs/memory.md` Reminders section.

**Tests:**
- `hook_remind_shows_once_then_silent` (temp `CORT_MEMORY_DIR`, hand-written snapshot, no config).
- `hook_remind_stale_snapshot_line_once`; `hook_remind_no_snapshot_silent`; `hook_remind_bad_stdin_exit_0`.
- Settings: install with four Claude Code events then uninstall → byte-identical file; codex/kimi files unchanged in entry count; `--status` lists `prompt` for claude-code only.
