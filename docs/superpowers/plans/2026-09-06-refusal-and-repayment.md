# Refusal and Repayment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the edit hook from silently performing migrations it cannot finish, without trading a loud treadmill for a quiet wrong answer.

**Architecture:** Version drift becomes a cause of `index_is_stale`, so every `impact`, `context`, `struct` and `status` answer discloses it beside the rows it prints. Only then does `incremental_index` gain a caller-supplied policy that lets the `PostToolUse` hook refuse a full rebuild, and the hook stops running schema migrations on the way in.

**Tech Stack:** Rust (`rust/` crate), rusqlite, SQLite.

**Spec:** `docs/superpowers/specs/2026-09-06-cort-upgrade-design.md` (§4, plus §10 item 1)

**Plan 2 of 3.** Plan 1 (`2026-09-06-index-drift-diagnosis.md`) shipped the diagnosis and is merged. Plan 3 is the upgrader itself.

## Global Constraints

- Repo is pure Rust; the only executable Bash is `install.sh` and `tests/install-smoke.sh`.
- Run `cargo fmt --all`, then `cargo clippy --all-targets -- -D warnings`, then `cargo test --locked --all-targets` in **both** `rust/` and `evals/` before every commit, and let a non-zero exit stop the commit — never end a verification pipeline in `tail`, which swallows the status.
- Storage failures are returned, never panicked on.
- `hook-refresh` is silent, exits 0 whatever happens, and gives up rather than wait.
- No absolute developer paths anywhere, including fixtures.
- Every task's Step 5 deliberately breaks the implementation and confirms the new test goes red. Three external review rounds on plan 1 each found a test that could not detect its own regression, and one of plan 1's nine breaks failed to go red and thereby exposed an unguarded check. This step is where the value is.

## A deliberate divergence from the spec, stated up front

Spec §4 proposes extending `graph_pending` into "a reason set with target generations", where the
upgrader records `extractor_changed` / `schema_changed` and only a full rebuild for the recorded
target may clear them.

**This plan does not add that record, and the reviewer should attack this decision first.**

The debt is not a claim that needs storing. It is a *comparison* between two facts that are already
durable in the database — `_cortex_meta`'s `SCHEMA_VERSION` and `extractor_version`, which plan 1
already reads (`rust/src/db.rs`) — and the binary's own current values. Deriving it has three
properties a stored target does not:

* **Nothing can clear it wrongly.** The deleted `repair_owed` design failed because a clean
  incremental could clear an announcement without repaying the debt
  (`docs/2026-09-05-hook-refresh-follows-the-file.md` §7). A derived condition has nothing to clear:
  it stops being true exactly when a successful `full_index` stamps the new version in the same
  transaction that replaces the rows (`rust/src/indexer.rs:436-437`).
* **A third fact can disagree with the other two.** A stored target is a third copy of a fact this
  database already holds twice — and §10 item 2 already records that `extractor_version` living in
  both `projects` and `_cortex_meta` is an anti-drift violation to collapse, not to add to.
* **Re-derivation is the behaviour we want.** If the extractor changes again mid-debt, the debt
  should re-derive against the *new* current, not against a stale recorded target.

`graph_pending` stays exactly as it is. It is not a version comparison — it records that a commit
changed chunks without rebuilding the derived graph (`rust/src/incremental.rs:200`, `:261`,
`rust/src/db.rs:322`) — so it is a genuine stored fact with no other source, and it already has the
paired semantics this plan needs (`rust/src/staleness.rs:95`, `rust/src/incremental.rs:323`).

If a reviewer can name a state that derivation gets wrong and a stored target gets right, that is a
blocker and this section is where to aim.

**Reviewed 2026-09-06, and the divergence stands — but the review found the real defect underneath
it.** Three states were offered and checked one by one:

* *A `full_index` against a partial pack stamps a plausible-but-wrong identity, and derivation then
  says fresh.* True, but narrow: a stored target only helps if the target was recorded while the
  pack was whole **and** the rebuild ran while it was not. The actual fix is plan 3's staging
  validation, which refuses to activate an incomplete pack (spec §3a) — and until then derivation
  self-heals the moment the pack is whole again, because current then stops equalling stored.
* *Two binaries of different versions against one index.* Refuted. The old binary stamps `old` and
  calls itself fresh, which is correct for it; the new binary still reads `stored=old` against
  `current=new` and still reports drift. No information is lost.
