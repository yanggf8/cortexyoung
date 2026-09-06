# Index Drift Diagnosis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make schema and extractor drift in every indexed project visible, and make an index that will not answer impossible to mistake for one that is not there.

**Architecture:** `list_projects` reads only the `projects` row and silently drops any database it cannot open. It gains the two version facts that decide whether an index is usable (`SCHEMA_VERSION`, `extractor_version`, both from `_cortex_meta`), and a return variant for a database that exists and will not answer. `cort projects` then reports drift per row, and `--verdict` emits one lean TSV line so the compatibility decision has a single home in Rust.

**Tech Stack:** Rust (`rust/` crate), rusqlite, SQLite, the `env_guard()` / `with_var` helpers already in `rust/tests/db.rs`, the `sandbox()` / `run_cort` helpers already in `rust/tests/cli.rs`.

**Spec:** `docs/superpowers/specs/2026-09-06-cort-upgrade-design.md`

**Review:** v2 after one Codex round on the plan itself. Six blockers were found and are fixed here; §"What the review changed" at the end records them so an executor does not reintroduce one.

## Global Constraints

- Repo is pure Rust; the only executable Bash is `install.sh` and `tests/install-smoke.sh`.
- Run `cargo fmt --all` then `cargo clippy --all-targets -- -D warnings` then `cargo test --locked --all-targets` in **both** `rust/` and `evals/` before every commit, and let a non-zero exit stop the commit (never end a verification pipeline in `tail`, which swallows the status).
- Storage failures are returned, never panicked on.
- "Unreadable" is never reported as "absent" (precedent: `RootProbe::Unreadable`, produced at `rust/src/db.rs:544` and `:556`), and it is never reported as "drifted" either — a version mismatch is a fact, an unreadable database is an absence of facts.
- `--lean` TSV must never contain an empty field.
- No absolute developer paths anywhere, including fixtures.

## Scope

**Plan 1 of 3**, per the spec's own build order (§1). It ships working software alone: after it, the 2026-09-05 incident (7 of 10 projects on a superseded extractor while `--check` said "all current") is visible.

Deferred, deliberately: **Plan 2** — extend `graph_pending` into a reason set with a target, bind it to `index_is_stale`, give `incremental_index` a typed call policy so the edit hook stops performing full rebuilds and stops being the structural migrator (spec §4). **Plan 3** — the upgrader itself (spec §3, §5, §6) and the single-home moves for the shim template, ast-grep pin and manifest key-set (spec §1).

**This plan ships no repair.** Nothing here rebuilds an index. That is why Task 4 reports drift and does **not** fail `--check`: naming an action whose executable does not exist yet would make every ordinary machine permanently red, and a permanently red check trains its reader to ignore it.

---

## File Structure

- `rust/src/db.rs` — the only file that knows how a project database is probed: the two new fields, the `ProjectEntry` enum, the `usage.db` exclusion.
- `rust/src/main.rs` — `cmd_projects` renders facts and computes drift against the running binary; `cmd_delete` filters to readable entries. No new knowledge.
- `rust/src/render.rs` — the lean renderer for the verdict line.
- `rust/tests/db.rs`, `rust/tests/cli.rs` — tests.
- `install.sh` — gains a verdict line; its existing stale/gone reporting is untouched.
- `README.md` — the `--check` paragraph gains the new axis.

---

### Task 1: report the schema and extractor each index was built with, and stop scanning `usage.db`

**Files:**
- Modify: `rust/src/db.rs:333-382`
- Test: `rust/tests/db.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `ProjectListRow` with `pub schema_version: Option<String>` and `pub extractor_version: Option<String>`. Task 2 wraps this struct; Task 3 renders it.

**Why `usage.db` first.** `usage_db_path()` is `CORT_CACHE_DIR/usage.db` (`rust/src/usage.rs:100-109`) — the same directory `list_projects` scans, and it ends in `.db`. It has `_usage_meta`, not `_cortex_meta`, and no `projects` table. Today it is swallowed by the silent `continue`. The moment Task 2 gives that failure a variant, every ordinary machine reports a bogus unreadable index and three existing tests break. Exclude it here, before the variant exists.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/db.rs`:

```rust
/// The two facts that decide whether an index is usable live in `_cortex_meta`, and until 2026-09-06
/// nothing that enumerates projects read either. That is why 7 of 10 projects sat on a superseded
/// extractor while `--check` reported "all current".
///
/// The stored schema is asserted at an **old** value on purpose: a fixture that stores the current
/// one cannot tell a real read from `SCHEMA_VERSION.to_string()`.
#[test]
fn list_projects_reports_the_schema_and_extractor_each_index_was_built_with() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        let root = tempfile::tempdir().unwrap();
        let root_s = root.path().to_str().unwrap();
        let db = open_db(db_path_for(root_s)).unwrap();
        ensure_schema(&db).unwrap();
        let pid = project_id_for(root_s);
        let name = root.path().file_name().unwrap().to_string_lossy().to_string();
        db.execute(
            "INSERT INTO projects (project_id, name, path, extractor_version)
             VALUES (?1, ?2, ?3, 'stale-extractor')",
            params![pid, name, root_s],
        )
        .unwrap();
        set_meta(&db, "extractor_version", "stale-extractor").unwrap();
        set_meta(&db, "SCHEMA_VERSION", "3").unwrap();
        drop(db);

        let rows = list_projects();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].extractor_version.as_deref(), Some("stale-extractor"));
        assert_eq!(
            rows[0].schema_version.as_deref(),
            Some("3"),
            "the stored schema must be read, not assumed: {:?}",
            rows[0].schema_version
        );
    });
}

/// `usage.db` lives in the same cache directory and ends in `.db`, but it is the recorder, not an
/// index: `_usage_meta`, no `projects` table (`usage.rs:100-109`). It must never appear in the
/// project population. Three recorder-isolation tests (`rust/tests/usage.rs`) deliberately make it
/// busy or read-only and assert that `cort projects` stdout is byte-identical, so a version of this
/// scan that notices `usage.db` at all breaks them.
#[test]
fn the_usage_recorder_is_not_a_project() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        std::fs::write(cache.path().join("usage.db"), b"not a project index").unwrap();
        assert!(
            list_projects().is_empty(),
            "the recorder is not part of the project population: {:?}",
            list_projects()
        );
    });
}
```

