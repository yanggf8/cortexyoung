# Hook Seed-or-Edge Predicate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the `PreToolUse` hook suggesting `cort impact` for symbols the index holds nothing
about, without silencing the deletion-verification case it was built for.

**Architecture:** `judge` gains one **lazily evaluated** evidence lookup and returns a `Verdict` that
names its silences. The lookup is called only after every shape check passes, and it *builds* the
index probe itself — so the ~95% of searches the shape gate rejects pay for neither the database nor
`git rev-parse`. The predicate is "a seed **or** a raw edge naming this symbol", not "a seed",
because `raw_edges` outlives a deleted symbol's `chunks` row, which is what keeps the deletion case
firing.

**Tech Stack:** Rust (`rust/` and `evals/` crates), SQLite.

**Spec:** `docs/2026-09-04-hook-precision-and-the-seed-gate.md` — §10 exit D is what this builds; §6,
§9 and the "new constraint" subsection are why its shape is what it is. Read both.

**Revision:** v2.2, 2026-09-04. v1 was reviewed by Codex (`01a06c99-ec0b-7d21-bc04-d9629d9bb25e`) and
corroborated against source: two blockers, five majors. v2's review (`01a06ce8-992a-7893-aa2c-03ee7b65282f`)
was cut short by quota after confirming the structural fixes hold "on paper"; the two checks it named
next were then done by hand and **both found something** — §Review record rows 10-12. **Do not execute
v1 or v2.**

v2.1 was then reviewed by Kimi, which found two more blockers and four majors — rows 13-20 — including
the one I asked it for by name: **nothing asserted that the new outcome reaches `usage.db` at all**.
Both items v2.1 listed as unverified are now settled: the rename has no machine consumer outside
`evals/tests/adopt.rs` (row 14), and `judge`'s `impl FnOnce` is callable from `evals/src/hook.rs:408`
without borrow trouble. **Do not execute v1, v2 or v2.1.**

---

## Why this predicate and not the obvious one

Measured over the 110 hook fires that landed in projects with an index, one run at `dbd5bf13`:

| Bucket | Count | What it is |
|---|---:|---|
| A — has a seed | 36 (32.7%) | `impact` can answer |
| B — raw edge only | **14 (12.7%)** | external types (`Option`, `JSON`), deleted symbols |
| C — neither | **60 (54.5%)** | concepts (`tide`), fields (`owner`), file listings (`chart`) |

| | Fires | Empty answers | Deletion case | `tests/hook.rs:108-116` |
|---|---:|---:|---|---|
| today | 110 | **74 (67%)** | fires | green |
| gate on seed alone | 36 | 0 | **silenced** | **must be inverted** |
| **gate on seed-or-edge** | **50** | **14 (13%)** | **fires** | **green** |

The deletion case survives because `raw_edges` is rebuilt across files and therefore outlives the
`chunks` row (`rust/src/schema.sql`, the F-01 comment). Verified by deleting a definition and running
`index --incremental`, which is what the `PostToolUse` hook does: the `chunks` row went to 0 while one
`raw_edges` row still named it.

**What this does not do.** The hook keeps firing on deletion verification and `impact` keeps
answering `seeds=0 / nothing was looked at` there (spec §9). This plan declines to make that worse
while fixing most of the rest; fixing it is the tombstone project (spec §10 exit B).

## Review record — every v1 defect and where it is handled

