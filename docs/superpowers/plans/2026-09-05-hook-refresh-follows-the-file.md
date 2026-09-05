# `hook-refresh` Follows the File Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `PostToolUse` repair hook refresh the project that owns the **edited file**, not
whatever project happens to sit at the shell's working directory.

**Architecture:** `cmd_hook_refresh` already reads the payload for two fields; it gains a third. The
edited path resolves to a project by walking up to the nearest directory with a row in `projects`,
falling back to cwd only when the payload carries no path at all. A refusal still records nothing —
see §Deferred for why the mark that would have fixed that is not in this plan.

**Tech Stack:** Rust (`rust/` crate), SQLite.

**Spec:** This document. §Evidence is the measurement; §Design argues from it.

**Revision:** v3, 2026-09-05. v1 was reviewed by Codex (`01a06d4d-fc8c-7632-bd4e-d8324af73217`) and
corroborated against source: **three blockers and six majors**, two of them design errors rather than
wording. v1's Task 4 is **deleted**, not fixed — see §Deferred. §Review record lists all nine.
v2 was then reviewed by Kimi, which found five more — §Review record rows 10-14 — including the one
it was asked for by name: **Task 3's headline test could not tell an implementation that reads the
payload from one that ignores it**, which is the only property this change exists to establish.
**Do not execute v1 or v2.**

---

## Evidence

Reproduced on 2026-09-05, same file, same edit, two working directories:

```
$ echo '{"tool_input":{"file_path":"rust/src/hook.rs"}}' | cort hook-refresh
{}                                       outcome: already_current

$ cd /home/yanggf/a/cortexyoung/rust && echo '{"tool_input":{"file_path":"src/hook.rs"}}' | cort hook-refresh
{}                                       outcome: no_index
```

`/home/yanggf/a/cortexyoung/rust` has no row in `projects` (`cort projects` lists 9, and that is not
one of them), so `index_state()` returns `Missing` and the hook exits silently having repaired
nothing. The payload named the file. The hook did not look.

**What it cost, in this session.** Over four hours of editing this repository, `usage.db` recorded:

| outcome | count |
|---|---:|
| `already_current` | 187 |
| `no_index` | **79** |
| `db_unavailable` | 20 |
| `busy_or_failed` | 16 |
| `refreshed` | 16 |

The 79 `no_index` rows are this bug: an agent that runs `cd rust && cargo test` — which this session
did constantly — takes every subsequent edit's repair with it. The index then drifted **two lines**
behind `rust/src/hook.rs` while `cort status` reported `index_is_stale: false`, `changed_files: []`
and a `git_head` equal to `HEAD`. An `impact` answer read at that moment named line 435 for a
dependent that was at 437; the claim was checkable in one line, which is the only reason it was
caught.

**Why the freshness check did not save it.** `git_candidates` is correct (`incremental.rs:111-130`):
it diffs the dirty tree *and* `indexed_head..HEAD`. But by the time a refresh ran at the repo root
again, the tree was clean and the head had already been stamped by an earlier root-level refresh
whose own diff did not include `hook.rs`. The repair was owed and nothing recorded that. This is the
mechanism `2026-09-03-installer-dedup-and-attribution.md` §9 describes for `git pull`, reached by a
different road: **a candidate set that narrowed to nothing without saying so.**

## Design

### D1 — the project comes from the edited path, and cwd is the fallback

`cmd_hook_refresh`'s payload comment (`rust/src/main.rs:741-745`) states the current decision
plainly: *"Which file changed is `incremental_index`'s question, not ours."* That is true — and it
is only true once the right project is open. Choosing the project is a separate question and the
payload already answers it.

The resolution walks up from the edited file to the nearest ancestor with a row in `projects`. Not
"the git root", because a repository may hold several indexed projects (this machine has
`/home/yanggf/b/finance-engineering` and `/home/yanggf/b/finance-engineering/tools/finance-cli`, and
the nearest ancestor is the right answer for a file under the latter). Not `list_projects()` scanning
every cache file either — that reads every database in the cache directory, which is the wrong cost
for a hook on every edit.

The test per ancestor is **a `projects` row**, not a database file. v1 used
`db_path_for(candidate).exists()` and that is wrong in a way that violates this hook's first refusal:
`ensure_schema` creates the file and an empty `projects` table, `status_of` correctly calls that
`indexed: false` (`rust/src/indexer.rs:480`), but `open_project_tracked` does not check for the row
(`rust/src/main.rs:133`) and `incremental_index` will fall through to `full_index` on a version or
candidate mismatch (`rust/src/incremental.rs:312`, `:330`) — and `full_index` **inserts the project
row** (`rust/src/indexer.rs:377`). A resolver that stopped at a schema-only database would therefore
make the repair hook create an index in a directory nobody asked about.