Add `set_meta` to the `use cort::db::{...}` list at the top of `rust/tests/db.rs` (it currently
imports `get_meta` only).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test db -- list_projects_reports_the_schema_and_extractor the_usage_recorder_is_not_a_project`
Expected: FAIL to compile — `no field 'extractor_version' on type 'ProjectListRow'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/db.rs`, extend the struct:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectListRow {
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub git_head: Option<String>,
    pub last_indexed_at: Option<i64>,
    pub db_path: String,
    /// The schema this database is stamped at, read from `_cortex_meta`. `None` means the key is
    /// absent -- a database that predates the meta table, which is not the same as a current one.
    pub schema_version: Option<String>,
    /// The extractor identity the derived rows were built with -- the same key
    /// `incremental_index` reads (`incremental.rs:310`).
    ///
    /// It reports, and does not predict: `incremental_index` only rebuilds on a mismatch when a
    /// stored value *exists* (`if let Some(stored)`, `incremental.rs:312-318`), so an index whose
    /// meta key is absent reads as drifted here and triggers no extractor rebuild there. That
    /// divergence is on the predates-meta population, and closing it belongs to plan 2.
    pub extractor_version: Option<String>,
}
```

Inside the loop in `list_projects`, before opening anything, skip the recorder. `usage_db_path()`
is the one home for that path; do not spell `"usage.db"` a second time:

```rust
        let db_path = dir.join(&name);
        if crate::usage::usage_db_path().as_deref() == Some(db_path.as_path()) {
            continue;
        }
```

Then read the two values with the existing `get_meta` — **not** a new helper. `get_meta` returns
`rusqlite::Result<Option<String>>`, which keeps "key absent" (`Ok(None)`) distinct from "the read
failed" (`Err`); a `.ok()` wrapper would flatten a lock error, a missing table and a corrupt page
into the same `None`. In this task both are read after the `projects` row succeeds, and a failure is
treated as absent; Task 2 gives the failure its own variant.

```rust
        let row = db.query_row(
            "SELECT project_id, name, path, git_head, last_indexed_at FROM projects",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            },
        );
        if let Ok((project_id, name, path, git_head, last_indexed_at)) = row {
            out.push(ProjectListRow {
                project_id,
                name,
                path,
                git_head,
                last_indexed_at,
                db_path: db_path_str,
                schema_version: get_meta(&db, "SCHEMA_VERSION").ok().flatten(),
                extractor_version: get_meta(&db, "extractor_version").ok().flatten(),
            });
        }
```

Reading the tuple out of the closure first avoids constructing `ProjectListRow` inside a closure
that would also need to borrow `db` for `get_meta`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test db -- list_projects_reports_the_schema_and_extractor the_usage_recorder_is_not_a_project`
Expected: both PASS.

- [ ] **Step 5: Verify each test can actually fail**

Two separate breaks, each restored afterwards:

1. Replace `get_meta(&db, "SCHEMA_VERSION").ok().flatten()` with
   `Some(SCHEMA_VERSION.to_string())`. Expected: RED with `Some("5")` vs `Some("3")`. This is the
   break that matters — a fixture storing the current schema would stay green here.
2. **Do not try to break the recorder skip in this task.** Deleting it leaves the test GREEN, and
   that is correct rather than a fixture defect: in Task 1 a junk `usage.db` opens fine (SQLite is
   lazy — verified) and is then swallowed by `if let Ok(row)`, so `list_projects()` is empty with or
   without the skip. The skip only becomes observable in Task 2, when the failure gains a variant.
   The test is written here as a canary, and Task 2 Step 5 is where it earns its keep.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/db.rs rust/tests/db.rs
git commit -m "feat(db): report each index's schema and extractor, and skip the recorder

The two facts that decide whether an index is usable live in _cortex_meta and
nothing that enumerates projects read either, which is why seven projects sat
on a superseded extractor while --check said all current. extractor_version
is read from the same key incremental_index reads, so the report and the
rebuild decision cannot disagree about which value is stored -- though they
do diverge where no value is stored at all, which plan 2 closes.

usage.db shares the cache directory and ends in .db but is the recorder, not
an index. It was swallowed by a silent continue; excluding it by
usage_db_path() keeps that skip deliberate before the next commit gives an
unreadable database a variant of its own."
```

---