| # | Defect | Handled |
|---|---|---|
| 1 | **Blocker.** Task 1 changed `judge`'s signature but updated no caller, while requiring `clippy --all-targets`. Four call sites: `main.rs:861`, `tests/hook.rs:253`, `:279`, `:318` | Task 1 updates all four in the same commit |
| 2 | **Blocker.** `index_state() -> IndexProbe` breaks `hook-refresh`'s `index_state() == IndexState::Missing` (`main.rs:771`) | Task 3 leaves `index_state` alone and adds a separate `evidence_at_cwd` |
| 3 | Probing before `judge` cost 3.3–4.6ms on every parsed search and mis-attributed `no_index` to shape-rejected searches, contradicting the semantics `tests/cli.rs:469` documents | Task 3 builds the probe **inside** the closure; a `NoShape` search touches nothing |
| 4 | `seed_state_in` returned a bare `SeedState` with `.unwrap_or(false)`, so two SQL errors produced `Neither` → silence — the opposite of the stated policy | Task 2 returns `rusqlite::Result<Evidence>`; Task 3 maps `Err` to `Unknown`, which fires. Both pinned by tests |
| 5 | The plan claimed both queries were index-covered. The `raw_edges` one is a scan: `SEARCH raw_edges USING COVERING INDEX sqlite_autoindex_raw_edges_1 (project_id=?)`, ~2.95ms for an absent symbol over 15,850 rows | Task 2 states the real plan and the real cost; no new index (see §Known risk) |
| 6 | Adding `shape_fired` beside `fired` leaves `fired`, `fired_shell`, `fired_structured` and three `fire_rate_*` fields carrying full-verdict names for a shape-only number | Task 4 renames the whole family and bumps to `hook-probe-v3` |
| 7 | `adopt-mine` also calls `suggests_impact` (`evals/src/adopt.rs:447`); its `rule_would_fire` denominator silently becomes shape-only | Task 4 renames that field too |
| 8 | The pinned deletion test stays green because `Unknown` fires, not because production finds the raw edge — so nothing tests the shipped behaviour | Task 3 adds the end-to-end test: index, delete, re-index incrementally, run `hook-suggest`, assert it still fires |
| 9 | Task 5 declared a stop condition on a `no_evidence` number that Task 4 makes `hook-probe` structurally unable to produce; and used a Python one-liner against `AGENTS.md`'s pure-Rust rule | Task 5 measures through `cort hook-suggest` itself and through `usage.db`, with no scripting |
| 10 | **v2 defect.** `evidence_at_cwd` decided `NoIndex` from `!path.exists()` alone, so a db file that exists with no `projects` row — the state `rust/tests/cli.rs:433` builds and pins — would have been recorded as `no_evidence` rather than `no_index`. The same attribution class as #3, in the other direction, corrupting the very measurement this change exists to produce | Task 3 shares `index_state`'s real test (`status_of(..).indexed`) through one `probe_index` helper (there is no `evidence_at_cwd` in v2.2) |
| 11 | **v2 defect.** `index_state()` was called a second time after `judge` fired, so a firing search paid canonicalize + open + `status_of` + `git rev-parse` twice | Task 3 fuses them: one probe, its `IndexState` handed back through a `Cell` the closure writes |
| 12 | `sandbox()`'s `SAMPLE` fixture is **TypeScript** (`src/helper.ts`, `src/alpha.ts`) and `FIRING_SEARCH` already exists at `rust/tests/cli.rs:430` | Task 3's tests use both rather than inventing fixtures |
| 13 | **v2.1 blocker.** Task 1's caller list omitted the entire `evals` crate — `evals/src/hook.rs:20` imports `judge`, `:21` re-exports `suggests_impact`, `evals/src/adopt.rs:18`/`:447` use it — and its verify step built only `rust/`, so the commit would break the workspace invisibly | Task 1 updates all four and every task now verifies **both** crates |
| 14 | **v2.1 blocker.** Task 4 renamed `rule_would_fire` but never touched `evals/tests/adopt.rs:138`, `:245`, `:365`, which assert it by serde index — compiles, then fails at runtime, so Task 4's "Expected: PASS" was unreachable | Task 4 lists that file and updates all three |
| 15 | **v2.1, the "test does not test the thing" gap.** Nothing asserted that `SilenceReason::NoEvidence` reaches `usage.db` as `no_evidence`. Wiring it to `"no_index"` by a one-token slip would leave every planned test green while Task 5's aggregate never gained a row | Task 3's first test reads `hook_outcomes_at` and asserts the outcome, following `rust/tests/cli.rs:477` |
| 16 | **v2.1.** All three new CLI tests lacked the suite's `SKIP:` guard, so test 1 passed *vacuously* when ast-grep is absent — no index, `no_index` silence, `{}` on stdout, nothing about the predicate exercised | All three carry the guard; test 1 additionally asserts the outcome, which a vacuous run cannot satisfy |
| 17 | **v2.1.** Both `RawOnly` fixtures used a bare call, so only the exact-match arm was queried; the two `LIKE` arms covering `crate::m::f` and `x.method` — the common shape — were unexercised, and deleting one would keep every test green | Task 3's deletion fixture calls through `crate::gone::…` |
| 18 | **v2.1.** `evidence_in` did not mirror `extracted_but_unresolved` as its comment claimed: no `rel_type` filter (so `imports` leaked — `./tide` matches `%.tide`) and the exact arm took the full symbol rather than the leaf | Task 2 matches `coverage.rs:383-390` exactly. Measured: 7 of 110 fires move from B to C, empty answers 21 → **14** |
| 19 | **v2.1.** The `Cell` hand-back relies on "`Fire` implies the closure ran", a coupling invisible at the read site | Task 3 adds a `debug_assert!` beside it |
| 20 | **v2.1, cosmetic.** The File Structure table and row 10 named `evidence_at_cwd`, which v2.1 replaced with `probe_index` | Both corrected below |

## Global Constraints

- The routing **decision** is singular. Parsers may be plural; a second copy of the verdict would make
  `hook-probe`'s calibration describe something other than what ships (CLAUDE.md; a hand-rolled
  approximation once over-counted by 48% and 4x). The predicate lives in `judge`.
- The hook's timeout is **5 seconds** (`rust/src/settings.rs:80`). `git rev-parse` may spend 400ms of
  it (`HOOK_GIT_BUDGET_MS`, `main.rs:959`). Neither may be paid by a search the shape gate rejects.
- Storage failures **fire**, never silence, and never panic. `hook-suggest` stays silent and exits 0
  whatever happens.
- The repo is pure Rust plus `install.sh` and `tests/install-smoke.sh`. **No Python, not even a
  one-liner in a measurement step** (`AGENTS.md`).
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` in **both** crates before each
  commit; every commit must compile on its own.
- Commit messages end with:
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01VZYNBd2gsdwcdZbLi86nzt`
- The user gates every commit. Do not push.

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `rust/src/hook.rs` | modify `:20-28`, `:205-207`, `:329-373`; add `evidence_in` | `Evidence`, `Verdict`, `judge`, the lookup |
| `rust/src/main.rs` | modify `:857-881`; split `index_state` into `probe_index` | supply the lazy closure, record the outcome |
| `rust/tests/hook.rs` | modify `:253`, `:279`, `:318`; add 4 tests | verdict semantics and the lookup |
| `rust/tests/cli.rs` | add 3 tests | shipped behaviour, including the deletion case |
| `evals/src/hook.rs` | modify `:408`, `:450-462`, the `reading` | pass `Unknown`; rename the shape-only family |
| `evals/src/adopt.rs` | modify `:18`, `:447`, `~:660` | call-site rename (Task 1), field rename (Task 4) |
| `evals/tests/adopt.rs` | modify `:138`, `:245`, `:365` | the only consumers of `rule_would_fire` |
| `docs/…-seed-gate.md`, `README.md` | modify | record what shipped and what it measured |

---

### Task 1: `Evidence`, `Verdict`, and a lazy lookup in `judge`

**Files:**
- Modify: `rust/src/hook.rs` (above `HookHit` at `:20`, `suggests_impact` at `:205-207`, `judge` at `:329-373`)
- Modify: `rust/src/main.rs:861` — the one production caller, so the crate still compiles
- Modify: `rust/tests/hook.rs:253`, `:279`, `:318` — the three direct `judge` callers
- Modify: `evals/src/hook.rs:20-21` (imports `judge`, re-exports `suggests_impact`) and
  `evals/src/adopt.rs:18`, `:447` — **the evals crate breaks too**, and its build is not part of
  `cd rust && cargo test`
