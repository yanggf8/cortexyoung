# Memory tiers phase 2a (cloud search) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cort memory recall` returns only the rows its query can match from the shared database (the server still scans the component per statement), and gives byte-identical answers; it also gains `--also` (several queries) and `--keys` (layer 1).

**Architecture:** A new pure module `rust/src/memory_search.rs` analyses terms and builds every SQL statement; `memory_store.rs` sends them in one pipeline (at most two), decodes the returned subset under a "missing predecessor is fine" rule, folds the lifecycle on it, and hands the rows to today's unchanged `select_terms` decision. Today's whole-ledger recall stays, renamed `recall_whole_with`, as the test oracle and the eval baseline.

**Tech Stack:** Rust, the Hrana pipeline client in `rust/src/remote.rs`, SQLite JSON1 (`json_extract`, `json_each`, `json_valid`, `instr`), `SqliteTransport` for tests.

**Spec:** `docs/superpowers/specs/2026-11-15-memory-tiers-phase2-design.md` (section "2a: cloud search"; 04a688ea). Read it before any task.

## Global Constraints

- A single-query `cort memory recall "<q>"` (with or without `--all`) prints exactly what it prints today: same hits, order and fields. Every task's tests compare against `recall_whole_with`.
- Terms are `query.split_whitespace()` lowercased with Rust `to_lowercase`; bound as parameters, never interpolated.
- Haystack: `lower(json_extract(body,'$.key') || ' ' || json_extract(body,'$.claim'))`; layer 1 uses `lower(json_extract(body,'$.key'))`. Match form: `instr(<haystack>, ?) > 0`.
- Every statement over `project_memory` rows carries `json_valid(body)`; every lifecycle statement used in SQL decisions carries it too.
- Active: the body's `$.id` is not named by any row's `$.supersedes` nor by any element of any row's `$.resolves`, in the alias component (`COMPONENT`).
- At most two round trips per recall. No new local database, cache file or table.
- No Python, JS or other script anywhere; `cargo fmt --all` in both crates and clippy clean before every commit.
- Build/test with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15`, one cargo at a time. Implementers never commit; the coordinator commits named files only.
- Gate: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15 cargo run -q --locked --manifest-path evals/Cargo.toml -- gate` must end `"ok":true`.

## Review Focus

1. **Terms with SQL-looking characters** (`50%_done`, `it's`, `a\b`): matched literally, as today. → Task 4 test `literal_special_characters_match_as_today`.
2. **Empty query** `recall ""`: every visible active entry, as today (stage 1 with no term condition). → Task 4 test `empty_query_equals_oracle`.
3. **Fresh database** (tables not yet created, no rows): `ledger_status: "absent"`, no error, tables created as today's read does. → Task 4 test `fresh_database_is_absent_not_an_error`.
4. **ASCII case in stored text** (`Deploy PLAN` in a claim, term `deploy`): SQLite `lower()` folds ASCII, so it matches. → Task 2 test `ascii_upper_case_in_claim_matches`.
5. **Many terms** (a 30-term pasted sentence): the stage-2 sum and parameter count stay valid and the answer equals the oracle. → Task 4 test `thirty_term_query_equals_oracle`.

---

### Task 1: SessionStart names an integrity error (`ledger_invalid`)

Independent of the rest; ships first.

**Files:**
- Modify: `rust/src/memory_store.rs` (`session_recall_with`, ~line 733-915)
- Modify: `rust/src/main.rs` (~line 1990-2036, the hook-recall read and its outcome match)
- Modify: `rust/src/hook.rs` (`RECALL_OUTCOMES` ~101: add `ledger_invalid`; the closed vocabulary `usage.rs` ~864 checks)
- Modify: `docs/memory.md` (session-start bullet), `AGENTS.md` (the "third event, recall" paragraph: one sentence)
- Test: `rust/tests/hook_recall.rs`

**Interfaces:**
- Produces: `pub enum SessionRecallError { Remote(ErrorKind), LedgerInvalid(String) }` in `memory_store.rs`; `session_recall_with(...) -> Result<SessionRecall, SessionRecallError>`. `From<ErrorKind> for SessionRecallError` so existing `?` sites keep compiling.