### Task 2: an index that will not answer stops being invisible

**Files:**
- Modify: `rust/src/db.rs` (`list_projects`), `rust/src/main.rs:1613` and `:1657`
- Test: `rust/tests/db.rs`

**Interfaces:**
- Consumes: `ProjectListRow` from Task 1.
- Produces: `pub enum ProjectEntry { Indexed(ProjectListRow), Unreadable { db_path: String, reason: String } }` and `pub fn list_projects() -> Vec<ProjectEntry>`.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/db.rs`:

```rust
/// A database that exists and will not answer is not an absent project -- the same conflation
/// `RootProbe::Unreadable` exists to prevent one level down (`db.rs:544`, `db.rs:556`).
///
/// Both failure arms are exercised, because they are different code: a **directory** named `*.db`
/// fails at `Connection::open_with_flags`, while a **file of junk** usually opens fine and fails on
/// the first query. A fixture with only the second cannot detect a regression in the first.
///
/// A database with no `projects` row must stay skipped: `ensure_schema` creates that shape before
/// anything is indexed, so it is correctly not a project.
#[test]
fn an_index_that_will_not_answer_is_reported_rather_than_skipped() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        std::fs::create_dir(cache.path().join("adirectory.db")).unwrap();
        std::fs::write(cache.path().join("junk.db"), b"this is not a sqlite file").unwrap();

        let empty_root = tempfile::tempdir().unwrap();
        let db = open_db(db_path_for(empty_root.path().to_str().unwrap())).unwrap();
        ensure_schema(&db).unwrap();
        drop(db);

        let entries = list_projects();
        let mut unreadable: Vec<String> = entries
            .iter()
            .filter_map(|e| match e {
                ProjectEntry::Unreadable { db_path, .. } => Some(db_path.clone()),
                ProjectEntry::Indexed(_) => None,
            })
            .collect();
        unreadable.sort();
        assert_eq!(unreadable.len(), 2, "entries: {entries:?}");
        assert!(unreadable[0].ends_with("adirectory.db"), "{unreadable:?}");
        assert!(unreadable[1].ends_with("junk.db"), "{unreadable:?}");

        assert_eq!(
            entries.iter().filter(|e| matches!(e, ProjectEntry::Indexed(_))).count(),
            0,
            "a schema-only database is not a project: {entries:?}"
        );
    });
}
```

Add `ProjectEntry` to the `use cort::db::{...}` list in `rust/tests/db.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test db an_index_that_will_not_answer`
Expected: FAIL to compile — `cannot find type 'ProjectEntry'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/db.rs`:

```rust
/// One entry from the cache-directory scan. `Unreadable` is a variant rather than an omission so a
/// caller cannot read a database that exists and will not answer as "nothing here".
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectEntry {
    Indexed(ProjectListRow),
    Unreadable { db_path: String, reason: String },
}
```

Change the signature to `pub fn list_projects() -> Vec<ProjectEntry>`. Clone `db_path_str` in the
open-error arm (which `continue`s) and move it in the final arm — no borrow puzzle:

```rust
        let db = match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(db) => db,
            Err(e) => {
                out.push(ProjectEntry::Unreadable {
                    db_path: db_path_str.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
        };
```

and replace the `if let Ok(...)` from Task 1 with an exhaustive match:

```rust
        match row {
            Ok((project_id, name, path, git_head, last_indexed_at)) => {
                out.push(ProjectEntry::Indexed(ProjectListRow {
                    project_id,
                    name,
                    path,
                    git_head,
                    last_indexed_at,
                    db_path: db_path_str,
                    schema_version: get_meta(&db, "SCHEMA_VERSION").ok().flatten(),
                    extractor_version: get_meta(&db, "extractor_version").ok().flatten(),
                }));
            }
            // Not a failure: `ensure_schema` creates this shape before anything is indexed.
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => out.push(ProjectEntry::Unreadable {
                db_path: db_path_str,
                reason: e.to_string(),
            }),
        }
```

In `rust/src/main.rs`, add `ProjectEntry` to the `use cort::db::{...}` list at `main.rs:7-9` (it
currently imports `list_projects` but not the new type).

`cmd_delete` at `main.rs:1657` needs readable rows only:

```rust
            if let Some(row) = cort::db::list_projects()
                .into_iter()
                .filter_map(|e| match e {
                    ProjectEntry::Indexed(r) => Some(r),
                    ProjectEntry::Unreadable { .. } => None,
                })
                .find(|r| r.path.trim_end_matches('/') == want)
```

`cmd_projects` at `main.rs:1613` is replaced in full — this is the complete body, not a fragment.
Task 3 replaces it again; this intermediate version must compile and keep the suite green on its own:

```rust
fn cmd_projects(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let _a = FormatOnlyArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let rows: Vec<Value> = list_projects()
        .into_iter()
        .map(|entry| match entry {
            ProjectEntry::Unreadable { db_path, reason } => json!({
                "db_path": db_path,
                "unreadable": reason,
            }),
            ProjectEntry::Indexed(r) => {
                let exists = Path::new(&r.path).is_dir();
                let stale = match (r.git_head.as_deref(), git_head_quickly(Path::new(&r.path))) {
                    (Some(stored), Some(now)) => Some(stored != now),
                    _ => None,
                };
                json!({
                    "project_id": r.project_id,
                    "name": r.name,
                    "path": r.path,
                    "git_head": r.git_head,
                    "last_indexed_at": r.last_indexed_at,
                    "db_path": r.db_path,
                    "exists": exists,
                    "stale": stale,
                })
            }
        })
        .collect();
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: Value::Array(rows),
    })
}
```

**Two** tests index into the returned Vec and must be updated, or this task's own Step 4 fails to
compile rather than passing:

- `rust/tests/db.rs:233` (`list_projects_enumerates_every_indexed_project_in_the_cache_dir`), which
  reads `rows[0].path`.
- `list_projects_reports_the_schema_and_extractor_each_index_was_built_with`, **the test Task 1 just
  added**, which reads `rows[0].extractor_version` and `rows[0].schema_version`.

Both take the same shape — bind the row out of the variant first:

```rust
        let rows = list_projects();
        assert_eq!(rows.len(), 1);
        let ProjectEntry::Indexed(r) = &rows[0] else {
            panic!("expected an indexed project, got {:?}", rows[0]);
        };
        assert_eq!(r.extractor_version.as_deref(), Some("stale-extractor"));