- Test: `rust/tests/hook.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum Evidence { Seed, RawOnly, Neither, NoIndex, Unknown }`
  - `pub enum SilenceReason { NoShape, NoIndex, NoEvidence }`
  - `pub enum Verdict { Fire(HookHit), Silent(SilenceReason) }`
  - `pub fn judge(search: &Search, evidence: impl FnOnce(&str) -> Evidence) -> Verdict`
  - `pub fn suggests_impact_shape(command: &str) -> Option<HookHit>` — renamed from
    `suggests_impact`, passes `Evidence::Unknown`, and the name now says it tests the shape half only.

  Task 2 supplies the real lookup; Task 3 matches on `Verdict`; Task 4 calls the renamed wrapper.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/hook.rs`:

```rust
/// The predicate is "a seed OR a raw edge naming this symbol", and the second half is what keeps the
/// deletion case alive: `raw_edges` outlives the `chunks` row it pointed at, so a just-deleted symbol
/// still has evidence even though `impact` can no longer seed on it.
///
/// `Unknown` fires because a lookup that could not run is not a finding. `NoIndex` is a distinct
/// silence from `NoEvidence`: one is a missed opportunity, the other is a correct refusal, and
/// `tests/cli.rs` documents why that distinction has to survive into the usage row.
#[test]
fn the_verdict_names_which_silence_it_chose() {
    let s = search_from_shell("grep -rn 'ensureSeedUserPasswords' src/").expect("parses");

    for (ev, label) in [
        (Evidence::Seed, "a symbol impact can seed on"),
        (Evidence::RawOnly, "a deleted symbol a surviving caller still names"),
        (Evidence::Unknown, "a lookup that could not run must not silence"),
    ] {
        assert!(
            matches!(judge(&s, |_| ev), Verdict::Fire(_)),
            "{label}: expected Fire"
        );
    }
    assert_eq!(
        judge(&s, |_| Evidence::Neither),
        Verdict::Silent(SilenceReason::NoEvidence)
    );
    assert_eq!(
        judge(&s, |_| Evidence::NoIndex),
        Verdict::Silent(SilenceReason::NoIndex)
    );
}

/// The lookup must not run for a search the shape gate already rejects, and the lookup is what opens
/// the database and shells out to git. On this machine's corpus the shape gate turns down about 95%
/// of searches; the hook's whole budget is 5s and `git rev-parse` may take 400ms of it.
#[test]
fn the_evidence_lookup_is_not_consulted_when_the_shape_gate_rejects() {
    let s = search_from_shell("grep -rn -A 3 'helper' src/").expect("parses");
    let mut consulted = false;
    let v = judge(&s, |_| {
        consulted = true;
        Evidence::Seed
    });
    assert_eq!(v, Verdict::Silent(SilenceReason::NoShape));
    assert!(
        !consulted,
        "a shape rejection must not open a database or run git"
    );
}
```

Extend the file's `use cort::hook::{...}` with `Evidence`, `SilenceReason`, `Verdict`,
`search_from_shell`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test hook the_verdict_names_which_silence`
Expected: FAIL to compile — "cannot find type `Evidence`".

- [ ] **Step 3: Write minimal implementation**

In `rust/src/hook.rs`, above `HookHit`:

```rust
/// What this project's index has to say about one symbol, asked as cheaply as it can be.
///
/// `RawOnly` is the variant this whole design turns on. `raw_edges` is rebuilt across files and
/// therefore outlives the `chunks` row it pointed at (schema F-01), so a symbol whose definition was
/// just deleted still has a surviving caller's raw edge naming it. Gating on `Seed` alone would
/// silence the hook on exactly the deletion-verification search the goal sentence names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// A `chunks` row: `impact` can seed on it.
    Seed,
    /// No chunk, but a `raw_edges` row names it: deleted, or an external type this project uses.
    RawOnly,
    /// Nothing in the index names it at all: a concept, a field, a domain word.
    Neither,
    /// This project has no index, so there was nothing to ask.
    NoIndex,
    /// The lookup could not run -- a replay with no recoverable state, or a database that would not
    /// open or answer. Fires: a question that was never put is not a negative answer.
    Unknown,
}

/// Why the hook stayed quiet. Separate variants because the mining has to tell a heuristic problem
/// from an index problem from a missing index, and a single `None` collapses all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilenceReason {
    /// Not the narrow shape where `impact` beats `rg`.
    NoShape,
    /// Right shape, no index for this project -- a missed opportunity, not a refusal.
    NoIndex,
    /// Right shape, indexed project, and the index holds neither a seed nor a raw edge naming the
    /// symbol. `impact` would answer `seeds=0 dependents=0` and nothing else.
    NoEvidence,
}

/// The whole routing decision for one search.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Fire(HookHit),
    Silent(SilenceReason),
}
```

Change `judge`'s signature to
`pub fn judge(search: &Search, evidence: impl FnOnce(&str) -> Evidence) -> Verdict`, replace every
`return None;` in its body with `return Verdict::Silent(SilenceReason::NoShape);`, and replace the
tail:

```rust
    let reason = if targets.trim().is_empty() {
        "bare symbol, search scoped to the working tree"
    } else if SOURCE_MARKERS.iter().any(|m| targets.contains(m)) {
        "bare symbol, search scoped to project source"
    } else {
        return Verdict::Silent(SilenceReason::NoShape);
    };
    // Only now, with every shape check passed, is the index asked -- and the closure is what opens
    // it. This ordering is the budget: the shape gate turns down about 95% of searches and none of
    // them may cost a database open or a `git rev-parse`.
    match evidence(&symbol) {
        Evidence::Neither => Verdict::Silent(SilenceReason::NoEvidence),
        Evidence::NoIndex => Verdict::Silent(SilenceReason::NoIndex),
        Evidence::Seed | Evidence::RawOnly | Evidence::Unknown => {
            Verdict::Fire(HookHit { symbol, reason })
        }
    }
}
```