* *Migrated by `ensure_schema` and never re-extracted.* The reviewer's own conclusion is that "the
  present full-rebuild fallback still protects the data" — `ensure_schema` sets `graph_pending`
  (`db.rs:322`), which is a stored fact with no other source and which this plan keeps. What is lost
  is the *name* of the cause, not the protection.

**What the review did find, by aiming at the divergence, is that this plan had already made the very
mistake the divergence claims to avoid:** Task 1 computed one reason set and Task 2 hand-wrote a
second, and the two had already disagreed — Task 2's gate omitted `SCHEMA_VERSION` while Task 3
claimed to rely on it. Two copies of one decision is the sin this repository keeps losing to
(`HOOK_TARGETS`), and it does not need stored state to happen.

So: **derivation stays, and the decision gets exactly one home.** `rebuild_reasons` below is that
home. Spec §4's real requirement was "one shared requirement reader supplies both `index_is_stale`
visibility and the full-rebuild decision" — that is satisfied by a shared function, not by a
persisted target.

---

## File Structure

- `rust/src/staleness.rs` — the one place that decides `index_is_stale`. Gains the version
  comparison and the reasons behind it.
- `rust/src/incremental.rs` — the one place that decides whether a full rebuild happens. Gains the
  caller-supplied policy and a typed refusal.
- `rust/src/main.rs` — the two call sites declare their policy; `hook-refresh` stops migrating.
- `rust/tests/staleness.rs`, `rust/tests/db.rs`, `rust/tests/cli.rs` — tests.

---

### Task 1: version drift becomes staleness, so a refusal can never be silent

**Files:**
- Modify: `rust/src/staleness.rs:13-17` and `:92-101`
- Test: `rust/tests/staleness.rs`

**Interfaces:**
- Consumes: `cort::db::get_meta`, `cort::db::SCHEMA_VERSION`, `cort::pack::extractor_version`.
- Produces: `StaleReport` gains `pub rebuild_required: Vec<String>`, holding zero or more of the
  literals `"extractor_changed"`, `"schema_changed"`, `"graph_incomplete"`. `index_is_stale` is true
  whenever that vector is non-empty. Task 2 reads the same reasons; Task 3 reports them.

**Why this is Task 1 and not Task 2.** Refusing the rebuild before the refusal is visible would
trade a loud treadmill for a silent wrong answer: `compute_stale` is git- and content-based
(`rust/src/staleness.rs:95-98`), and the 2026-09-05 incident moved no git head in any project tree,
so a drifted index would answer `impact` with `index_is_stale: false` on every row while the only
repair path was disabled. Order matters here, and this is why.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/staleness.rs`. **That file already exists** and already has what these tests
need: `setup(SAMPLE)` builds a real index with `full_index` and returns
`(dir, root, db, project_id, bin)` (`rust/tests/staleness.rs:80-84`). Use it — an index built by
this binary is the only honest starting point for "and then the extractor changed", and the
signature is `compute_stale(&db, &bin, &root, &project_id)`, in that order.

```rust
/// The 2026-09-05 shape: the tree is clean, the git head has not moved, and every file hash
/// matches -- but the index was built by an extractor this binary no longer uses. Until now that
/// read as fresh, which is what let seven projects answer `impact` with `index_is_stale: false`
/// while their rows were computed by superseded semantics.
#[test]
fn an_index_built_by_another_extractor_is_stale() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();

    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(s.index_is_stale, "a superseded extractor is staleness: {s:?}");
    assert!(
        s.rebuild_required.iter().any(|r| r == "extractor_changed"),
        "the reason is named, not merely implied: {s:?}"
    );
    assert!(
        s.changed_files.is_empty() && s.deleted_files.is_empty(),
        "nothing in the tree moved -- this is the case the old check called fresh: {s:?}"
    );
}

/// The schema axis is independent: an index at a superseded schema is stale even when its extractor
/// is current. A `rebuild_required` computed from the extractor alone passes the test above and
/// fails this one.
#[test]
fn an_index_at_an_older_schema_is_stale_independently() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    cort::db::set_meta(&db, "SCHEMA_VERSION", "3").unwrap();

    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(s.index_is_stale, "{s:?}");
    assert!(
        s.rebuild_required.iter().any(|r| r == "schema_changed"),
        "{s:?}"
    );
    assert!(
        !s.rebuild_required.iter().any(|r| r == "extractor_changed"),
        "the extractor was untouched, so only one axis may fire: {s:?}"
    );
}