```

Task 1's other test, `the_usage_recorder_is_not_a_project`, needs no change: `is_empty()` works on
any `Vec`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test db an_index_that_will_not_answer`
Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail — both arms**

1. Restore `Err(_) => continue` on the **open** error. Expected: RED, `unreadable.len()` is 1.
2. Restore it on the **query** error instead. Expected: RED, `unreadable.len()` is 1.

If either break leaves the test green, the fixture is not reaching that arm and must be fixed before
continuing.

3. Delete Task 1's `usage_db_path()` skip. Expected: `the_usage_recorder_is_not_a_project` goes RED
   **now** (it could not in Task 1 — see that task's Step 5), and the three `rust/tests/usage.rs`
   recorder-isolation tests go red with it. Restore. That pairing is the point: the skip is what
   keeps `usage.db` out of a population that is now allowed to report failures.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0. Two suites are the ones at risk and must be green without editing them:
`rust/tests/usage.rs` recorder isolation (three tests that make `usage.db` busy or read-only and
compare `cort projects` stdout byte for byte) and `rust/tests/cli.rs:661` (asserts `projects` is an
empty array after a delete). Both stay green **only** because Task 1 excluded the recorder. If either
fails, the exclusion regressed — fix that, do not edit the tests.

- [ ] **Step 7: Commit**

```bash
git add rust/src/db.rs rust/src/main.rs rust/tests/db.rs
git commit -m "feat(db): an index that will not answer is reported, not skipped

list_projects dropped every database it could not open, so the indexes most
worth repairing were the ones the scan could not see. ProjectEntry gives that
failure a variant of its own.

A database with no projects row stays skipped: ensure_schema creates that
shape before anything is indexed, so it is correctly not a project. The two
failure arms are different code -- a directory fails at open, a junk file
fails at the first query -- and the fixture exercises both."
```

---

### Task 3: `cort projects` says whether each index drifted, and `--verdict` says it once

**Files:**
- Modify: `rust/src/main.rs` (`cmd_projects`, args struct, CLI help at `main.rs:48-54`), `rust/src/render.rs:402-413`
- Test: `rust/tests/cli.rs`

**Interfaces:**
- Consumes: `ProjectEntry`, `schema_version`, `extractor_version`.
- Produces: per-row JSON fields `schema_version`, `extractor_version`, `drifted`; and `cort projects --verdict` printing exactly `indexes\t<word>\t<drifted>\t<unreadable>\n`. Plan 3's upgrader consumes both.

**Two numbers, not one.** A single count cannot stand behind both words: a reader told
`drifted 3` when two of those three are databases the binary could not open has been told something
false. So the line carries both counts, always, and each word is answerable by the number beside it:

| field | meaning |
|---|---|
| `<word>` | the worst state found: `drifted` if any, else `unknown` if any unreadable, else `compatible` |
| `<drifted>` | readable projects whose stored schema or extractor differs from this binary's |
| `<unreadable>` | entries that exist and would not answer |

Both numbers are always present, so the line never carries an empty field.

**`drifted` counts a project whose directory is gone.** Drift is a fact about the index, and the
index is readable; excluding it from a *count* and denying the *fact* are different decisions, and
only the first is defensible. The gone directory is already reported on its own line by `--check`
and as `exists: false` in the JSON row.

**What the verdict deliberately does not cover.** Git staleness (`stale`) and `graph_pending` are
different axes and keep their own reporting; this line answers compatibility only.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`:

```rust
/// The incident in one assertion: an index whose stored extractor is not the one this binary uses
/// must say so, while `stale` stays false because no git head moved -- which is exactly the shape
/// that read as "all current" on 2026-09-05.
#[test]
fn projects_reports_extractor_drift_while_git_says_fresh() {
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

    let r = run_cort(&["projects"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let rows: Value = serde_json::from_str(&r.stdout).expect("projects emits json");
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("path").and_then(Value::as_str) == Some(cwd.to_str().unwrap()))
        .expect("our project is listed");
    assert_eq!(
        row.get("extractor_version").and_then(Value::as_str),
        Some("not-the-one-that-ships")
    );
    assert_eq!(row.get("drifted").and_then(Value::as_bool), Some(true), "{row}");
    assert_eq!(
        row.get("stale").and_then(Value::as_bool),
        Some(false),
        "git head did not move: {row}"
    );

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert_eq!(v.stdout, "indexes\tdrifted\t1\t0\n", "verdict was {:?}", v.stdout);
}

/// Schema drift is a second, independent axis. A `drifted` computed only from the extractor passes
/// the test above and fails this one.
#[test]
fn projects_reports_schema_drift_independently_of_the_extractor() {
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
    cort::db::set_meta(&db, "SCHEMA_VERSION", "3").unwrap();
    drop(db);

    let r = run_cort(&["projects"], &cwd, &cache);
    let rows: Value = serde_json::from_str(&r.stdout).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("path").and_then(Value::as_str) == Some(cwd.to_str().unwrap()))
        .expect("our project is listed");
    assert_eq!(row.get("schema_version").and_then(Value::as_str), Some("3"));
    assert_eq!(row.get("drifted").and_then(Value::as_bool), Some(true), "{row}");

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.stdout, "indexes\tdrifted\t1\t0\n", "verdict was {:?}", v.stdout);
}