So: `db_path_for(candidate).exists()` as the cheap filter, then a read-only open and the same
one-row question `readings.rs:477` already asks —
`SELECT 1 FROM projects WHERE project_id = ?1 AND last_indexed_at IS NOT NULL`. A schema-only
database fails it and the walk continues outward.

cwd is the fallback **only when the payload carries no path at all**, not when a path is present but
resolves to nothing. v1 fell back in both cases, which meant an edit outside every indexed tree, made
from inside one, would refresh a project that was never edited. An explicit path that owns no index
is a finding: `no_index`, and nothing touched.

### D2 — a refusal leaves no mark, and that stays true for now

v1 proposed writing `repair_owed` into `_cortex_meta` on every refusal, cleared by a successful
refresh, forcing `staleness` to `true` while set. It is deleted rather than corrected, because the
review showed it promises something it cannot deliver — see §Deferred for the three reasons and what
a real version would have to do.

### D3 — the payload field name is not assumed

Nobody has ever intercepted a `PostToolUse` payload from any of these harnesses. The only captured
payload in the repository is a `PreToolUse`/Bash one
(`docs/2026-09-03-installer-dedup-and-attribution.md:153`), and CLAUDE.md is explicit that a matcher
— and by the same argument a field name — may be judged **only from an intercepted payload, never
from a transcript**. Codex's `PostToolUse` firing on a file edit was itself established by which hook
stayed *silent*, not by reading one.

So Task 1 captures real payloads before anything reads a field. If they disagree across harnesses,
that is a per-harness accessor beside `search_of_payload`'s two spellings — parsing may be plural,
the decision may not.

## Deferred: why `repair_owed` is not in this plan

v1's Task 4 was deleted after review. Three reasons, each verified against source:

**It only did half of what its own precedent does.** `graph_pending` works because it forces stale
output (`rust/src/staleness.rs:95`) **and** forces `incremental_index` into a full rebuild
(`rust/src/incremental.rs:323`). `repair_owed` did only the first. Worse, a clean incremental pass
that finds no candidates returns `Ok` and commits (`incremental.rs:405`), and v1's success arm then
cleared the mark — so the index would announce itself fresh without the missed edit ever being
re-extracted. **The task would have produced the exact failure it existed to prevent.**

**It could not write where it was needed.** `no_index` means there is no usable database;
`db_unavailable` means opening one failed; `busy_or_failed` may mean the same database will not take
another write. Writing to cwd's database instead changes an unrelated project's state. A second write
after a busy failure can also wait on the 5s SQLite busy timeout (`rust/src/db.rs:131`), against this
hook's documented "give up rather than wait" contract (`rust/src/main.rs:719`).

**Its refusal list was incomplete.** `no_ast_grep` (`rust/src/main.rs:777`) also repairs nothing and
was not in it.

A real version has to make `incremental_index` repay the debt, not merely announce it — the shape
`graph_pending` already has. That is a separate design and it should be measured against the 36
non-`no_index` refusals this session recorded before anyone builds it. Task 3 alone addresses the 79
that were this bug.

## Global Constraints

- `hook-refresh` keeps all three of its refusals: it never creates an index, it gives up rather than
  wait on a busy database, and it is silent and exits 0 whatever happens (`rust/src/main.rs:703-729`).
- The repo is pure Rust plus `install.sh` and `tests/install-smoke.sh`. No scripts.
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` in **both** crates before each
  commit; every commit compiles on its own and both crates are tested.
- Storage failures are returned or degrade to a refusal, never panic.
- Commit messages end with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- The user gates every commit. Do not push.

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `docs/2026-09-05-…-payloads.md` | create | the intercepted payloads, verbatim |
| `rust/src/main.rs` | modify `:741-772` | read the path, resolve the project, mark refusals |
| `rust/src/db.rs` | add | `project_root_for_path` |
| `rust/tests/cli.rs` | add 4 tests | end-to-end through the binary |
| `rust/tests/db.rs` | add 2 tests | the resolver |

---

### Task 1: Capture real `PostToolUse` payloads — blocking

**Files:**
- Create: `docs/2026-09-05-posttooluse-payloads.md`

**Interfaces:**
- Consumes: nothing.
- Produces: the verbatim payload each harness sends on a file edit, and the field that names the
  edited path. Task 2 reads that field name and nothing else.

**Nothing else in this plan may start until this task is done.** Every later step depends on a field
name that has never been observed. Assuming `tool_input.file_path` because Claude Code's docs say so
is exactly the error `2026-09-03-installer-dedup-and-attribution.md` records: a matcher was changed
from an intercepted-payload fact to a transcript-derived guess, a working hook broke, and four
hypotheses were spent on a symptom that change had created.

- [ ] **Step 1: Add a capture path that cannot fire by accident**

In `cmd_hook_refresh`, immediately after the payload is parsed (`rust/src/main.rs:748`):

```rust
    // Capture harness for one measurement, off unless the env var is set. It writes the raw payload
    // and exits before any refresh work, so a run with it set repairs nothing and a run without it
    // is byte-identical to today.
    if let Ok(dest) = std::env::var("CORT_HOOK_CAPTURE") {
        // One line per payload, or three captures concatenate into invalid JSON.
        let line = format!("{}\n", payload.trim_end());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        // Return before any refresh work. Without this the capture run also repairs, so a capture
        // session is no longer a pure observation and the payloads are taken from a hook that is
        // doing something else at the same time.
        return quiet("captured", usage);
    }