/// And the healthy answer. Plan 1 shipped a verdict word that no test required the implementation
/// to be able to produce, and an implementation that could never produce it passed everything; that
/// mistake is not repeated here.
#[test]
fn a_freshly_indexed_tree_owes_no_rebuild() {
    let (_dir, root, db, project_id, bin) = setup(SAMPLE);
    let s = compute_stale(&db, &bin, &root, &project_id).unwrap();
    assert!(
        s.rebuild_required.is_empty(),
        "an index this binary just built owes nothing: {s:?}"
    );
}
```

The snippets call `cort::db::set_meta` fully qualified, so **do not** add it to the file's
`use cort::db::{...}` list (`rust/tests/staleness.rs:4`) — an unused import fails the `-D warnings`
gate this plan requires.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test staleness -- an_index_built_by_another an_index_at_an_older a_freshly_indexed_tree_owes`
Expected: FAIL to compile — `no field 'rebuild_required' on type 'StaleReport'`.

- [ ] **Step 3: Write minimal implementation**

**Do not memoize `extractor_version`.** An earlier draft of this plan added a `OnceLock` cache,
reasoning that Task 1 puts the pack hash on a path `impact`, `context`, `struct` and `status` all
reach. Two things killed it, and both are worth stating so nobody re-adds it:

* **It is wrong, not merely unnecessary.** `extractor_version()` reads `CORT_PACK_DIR` and the pack
  files' bytes, and both change within a single test process:
  `extractor_version_changes_when_any_pack_file_changes` (`rust/tests/pack.rs:37`) edits a pack file
  and asserts the value moves, and `rust/tests/pack.rs:368` overrides the pack directory. Caching
  makes the answer depend on which test populated the cache first — non-deterministic, in a
  function whose whole job is to be an identity.
* **The cost it was avoiding is 0.049 ms.** Measured on this machine: 6 files, 36 KB total. Against
  a command that already opens a database and shells out to git, that is not a cost.

Leave `rust/src/pack.rs` alone.

Then, in `rust/src/staleness.rs`, extend the struct:

```rust
pub struct StaleReport {
    pub index_is_stale: bool,
    pub deleted_files: Vec<String>,
    pub changed_files: Vec<String>,
    /// Why a full rebuild is owed, if one is: zero or more of `extractor_changed`,
    /// `schema_changed`, `graph_incomplete`. Derived, never stored -- the two version facts are
    /// already durable in `_cortex_meta`, and a third copy could disagree with them.
    pub rebuild_required: Vec<String>,
}
```

Then add the **one home for the decision**, above `compute_stale` in the same file. Both the
visibility path and the refusal path call this and nothing else; a second hand-written copy in
`incremental.rs` is how Task 2 and Task 3 disagreed with each other in the first draft of this plan:

```rust
/// Why a full re-extraction is owed, if one is. The single reader behind both halves of the
/// contract: `compute_stale` turns it into `index_is_stale`, and `incremental_index` turns it into
/// a rebuild or a refusal. Two copies of this decision is the mistake `HOOK_TARGETS` cost this
/// repository once already.
///
/// Derived, never stored. The two version facts are already durable in `_cortex_meta`, and a
/// recorded target would be a third copy that can disagree with them; nothing here can be cleared
/// wrongly because there is nothing to clear -- a reason stops holding exactly when `full_index`
/// stamps the new value in the transaction that replaces the rows (`indexer.rs:436-438`).
///
/// A **missing** key counts as owed: an index that predates the meta table is not a current one.
pub fn rebuild_reasons(db: &Db) -> Result<Vec<String>, IndexError> {
    let mut reasons = Vec::new();
    if get_meta(db, "graph_pending")?.as_deref() == Some("1") {
        reasons.push("graph_incomplete".to_string());
    }
    if get_meta(db, "extractor_version")?.as_deref()
        != Some(crate::pack::extractor_version().as_str())
    {
        reasons.push("extractor_changed".to_string());
    }
    if get_meta(db, "SCHEMA_VERSION")?.as_deref()
        != Some(crate::db::SCHEMA_VERSION.to_string().as_str())
    {
        reasons.push("schema_changed".to_string());
    }
    Ok(reasons)
}
```

and replace the `graph_pending` block at `:92-101` with a call to it:

```rust
    // The per-file hash comparison above cannot see a half-rebuilt graph: every file hash can
    // match while cross-file edges are missing. Nor can it see an index built by semantics this
    // binary no longer uses -- on 2026-09-05 no git head had moved in any project tree, so every
    // drifted index read as fresh. Both are staleness, and both must surface through the field
    // agents already check.
    let rebuild_required = rebuild_reasons(db)?;

    Ok(StaleReport {
        index_is_stale: !rebuild_required.is_empty()
            || !deleted.is_empty()
            || !changed_files.is_empty(),
        deleted_files: deleted,
        changed_files,
        rebuild_required,
    })
```

`rebuild_reasons` is written so that a **missing** key counts as owed, which is what plan 1's
`drifted` already does and what the predates-meta population deserves.

Every construction site of `StaleReport` outside this function must gain the field. Find them with
`rg 'StaleReport \{' rust/ evals/` and add `rebuild_required: Vec::new()` where a test builds one
by hand.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test staleness -- an_index_built_by_another an_index_at_an_older a_freshly_indexed_tree_owes`
Expected: all three PASS.

- [ ] **Step 5: Verify each test can actually fail**

Each break restored afterwards. Report the literal assertion message for each.

1. Drop the `extractor_changed` push. Expected: `an_index_built_by_another_extractor_is_stale` RED.
2. Drop the `schema_changed` push. Expected: `an_index_at_an_older_schema_is_stale_independently`
   RED, and the extractor test stays green — if both go red the two axes are not independent.
3. Push `"extractor_changed"` unconditionally. Expected: `a_freshly_indexed_tree_owes_no_rebuild` RED. This
   is the break that matters: without it, an implementation that always claims drift passes the
   first two tests and makes every index permanently stale.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0. Existing tests that assert `index_is_stale: false` on a freshly built index
stay green **because the fixture indexed with this binary** — if one goes red, read it before
touching it: it may be asserting freshness about an index this binary genuinely cannot use.

- [ ] **Step 7: Commit**

```bash
git add rust/src/staleness.rs rust/tests/staleness.rs
git commit -m "feat(staleness): an index built by other semantics is stale

compute_stale compared git heads and file hashes, and on 2026-09-05 neither
had moved: the extractor changed, not the trees. So every drifted index
answered impact with index_is_stale: false while its rows were computed by
superseded semantics.

The two version facts are already durable in _cortex_meta; this derives the
debt from them rather than storing a third copy that could disagree. Nothing
can clear it wrongly because there is nothing to clear -- it stops being true
when full_index stamps the new version in the transaction that replaces the
rows.

rebuild_reasons is the one home for the decision: compute_stale turns it into
index_is_stale and incremental_index turns it into a rebuild or a refusal.
Two copies is the mistake HOOK_TARGETS cost this repository once already, and
the first draft of this plan had already made it -- its two copies disagreed
about the schema axis."
```

---

### Task 2: the caller decides whether a full rebuild may happen

**Files:**
- Modify: `rust/src/incremental.rs:301-335`, `rust/src/main.rs:840` and `:1509`
- Test: `rust/tests/db.rs`, `rust/tests/cli.rs`

**Interfaces:**
- Consumes: Task 1's reasons.
- Produces: `pub enum RebuildPolicy { Allow, Forbid }`; `incremental_index(db, bin, root, policy)`;
  and `IndexError::FullRebuildRequired { reasons: Vec<String> }` returned when `Forbid` meets a
  trigger. Task 3 maps that error to a hook outcome.

**All three triggers, not one.** `incremental_index` falls through to `full_index` on an extractor
mismatch (`incremental.rs:312-318`), on `graph_pending` (`:323-326`), and on a git candidate set it
could not narrow (`:330-333`). Every one of them can exceed the five-second hook budget
(`TIMEOUT_SECS`, shared across the three dialects in `settings.rs`, `settings_toml.rs`,
`settings_kimi.rs`), so the policy must cover all three. A version that refuses only the extractor
mismatch leaves the same treadmill running on the other two.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`:

```rust
/// The treadmill, as a test. An index whose extractor is superseded used to make every edit hook
/// attempt a full rebuild inside a five-second budget; killed before commit, it advanced nothing
/// and tried again on the next edit, forever and silently.
#[test]
fn the_refresh_hook_refuses_a_full_rebuild_and_says_so() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 11; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(
        &[],
        serde_json::from_str(&payload).unwrap(),
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);
    let counts = refresh_outcomes(&cache);
    assert_eq!(
        counts.get("rebuild_required").and_then(Value::as_i64),
        Some(1),
        "the hook records the debt instead of attempting it: {counts:?}"
    );

    // And the stored extractor is untouched: a refusal must not look like a repair.
    let db = cort::db::open_db(&db_file).unwrap();
    assert_eq!(
        cort::db::get_meta(&db, "extractor_version").unwrap().as_deref(),
        Some("not-the-one-that-ships"),
        "refusing must not stamp a version it did not build"
    );
}

/// The foreground keeps its rebuild. `cort index --incremental` is a typed command a person ran on
/// purpose; refusing it would leave the debt with no actor at all, which this repository has
/// measured as worthless (19 re-index runs against 2,700+ hook fires).
#[test]
fn a_foreground_incremental_still_rebuilds() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);

    let r = run_cort(&["index", "--incremental"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let db = cort::db::open_db(&db_file).unwrap();
    assert_eq!(
        cort::db::get_meta(&db, "extractor_version").unwrap(),
        Some(cort::pack::extractor_version()),
        "the foreground path repaid the debt"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test cli -- the_refresh_hook_refuses a_foreground_incremental_still`
Expected: `the_refresh_hook_refuses_a_full_rebuild_and_says_so` FAILS with the outcome map holding
**`already_current`**, not `refreshed` and not `rebuild_required`.

That is worth pausing on, because it is how invisible the treadmill actually is: `from_full` reports
`files_reindexed: 0` (`incremental.rs:292`), and the hook's success arm requires
`files_reindexed > 0 || files_removed > 0` (`main.rs:841`), so a full rebuild that the hook triggered
**and completed** is recorded as `already_current` — byte for byte the same row as "nothing to do".

- [ ] **Step 3: Write minimal implementation**

In `rust/src/incremental.rs`:

```rust
/// Whether this call site may spend a full re-extraction. The edit hook may not: it runs under a
/// five-second harness budget, and a rebuild killed before commit advances nothing and is attempted
/// again on the next edit. The decision belongs to the caller, which knows its own budget, rather
/// than to a second copy of the policy in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildPolicy {
    Allow,
    Forbid,
}
```

Add a variant to `IndexError` (`rust/src/indexer.rs:49`). **Two matches enumerate its variants
exhaustively and both break without a new arm** — `Display` at `rust/src/indexer.rs:55` and
`map_index` at `rust/src/main.rs:78`:

```rust
    /// A full rebuild is owed and this call site may not spend one. Carries the reasons so the
    /// caller can record which debt it declined, rather than an anonymous refusal.
    FullRebuildRequired { reasons: Vec<String> },
```

`Display` (`rust/src/indexer.rs:55`):

```rust
            Self::FullRebuildRequired { reasons } => {
                write!(f, "full rebuild required: {}", reasons.join(", "))
            }
```

`map_index` (`rust/src/main.rs:78`) — this is a refusal, not a storage failure, so it keeps its own
code rather than being folded into `file_not_found` or `classify_sqlite`:

```rust
        IndexError::FullRebuildRequired { reasons } => CortError::new(
            "full_rebuild_required",
            json!({ "reasons": reasons }),
        ),
```

Change the signature to
`pub fn incremental_index(db: &mut Db, bin: &str, root: impl AsRef<Path>, policy: RebuildPolicy)`
and replace the three fall-through sites with one gate. Compute the reasons once, from the same
comparison Task 1 uses:

```rust
    // The stored-version reasons come from the one reader, so this cannot disagree with what
    // `index_is_stale` says about the same database.
    let reasons = crate::staleness::rebuild_reasons(db)?;
    if !reasons.is_empty() {
        if policy == RebuildPolicy::Forbid {
            return Err(IndexError::FullRebuildRequired { reasons });
        }
        eprintln!("full reindex required: {}", reasons.join(", "));
        let full = full_index(db, bin, &canon.path)?;
        return Ok(from_full(full, started));
    }

    // Git stays *below* the stored-version gate, exactly where the two original gates left it. The
    // extractor and `graph_pending` checks short-circuit before any subprocess runs today
    // (`incremental.rs:312`, `:323`); hoisting `git_candidates` above them would spend up to four
    // untimed subprocesses out of a five-second hook budget on a refusal already decided.
    let indexed_head = crate::db::indexed_head(db, &canon.project_id)?;
    let cands = git_candidates(&canon.path, indexed_head.as_deref());
    if !cands.narrowed {
        if policy == RebuildPolicy::Forbid {
            return Err(IndexError::FullRebuildRequired {
                reasons: vec!["candidates_not_narrowed".to_string()],
            });
        }
        let full = full_index(db, bin, &canon.path)?;
        return Ok(from_full(full, started));
    }
```