/// An unreadable index is an absence of facts, not a version mismatch. Reporting it as `drifted`
/// would claim knowledge the binary does not have (spec §2).
#[test]
fn an_unreadable_index_makes_the_verdict_unknown_not_drifted() {
    let (_p, cwd, _c, cache) = sandbox();
    std::fs::create_dir(cache.join("adirectory.db")).unwrap();
    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert_eq!(v.stdout, "indexes\tunknown\t0\t1\n", "verdict was {:?}", v.stdout);
}

/// Drift and unreadable together: the word names the half we know, and each number stands only for
/// its own fact. An implementation that lets any unreadable entry force `unknown` passes all three
/// tests above while violating the spec's rule that drift is a fact and must be named.
#[test]
fn drift_outranks_unreadable_and_each_count_stands_alone() {
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
    std::fs::create_dir(cache.join("adirectory.db")).unwrap();
    std::fs::write(cache.join("junk.db"), b"not a sqlite file").unwrap();

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t2\n",
        "one drifted and two unreadable: the word names the drift, and neither count absorbs the \
         other: {:?}",
        v.stdout
    );
}

/// A drifted index whose directory is gone is still a drifted index. Excluding it from the count
/// and denying the fact are different decisions; only the first would be defensible, and this
/// pins that the fact is not denied.
#[test]
fn a_drifted_index_whose_directory_is_gone_is_still_counted_as_drift() {
    let (_p, _cwd, _c, cache) = sandbox();
    let gone = tempfile::tempdir().unwrap();
    let gone_path = gone.path().to_path_buf();
    git_in_fixture(&gone_path);
    std::fs::write(gone_path.join("a.ts"), "export function a() { return 1; }\n").unwrap();
    let idx = run_cort(&["index"], &gone_path, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    let db_file = cache.join(
        cort::db::db_path_for(gone_path.to_str().unwrap())
            .file_name()
            .unwrap(),
    );
    let db = cort::db::open_db(&db_file).unwrap();
    cort::db::set_meta(&db, "extractor_version", "not-the-one-that-ships").unwrap();
    drop(db);
    drop(gone);

    let elsewhere = tempfile::tempdir().unwrap();
    let v = run_cort(&["projects", "--verdict"], elsewhere.path(), &cache);
    assert_eq!(
        v.stdout, "indexes\tdrifted\t1\t0\n",
        "the index is readable and does not match, whether or not its tree still exists: {:?}",
        v.stdout
    );
}

/// `install.sh --check` parses this line with `read`, so its shape is a contract: exactly one line,
/// four tab-separated non-empty fields, and two numeric counts.
#[test]
fn the_verdict_line_is_one_line_of_four_non_empty_fields() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.lines().count(), 1, "stdout was {:?}", r.stdout);
    let fields: Vec<&str> = r.stdout.trim_end_matches('\n').split('\t').collect();
    assert_eq!(fields.len(), 4, "stdout was {:?}", r.stdout);
    assert!(fields.iter().all(|f| !f.is_empty()), "stdout was {:?}", r.stdout);
    assert_eq!(fields[0], "indexes");
    assert!(
        matches!(fields[1], "compatible" | "drifted" | "unknown"),
        "stdout was {:?}",
        r.stdout
    );
    assert!(fields[2].parse::<u64>().is_ok(), "stdout was {:?}", r.stdout);
    assert!(fields[3].parse::<u64>().is_ok(), "stdout was {:?}", r.stdout);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test cli -- projects_reports_ an_unreadable_index_makes drift_outranks a_drifted_index_whose the_verdict_line_is`
Expected: FAIL — `unexpected argument '--verdict'`.

- [ ] **Step 3: Write minimal implementation**

Add an args struct beside `FormatOnlyArgs` in `rust/src/main.rs`. It must keep `-f/--format`,
because `cort projects -f json` parses today and removing it is a breaking change:

The three `#[command(...)]` attributes are not decoration: every args struct in this file carries
all three (`rust/src/main.rs:391-394`), and `dispatch` passes `&args[1..]`, so the subcommand word is
already stripped by the time the parser sees it (`main.rs:514`). Omitting `disable_help_flag` would
make `cort projects --help` behave unlike every other command.