Rename the wrapper and say what it is:

```rust
/// The **shape half** of the rule, for unit tests and for any caller with no index to ask. It passes
/// `Evidence::Unknown`, which fires, so it answers exactly the question it always answered: is this
/// the shape? It is deliberately weaker than what ships, and any metric built on it must say so --
/// see `evals::hook`'s `shape_fired` family.
pub fn suggests_impact_shape(command: &str) -> Option<HookHit> {
    match judge(&search_from_shell(command)?, |_| Evidence::Unknown) {
        Verdict::Fire(hit) => Some(hit),
        Verdict::Silent(_) => None,
    }
}
```

Update the four callers so the workspace compiles in this same commit:

- `rust/src/main.rs:861` — temporarily `cort::hook::judge(&search, |_| cort::hook::Evidence::Unknown)`
  matched to `Verdict::Fire(hit) => hit, Verdict::Silent(_) => return quiet()`. Task 3 replaces the
  closure with the real one; behaviour is unchanged until then.
- `rust/tests/hook.rs:253` — `judge(&search_from_grep_fields(...).expect(...), |_| Evidence::Unknown)`,
  and the helper's return type becomes `Verdict`; its two callers at `:279` and `:318` match on it.
- Every `suggests_impact(` in `rust/tests/hook.rs` becomes `suggests_impact_shape(`
  (about 30 call sites; a whole-file rename is correct here — they are all shape assertions).
- `evals/src/hook.rs:20` imports `judge`; `:21` re-exports `suggests_impact`; `evals/src/adopt.rs:18`
  imports it and `:447` calls it. Update all four in this commit: the re-export and the import become
  `suggests_impact_shape`, and the `judge` call at `evals/src/hook.rs:408` becomes
  `match judge(&search, |_| cort::hook::Evidence::Unknown) { Verdict::Fire(hit) => { …existing body… }, Verdict::Silent(_) => {} }`.
  Task 4 then only renames report fields, not call sites.
- `judge`'s first line is `let symbol = symbol_of_pattern(&search.pattern)?;` — that `?` returns
  `None`, not a `Verdict`, and is not one of the `return None;` statements. Rewrite it as a
  `let … else { return Verdict::Silent(SilenceReason::NoShape); }`.

- [ ] **Step 4: Run tests to verify they pass**

Run **both crates**, because this commit changes an API the `evals` crate imports:

```bash
cd rust && cargo test && cargo clippy --all-targets -- -D warnings
cd ../evals && cargo test && cargo clippy --all-targets -- -D warnings
```
Expected: PASS in both, including `tests/hook.rs:108-116`. Behaviour is unchanged at this commit —
only the types moved. **Every later task's verify step runs both crates too**; a plan that builds one
of two crates cannot honour "every commit must compile on its own".

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/hook.rs rust/src/main.rs rust/tests/hook.rs
git commit -m "feat(hook): judge names its silences and takes a lazy evidence lookup

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VZYNBd2gsdwcdZbLi86nzt"
```

---

### Task 2: The lookup itself

**Files:**
- Modify: `rust/src/hook.rs` (add beside `Evidence`)
- Test: `rust/tests/hook.rs`

**Interfaces:**
- Consumes: `Evidence` (Task 1).
- Produces: `pub fn evidence_in(db: &rusqlite::Connection, project_id: &str, symbol: &str) -> rusqlite::Result<Evidence>`
  — returns `Seed`, `RawOnly` or `Neither`, or the sqlite error. It never returns `Unknown` or
  `NoIndex`; those are the caller's to decide. Task 3 maps `Err` to `Unknown`.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/hook.rs`:

```rust
fn indexed_project(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
    }
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let mut db = cort::db::open_db(":memory:").unwrap();
    cort::db::ensure_schema(&db).unwrap();
    let project_id = cort::db::project_id_for(root.to_str().unwrap());
    let bin = cort::ast_grep::resolve_ast_grep_bin().expect("ast-grep on PATH");
    cort::indexer::full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

/// The three states against a real index. The `RawOnly` case is built the way it happens in life --
/// index, delete the definition, re-index incrementally, which is what the PostToolUse hook does --
/// because a hand-inserted row would not prove that `raw_edges` survives that path.
#[test]
fn evidence_reads_chunks_then_raw_edges() {
    let (_dir, root, mut db, project_id, bin) = indexed_project(&[
        ("src/gone.rs", "pub fn ensure_seed_user_passwords() -> u8 { 1 }\n"),
        (
            "src/user.rs",
            "use crate::gone::ensure_seed_user_passwords;\npub fn boot() -> u8 { ensure_seed_user_passwords() }\n",
        ),
    ]);
    assert_eq!(
        evidence_in(&db, &project_id, "ensure_seed_user_passwords").unwrap(),
        Evidence::Seed
    );
    assert_eq!(
        evidence_in(&db, &project_id, "no_such_name_anywhere").unwrap(),
        Evidence::Neither
    );

    std::fs::remove_file(root.join("src/gone.rs")).unwrap();
    std::fs::write(
        root.join("src/user.rs"),
        "pub fn boot() -> u8 { ensure_seed_user_passwords() }\n",
    )
    .unwrap();
    cort::incremental::incremental_index(&mut db, &bin, &root).unwrap();

    assert_eq!(
        evidence_in(&db, &project_id, "ensure_seed_user_passwords").unwrap(),
        Evidence::RawOnly,
        "the definition is gone but the surviving caller's raw edge still names it"
    );
}

/// A storage failure must be returned, not flattened into `Neither`. `Neither` silences the hook;
/// an unreadable database must fire instead, which is the caller's job and it needs the error to do
/// it. Dropping the table is the cheapest way to make both queries fail.
#[test]
fn a_storage_failure_is_returned_rather_than_read_as_absence() {
    let (_dir, _root, db, project_id, _bin) =
        indexed_project(&[("src/lib.rs", "pub fn helper() -> u8 { 1 }\n")]);
    db.execute_batch("DROP TABLE chunks").unwrap();
    assert!(
        evidence_in(&db, &project_id, "helper").is_err(),
        "a missing table is an error, not an absent symbol"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test hook evidence_reads_chunks`