```

- [ ] **Step 2: Capture from each harness**

Deploy (`./install.sh`), then in each harness in turn — Claude Code, Codex, Kimi — set
`CORT_HOOK_CAPTURE=/tmp/cort-payloads.jsonl` in the environment the harness runs hooks in and collect
one payload **per matched tool**, not one per harness. The refresh entry matches
`Bash|Edit|Write|MultiEdit|NotebookEdit` (`rust/src/settings.rs:82`, `:106`), and the field that
names the edited path may differ by tool as well as by harness — `Bash` legitimately carries no path
at all, which is the case D1's cwd fallback exists for and which must be observed rather than
assumed.

**Do not rewrite a harness's wired command to get the capture.** v1 proposed adding a `--capture`
flag temporarily; the flag does not exist (hook arguments accept only `--harness`,
`rust/src/main.rs:328`), and more importantly rewriting a Codex entry costs a live re-review:
`trusted_at` only asks whether *some* hash exists at that position and cannot tell whether it matches
the entry now there (`rust/src/settings_toml.rs:423`), so `install.sh --check` can report a stale
`trusted=true` over an entry Codex has never approved
(`docs/2026-09-03-installer-dedup-and-attribution.md` §12-13). If a harness's hook cannot see an
environment variable, **skip that harness and record it as uncaptured** rather than touching its
settings file.

**Redeploy after Step 1.** Editing `rust/src/main.rs` does not change the installed binary; run
`./install.sh` before capturing and again after Step 4, or the capture surface stays deployed.

- [ ] **Step 3: Write down what arrived**

Create `docs/2026-09-05-posttooluse-payloads.md` with each payload, one section per harness and one
row per tool. **Elide the identifiers before committing** — the existing captured payload in
`docs/2026-09-03-installer-dedup-and-attribution.md:149` deliberately elides session, transcript,
cwd, model and tool-use ids, and this file must do the same. What matters is the field names and the
path convention, not the values. Each section states
each stating: the `tool_name` value, the field that carries the edited path, whether that path is
absolute or relative, and **what it is relative to** if relative. Note any harness that sends no path
at all — that is the case D1's cwd fallback exists for, and it must be named rather than assumed
away.

- [ ] **Step 4: Remove the capture path**

Revert the Step 1 block. It was a measurement, not a feature; leaving an env-var-triggered file write
in a hook that runs on every edit is a surface nobody asked for.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
git add docs/2026-09-05-posttooluse-payloads.md
git commit -m "docs: the PostToolUse payload each harness actually sends

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: `project_root_for_path`

**Files:**
- Modify: `rust/src/db.rs`
- Test: `rust/tests/db.rs`

**Interfaces:**
- Consumes: nothing from Task 1 except the knowledge that a path arrives.
- Produces:
  - `pub enum RootProbe { Indexed, Absent, Unreadable }`
  - `pub fn project_root_for_path(path: &Path) -> Result<Option<PathBuf>, ()>` — the nearest ancestor
    of `path` (including `path` itself when it is a directory) that has a `projects` row with
    `last_indexed_at` set. `Ok(None)` when no ancestor does; `Err(())` when a candidate database
    exists and will not answer, which Task 3 maps to `db_unavailable` rather than walking past it.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/db.rs`:

```rust
/// The nearest indexed ancestor, not the git root: one repository can hold several indexed projects
/// (this machine has `b/finance-engineering` and `b/finance-engineering/tools/finance-cli`), and a
/// file under the inner one belongs to the inner one.
#[test]
fn a_path_resolves_to_its_nearest_indexed_ancestor() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let outer = std::fs::canonicalize(tree.path()).unwrap();
    let inner = outer.join("tools/inner");
    std::fs::create_dir_all(inner.join("src")).unwrap();
    let inner = std::fs::canonicalize(&inner).unwrap();
    std::fs::write(inner.join("src/lib.rs"), "pub fn f() {}\n").unwrap();

    with_var("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()), || {
        // Nothing indexed yet.
        assert_eq!(project_root_for_path(&inner.join("src/lib.rs")), None);

        // A schema-only database is NOT an index and must be walked past. This is the case that
        // makes the file-existence test dangerous: `full_index` would insert the missing row and
        // the repair hook would have created an index nobody asked for.
        mark_schema_only(&outer);
        assert_eq!(
            project_root_for_path(&inner.join("src/lib.rs")),
            None,
            "a db file with no projects row is not an index"
        );

        // Index the outer project for real: the file resolves outward.
        mark_indexed(&outer);
        assert_eq!(
            project_root_for_path(&inner.join("src/lib.rs")),
            Some(outer.clone())
        );

        // Index the inner one too: the nearer answer wins.
        mark_indexed(&inner);
        assert_eq!(
            project_root_for_path(&inner.join("src/lib.rs")),
            Some(inner.clone())
        );
    });
}

/// A path that does not exist still resolves through its existing ancestors -- an edit hook can be
/// handed a file the tool just deleted.
#[test]
fn a_deleted_path_still_resolves_through_its_parents() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tree.path()).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();

    with_var("CORT_CACHE_DIR", Some(cache.path().to_str().unwrap()), || {
        mark_indexed(&root);
        assert_eq!(
            project_root_for_path(&root.join("src/gone.rs")),
            Some(root.clone()),
            "the file is gone; its directory is not"
        );
    });
}
```

`env_guard` and `with_var` already exist in `rust/tests/db.rs`. Add `project_root_for_path` to the
file's `use cort::db::{…}` list. Two helpers this test needs, written once at the top of the file:

```rust
/// A database file with a schema and no project row -- the state any command that opens a project
/// leaves behind, and the one `project_root_for_path` must refuse.
fn mark_schema_only(root: &std::path::Path) {
    let db = cort::db::open_db(&cort::db::db_path_for(root.to_str().unwrap())).unwrap();
    cort::db::ensure_schema(&db).unwrap();
}

/// A real index: schema plus the `projects` row with `last_indexed_at` set, which is what
/// `full_index` writes and what `status_of` calls `indexed: true`.
fn mark_indexed(root: &std::path::Path) {
    let db = cort::db::open_db(&cort::db::db_path_for(root.to_str().unwrap())).unwrap();
    cort::db::ensure_schema(&db).unwrap();
    db.execute(
        "INSERT INTO projects (project_id, name, path, last_indexed_at, extractor_version)
         VALUES (?1, 'p', ?2, 1, 'v')
         ON CONFLICT(project_id) DO UPDATE SET last_indexed_at = 1",
        rusqlite::params![
            cort::db::project_id_for(root.to_str().unwrap()),
            root.to_str().unwrap()
        ],
    )
    .unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test db a_path_resolves_to_its_nearest_indexed_ancestor`
Expected: FAIL to compile — "cannot find function `project_root_for_path`".

- [ ] **Step 3: Write minimal implementation**

In `rust/src/db.rs`:

```rust
/// The nearest ancestor of `path` that has an index, or `None`.
///
/// Walks up rather than scanning `list_projects()`, which opens every database in the cache
/// directory -- the wrong cost for a hook that runs on every edit. One `db_path_for(..).exists()`
/// per ancestor, nearest first, so a file under a nested indexed project resolves to that project
/// and not to the repository above it.
///
/// The path need not exist: an edit hook is handed the file a tool just deleted, so this
/// canonicalizes the nearest existing ancestor and appends the rest. A path outside any indexed
/// tree returns `None`, which is the caller's cue to fall back to the working directory.
pub fn project_root_for_path(path: &Path) -> Result<Option<PathBuf>, ()> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        let Some(parent) = probe.parent() else {
            return Ok(None);
        };
        probe = parent.to_path_buf();
    }
    let Ok(mut dir) = std::fs::canonicalize(&probe) else {
        return Ok(None);
    };
    if dir.is_file() {
        let Some(parent) = dir.parent() else {
            return Ok(None);
        };
        dir = parent.to_path_buf();
    }
    loop {
        match probe_root(&dir) {
            RootProbe::Indexed => return Ok(Some(dir)),
            // A database that will not answer stops the walk. Continuing outward would repair a
            // project the agent did not edit, which is worse than doing nothing.
            RootProbe::Unreadable => return Err(()),
            RootProbe::Absent => {}
        }
        let Some(parent) = dir.parent() else {
            return Ok(None);
        };
        dir = parent.to_path_buf();
    }
}

/// What one candidate directory turns out to be. Three states, not two: `is_ok()` on a query would
/// collapse "no row" with "the database is locked" and with "the file is not a database", and the
/// walk only stops on a positive row -- so a momentarily locked inner project would be skipped and
/// the repair would land on the outer one. That refreshes a project nobody edited, files the usage
/// row under the wrong `project_id`, and leaves the edited project unrepaired behind a row claiming
/// otherwise.
#[derive(Debug, PartialEq, Eq)]
pub enum RootProbe {
    /// A `projects` row with `last_indexed_at` set: a real index.
    Indexed,
    /// No database, or a database with no such row. Keep walking outward.
    Absent,
    /// The database exists and would not answer. Stop: the caller must refuse rather than guess.
    Unreadable,
}

/// Does this exact directory have an index -- not merely a database file?
///
/// The file test alone is wrong and dangerously so. `ensure_schema` creates the file and an empty
/// `projects` table the first time anything opens a project, and `status_of` correctly reports
/// `indexed: false` for that state. But `open_project_tracked` never checks for the row, and
/// `incremental_index` falls through to `full_index` on a version or candidate mismatch -- which
/// INSERTS the row. A resolver that stopped at a schema-only database would make the repair hook
/// create an index in a directory nobody asked about, breaking its first refusal.
///
/// The row test is the one `readings.rs:477` already asks, through a read-only connection so this
/// can never be the thing that creates a database. The connection sets the same 5s busy timeout
/// every other connection in this crate sets (`db.rs:131`); rusqlite's default is 0, and a hook
/// whose contract is "give up rather than wait" should still not fail on the first contended read.
fn probe_root(dir: &Path) -> RootProbe {
    let Some(dir) = dir.to_str() else {
        return RootProbe::Absent;
    };
    let path = db_path_for(dir);
    if !path.exists() {
        return RootProbe::Absent;
    }
    let Ok(db) = rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return RootProbe::Unreadable;
    };
    if db
        .busy_timeout(std::time::Duration::from_millis(5000))
        .is_err()
    {
        return RootProbe::Unreadable;
    }
    match db.query_row(
        "SELECT 1 FROM projects WHERE project_id = ?1 AND last_indexed_at IS NOT NULL",
        rusqlite::params![project_id_for(dir)],
        |_| Ok(()),
    ) {
        Ok(()) => RootProbe::Indexed,
        Err(rusqlite::Error::QueryReturnedNoRows) => RootProbe::Absent,
        Err(_) => RootProbe::Unreadable,
    }
}

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test db`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/db.rs rust/tests/db.rs
git commit -m "feat(db): resolve a path to its nearest indexed project

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: `hook-refresh` reads the path

**Files:**
- Modify: `rust/src/main.rs` (`cmd_hook_refresh`, `:741-772`)
- Test: `rust/tests/cli.rs`

**Interfaces:**
- Consumes: `project_root_for_path` (Task 2) and the field name established in Task 1.
- Produces: a refresh that repairs the edited file's project from any working directory.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`. Three things about that file have to be right or these tests cannot
pass, and v1 got all three wrong:

* **`run_hook_refresh(cwd, cache)` already exists** (`rust/tests/cli.rs:1064`) and sends no payload.
  Do not shadow it. Add `run_hook_refresh_with(payload, cwd, cache)` and make the existing two-arg
  helper call it with `{}`.
* **Refresh rows are not read by `hook_outcomes_at`** — that function always reads `hook-suggest` and
  its third parameter is a harness filter (`rust/src/usage.rs:378`). Use
  `cort::usage::outcomes_of_hook_at(&usage_db, 0, "hook-refresh", None)` (`usage.rs:393`).
* **`sandbox()` builds no git repository** (`rust/tests/cli.rs:80`), so `git_candidates` cannot narrow
  and `incremental_index` runs a full index whose adapter reports `files_reindexed: 0`
  (`rust/src/incremental.rs:286`) — the outcome is `already_current`, never `refreshed`. Give the
  fixture a git repo and a commit, the way `rust/tests/incremental.rs:76-80` does, or assert
  `already_current` and lose the ability to tell repair from a no-op. Use git.