```rust
#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct ProjectsArgs {
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// One lean line for a caller that wants the compatibility answer and not the population.
    #[arg(long)]
    verdict: bool,
}
```

Rewrite `cmd_projects`. Note the non-verdict path keeps `Format::Json` unconditionally, exactly as
today — `-f` is accepted and ignored — so the lean renderer only ever sees the verdict object:

```rust
fn cmd_projects(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = ProjectsArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let want_schema = cort::db::SCHEMA_VERSION.to_string();
    let want_extractor = cort::pack::extractor_version();

    let mut n_drifted = 0u64;
    let mut n_unreadable = 0u64;
    let mut rows: Vec<Value> = Vec::new();

    for entry in list_projects() {
        match entry {
            ProjectEntry::Unreadable { db_path, reason } => {
                n_unreadable += 1;
                rows.push(json!({ "db_path": db_path, "unreadable": reason }));
            }
            ProjectEntry::Indexed(r) => {
                // `stale` is what an installer or a person actually wants to know, and it is only
                // answerable here: the row stores the head it was built at, and the tree knows the
                // head it is on now. `null`, never `false`, when the two cannot be compared -- the
                // directory is gone, it is not a git tree, or `rev-parse` did not answer inside the
                // budget. A project that cannot be checked is not a project that is fresh.
                let exists = Path::new(&r.path).is_dir();
                let stale = match (r.git_head.as_deref(), git_head_quickly(Path::new(&r.path))) {
                    (Some(stored), Some(now)) => Some(stored != now),
                    _ => None,
                };
                let drifted = r.schema_version.as_deref() != Some(want_schema.as_str())
                    || r.extractor_version.as_deref() != Some(want_extractor.as_str());
                // Counted whether or not the directory still exists: the index is readable and
                // it does not match, and that is true regardless of whether anyone can act on it.
                if drifted {
                    n_drifted += 1;
                }
                rows.push(json!({
                    "project_id": r.project_id,
                    "name": r.name,
                    "path": r.path,
                    "git_head": r.git_head,
                    "last_indexed_at": r.last_indexed_at,
                    "db_path": r.db_path,
                    "exists": exists,
                    "stale": stale,
                    "schema_version": r.schema_version,
                    "extractor_version": r.extractor_version,
                    "drifted": drifted,
                }));
            }
        }
    }

    if a.verdict {
        // Drift is a fact; unreadable is the absence of one. A population with both is reported as
        // drifted, because that names the half we know -- and the other number is printed beside it
        // rather than folded into it.
        let word = if n_drifted > 0 {
            "drifted"
        } else if n_unreadable > 0 {
            "unknown"
        } else {
            "compatible"
        };
        return Ok(Emit {
            render_command: Some("projects"),
            format: Format::Lean,
            payload: json!({
                "indexes": word,
                "drifted": n_drifted,
                "unreadable": n_unreadable,
            }),
        });
    }

    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: Value::Array(rows),
    })
}
```

`Format::Lean` alone does **not** produce TSV: `render` falls through to `pretty(payload)` for any
command without an arm (`rust/src/render.rs:406-413`). Add the renderer:

```rust
fn render_projects(payload: &Value) -> String {
    let word = payload.get("indexes").and_then(Value::as_str).unwrap_or("unknown");
    let drifted = payload.get("drifted").and_then(Value::as_u64).unwrap_or(0);
    let unreadable = payload.get("unreadable").and_then(Value::as_u64).unwrap_or(0);
    format!("indexes\t{word}\t{drifted}\t{unreadable}\n")
}
```

and register it: `Some("projects") => render_projects(payload),`.

Update the CLI help line for `projects` at `rust/src/main.rs:48-54` to
`"cort projects [--verdict]"`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test cli -- projects_reports_ an_unreadable_index_makes drift_outranks a_drifted_index_whose the_verdict_line_is`
Expected: all six PASS.

- [ ] **Step 5: Verify the tests can actually fail**

1. Hard-code `let drifted = false;`. Expected: both drift tests RED **and** their verdict assertions
   RED. If a verdict assertion stays green, the verdict is not derived from the same computation.
2. Compute `drifted` from the extractor only. Expected: the schema test RED, the extractor test green.
3. Report `Unreadable` as `drifted`. Expected: `an_unreadable_index_makes_the_verdict_unknown` RED.
3b. Let any unreadable entry force `unknown` (check `n_unreadable` before `n_drifted`). Expected:
   `drift_outranks_unreadable_and_each_count_stands_alone` RED — the three tests before it stay
   green, which is why it exists.
3c. Restore `if drifted && exists`. Expected:
   `a_drifted_index_whose_directory_is_gone_is_still_counted_as_drift` RED.
4. Remove the `Some("projects")` renderer arm. Expected: every verdict assertion RED, because pretty
   JSON is not the TSV line.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/main.rs rust/src/render.rs rust/tests/cli.rs
git commit -m "feat(projects): report schema and extractor drift, and say it once

stale answers whether the git head moved. On 2026-09-05 it had not moved in
any project tree, so every drifted index read as fresh. drifted answers the
other question -- was this index built by the schema and extractor this
binary uses -- and --verdict reduces it to one lean line so the decision has
one home.

unknown is not drifted: an unreadable index is an absence of facts, and
reporting it as a version mismatch would claim knowledge we do not have. A
project whose directory is gone is reported and not counted, because the
count is of work someone can actually do."
```