Expected: FAIL to compile — "cannot find function `evidence_in`".

- [ ] **Step 3: Write minimal implementation**

In `rust/src/hook.rs`:

```rust
/// Ask one project's index what it holds about `symbol`, in at most two queries.
///
/// `chunks` first: a seed is the strong answer and the common one, and the query is covered by
/// `idx_chunks_symbol (project_id, symbol_name)`.
///
/// `raw_edges` second, matched **exactly** the way `coverage::extracted_but_unresolved` matches
/// (`rust/src/coverage.rs:383-390`): the same `rel_type IN ('calls','references')` filter, and the
/// leaf passed for all three parameters. Both halves of that matter and a first draft got both wrong.
///
/// Without the `rel_type` filter the query also counts `imports`, whose raw targets are module
/// specifiers -- `./tide` ends with `.tide` and matches the `LIKE '%.'` arm -- so a bare `tide`
/// search fires in any project that imports a file by that name. Measured on this corpus, filtering
/// moves 7 of 110 fires from "raw edge" to "nothing", cutting remaining empty answers from 21 to 14.
/// Nothing is lost by it: an `imports` raw edge never becomes a relationship at all
/// (README limitation 8), so `impact` could not have answered those anyway.
///
/// Using the same splitter and the same parameters as the coverage screen is deliberate: two answers
/// to "which name is this" is how a gate and a report start disagreeing. One shared looseness comes
/// with it -- `LIKE` treats `_` as a single-character wildcard, so every snake_case leaf is a pattern
/// rather than a literal. Coverage has always had that, so this is not a new divergence, but neither
/// place has measured it.
///
/// That second query is **a scan of the project's raw edges**, not an indexed seek: the only index
/// on the table is `(project_id, file_path)` and the `LIKE` terms lead with a wildcard, so SQLite
/// plans `SEARCH raw_edges USING COVERING INDEX sqlite_autoindex_raw_edges_1 (project_id=?)`.
/// Measured on this repository's 15,850 rows: ~2.95ms for an absent symbol, ~0.16-1.05ms when it
/// hits. That is affordable only because this runs after the shape gate, on about one search in
/// twenty. If it ever stops being affordable, the fix is a stored `raw_target_leaf` column with its
/// own index -- not a tighter match, which would cost the deletion case.
///
/// Errors are returned. A caller that reads a storage failure as "absent" would silence the hook on
/// a disk problem, which is the opposite of this crate's rule that storage failures degrade loudly.
pub fn evidence_in(
    db: &rusqlite::Connection,
    project_id: &str,
    symbol: &str,
) -> rusqlite::Result<Evidence> {
    let seed = db
        .query_row(
            "SELECT 1 FROM chunks WHERE project_id = ?1 AND symbol_name = ?2 LIMIT 1",
            rusqlite::params![project_id, symbol],
            |_| Ok(()),
        )
        .optional()?;
    if seed.is_some() {
        return Ok(Evidence::Seed);
    }
    let leaf = crate::chunker::bare_name(symbol);
    let edge = db
        .query_row(
            "SELECT 1 FROM raw_edges
              WHERE project_id = ?1 AND rel_type IN ('calls', 'references')
                AND (raw_target = ?2 OR raw_target LIKE '%:' || ?3 OR raw_target LIKE '%.' || ?4)
              LIMIT 1",
            rusqlite::params![project_id, leaf, leaf, leaf],
            |_| Ok(()),
        )
        .optional()?;
    Ok(if edge.is_some() {
        Evidence::RawOnly
    } else {
        Evidence::Neither
    })
}
```

`.optional()` needs `use rusqlite::OptionalExtension;` at the top of the file.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test hook`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/hook.rs rust/tests/hook.rs
git commit -m "feat(hook): evidence_in reads chunks then raw edges, and returns its errors

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VZYNBd2gsdwcdZbLi86nzt"
```

---

### Task 3: Wire it into the shipped hook

**Files:**
- Modify: `rust/src/main.rs` (`cmd_hook_suggest` at `:857-881`; add `evidence_at_cwd` beside `index_state` at `:995`)
- Test: `rust/tests/cli.rs`

**Interfaces:**
- Consumes: `Verdict`, `SilenceReason`, `Evidence`, `evidence_in` (Tasks 1-2).
- Produces: `fn probe_index() -> (IndexState, Option<Connection>, String)`; `index_state()` unchanged
  in signature; and a hook that records `no_evidence` only for a genuinely indexed project, while
  `no_index` and `no_shape` keep exactly the meanings `rust/tests/cli.rs` pins.

**`index_state` keeps its exact signature**, because `hook-refresh` compares it against
`IndexState::Missing` (`main.rs:771`). Its body moves into `probe_index`, which also hands back the
open connection and the project id; `index_state` becomes `probe_index().0`. That is what lets the
closure apply the *same* "is this an index" test the old gate did — `status.indexed`, not merely a db
file on disk — and lets the fire path probe once instead of twice.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`. Use its existing helpers: `sandbox()` (`:80`) returns
`(proj, cwd, cache_dir, cache)`, `run_cort(args, cwd, cache)` (`:57`) sets the working directory, and
`run_hook_suggest(command, cwd, cache)` (`:385`) builds the payload and pipes it to stdin.

```rust
/// A concept search in an indexed project: right shape, and nothing in the index named `tide`. The
/// hook must stay silent rather than send the agent to `impact` for an answer that would be
/// `seeds=0 dependents=0` and nothing else.
#[test]
fn the_hook_is_silent_when_the_index_holds_nothing_about_the_symbol() {
    let (_proj, cwd, _cache_dir, cache) = sandbox();
    // Without this guard the test passes vacuously when ast-grep is missing: indexing fails, the
    // project has no index, the hook silences as `no_index`, stdout is still `{}` -- and nothing
    // about the evidence predicate was exercised. The neighbouring tests already carry it.
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let out = run_hook_suggest("grep -rn 'tide' src/", &cwd, &cache);
    assert_eq!(
        out.stdout.trim(),
        "{}",
        "a name the index never heard of: {}",
        out.stdout
    );
    // The silence has to be attributed, not merely observed. This is the assertion the whole change
    // exists to make measurable: wire `NoEvidence => "no_index"` by a one-token slip and every other
    // test here still passes while Task 5's aggregate silently never gains a row.
    let counts =
        cort::usage::hook_outcomes_at(&cache.join("usage.db"), 0, None).expect("read usage db");
    assert_eq!(
        counts.get("no_evidence").and_then(Value::as_i64),
        Some(1),
        "an indexed project that holds nothing about the symbol is `no_evidence`, \
         not `no_index` and not `no_shape`: {counts:?}"
    );
}