The rest of the function is unchanged — it already continues from `cands`.

**Delete the two now-unused locals** at the top of the function (`incremental.rs:309-310`):

```rust
    let version = crate::pack::extractor_version();
    let stored = get_meta(db, "extractor_version")?;
```

`rebuild_reasons` makes both comparisons now, and leaving them behind fails the `-D warnings` gate
this plan requires. Check whether `get_meta` and `crate::pack` are still used elsewhere in the file
before removing any `use`.
```

Note the original code only compared the extractor when a stored value existed
(`if let Some(stored)`); this compares unconditionally, so an index with no stored extractor is a
rebuild rather than a silent pass. That matches Task 1's staleness and closes the divergence
recorded in `rust/src/db.rs`'s `extractor_version` doc comment.

In `rust/src/main.rs`, add `RebuildPolicy` to the `use cort::incremental::{...}` list at
`main.rs:13` — it currently names only `incremental_index`, so both call sites below would not
compile. Then the two call sites declare themselves:

```rust
    match incremental_index(&mut db, &bin, &canon.path, RebuildPolicy::Forbid) {   // main.rs:840, hook-refresh
```
```rust
                incremental_index(&mut db, &bin, &canon.path, RebuildPolicy::Allow).map_err(IdxWrap)   // main.rs:1509, cmd_index
```

Task 3 handles the hook's new error arm; for this task, add it to the existing `match` so the tree
compiles and the outcome is recorded:

```rust
        Err(cort::indexer::IndexError::FullRebuildRequired { reasons }) => {
            let mut row = hook_row(
                "rebuild_required",
                &harness,
                declared_differs.as_deref(),
                model.as_deref(),
            );
            row["reasons"] = json!(reasons);
            usage.args_summary = row.to_string();
            Ok(Emit {
                payload: json!({}),
                format: Format::Lean,
                render_command: Some("hook-refresh"),
            })
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test cli -- the_refresh_hook_refuses a_foreground_incremental_still`
Expected: both PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Pass `RebuildPolicy::Allow` at the hook call site. Expected:
   `the_refresh_hook_refuses_a_full_rebuild_and_says_so` RED with outcome `refreshed`.
2. Gate only the extractor reason (leave `graph_pending` and `candidates_not_narrowed` rebuilding
   under `Forbid`). Expected: **both tests stay green.** That is a gap, not a pass — add a third
   test that sets `graph_pending` to `1` with a current extractor and asserts the hook still
   refuses, then confirm break 2 turns *that* one red.
3. Return `FullRebuildRequired` regardless of policy. Expected: `a_foreground_incremental_still_rebuilds`
   RED — without it, a refusal that also blocks the foreground leaves the debt with no actor.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0.

**Changing `incremental_index`'s signature breaks nineteen call sites, not two.** The two production
callers are `main.rs:840` and `main.rs:1509`; the rest are tests, and they must each gain
`RebuildPolicy::Allow` — they are foreground fixtures asserting that indexing happens:

- `rust/tests/context.rs:987`
- `rust/tests/hook.rs:464`
- `rust/tests/incremental.rs` at 94, 106, 120, 149, 174, 191, 251, 289, 346, 390, 448, 677, 718, 730, 748, 838, 887

There are **zero callers in `evals/`** — it depends on the crate but imports hook APIs, not indexing
(`evals/Cargo.toml:14`, `evals/src/hook.rs:19`). Run the evals suite anyway; that is the constraint,
and a green run there is evidence rather than ceremony.

- [ ] **Step 7: Commit**

```bash
git add rust/src/indexer.rs rust/src/incremental.rs rust/src/main.rs rust/tests/cli.rs
git commit -m "feat(incremental): the caller decides whether a full rebuild may happen

incremental_index fell through to full_index on three separate triggers, and
the caller reaching all three was a PostToolUse hook with a five-second
budget. Killed before commit it advanced nothing and tried again on the next
edit -- measured as 1.4-3.3s per edit against a 23-37ms baseline, invisibly.

RebuildPolicy is passed by the call site rather than inferred, so there is no
second copy of the policy inside incremental_index. The hook forbids and
records which debt it declined; the foreground allows, because a debt with no
actor is worth nothing -- 19 re-index runs against 2,700+ hook fires is this
repository's own measurement of that.

The extractor comparison is now unconditional: an index with no stored
extractor owes a rebuild rather than passing silently."
```

---

### Task 3: the edit hook stops being the structural migrator

**Files:**
- Modify: `rust/src/main.rs:133-143` (`open_project_tracked`) and the `hook-refresh` open at `:837`
- Test: `rust/tests/cli.rs`

**Interfaces:**
- Consumes: Task 2's hook outcome shape.
- Produces: `open_project_unmigrated(root, usage) -> Result<(CanonicalRoot, Db), CortError>`, which
  opens the database and **does not migrate it**. It makes no compatibility decision of its own —
  that belongs to `rebuild_reasons`, which `incremental_index` already consults under
  `RebuildPolicy::Forbid`, so a behind schema comes back as `rebuild_required` rather than as a
  silent migration. The name says what it does not do, because an earlier draft called it
  `open_project_readable` and claimed it "checks the schema", which it never did.
  `open_project_tracked` keeps its migrating behaviour for the foreground.

**Why.** `open_project_tracked` calls `ensure_schema` unconditionally (`main.rs:141`), and
`ensure_schema` performs migrations and sets `graph_pending` (`db.rs:304-323`). `hook-refresh`
reaches it before `incremental_index` (`main.rs:837`), so Task 2 stops the hook performing
*extractor* rebuilds while it remains the *schema* migrator — running a table rebuild inside the
same five-second budget, on a database another process may be reading.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`:

```rust
/// A schema migration is a table rebuild. The edit hook has a five-second budget and no way to
/// report failure, so it must not be the thing that performs one -- it records the debt and leaves
/// the database exactly as it found it.
#[test]
fn the_refresh_hook_does_not_migrate_the_schema() {
    let (p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "SCHEMA_VERSION", "4").unwrap();
    drop(db);
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 13; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(
        &[],
        serde_json::from_str(&payload).unwrap(),
        &cwd,
        &cache,
    );
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);

    let db = cort::db::open_db(&db_file).unwrap();
    assert_eq!(
        cort::db::get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some("4"),
        "the hook migrated a schema it has no budget to migrate"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test cli -- the_refresh_hook_does_not_migrate`
Expected: FAIL — the stored version is `5`, because opening migrated it.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/main.rs`, add beside `open_project_tracked`:

```rust
/// Open a project without migrating it.
///
/// `open_project_tracked` calls `ensure_schema`, which migrates and sets `graph_pending`
/// (`db.rs:304-323`). That is right for a command a person ran and wrong for a hook: a migration is
/// a table rebuild, the hook has a five-second budget, and its contract is to be silent and exit 0
/// -- so it cannot report a migration it could not finish. It reads the stored version instead and
/// leaves the database as it found it.
fn open_project_unmigrated(
    root: &Path,
    usage: &mut UsageEvent,
) -> Result<(CanonicalRoot, Db), CortError> {
    match canonicalize_root(root) {
        Ok(canon) => {
            usage.project_id = Some(canon.project_id.clone());
            let db =
                open_db(db_path_for(&canon.path_str)).map_err(|e| cort::db::classify_sqlite(&e))?;
            Ok((canon, db))
        }
        Err(e) => {
            usage.project_id = Some("_unknown".into());
            Err(map_index(e))
        }
    }
}
```

The `Err` arm is copied verbatim from `open_project_tracked` (`main.rs:145-148`) rather than
invented: it records `_unknown` for the usage row and calls `map_index`, and both are depended on.

Change the `hook-refresh` open at `main.rs:837`:

```rust
    let Ok((canon, mut db)) = open_project_unmigrated(&root, usage) else {
        return quiet("db_unavailable", usage);
    };
```

`incremental_index` under `Forbid` already refuses when the schema is behind, because Task 1's
`schema_changed` reason is one of the triggers Task 2 gates — so the hook records
`rebuild_required` and no migration happens.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test cli -- the_refresh_hook_does_not_migrate`
Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Point the hook back at `open_project_tracked`. Expected: RED with `Some("5")` vs `Some("4")`.
If it stays green, the fixture is not reaching the migration and must be fixed before continuing.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0. Watch `rust/tests/cli.rs`'s existing hook-refresh tests: they index first, so
their schema is already current and they must stay green without edits.

- [ ] **Step 7: Commit**

```bash
git add rust/src/main.rs rust/tests/cli.rs
git commit -m "fix(hook): the edit hook stops migrating the schema

open_project_tracked calls ensure_schema unconditionally, and ensure_schema
migrates and sets graph_pending. hook-refresh reached it on the way in, so
forbidding extractor rebuilds left the hook performing table rebuilds instead
-- inside a five-second budget, on a database another process may be reading,
with no way to report a migration it could not finish.

open_project_unmigrated opens the database and does not migrate it. It makes
no compatibility decision of its own; the schema debt flows through the same
refusal as every other, because rebuild_reasons already names it and
incremental_index already gates on it under Forbid."
```

---

## Self-Review

**Spec coverage.** Implements spec §4's two load-bearing properties — the refusal is bound to
`index_is_stale` (Task 1) and the hook performs neither rebuild nor migration (Tasks 2, 3) — and
§10 item 1 (the scan connection's missing busy timeout) is **not** in this plan; see below. §4's stored reason-set is deliberately not implemented; the divergence and its argument are
stated at the top for a reviewer to attack.

**Not in this plan:** everything in §3, §5 and §6 (the upgrader itself), §10 items 2, 3 and 4, and
the scan connection's missing busy timeout, which §10 item 1 assigns here but which is a separate
change in `db.rs` and belongs with plan 3's `with_busy_retry` work rather than in the middle of a
policy change.

**Placeholders:** none. Every step carries its code. Task 2 Step 5 break 2 deliberately predicts a
*green* result and requires the implementer to add a test — that is a written instruction, not a gap.

**Type consistency:** `rebuild_required: Vec<String>` (Task 1) and `FullRebuildRequired { reasons:
Vec<String> }` (Task 2) both carry the same literals — `extractor_changed`, `schema_changed`,
`graph_incomplete` — plus `candidates_not_narrowed`, which is Task 2's alone because it is a
property of the git candidate set rather than of the stored index.

## What the review changed

One round of external review on this plan; fourteen defects, all verified against source before
being accepted, plus a refutation of the spec divergence that was checked state by state and
**partly rejected** — see the divergence section above for which of its three states survived.

**The finding that mattered was found by aiming at the divergence and hitting something else.**
Task 1 computed one reason set and Task 2 hand-wrote a second, and the two had already disagreed:
Task 2's gate omitted `SCHEMA_VERSION` while Task 3 claimed to rely on it, so a database at schema 4
with a current extractor would have passed the gate and gone on to write. `rebuild_reasons` now
exists because of that, and it is the change that makes the divergence defensible rather than merely
argued.

**Compile defects, each confirmed by reading the code:** `IndexError` has exactly three variants and
both `Display` (`indexer.rs:55`) and `map_index` (`main.rs:78`) enumerate them exhaustively, so the
new variant needs an arm in each; `incremental.rs` imports `IndexError` privately, so
`cort::incremental::IndexError` does not exist and the path is `cort::indexer::IndexError`;
`RebuildPolicy` needs adding to `main.rs:13`'s import list.

**The memoization was worse than unnecessary.** It would have broken
`extractor_version_changes_when_any_pack_file_changes` (`rust/tests/pack.rs:37`) and made
`rust/tests/pack.rs:368` order-dependent, to avoid a cost measured at 0.049 ms. Removed, with the
measurement recorded so nobody re-adds it.

**Two errors of evidence rather than of code.** The plan predicted the failing test would show
`refreshed`; it shows `already_current`, because `from_full` reports `files_reindexed: 0`
(`incremental.rs:292`) and the hook's success arm requires a positive count (`main.rs:841`). That is
not a typo — it means a full rebuild the hook triggered **and completed** is recorded byte for byte
the same as "nothing to do", which is how the treadmill stayed invisible. And the signature change
touches **nineteen** call sites, not two; all nineteen are now named.

**A defect I reintroduced.** `cargo test --test X a b` is not valid for multiple filters — it needs
`--`. That exact error was found and fixed in plan 1 four days' work ago, and written into plan 1's
own review record, and I wrote it again. A review record in a finished plan is not a control.

**One I made while fixing another.** Rewriting Task 2's gate, I referenced an `incremental_from`
function that does not exist — inventing an API in the middle of removing a duplicated decision. It
was caught by re-reading the patch rather than by any reviewer.