---

### Task 4: `install.sh --check` reports the new axis without pretending there is a repair

**Files:**
- Modify: `install.sh:873-899`, `README.md:750-755`
- Test: `tests/install-smoke.sh` (assert the new line), plus manual verification

**Interfaces:**
- Consumes: `cort projects --verdict`.
- Produces: nothing later tasks depend on.

**What this task does not do.** It does not delete the existing `proj_json` query or the
`stale_names` / `gone_names` reporting — those are separate axes, `README.md:750-755` documents them,
and `gone_names` is computed from the same `proj_json` the plan's first draft proposed to delete. It
adds one line, and it does **not** set `ok=0`: this plan ships no repair, so failing the check would
name an action that does not exist and leave every drifted machine permanently red.

- [ ] **Step 1: Write the failing test**

Test 19 in `tests/install-smoke.sh` already installs the double this needs. `FAKESTALECORT`
(`tests/install-smoke.sh:653-661`) answers `hook-install` with an error and **every other
subcommand** with the single line `cort 0.1.0 (rust)` — which for `projects --verdict` is a
successful exit carrying an unparsable verdict. That is exactly the input the `case *)` arm exists
for, so no new arm in the double is needed.

Two things the assertion must get right, both of which the first draft of this plan got wrong:

- `assert_contains` takes a **file path** first, not a string: it runs `grep -qF "$2" "$1"`
  (`tests/install-smoke.sh:25-27`), and every existing call passes a file. Test 19 writes its output
  to `/tmp/smoke19.log`, so that is the first argument.
- It must be placed **before** `cp "$TMPHOME/cort.real" "$MANAGED_CORT"` (`tests/install-smoke.sh:674`),
  which restores the real binary. After that line the double is gone and the assertion tests nothing.

Insert immediately after the existing `hook: could not read` assertion block, just above the restore:

```bash
assert_contains /tmp/smoke19.log "indexes: compatibility unknown (unparsable verdict)" \
  "--check reads an unparsable verdict as unknown, never as current"
assert_not_contains /tmp/smoke19.log "schema and extractor current" \
  "--check must not claim compatibility it could not establish"
```

- [ ] **Step 2: Run it to verify it fails**

Run: `bash tests/install-smoke.sh`
Expected: FAIL — the string is absent.

- [ ] **Step 3: Write minimal implementation**

In `install.sh`, immediately after the existing `stale_names` / `gone_names` block (which stays
exactly as it is), add:

```bash
  # A second axis, and the one that was missing on 2026-09-05: `stale` compares git heads, and no
  # head had moved. `--verdict` answers whether the index was built by the schema and extractor this
  # binary uses. It is a report, not a decision -- there is no repair to run yet -- so it never sets
  # ok=0. Fail closed on a line we cannot parse: anything but the three known words is `unknown`.
  # Two counts, because one number cannot stand behind both words: saying "3 built by a superseded
  # extractor" when two of the three are databases we could not open is a false statement about them.
  local verdict_line vfield vword vdrift vunread
  if verdict_line="$("$managed_cort" projects --verdict 2>/dev/null)"; then
    IFS=$'\t' read -r vfield vword vdrift vunread <<<"$verdict_line"
    case "$vfield/$vword" in
      indexes/compatible) echo "indexes: schema and extractor current" ;;
      indexes/drifted)    echo "indexes: $vdrift built by a superseded schema or extractor, $vunread unreadable" ;;
      indexes/unknown)    echo "indexes: compatibility unknown ($vunread could not be read)" ;;
      *)                  echo "indexes: compatibility unknown (unparsable verdict)" ;;
    esac
  else
    echo "indexes: compatibility unknown — installed cort predates \`cort projects --verdict\`"
  fi
```

Update `README.md:750-755` to add the second axis and say plainly that neither fails the check.

- [ ] **Step 4: Run it and verify**

```bash
bash tests/install-smoke.sh
./install.sh --check | grep indexes
```
Expected: the smoke test passes; the real check prints both the existing `indexes:` line and the new
compatibility line.

- [ ] **Step 5: Verify the fallback branch**

`managed_cort` is read from the manifest (`install.sh:829-834`, `manifest_get cort_bin`), not the
environment, so point that key at a stub and restore it afterwards:

```bash
MANIFEST="${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung/manifest"
cp "$MANIFEST" /tmp/manifest.bak
STUB=$(mktemp); printf '#!/bin/sh\nexit 2\n' > "$STUB"; chmod 755 "$STUB"
sed -i "s|^cort_bin:.*|cort_bin:$STUB|" "$MANIFEST"
./install.sh --check 2>&1 | grep indexes
cp /tmp/manifest.bak "$MANIFEST"
```
Expected: the `predates` line, and the check does not crash.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
bash tests/install-smoke.sh
```
Expected: all exit 0.

- [ ] **Step 7: Commit**

```bash
git add install.sh README.md tests/install-smoke.sh
git commit -m "feat(install): --check reports schema and extractor compatibility

stale compares git heads, and on 2026-09-05 no head had moved in any project
tree, so seven indexes built by a superseded extractor were reported as all
current. The check now also prints the compatibility verdict the binary
computes.