- [ ] **Step 1: Write the failing tests** in `rust/tests/hook_recall.rs`:
  - `ledger_with_invalid_supersession_is_ledger_invalid`: seed two rows where `b` has `"supersedes":"a"` and a different key; call `session_recall_with`; assert `Err(SessionRecallError::LedgerInvalid(m))` with `m == "memory b has invalid supersession"` (the id as seeded).
  - `ledger_with_missing_predecessor_keeps_alias_hint`: `b` supersedes an absent id; assert the message ends with `; re-add the alias with cort memory alias add` (today's `ledger_for_read` wording).
  - `sql_error_on_ledger_statement_is_still_remote`: `database_with_step_failure("FROM project_memory WHERE", "SQLITE_ERROR")` on both tries; assert `Err(SessionRecallError::Remote(ErrorKind::Sql))`.
  - A CLI-level test (same file, using the existing hook-recall process helpers) asserting the printed line is exactly
    `cort memory: the shared ledger has an integrity problem (memory b has invalid supersession); run \`cort memory status\` to see it, and do not trust recall's silence until it is fixed`
    and the usage row's `outcome` is `"ledger_invalid"`.
- [ ] **Step 2: Run** `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 nice -n 15 cargo test --locked --manifest-path rust/Cargo.toml --test hook_recall` → the four fail (type not found / wrong outcome).
- [ ] **Step 3: Implement.** In `session_recall_with`, map `merge_bodies` and `ledger_from_bodies` failures (today `.map_err(|_| ErrorKind::Sql)`, ~line 910-911) to `LedgerInvalid(message)`, passing the message through `ledger_for_read`'s alias-hint rule (factor that rule into `fn with_alias_hint(message: String) -> String` and use it in both places). The unaliased retry loop is unchanged. In `main.rs`, `ledger_invalid` becomes an outcome alongside the `remote_*` ones, printing the line above; the `remote_*` arm is unchanged.
- [ ] **Step 4: Run** the same command → all pass; then the gate with `--crate rust`.
- [ ] **Step 5: Commit** `rust/src/memory_store.rs rust/src/main.rs rust/src/hook.rs rust/tests/hook_recall.rs docs/memory.md AGENTS.md` — `fix(memory): SessionStart names a ledger integrity error instead of remote_unreachable (#11 2a)`.

### Task 2: Term analysis and the haystack parameter

**Files:**
- Modify: `rust/src/memory.rs` (`select_terms` ~407, `recall_selection` ~472, `recall_ledger_inner` ~486)
- Create: `rust/src/memory_search.rs` (term analysis only in this task); register in `rust/src/lib.rs`
- Test: `rust/src/memory.rs` tests module, `rust/tests/memory_search.rs` (new)

**Interfaces:**
- Produces, in `memory.rs`:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Haystack { KeyClaim, Key }` with `pub fn text(self, entry: &MemoryEntry) -> String` (lowercased `key + " " + claim`, or lowercased `key`).
  - `pub(crate) fn partial_terms(terms: &[String]) -> Vec<String>`: distinct, not in `PARTIAL_IGNORED`, not a lone ASCII char, first-occurrence order. `select_terms` calls it instead of its inline loop.
  - `select_terms(terms, active, allow_partial, haystack: Haystack)`; every existing caller passes `Haystack::KeyClaim`.
  - `pub fn recall_selection_in(query: &str, active: &[MemoryEntry], haystack: Haystack) -> (Vec<String>, bool)`; `recall_selection` keeps its signature and calls it with `KeyClaim`.
- Produces, in `memory_search.rs`:
  - `pub fn sql_exempt(term: &str) -> bool`: true when the term has a char whose `to_uppercase()` differs from its `to_lowercase()` and is not ASCII, or a char in U+0300..=U+036F.
  - `pub struct QueryTerms { pub terms: Vec<String>, pub partial: Vec<String>, pub exempt: bool }` and `pub fn query_terms(query: &str) -> QueryTerms` (`exempt` = any term in `terms` is `sql_exempt`).

- [ ] **Step 1: Write the failing tests.**
  - `memory.rs`: `key_haystack_ignores_claim_text` — entries `{key:"deploy-plan", claim:"nuc runs it"}`, `{key:"other", claim:"deploy plan"}`; `recall_selection_in("deploy", .., Key)` returns only the first; with `KeyClaim` returns both.
  - `memory.rs`: `partial_terms_matches_select_terms_rule` — `partial_terms(["the","a","部署","部署","nuc"])` is `["部署","nuc"]`.
  - `rust/tests/memory_search.rs`: `exempt_terms` — `sql_exempt("café")`, `sql_exempt("ｄｅｐｌｏｙ")`, `sql_exempt("i\u{307}stanbul")`, `sql_exempt("Ωmega")` are true; `sql_exempt("部署")`, `sql_exempt("deploy-v2")`, `sql_exempt("50%_done")` are false.
- [ ] **Step 2: Run** `... cargo test --locked --manifest-path rust/Cargo.toml memory` → fail (items not defined).
- [ ] **Step 3: Implement** the signatures above. No behaviour change for `KeyClaim`.
- [ ] **Step 4: Run** the full `rust` test suite (all existing recall tests must still pass unchanged), then gate `--crate rust`.
- [ ] **Step 5: Commit** `rust/src/memory.rs rust/src/memory_search.rs rust/src/lib.rs rust/tests/memory_search.rs` — `feat(memory): recall's term rule as shared helpers, haystack parameter (#11 2a)`.

### Task 3: The stage statements (active, archived, stage 1, stage 2)

**Files:**
- Modify: `rust/src/memory_search.rs`
- Test: `rust/tests/memory_search.rs`

**Interfaces:**
- Consumes: Task 2's `QueryTerms`, `query_terms`, `Haystack`; `memory_store::COMPONENT` (make it `pub(crate)` if not already — it is).
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Stage2 { Gated, Ungated }`
  - `pub struct StageQuery<'a> { pub terms: &'a QueryTerms, pub haystack: Haystack, pub archived_in_sql: bool, pub stage2: Stage2 }`
  - `pub fn stage1(project: &str, q: &StageQuery) -> Stmt` and `pub fn stage2(project: &str, q: &StageQuery) -> Option<Stmt>`; both return `SELECT project, body` rows ordered by `json_extract(body,'$.id'), project`.
  - `pub fn archived_excluded(project: &str) -> Stmt`: `SELECT project, body` of active rows the archived condition excludes.
  - `fn active_rows_sql() -> String` and `fn archived_sql() -> String` fragments (built with `format!` around `COMPONENT`), shared by the three statements so the conditions exist once.
- SQL decisions the implementer must not change:
  - Guard every `json_extract` on a row body as `CASE WHEN json_valid(body) THEN json_extract(...) END`, not by relying on `WHERE` evaluation order.
  - Archived (only when `archived_in_sql`): project enabled = the latest (`CAST(recorded_at AS INTEGER)`, then `$.id`) lifecycle row of kind `on`/`off` is `on`; entry excluded when, among its rows of kind `archive`/`drop`/`restore`, the latest is `archive`. Canonical lifecycle rows only: `json_valid(body)`, `json_type(body,'$.recorded_at') = 'text'`, `$.recorded_at <> ''` and `NOT GLOB '*[^0-9]*'`, `$.kind` in the six known kinds, and for per-entry kinds `json_type(body,'$.entry') = 'text'` and non-empty.
  - Stage 1: active, not archived (when applied), and `instr(hay, ?) > 0` for each term that is not `sql_exempt`; no term condition when none is left.
  - Stage 2: `None` when `terms.terms.len() < 2` or `terms.partial` is empty. Otherwise rows whose `(instr(hay,?)>0) + ... + <count of exempt partial terms> >= ceil(partial.len() / 2)`, over `partial` only. When `stage2 == Gated` add `NOT EXISTS (<stage 1's condition>)`.

- [ ] **Step 1: Write the failing tests** (each seeds `project_memory` / `project_memory_lifecycle` through a `SqliteTransport`-backed `RemoteDb` and runs the statement):
  - `stage1_is_a_superset_of_the_client_full_match` — for a fixture of ~15 entries (ASCII, CJK, identifiers with `-_./:`, mixed case) and queries `["deploy","部署 nuc","deploy-v2","DEPLOY plan","nope"]`, for both haystacks: the ids `select_terms` full-matches over the whole active set ⊆ stage-1 ids.
  - `stage2_is_a_superset_of_the_client_partial_rule` — same fixture, queries with ≥2 terms where stage 1 is empty, both haystacks: the client partial selection (before its cap of 5) ⊆ stage-2 ids; plus a word-start case (`"ploy nuc"` matches `deploy` by substring in SQL, not by the client) to show SQL may be wider.
  - `stage2_not_built_for_one_term_or_only_noise` — `"deploy"` and `"the a"` give `None`.
  - `stage2_gate` — gated stage 2 returns no rows when stage 1 matches; ungated returns its rows.
  - `stage1_returns_exactly_the_matching_rows` — for every fixture query and both haystacks, stage-1 ids equal the active, non-archived rows whose SQL haystack contains every non-exempt term (computed in the test with Rust `to_lowercase` on ASCII-only fixture text), not merely a superset.
  - `stage2_threshold` — query `"alpha beta gamma"` (n = 3, threshold 2): a row holding only `alpha` is not returned; rows holding two or three are.
  - `ascii_upper_case_in_claim_matches` — claim `Deploy PLAN`, term `deploy`.
  - `exempt_term_keeps_rows` — `"café 部署"`: stage 1 returns the `部署` rows (café gave no condition).
  - `active_rule_counts_supersedes_and_resolves` — superseded, resolved (two-element `resolves`) entries never returned.
  - `json_invalid_row_does_not_fail_the_statement` — a row with body `{not json` present; statements succeed.
  - `archived_condition_agrees_with_the_fold` — for: archive; restore after archive; archive after restore; drop (not excluded by SQL); archive written before an `off`/`on` cycle (still excluded); project switched off (nothing excluded): the ids `archived_excluded` returns equal the entries `memory_lifecycle::fold` marks `Archived` (enabled project) or empty (disabled).
- [ ] **Step 2: Run** `... cargo test --locked --manifest-path rust/Cargo.toml --test memory_search` → fail.
- [ ] **Step 3: Implement** the four builders.
- [ ] **Step 4: Run** → pass; gate `--crate rust`.
- [ ] **Step 5: Commit** `rust/src/memory_search.rs rust/tests/memory_search.rs` — `feat(memory): SQL stage statements for cloud recall (#11 2a)`.

### Task 4: Subset decoding, the integrity screen, and the search itself

**Files:**
- Modify: `rust/src/memory_search.rs` (screen, count, conflict and lifecycle statements; pipeline layout)
- Modify: `rust/src/memory_store.rs` (`ledger_from_bodies` ~535, `ledger_for_read` ~247, `recall_all_with` ~1884; new `search_with`)
- Modify: `rust/src/memory.rs` (`validate` ~161, `recall_ledger_view` ~359, `recall_ledger_inner` ~486)
- Modify: `rust/src/remote.rs` (new `RemoteDb::read_snapshot`, beside `transaction` ~783)
- Modify: `docs/memory.md` (recall section: what recall reads now, and that the five whole-ledger checks listed in the spec moved to `status`/`list`)
- Test: `rust/tests/memory_search.rs`, `rust/tests/memory_recall_lifecycle.rs` (existing tests must pass unchanged), `rust/tests/remote.rs`

**Interfaces:**
- Produces, `memory_search.rs`:
  - `pub fn ledger_count(project) -> Stmt` (`count(DISTINCT $.id)` over valid rows), `pub fn conflict_keys(project) -> Stmt` (keys with ≥2 distinct active ids, hidden included), `pub fn lifecycle_rows(project) -> Stmt` (every lifecycle row of the component, `SELECT body`), `pub fn integrity_screen(project) -> Stmt` returning `(body, suspect INTEGER)`: suspect rows (invalid JSON; `$.id`/`$.key`/`$.claim` null or empty after trimming `char(32,9,10,11,12,13)`; a `supersedes` or `resolves` element naming an id absent from the component, naming its own id, or naming a row with a different key; an id held by ≥2 rows whose bodies are not byte-equal) with `suspect = 1`, plus every existing row a suspect names with `suspect = 0`; ordered by `$.id`, then `project`. Suspect is a screen, not a verdict: a suspect row that passes the client's checks (an `id` of spaces, which today's `validate` accepts) is not an error.