```rust
/// The bug, as a test -- and the fixture is built so that **cwd and the edited file resolve to
/// different projects**. That is the whole point. A fixture where both walk up to the same root
/// cannot tell an implementation that reads the payload from one that ignores it and resolves from
/// cwd, which is the only property this change exists to establish and the way it will regress.
///
/// Here cwd is an unindexed scratch directory: resolving from cwd yields `None` and the hook would
/// refuse. `refreshed` therefore proves the payload path was read.
#[test]
fn a_refresh_resolves_the_project_from_the_payload_not_the_shell() {
    let (p, cwd, _c, cache) = sandbox();
    git_init(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 3; }\nexport function extra() { return 1; }\n",
    )
    .unwrap();

    let payload = format!(
        r#"{{"tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        p.path().join("src/helper.ts").display()
    );
    let r = run_hook_refresh_with(&payload, elsewhere.path(), &cache);
    assert_eq!(r.code, 0, "the hook always exits 0: {}", r.stderr);

    let counts =
        cort::usage::outcomes_of_hook_at(&cache.join("usage.db"), 0, "hook-refresh", None)
            .expect("read usage db");
    assert_eq!(
        counts.get("refreshed").and_then(Value::as_i64),
        Some(1),
        "cwd owns no index, so a `refreshed` here can only come from the payload path: {counts:?}"
    );
}

/// The shape the Evidence section actually reproduced: a **relative** path, from a subdirectory.
/// Both of the other tests send absolute paths, so a harness that sends project-root-relative or
/// home-relative paths would fail silently in production with those green.
#[test]
fn a_relative_path_in_the_payload_resolves_against_the_hooks_own_directory() {
    let (p, cwd, _c, cache) = sandbox();
    git_init(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    std::fs::write(
        p.path().join("src/helper.ts"),
        "export function helper(n: number) { return n * 5; }\n",
    )
    .unwrap();
    let sub = cwd.join("src");
    let r = run_hook_refresh_with(
        r#"{"tool_name":"Edit","tool_input":{"file_path":"helper.ts"}}"#,
        &sub,
        &cache,
    );
    assert_eq!(r.code, 0);
    let counts =
        cort::usage::outcomes_of_hook_at(&cache.join("usage.db"), 0, "hook-refresh", None)
            .expect("read usage db");
    assert_eq!(
        counts.get("refreshed").and_then(Value::as_i64),
        Some(1),
        "a relative path is resolved against the directory the hook runs in: {counts:?}"
    );
}

/// No usable path in the payload -- `Bash` carries none. cwd stays the answer, which is the whole of
/// today's behaviour for that shape.
#[test]
fn a_refresh_with_no_path_in_the_payload_still_uses_the_working_directory() {
    let (_p, cwd, _c, cache) = sandbox();
    git_init(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let r = run_hook_refresh_with(r#"{"tool_name":"Bash","tool_input":{}}"#, &cwd, &cache);
    assert_eq!(r.code, 0);
    let counts =
        cort::usage::outcomes_of_hook_at(&cache.join("usage.db"), 0, "hook-refresh", None)
            .expect("read usage db");
    assert_eq!(
        counts.get("already_current").and_then(Value::as_i64),
        Some(1),
        "{counts:?}"
    );
}

/// A schema-only database at cwd is not an index, and the repair hook must not turn it into one.
/// This pins a bug the rewrite fixes as a side effect: today `index_state()` passes a db file that
/// exists, `open_project_tracked` does not check for the row, and `incremental_index` falls through
/// to `full_index`, which INSERTS it (`rust/src/indexer.rs:377`) -- the hook creating an index
/// nobody asked for, against its own first refusal.
#[test]
fn a_schema_only_database_at_cwd_is_not_refreshed_into_an_index() {
    let (_p, cwd, _c, cache) = sandbox();
    // Any command that opens the project writes the schema without indexing anything.
    run_cort(&["impact", "--symbol", "helper"], &cwd, &cache);
    let r = run_hook_refresh_with(r#"{"tool_name":"Bash","tool_input":{}}"#, &cwd, &cache);
    assert_eq!(r.code, 0);
    let counts =
        cort::usage::outcomes_of_hook_at(&cache.join("usage.db"), 0, "hook-refresh", None)
            .expect("read usage db");
    assert_eq!(
        counts.get("no_index").and_then(Value::as_i64),
        Some(1),
        "an empty index is not an index: {counts:?}"
    );
    let db = cort::db::open_db(&cache.join(
        cort::db::db_path_for(cwd.to_str().unwrap())
            .file_name()
            .unwrap(),
    ))
    .unwrap();
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the hook must not have created a project row");
}
```

`git_init(root)` does not exist in `rust/tests/cli.rs`; add it, copying the four commands
`rust/tests/incremental.rs:76-80` runs (`init -q`, `config user.email`, `config user.name`,
`add -A`, `commit -qm init`). Without a git repository `git_candidates` cannot narrow,
`incremental_index` runs a full index, and its adapter reports `files_reindexed: 0`
(`rust/src/incremental.rs:286`) -- so every `refreshed` assertion above would read `already_current`
instead, and the tests would be measuring nothing.