It does not fail the check. This ships no repair, so a red check would name
an action that does not exist and every drifted machine would stay red --
and a permanently red check trains its reader to ignore it. An unparsable or
unavailable verdict reads as unknown, never as current."
```

---

## What round 2 changed

A second review round, all five findings verified against source before accepting.

1. **Task 4's failing test could never have passed.** `assert_contains` takes a **file path** first
   (`grep -qF "$2" "$1"`, `tests/install-smoke.sh:25-27`) and the plan passed shell output; and it
   asserted an `unknown` message while telling the executor to make the double return *drift*, so
   following the plan literally would have driven the implementation to print `unknown` for a real
   drift verdict — the exact failure the line exists to catch. It also had to run before the double
   is restored at `:674`. Rewritten, and simplified: the existing `FAKESTALECORT` double already
   emits an unparsable line, so no new arm is needed.
2. **Task 2's own commit was red.** It renamed the return type and named only `db.rs:233` as needing
   an update — but Task 1's *own new test* reads `rows[0].extractor_version`, so Task 2 Step 4 would
   have hit a compile error where the plan promised PASS. Both tests are now named, with the
   binding written out.
3. **The verdict's single count could not stand behind its word.** `drifted 3` where two of the
   three are unreadable databases is a false statement about them; and a drifted index whose
   directory is gone was reported as `compatible` while its own JSON row said `"drifted": true`.
   The line now carries **two** counts, each answerable on its own, and drift is counted as the fact
   it is regardless of whether the tree still exists. Two new tests pin the precedence and the gone
   case — Kimi noted an implementation letting unreadable force `unknown` passed every earlier test.
4. **Task 1's second deliberate break could never go red.** A junk `usage.db` opens fine (SQLite is
   lazy — verified empirically) and is then swallowed by `if let Ok(row)`, so the skip is invisible
   until Task 2 gives the failure a variant. The break moved to Task 2, where it also takes the
   recorder-isolation tests red with it.
5. **Three citations were wrong and one claim overclaimed.** The extractor meta write is
   `indexer.rs:436-437`, not `:416-419`; `RootProbe::Unreadable` is produced at `db.rs:544` and
   `:556`, not `:559-562` (that is the `RootUnreadable` struct); and "predicts the rebuild decision"
   was too strong — `incremental_index` only rebuilds on mismatch when a stored value *exists*
   (`incremental.rs:312-318`), so the report and the rebuild diverge precisely on the predates-meta
   population. Recorded rather than papered over.

Also fixed by self-check, before this round: `ProjectsArgs` was missing `disable_help_flag` and
`disable_version_flag`, which every args struct in `main.rs` carries (`main.rs:391-394`).

## What round 1 changed

Recorded so an executor does not reintroduce one. All six were verified against source before being
accepted.

1. **`usage.db` was the blocker.** It shares the cache directory and ends in `.db`
   (`usage.rs:100-109`). Giving unreadable databases a variant without excluding it first would have
   made every ordinary machine report a bogus unreadable index and would have broken the three
   `rust/tests/usage.rs` recorder-isolation tests — which exist precisely to assert that `usage.db`
   cannot change `cort projects` stdout — plus `rust/tests/cli.rs:661`. The exclusion moved into
   Task 1, before the variant exists.
2. **`Format::Lean` does not produce TSV on its own.** `render` falls through to `pretty(payload)`
   for any command without an arm (`render.rs:406-413`). Task 3 now includes `render.rs` and the
   renderer, and a deliberate break that removes the arm.
3. **The first draft deleted the `awk` that also computed `gone_names`.** Task 4 now adds a line and
   deletes nothing.
4. **`current` was doing too much work.** It ignored git staleness and `graph_pending` while calling
   itself current, and it collapsed `Unreadable` into `drifted` — which the spec forbids, because an
   unreadable index is an absence of facts. Three words now, each defined, with a test per word.
5. **A `meta_on(...).ok()` helper would have flattened a lock error, a missing table and a corrupt
   page into "key absent".** `get_meta` already exists and preserves the distinction (`db.rs:153`).
6. **Two false claims of my own, corrected.** The plan said Task 2's move ambiguity was for "the
   compiler to say" — it is a clone in one arm and a move in the other, written out now. And it said
   the index block sits inside `if [ -x "$managed_cort" ]`; that guard closes after
   `check_all_hooks` at `install.sh:841`, and the index block at `:873` is inside
   `if [ -f "$MANIFEST_FILE" ]`. I had told the user the opposite.

## Self-Review

**Spec coverage.** Implements the diagnosis half of spec §2 (row 7, plus rows 8's exclusion as a
side effect, plus the "unreadable ≠ absent" rule) and §1's build order. Spec §2 rows 1-6 and 9, and
§§3-6, are deferred to plans 2 and 3 and named under **Scope**.

**Known gap, recorded rather than hidden.** `extractor_version` is stored twice — in
`projects.extractor_version` and in `_cortex_meta` — written in one transaction by
`indexer.rs:436-437`, beside the projects-row upsert at `:377-381`. This plan reads the `_cortex_meta` copy because that is what
`incremental_index` compares against. The duplication is an anti-drift violation inside the database
and belongs in plan 3.

**Placeholders:** none remain; Task 2's `cmd_projects` is written out in full rather than elided.

**Type consistency:** `ProjectEntry` and `ProjectListRow` field names are identical across Tasks 1-3;
the four-field TSV contract is defined in Task 3 and consumed in Task 4.