/// The same project, a symbol it does hold. Still fires.
#[test]
fn the_hook_still_fires_for_a_symbol_the_index_holds() {
    let (_proj, cwd, _cache_dir, cache) = sandbox();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let out = run_hook_suggest(FIRING_SEARCH, &cwd, &cache);
    assert!(
        out.stdout.contains("cort impact --symbol 'helper'"),
        "expected a suggestion: {}",
        out.stdout
    );
}

/// The case the whole predicate exists for, end to end and through the shipped binary rather than
/// through the shape-only wrapper. Index, delete the definition, re-index incrementally -- which is
/// exactly what the PostToolUse hook does after an edit -- then run the deletion-verification grep.
/// The `chunks` row is gone, so a seed-only gate would go quiet here; the surviving caller's raw
/// edge is what keeps it firing.
#[test]
fn the_hook_still_fires_after_the_definition_is_deleted_and_reindexed() {
    let (proj, cwd, _cache_dir, cache) = sandbox();
    std::fs::write(
        proj.path().join("src/gone.rs"),
        "pub fn ensure_seed_user_passwords() -> u8 { 1 }\n",
    )
    .unwrap();
    // The surviving caller uses a QUALIFIED path on purpose. A bare `ensure_seed_user_passwords()`
    // leaves a raw target that only the exact-match arm of the query sees, so both `LIKE` arms --
    // the half that covers `crate::m::f` and `x.method`, which `rust/src/graph.rs:185-189` documents
    // as the common shape -- would go unexercised, and swapping or deleting them would keep every
    // test in this plan green while real deletion searches went silent.
    std::fs::write(
        proj.path().join("src/user.rs"),
        "pub fn boot() -> u8 { crate::gone::ensure_seed_user_passwords() }\n",
    )
    .unwrap();
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }

    std::fs::remove_file(proj.path().join("src/gone.rs")).unwrap();
    run_cort(&["index", "--incremental"], &cwd, &cache);

    let out = run_hook_suggest(
        r#"grep -rn "ensure_seed_user_passwords" src/ 2>/dev/null"#,
        &cwd,
        &cache,
    );
    assert!(
        out.stdout.contains("ensure_seed_user_passwords"),
        "a deleted symbol its callers still name must keep firing: {}",
        out.stdout
    );
}
```

`SAMPLE` (`rust/tests/cli.rs:11`) is **TypeScript** — `src/helper.ts` and `src/alpha.ts` — so
`helper` is already an indexed symbol and the second test needs no new files. `FIRING_SEARCH`
(`:430`) is the established firing command; use it rather than a new string. The third test adds
`.rs` files to that same project, which the pack indexes alongside the TypeScript.

- [ ] **Step 2: Run tests to verify the first fails**

Run: `cd rust && cargo test --test cli the_hook_is_silent_when_the_index_holds_nothing`
Expected: FAIL — the hook currently suggests `impact` for `tide`.

- [ ] **Step 3: Write minimal implementation**

Split `index_state`'s body into a reusable probe, leaving `index_state` itself byte-compatible for its
`hook-refresh` caller (`rust/src/main.rs:771`). In `rust/src/main.rs`:

```rust
/// One canonicalize, one read-only open, one `status_of`, one bounded `git rev-parse` -- and the
/// connection handed back so a caller that also needs to ask about a symbol does not repeat any of
/// it. `index_state` is the same thing with the extras discarded, so `hook-refresh` keeps compiling
/// against the signature it has always used.
fn probe_index() -> (IndexState, Option<Connection>, String) {
    let Ok(canon) = canonicalize_root(cwd()) else {
        return (IndexState::Missing, None, String::new());
    };
    let path = db_path_for(&canon.path_str);
    if !path.exists() {
        return (IndexState::Missing, None, String::new());
    }
    let Ok(db) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return (IndexState::Missing, None, String::new());
    };
    let Ok(status) = status_of(&db, &canon.path) else {
        return (IndexState::Missing, None, String::new());
    };
    // An empty index is not an index. A db file exists as soon as anything opens the project, and
    // the hook once told an agent `cort has an index` on a tree where `impact` could only answer
    // `no_seed_resolved` -- `rust/tests/cli.rs:433` pins that it must not.
    if !status.indexed {
        return (IndexState::Missing, None, String::new());
    }
    let state = match (status.git_head.as_deref(), git_head_quickly(&canon.path)) {
        (Some(stored), Some(now)) if stored != now => IndexState::BehindHead,
        _ => IndexState::HeadMatches,
    };
    (state, Some(db), project_id_for(&canon.path_str))
}