`run_hook_refresh_with(payload, cwd, cache)` also does not exist. Add it beside the existing
two-argument `run_hook_refresh` (`rust/tests/cli.rs:1064`) and make that one call it, rather than
shadowing it -- its current callers depend on the payload it already sends.

- [ ] **Step 2: Run tests to verify the first fails**

Run: `cd rust && cargo test --test cli a_refresh_repairs_the_edited_files_project`
Expected: FAIL — the outcome is `no_index`, not `refreshed`.

- [ ] **Step 3: Write minimal implementation**

Replace the project selection in `cmd_hook_refresh`. The two existing lines

```rust
    if index_state() == IndexState::Missing {
        return quiet("no_index", usage);
    }
```

become a resolution that prefers the payload's path:

```rust
    // Which project to repair comes from the edited file, not from the shell's working directory.
    // The comment above says which *file* changed is `incremental_index`'s question -- true, and
    // only once the right project is open. An agent that runs `cd rust && cargo test` used to take
    // every later edit's repair with it: measured at 79 `no_index` rows in four hours of editing
    // this repository, while the index drifted two lines behind and `status` still said fresh.
    let edited = parsed
        .as_ref()
        .and_then(|v| v.get("tool_input"))
        .and_then(|i| i.get(EDITED_PATH_FIELD))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    // A path decides on its own. Falling back to cwd when an explicit path owns no index would
    // refresh a project that was never edited, which is what a first draft did. `Bash` carries no
    // path at all, and for that shape cwd is the whole of today's behaviour.
    let resolved = match edited {
        Some(p) => project_root_for_path(Path::new(p)),
        None => project_root_for_path(&cwd()),
    };
    let root = match resolved {
        Ok(Some(root)) => root,
        Ok(None) => return quiet("no_index", usage),
        // A database that would not answer. The existing outcome already means "could not do the
        // work"; guessing an outer project instead would repair the wrong index.
        Err(()) => return quiet("db_unavailable", usage),
    };
```

Task 3 removes the only caller of the private `index_state()` (`rust/src/main.rs:1056`). Delete it,
or `clippy --all-targets -- -D warnings` fails on dead code — and this plan's own constraint requires
that gate on every commit.

`EDITED_PATH_FIELD` is the constant Task 1 established. If Task 1 found the harnesses disagree, it is
a small per-harness accessor beside `search_of_payload` instead — parsing may be plural, the decision
may not.

Then use `root` in place of `cwd()`:

```rust
    let Ok((canon, mut db)) = open_project_tracked(&root, usage) else {
        return quiet("db_unavailable", usage);
    };
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test && cd ../evals && cargo test`
Expected: PASS in both.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/main.rs rust/tests/cli.rs
git commit -m "fix(hook): refresh the edited file's project, not the shell's

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Measure it, then write down what is true