- Produces, `remote.rs`: `pub fn read_snapshot(&self, stmts: &[Stmt]) -> Result<Vec<StmtResult>, RemoteError>` — one request, the statements wrapped in `BEGIN` … `COMMIT` with `batch_body`'s step conditions, so every statement reads the same state (a `pipeline` runs each statement on its own, `remote.rs` ~665, and a `resolve` landing between `conflict_keys` and `stage1` would yield a state that never existed). The first failing step is the error, as `pipeline`. Test in `rust/tests/remote.rs`: results map one per statement, and a failing statement returns its step.
- Produces, `memory.rs`:
  - `pub enum Scope { Whole, Subset }`; `validate(ledger, scope)` (every existing caller passes `Whole`) — under `Subset` an absent predecessor is skipped in the `by_id` lookup (a present one is still checked for key, self and order).
  - `recall_ledger_view(.., conflicts: Option<&BTreeSet<String>>)` as a new last parameter, passed down to `recall_ledger_inner`: `None` counts active entries per key over the given ledger (today's rule; `recall_whole_with`, `list` and the hook pass `None`); `Some(keys)` marks a hit `conflict` when its key is in the set. Replace the panicking `counts[entry.key.as_str()]` (~566) with a lookup that cannot panic.
  - "Full match over the visible stage-1 rows is empty" is `recall_selection_in(query, &visible, haystack)` returning `partial == true` or no ids.
- Produces, `memory_store.rs`:
  - `fn ledger_from_bodies(bodies, scope: Scope)`: under `Subset` the all-ids check is skipped and the topological order treats an absent predecessor as already placed. `Whole` is today's behaviour; every existing caller passes `Whole`.
  - `fn screen_check(rows: Vec<(String, bool)>) -> Result<(), String>`, checks in today's phase order, each phase over the rows in the screen's `$.id`, `project` order: (1) serde-decode every row (today's `invalid remote memory row: {e}`); (2) `merge_bodies` on all (collision message); (3) for each suspect row, a predecessor not among the returned ids → `memory {id} supersedes missing id {old}`; (4) `ledger_from_bodies(.., Subset)` (its topological order and `validate` give the remaining messages). The first error wins. Messages pass through `with_alias_hint` (Task 1).
  - `pub struct SearchRequest<'a> { pub queries: &'a [String], pub haystack: Haystack, pub include_hidden: bool }`
  - `pub struct Searched { pub ledger: MemoryLedger, pub present: bool, pub conflicts: BTreeSet<String>, pub view: LifecycleView, pub unreadable: usize, pub trips: usize }` — `ledger` is the merged, subset-decoded union of all stage rows of all queries.
  - `pub fn search_with(ledger: &Ledger, project: &str, req: &SearchRequest, now_ns: i128) -> Result<Searched, LedgerError>`
  - `pub fn recall_whole_with(root, query, ledger, include_hidden) -> Result<MemoryRecall, LedgerError>` — today's `recall_all_with` body, unchanged, kept public as the oracle and the eval baseline.
  - `pub fn select_whole_with(ledger: &Ledger, project: &str, query: &str, include_hidden: bool, now_ns: i128) -> Result<(Vec<String>, bool), LedgerError>` and `pub fn select_search_with(..same..) -> Result<(Vec<String>, bool, usize, usize), LedgerError>` (rows returned, then trips): ids and partial flag only, no root and no source checks, for the eval. `select_whole_with` reads the whole ledger as `recall_whole_with` does.
  - `recall_all_with` keeps its signature and is reimplemented as `search_with` + `recall_ledger_view` over `Searched.ledger`, with conflicts taken from `Searched.conflicts` instead of the subset's own counts.
- Trip rules (the implementer must not change them). Each trip is one `read_snapshot`. Trip 1 = the five `CREATE`s, `ledger_count`, `conflict_keys`, `integrity_screen`, `lifecycle_rows`, `archived_excluded` (unless `include_hidden`), then per query `stage1` and `stage2` (`Ungated` when `QueryTerms.exempt`, else `Gated`). The screen is checked first; a failure returns `LedgerError::Message(<today's message>)`. Then dedup rows by id with `merge_bodies`, decode as `Subset`, fold. A second pipeline (the same statements with `archived_in_sql = false`, every stage 2 `Ungated`, no `archived_excluded`) is sent once when either: the fold of {excluded rows ∪ stage rows} shows any excluded entry as not hidden; or, for some query, stage 1 returned rows, stage 2 was gated, `select_terms` full match over the visible stage-1 rows is empty, and the query has ≥2 terms. Never a third. When trip 2 is sent, its answer replaces trip 1's entirely (screen, count, conflicts, lifecycle rows, stage rows, fold); nothing from trip 1 is mixed in.

- [ ] **Step 1: Write the failing tests** in `rust/tests/memory_search.rs`. Shared helpers: `fn fixture() -> Fixture` (git repo + `Ledger::with_transport` over a counting wrapper of `SqliteTransport`, as `memory_recall_lifecycle.rs` builds it, with an `AtomicUsize` incremented per `post`) and `fn assert_same(f, query, include_hidden)` asserting `serde_json::to_value(recall_all_with(..))` equals `serde_json::to_value(recall_whole_with(..))`.
  - `queries_equal_oracle` — a rich fixture (supersession chains, a two-element `resolves`, an alias pair via `project_aliases`, lifecycle on with archive/drop/restore, sourced and unsourced rows) and ≥25 queries (ASCII, CJK, identifiers, partial, no-match), each with `include_hidden` false and true.
  - `empty_query_equals_oracle`, `thirty_term_query_equals_oracle`, `literal_special_characters_match_as_today` (`50%_done`, `it's`, `a\b`).
  - `fresh_database_is_absent_not_an_error` — no seeded rows: `ledger_status == "absent"`, `hits == []`.
  - `cafe_query_falls_through_to_partial_as_today` — `"café 部署"` equals oracle (including `match: "partial"` when the oracle has it); trips == 1.
  - `full_match_only_on_a_dropped_entry` — query fully matches only a dropped entry and partially matches visible ones: equals oracle; trips == 2.
  - `no_match_costs_one_trip` — `"zzz qqq"`: trips == 1.
  - `excluded_entry_the_fold_shows_is_resent` — three cases, each equals oracle and trips == 2: a `restore` whose `recorded_at` is `+<digits>`; an `archive` row with `"note": 5`; an `archive` row with a duplicated `kind` key written `{"kind":"archive",...,"kind":"restore"}` (SQLite reads the first, so SQL excludes it; serde rejects the duplicate field, so the fold never applies the row and the entry stays visible).
  - `same_id_under_two_aliases` — byte-equal and stamp-differing copies: one hit, `evidence_status` not `conflict`.
  - `conflict_across_a_hidden_partner` — two active entries under one key, one archived: the visible hit has `evidence_status == "conflict"`, as the oracle.
  - `subset_fold_equals_whole_fold` — events naming entries the query does not match: every hit's `lifecycle` equals the oracle's.
  - `predecessor_not_returned_is_not_an_error` — a hit whose superseded predecessor does not match the query.
  - `reads_only_matching_rows` — on a 60-entry fixture where `"deploy"` fully matches 3 entries, `select_search_with` reports at most 3 stage rows; on `"alpha beta gamma"` with nothing fully matching, at most the rows holding two or more of the terms.
  - `two_errors_report_the_earlier_phase` — a ledger with one invalid-JSON row and one row superseding a different key: both paths report the `invalid remote memory row` message.
  - `screen_messages_equal_today` — one ledger per error, `recall_all_with` error equals `recall_whole_with` error: invalid JSON; missing `key`; key `"  "`; `supersedes` a missing id (with alias hint); `supersedes` own id; `supersedes` a different key; one id under two aliases with different claims.
- [ ] **Step 2: Run** `... --test memory_search --test memory_recall_lifecycle --test memory_ledger` → new tests fail.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** the whole `rust` suite, then gate `--crate rust`.
- [ ] **Step 5: Commit** `rust/src/memory_search.rs rust/src/memory_store.rs rust/src/memory.rs rust/src/remote.rs rust/tests/memory_search.rs rust/tests/remote.rs docs/memory.md` — `feat(memory): recall reads only matching rows from the shared database (#11 2a)`.

### Task 5: `--also`, `--keys`, `--lean`

**Files:**
- Modify: `rust/src/main.rs` (`MemoryAction::Recall` ~775, its handler ~3135, `render_emit` ~461, the `memory` usage line ~68)
- Modify: `rust/src/memory_store.rs` (`recall_many_with`, `recall_keys_with`; both build each query's hits by calling `recall_ledger_view` on `Searched.ledger` with `Some(&Searched.conflicts)`, once per query, then combine)
- Modify: `docs/memory.md`, `skills/cort/SKILL.md` (one sentence on `--also` for Chinese + English and `--keys`; the file is deployed byte-for-byte, so edit text only), `AGENTS.md` (the Recall bullet: one clause)
- Test: `rust/tests/memory_search.rs`, `rust/tests/cli.rs`

**Interfaces:**
- Consumes: Task 4's `search_with`, `SearchRequest`, `Searched`.
- Produces:
  - Clap: `also: Vec<String>` (`--also`, repeatable), `keys: bool` (`--keys`), `lean: bool` (`--lean`, `requires = "keys"`).
  - `pub fn recall_many_with(root, queries: &[String], ledger, include_hidden) -> Result<Value, LedgerError>`: one `search_with` for all queries. Output = today's top-level fields minus `match`, plus `queries: [{query, match_kind, hits: [{id, matched_terms?}]}]` (`match_kind` is `"partial"` or `"all"`), and `hits` = each query's hits in its own order, queries in argument order, first occurrence kept, each hit with `matched_queries: [index, ...]` and without `matched_terms`.
  - `pub fn recall_keys_with(root, queries: &[String], ledger, include_hidden) -> Result<Value, LedgerError>`: `Haystack::Key`; hits are `{key, id, recorded_at_utc, state?, conflict?}` (`state` only on an enabled project, `conflict: true` only when the key has ≥2 active ids); plus `"next": "cort memory recall \"<words>\" --also \"<words in the other language>\""`. With `--lean`, `render_command: Some("memory-recall-lean")` prints one tab-separated line per hit with five fields always, `key id recorded_at_utc state conflict`, each through `memory_tiers::lean_field`, `-` for a state on a disabled project and `-` or `conflict` for the last field (no empty field, the `list --lean` rule), then the `next` line.
  - A single query without `--also`/`--keys` goes through `recall_all_with` exactly as today.
- [ ] **Step 1: Write the failing tests.**
  - `single_query_cli_output_is_unchanged` (`cli.rs`): `cort memory recall "<q>" --json` stdout equals, byte for byte, `recall_whole_with` serialized through the same handler fields (`harness_declared`, `remote`, `machine`) for 3 queries. The single-query branch of the handler is not edited in this task; Task 4 already proves `recall_all_with` equals `recall_whole_with`, so this test guards the branch selection.
  - `also_shape`: `"部署" --also "deploy"` — `queries.len() == 2`, a hit found by both has `matched_queries == [0,1]` and no `matched_terms`; per-query partial hits carry their own `matched_terms` inside `queries[i].hits`.
  - `unquoted_two_word_query_is_refused` (`cli.rs`): `cort memory recall deploy plan` exits non-zero with clap's unexpected-argument error.
  - `keys_rows`: `--keys "deploy"` returns only key matches, rows have exactly the fields above, `next` present; `--keys --lean` prints the TSV lines; `--lean` without `--keys` is refused.
  - `keys_with_also_and_all`: `--keys "deploy" --also "部署" --all` — `queries.len() == 2`, a hidden entry appears with its `state`.
- [ ] **Step 2: Run** → fail. **Step 3: Implement.** **Step 4: Run** all `rust` tests; gate `--crate rust`.
- [ ] **Step 5: Commit** `rust/src/main.rs rust/src/memory.rs rust/src/memory_store.rs rust/tests/memory_search.rs rust/tests/cli.rs docs/memory.md skills/cort/SKILL.md AGENTS.md` — `feat(memory): recall --also and --keys (#11 2a)`.

### Task 6: `cort-evals recall-sql` grades the product

**Files:**
- Modify: `evals/src/recall_sql.rs` (drop `Arm`, `like_escape`, `active_filter`, `fetch`, `replay_arm`), `evals/src/main.rs` (`RECALL_SQL_FLAGS`, `USAGE_RECALL_SQL` — it no longer runs SELECTs only: the product read issues `CREATE TABLE IF NOT EXISTS`, which changes nothing in an existing table; say so — and `recall_sql_main`)
- Test: `evals/src/recall_sql.rs` tests module

**Interfaces:**
- Consumes: `memory_store::{select_whole_with, select_search_with}` (Task 4). No git root is needed.
- Produces: `cort-evals recall-sql --project NAME --memory-project ID --url URL --token-file FILE [--dir DIR] [--days N]`. `--ledger` and `--projects` are removed (aliases come from `project_aliases` in the database). Report: per group `{queries, equal, rows: {sum,max}, ms: {p50,max}, trips: {sum,max}}` (`trips` from `select_search_with`) and `mismatches`, where baseline = `select_whole_with` and replay = `select_search_with`, `include_hidden = false`, rows from the latter.
- [ ] **Step 1: Write the failing test** `replay_equals_baseline_on_fixture`: a `SqliteTransport` database with an alias pair and ≥10 queries; `report(..)["mismatches"]` is empty and every group's `equal == queries`.
- [ ] **Step 2: Run** `... cargo test --locked --manifest-path evals/Cargo.toml recall_sql` → fail. **Step 3: Implement.** **Step 4: Run**; full gate (both crates).
- [ ] **Step 5: Commit** `evals/src/recall_sql.rs evals/src/main.rs` — `feat(evals): recall-sql replays through the product's search (#11 2a)`.

### After the tasks (coordinator, not an implementer)

- [ ] Whole-branch external review (read-only), findings checked line by line against the source.
- [ ] Re-run the replay on a fresh throwaway copy of agent-portal-tpm (needs the user's approval through bsr, as the first one did; destroy the database and log out afterwards). First, on that database, a read-only `SELECT lower('É'), lower('Ω')` must return `É` and `Ω`: the superset argument assumes the server's `lower()` folds ASCII only (no ICU); if it folds Unicode, stop and re-check `sql_exempt` before trusting the replay. The archived condition uses a window function (`row_number() OVER`), and the integrity screen compares row values (`(project, rid) IN (...)`), so the replay also proves the server supports both; if it refuses window functions, replace that fragment with the `NOT EXISTS` form (lifecycle rows x lifecycle rows, still uncorrelated with the memory rows). Two more readings on that database (Task 4 external review, 2026-11-16): the server's `rows_read` per trip, beside today's whole read of the same ledger, quoted in the record; and one recall run while another connection holds an open write transaction, to see whether the five `CREATE TABLE IF NOT EXISTS` inside `BEGIN` make the trip queue as a writer and answer Busy (if they do, send only the read statements and create the tables on `no such table`). Also read `SELECT hex(json_extract('{"a":"x\u0000y"}','$.a')), instr(json_extract('{"a":"x\u0000y"}','$.a'),'y')` there: `780079` and 3 mean a claim holding an escaped NUL is searched whole; anything shorter means the server truncates at NUL and the superset argument fails for such rows (whole-branch review, 2026-11-16; `length()` is no probe, since SQLite's own `length()` counts only up to the first NUL and answers 1 either way). The `rows_read` reading is a decision input for the push, not only a record: if the search reads far more server rows than the whole read, bring the numbers to the user before pushing. Expect 109 of 109 equal; record under ledger key `memory-tiers-phase2-search-replay` with `--replace`.
- [ ] Push only with the user's quoted, dated approval relayed by bsr.