fn index_state() -> IndexState {
    probe_index().0
}
```

`index_state`'s existing doc comment moves to `probe_index` and keeps every word about why only a
definite disagreement is called stale.

In `cmd_hook_suggest`, replace the `judge` call and the index gate:

```rust
    usage.args_summary = harness_args("no_shape");
    let Some(search) = search_of_payload(&v) else {
        return quiet();
    };
    // The probe runs inside the closure and nowhere else. A search rejected on shape therefore pays
    // for no canonicalize, no database open, no `status_of` and no `git rev-parse` -- and the shape
    // gate turns down about 95% of searches against a 5s budget. The `Cell` carries the freshness
    // half back out, so the fire path probes once rather than twice.
    let observed: std::cell::Cell<Option<IndexState>> = std::cell::Cell::new(None);
    let hit = match cort::hook::judge(&search, |symbol| {
        let (state, db, project_id) = probe_index();
        observed.set(Some(state));
        match db {
            None => cort::hook::Evidence::NoIndex,
            // A storage failure fires. A hook that goes quiet because a disk hiccuped would be
            // silently narrowing the product on a transient.
            Some(db) => cort::hook::evidence_in(&db, &project_id, symbol)
                .unwrap_or(cort::hook::Evidence::Unknown),
        }
    }) {
        cort::hook::Verdict::Fire(hit) => hit,
        cort::hook::Verdict::Silent(reason) => {
            usage.args_summary = harness_args(match reason {
                cort::hook::SilenceReason::NoShape => "no_shape",
                // Unchanged meaning: the rule matched a real call-site search and the gate declined
                // it -- a missed opportunity, which `tests/cli.rs` pins as its own name. An empty
                // index reaches here, not `no_evidence`, because `probe_index` applies the same
                // `status.indexed` test the old gate did.
                cort::hook::SilenceReason::NoIndex => "no_index",
                // New: the rule matched, the project is genuinely indexed, and the index holds
                // neither a seed nor a raw edge naming the symbol. A refusal, not a missed chance.
                cort::hook::SilenceReason::NoEvidence => "no_evidence",
            });
            return quiet();
        }
    };
    // `Fire` is only reachable from inside `match evidence(&symbol)`, so the closure ran and this is
    // `Some`. That coupling is invisible from here: the day someone adds a fast path to `judge` that
    // fires without consulting evidence, this would read `None` and silently downgrade `hit_stale`
    // to `hit` with no test noticing. Assert it rather than trust it.
    debug_assert!(
        observed.get().is_some(),
        "judge fired without consulting the evidence closure"
    );
    let stale = observed.get() == Some(IndexState::BehindHead);
    usage.args_summary = harness_args(if stale { "hit_stale" } else { "hit" });