**Files:**
- Modify: `docs/2026-09-04-hook-precision-and-the-seed-gate.md` (a new section) or a new dated doc
- Modify: `README.md` (the hook section's refresh paragraph)

**Interfaces:**
- Consumes: Tasks 1-3, deployed.

**This task may fail the plan.** Stop and report rather than documenting a regression if the
`no_index` share does not fall, or if any pre-existing hook test needed weakening.

- [ ] **Step 1: Record the before-number from this session**

`usage.db` already holds it, and it must be read before the new binary starts writing rows:
187 `already_current`, 79 `no_index`, 20 `db_unavailable`, 16 `busy_or_failed`, 16 `refreshed` over
four hours. Capture the same query's output now, with its window, so the after-number has something
to sit beside.

- [ ] **Step 2: Deploy and work normally for a while**

```bash
./install.sh && ./install.sh --check
```
Then edit in this repository the way the session that produced the bug did — including from
subdirectories — and re-read the outcome counts.

- [ ] **Step 3: Reproduce the original failure and confirm it is gone**

```bash
cd /home/yanggf/a/cortexyoung/rust
echo '{"tool_name":"Edit","tool_input":{"file_path":"'$PWD'/src/hook.rs"}}' | cort hook-refresh
```
Expected: an outcome of `refreshed` or `already_current`, never `no_index`.

- [ ] **Step 4: Write it down**

State the before/after outcome split, the reproduction, and — plainly — that the bug was found by
reading one line of an `impact` answer rather than by any check the product runs. That is the point
worth recording: `cort status` said `index_is_stale: false`, `changed_files: []` and a matching
`git_head`, and all three were true statements about an index that was two lines wrong.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
git add docs README.md
git commit -m "docs: hook-refresh follows the file, measured

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Review record — every v1 defect and where it is handled

| # | Defect | Handled |
|---|---|---|
| 1 | **Blocker.** `repair_owed` announced debt without repaying it: `graph_pending` forces stale **and** a full rebuild (`staleness.rs:95`, `incremental.rs:323`); v1 did only the first, and a no-candidate pass would clear the mark over an unrepaired index | Task 4 deleted; §Deferred records what a real version needs |
| 2 | **Blocker.** `db_path_for(dir).exists()` accepts a schema-only database, and `open_project_tracked` + `incremental_index` then reach `full_index`, which INSERTS the project row — the repair hook creating an index nobody asked for | D1 and Task 2 test the `projects` row through a read-only connection |
| 3 | **Blocker.** The mark could not be written on `no_index` or `db_unavailable`, could wait on the 5s busy timeout against the hook's own contract, and omitted `no_ast_grep` | Task 4 deleted |
| 4 | **Major.** Falling back to cwd when an explicit path resolved to nothing refreshes an unrelated project | D1 and Task 3: the fallback fires only when the payload carries no path |
| 5 | **Major.** The capture block had no `return`, no line delimiter, an unbounded synchronous write, and a `--capture` flag that does not exist | Task 1 Step 1 and Step 2 |
| 6 | **Major.** Temporarily rewriting a Codex entry costs a live re-review; `--check` can report a stale `trusted=true` | Task 1 Step 2 forbids it and records an uncaptured harness instead |
| 7 | **Major.** One edit per harness characterises one tool shape; the matcher covers five | Task 1 Step 2 captures per tool, and Step 3 elides identifiers |
| 8 | **Major.** Task 3's tests could not pass: `sandbox()` has no git so the outcome is `already_current`; `hook_outcomes_at` reads `hook-suggest` only; a two-arg `run_hook_refresh` already exists; `index_state()` becomes dead code | Task 3 Step 1 and Step 3 |
| 9 | **Minor.** `get_meta`/`set_meta` cited at `db.rs:129`, which opens SQLite; they are at `:153` and `:174` | the citation is gone with Task 4 |
| 10 | **v2, the "test does not test the thing" gap.** Task 3's headline fixture put cwd at `p/src` and the file at `p/src/helper.ts` — both walk up to `p`, so an implementation that ignores the payload and resolves from cwd passes. The one property this change exists to establish was unpinned | Task 3's first test now runs with cwd in an unindexed directory, where a `refreshed` can only come from the payload |
| 11 | **v2.** Goal, Architecture and the File Structure table still described the deleted `repair_owed` — the table even listed `staleness.rs \| modify`, which would have walked an executor straight back into v1's blocker 1 | all scrubbed; §Deferred is the only place it is mentioned |
| 12 | **v2.** The pasted tests called `hook_outcomes_at(.., Some("hook-refresh"))`, which reads `hook-suggest` rows filtered by a harness of that name and always returns empty — the tests failed regardless of the implementation — and used a three-argument `run_hook_refresh` the plan's own note forbade | both corrected in the code, not in a footnote |
| 13 | **v2.** `is_ok()` collapsed "no row" with "locked" and "corrupt", so a momentarily locked inner project would be skipped and the repair would land on the outer one; the read-only connection also set no busy timeout while every other connection in the crate sets 5s | `RootProbe` is tri-state and `Unreadable` stops the walk; the connection sets the same timeout |
| 14 | **v2.** The Evidence repro is a **relative** path from a subdirectory; both tests sent absolute paths | a third test sends the relative shape |

## Self-Review

**Spec coverage.** D1 → Tasks 2 and 3. D2 → Task 4. D3 → Task 1, which blocks everything else.
§Evidence's 79 `no_index` rows are the number Task 5 re-measures.

**Placeholder scan.** Three named soft spots, all deliberate: `EDITED_PATH_FIELD` in Task 3 is
whatever Task 1 observes, and cannot honestly be written before that; Task 3's tests say to check
`hook_outcomes_at`'s real signature, which was not read while writing this; Task 4 Step 3 describes
the meta-key wiring rather than pasting it, because `set_meta`/`get_meta` call shapes and
`compute_stale`'s early-return point must be read at the time.

**Type consistency.** `project_root_for_path(&Path) -> Option<PathBuf>` is defined in Task 2 and
called twice in Task 3. `repair_owed` is written in Task 4 and read in Task 4 only.

**Known risk, stated.** Task 4's mark is written to the project cwd resolves to when the edited path
resolves to nothing. If an agent edits a file outside every indexed tree from a directory inside one,
the mark lands on a project that was not edited — a false `stale=true`, which is the safe direction
but is still a wrong row. Whether that shape ever occurs is unmeasured; the alternative, marking
nothing, is the failure this task exists to fix.