```

Leave everything after that unchanged.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test`
Expected: PASS across the suite, including `tests/cli.rs`'s existing `no_index` test — its meaning is
unchanged because `NoIndex` now comes from the closure rather than from a pre-`judge` gate.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/main.rs rust/tests/cli.rs
git commit -m "feat(hook): stay silent when the index holds nothing about the symbol

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VZYNBd2gsdwcdZbLi86nzt"
```

---

### Task 4: Rename every shape-only metric, in both eval tools

**Files:**
- Modify: `evals/src/hook.rs` (`:408`, the report object at `:450-462`, the `reading` text)
- Modify: `evals/src/adopt.rs` (the field its `s_fire` counter feeds, around `:660`)
- Test: `evals/tests/hook.rs`, **and `evals/tests/adopt.rs:138`, `:245`, `:365`** — all three assert
  `r["rule_would_fire"] == json!(1)`. That is serde `Value` indexing, so it compiles against the
  renamed report and fails at *runtime* with `Null != 1`; without updating them this task cannot
  reach its own "Expected: PASS".

**Interfaces:**
- Consumes: `Verdict`, `Evidence`, `suggests_impact_shape` (Task 1).
- Produces: `hook-probe-v3` whose fire family is named `shape_fired*`, and an `adopt-mine` whose
  `rule_would_fire` becomes `shape_would_fire`.

**Why the rename rather than an alias.** After Task 1 these numbers count searches that pass the
shape gate, not searches the shipped hook would act on — the evidence half is not replayable, because
the index state at the time of each historical fire is not recoverable (spec §10). Keeping `fired`
would leave the report contradicting its own `reading`. A repository search found no machine consumer
of the JSON key; README discusses fires in prose.

- [ ] **Step 1: Write the failing test**

Append to `evals/tests/hook.rs`:

```rust
/// The probe replays the shape half and its field names have to say so, because the evidence half is
/// not replayable: the index state at the time of each historical fire is not recoverable.
#[test]
fn the_probe_names_its_numbers_shape_only_and_says_why() {
    let report = probe(&[], 0);
    assert_eq!(report["method"].as_str(), Some("hook-probe-v3"));
    for key in [
        "shape_fired",
        "shape_fired_shell",
        "shape_fired_structured",
        "shape_fire_rate_of_searches",
    ] {
        assert!(report.get(key).is_some(), "missing {key}: {report}");
    }
    assert!(
        report.get("fired").is_none(),
        "the old full-verdict name must not survive: {report}"
    );
    let reading = report["index_check_reading"].as_str().unwrap();
    assert!(
        reading.contains("not replayable") || reading.contains("cannot be replayed"),
        "the report must disclaim what it cannot know: {reading}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd evals && cargo test --test hook the_probe_names_its_numbers_shape_only`
Expected: FAIL — `method` is `hook-probe-v2` and `shape_fired` does not exist.

- [ ] **Step 3: Write minimal implementation**

In `evals/src/hook.rs`:

- At `:408`, `match judge(&search, |_| cort::hook::Evidence::Unknown) { cort::hook::Verdict::Fire(hit) => { …existing body… }, cort::hook::Verdict::Silent(_) => {} }`.
- Bump `"method"` to `"hook-probe-v3"`.
- Rename `fired` → `shape_fired`, `fired_shell` → `shape_fired_shell`, `fired_structured` →
  `shape_fired_structured`, and all three `fire_rate_*` → `shape_fire_rate_*`. Rename the local
  bindings to match so the code reads the same as the report.
- Append to `index_check_reading`:

```
This report replays the SHAPE half of the verdict only. Since 2026-09-04 the shipped rule also asks
the project's index whether it holds a seed or a raw edge naming the symbol, and that half cannot be
replayed: the index state at the time of each historical fire is not recoverable, so every row here
is judged with Evidence::Unknown, which fires. `shape_fired` is therefore comparable with runs from
before the predicate widened, and is an upper bound on what ships today. To measure the evidence
half, read the `no_evidence` outcome in usage.db from a live deployment.
```

In `evals/src/adopt.rs` the call site was already renamed in Task 1; here rename the report field
`rule_would_fire` → `shape_would_fire` and update that report's `reading` text to say the same thing
in one sentence. Then update the three assertions in `evals/tests/adopt.rs` (`:138`, `:245`, `:365`)
to the new key — they are the only consumers of it anywhere, in either crate or in any document.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd evals && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path evals/Cargo.toml
cargo clippy --manifest-path evals/Cargo.toml --all-targets -- -D warnings
git add evals/src/hook.rs evals/src/adopt.rs evals/tests/hook.rs
git commit -m "feat(evals): the fire counts are shape-only now, and are named for it

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VZYNBd2gsdwcdZbLi86nzt"
```

---

### Task 5: Measure the shipped predicate, then write down what is true

**Files:**
- Modify: `docs/2026-09-04-hook-precision-and-the-seed-gate.md` (§10)
- Modify: `README.md` (the hook section and its outcome vocabulary)

**Interfaces:**
- Consumes: Tasks 1-4, all committed and deployed.

**How this is measured, given the probe cannot.** Task 4 makes `hook-probe` structurally unable to
report `no_evidence`. The predicate is therefore measured two ways, both through the shipped binary
and neither through a script:

1. **Directly.** `cort hook-suggest` in a real project, per case, asserting silence or a suggestion.
2. **In aggregate.** `cort usage` after a period of live use, reading the `no_evidence` count beside
   `hit`, `hit_stale`, `no_index` and `no_shape`.

**This task may fail the plan.** Stop and report rather than documenting a regression if any
pre-existing hook test needed weakening, or if the deletion-case test in Task 3 does not pass.

- [ ] **Step 1: Record the shape number, before and after, back to back**

```bash
cd /home/yanggf/a/cortexyoung/evals && cargo build --release
./target/release/cort-evals hook-probe --examples 0 > /tmp/after.json
```
`hook-probe` has no `--since` and reads whatever transcripts are on disk, so if a before-number is
wanted it must be produced from a pre-Task-4 binary **in the same sitting** (spec §8). The shape
count should be unchanged — Task 4 renamed it, it did not change what it counts.

- [ ] **Step 2: Deploy**

```bash
./install.sh && ./install.sh --check
```

- [ ] **Step 3: Confirm the three shipped behaviours by hand**

In this repository, which has an index:

```bash
echo '{"tool_input":{"command":"grep -rn '"'"'tide'"'"' rust/src/"}}' | cort hook-suggest
echo '{"tool_input":{"command":"grep -rn '"'"'compose_symbol_name'"'"' rust/src/"}}' | cort hook-suggest
cort usage 1 | head -40
```
Expected: the first prints `{}`; the second prints a suggestion naming `compose_symbol_name`; the
usage report shows a `no_evidence` row beside the existing outcomes.

- [ ] **Step 4: Update §10 and the README**

In `docs/2026-09-04-hook-precision-and-the-seed-gate.md` §10, change exit D from "選定的方向" to
shipped, and state what was measured after the fact rather than predicted. Keep the 36/21/53
prediction beside it and say whether live use bore it out or not yet.

In `README.md`, state the predicate in one sentence in the hook section — the hook suggests `impact`
only when the project's index holds a seed or a raw edge naming the symbol — and add `no_evidence` to
the outcome vocabulary the README already enumerates, beside `no_shape`, `no_index`, `hit` and
`hit_stale`.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
git add docs/2026-09-04-hook-precision-and-the-seed-gate.md README.md
git commit -m "docs: the seed-or-edge predicate, as shipped

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VZYNBd2gsdwcdZbLi86nzt"
```

---

## Self-Review

**Spec coverage.** §10 exit D → Tasks 1-3. §10's new constraint (historical replay cannot know the
index state) → Task 4's rename and disclaimer. §9 (a zero-seed coverage screen is not deletion
verification) → nothing to build; it is why the predicate is seed-or-edge. §6 (the single-verdict
dilemma) → dissolved: the predicate lives in `judge`, and `suggests_impact_shape` is a named
shape-only seam rather than a second decision. All nine review-record rows map to a task.

**Placeholder scan.** Two named soft spots, both deliberate: Task 3 says to read `sandbox()`'s
`SAMPLE` fixture before writing the second test, in case it already provides a `helper`; Task 5's
README edit is described rather than pasted because it is prose into a section whose current wording
must be read at the time. Every other step carries its code.

**Type consistency.** `Evidence::{Seed, RawOnly, Neither, NoIndex, Unknown}`,
`SilenceReason::{NoShape, NoIndex, NoEvidence}` and `Verdict::{Fire, Silent}` are used identically in
Tasks 1-4. `evidence_in(db, project_id, symbol) -> rusqlite::Result<Evidence>` is defined in Task 2
and consumed by `evidence_at_cwd` in Task 3. `suggests_impact_shape` is renamed in Task 1 and used in
Task 4.

**Known risk, stated.** The `raw_edges` half matches by leaf name, so a project containing any call
to some other module's `SettingsError` gives `RawOnly` for a bare `SettingsError` search. That
looseness is deliberate — it is what keeps deletion verification firing — and it means bucket B
(14 of 110) is a **floor** on remaining empty answers, not a defect to tighten later without
re-measuring. The second query is also a project-wide scan (~2.95ms at 15,850 rows); if a much larger
repository makes that matter, the fix is a stored `raw_target_leaf` column with its own index, which
keeps the loose semantics while making them seekable.
