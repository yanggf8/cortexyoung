# Index Drift Diagnosis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make schema and extractor drift in every indexed project visible, and make an unreadable index impossible to mistake for an absent one.

**Architecture:** `list_projects` currently reads only the `projects` row and silently drops any database it cannot open. It gains the two version facts that actually decide whether an index is usable (`SCHEMA_VERSION`, `extractor_version`, both from `_cortex_meta`), and its return type gains a variant for a database that exists and will not answer. `cort projects` then reports drift per row, and a new `--verdict` emits one lean TSV line so the single "are the indexes current" decision lives in Rust rather than in an `awk` filter inside `install.sh`.

**Tech Stack:** Rust (`rust/` crate), rusqlite, SQLite, `tempfile` + `serial_test`-style env guards already present in `rust/tests/db.rs`.

**Spec:** `docs/superpowers/specs/2026-09-06-cort-upgrade-design.md`

## Global Constraints

- Repo is pure Rust; the only executable Bash is `install.sh` and `tests/install-smoke.sh`. No logic may be added to either.
- Run `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` in **both** `rust/` and `evals/` before every commit, and let a non-zero exit stop the commit (do not end a verification pipeline in `tail`).
- Run `cargo test --locked --all-targets` in **both** crates; the `evals` crate depends on `cort` and breaks on signature changes that `rust/` alone does not catch.
- Storage failures are returned, never panicked on (`db.rs` may not gain an `.expect()` on a storage call).
- "Unreadable" is never reported as "absent" — the existing precedent is `RootProbe::Unreadable` (`rust/src/db.rs:559-562`).
- No absolute developer paths anywhere, including fixtures.
- `--lean` TSV output must never contain an empty field (tab is IFS whitespace; `read` collapses runs of tabs).

## Scope

This is **plan 1 of 3** from the spec's own build order (§1, "診斷先,執行者後"). It ships working software on its own: after it, the 2026-09-05 incident (7 of 10 projects on a stale extractor while `--check` said "all current") is visible on day one.

Not in this plan, and deliberately so:
- **Plan 2 — refusal and repayment:** extend `graph_pending` into a reason set with a target generation, bind it to `index_is_stale`, give `incremental_index` a typed call policy so the edit hook stops performing full rebuilds and stops being the structural migrator (spec §4).
- **Plan 3 — the upgrader:** generation directory + symlink flip, the two `flock`s, ordering, verdict taxonomy and escape hatches, and moving the shim template / ast-grep pin / manifest key-set to a single home (spec §1, §3, §5, §6).

---

## File Structure

- `rust/src/db.rs` — `ProjectListRow` gains two fields; new `ProjectEntry` enum; `list_projects` reads `_cortex_meta` and reports unreadable databases. This is the only file that knows how a project database is probed.
- `rust/src/main.rs` — `cmd_projects` renders the new facts and computes `drifted` against the running binary's own versions; `cmd_delete`'s registry fallback filters to readable entries. No new knowledge lives here.
- `rust/tests/db.rs` — unit-level tests for the two new facts and the unreadable variant.
- `rust/tests/cli.rs` — end-to-end tests for `cort projects` output and `--verdict`.
- `install.sh` — one `awk` pipeline is **deleted** and replaced by printing the binary's verdict line. Bash loses logic; it gains none.

---

### Task 1: `list_projects` reports each index's schema and extractor version

**Files:**
- Modify: `rust/src/db.rs:333-382`
- Test: `rust/tests/db.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `ProjectListRow` with two new public fields, `pub schema_version: Option<String>` and `pub extractor_version: Option<String>`. Task 2 wraps this struct; Task 3 renders it.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/db.rs`:

```rust
/// The two facts that decide whether an index is usable are stored in `_cortex_meta`, and until
/// 2026-09-06 nothing that enumerates projects read either of them. That is why 7 of 10 projects
/// sat on a superseded extractor while `--check` reported "all current".
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
        drop(db);

        let rows = list_projects();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].extractor_version.as_deref(),
            Some("stale-extractor"),
            "the extractor the index was built with must be reported, not inferred"
        );
        assert_eq!(
            rows[0].schema_version.as_deref(),
            Some("5"),
            "ensure_schema stamps the current SCHEMA_VERSION: {:?}",
            rows[0].schema_version
        );
    });
}
```

Add `set_meta` to the `use cort::db::{...}` list at the top of `rust/tests/db.rs` if it is not already imported.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test db list_projects_reports_the_schema_and_extractor -- --nocapture`
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
    /// The schema this database was last stamped at, from `_cortex_meta`. `None` means the key is
    /// absent, which is a database that predates the meta table -- not a database that is current.
    pub schema_version: Option<String>,
    /// The extractor identity the derived rows were built with, from `_cortex_meta`. This is the
    /// value `incremental_index` compares against (`incremental.rs:310`), so reading the same key
    /// is what makes this report predict the rebuild decision rather than guess at it.
    pub extractor_version: Option<String>,
}
```

Inside `list_projects`, after the `projects` row is read successfully, read the two keys from the
same read-only connection. Add a small helper beside it so the `SELECT` is written once:

```rust
fn meta_on(db: &Connection, key: &str) -> Option<String> {
    db.query_row(
        "SELECT value FROM _cortex_meta WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}
```

and build the row with:

```rust
                    schema_version: meta_on(&db, "SCHEMA_VERSION"),
                    extractor_version: meta_on(&db, "extractor_version"),
```

Note the closure passed to `query_row` cannot borrow `db` while `db.query_row` holds it, so read the
two meta values into locals **before** the `projects` query and move them into the struct:

```rust
        let schema_version = meta_on(&db, "SCHEMA_VERSION");
        let extractor_version = meta_on(&db, "extractor_version");
        let row = db.query_row(
            "SELECT project_id, name, path, git_head, last_indexed_at FROM projects",
            [],
            |r| {
                Ok(ProjectListRow {
                    project_id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    git_head: r.get(3)?,
                    last_indexed_at: r.get(4)?,
                    db_path: db_path_str.clone(),
                    schema_version: schema_version.clone(),
                    extractor_version: extractor_version.clone(),
                })
            },
        );
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test db list_projects_reports_the_schema_and_extractor`
Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Temporarily change `meta_on(&db, "extractor_version")` to `None`, re-run the test, and confirm it
goes RED with `Some("stale-extractor")` vs `None`. Restore the line. A fixture that cannot detect
its own regression has been the finding of three consecutive external reviews on this repo; this
step is not optional.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0. `evals` compiles against `cort`; a struct field addition is source-compatible
only if nothing constructs `ProjectListRow` literally outside `db.rs`. If something does, add the
two fields there.

- [ ] **Step 7: Commit**

```bash
git add rust/src/db.rs rust/tests/db.rs
git commit -m "feat(db): report the schema and extractor each index was built with

The two facts that decide whether an index is usable live in _cortex_meta,
and nothing that enumerates projects read either of them. That is why 7 of
10 projects sat on a superseded extractor while --check said all current.

extractor_version is read from the same key incremental_index compares
against (incremental.rs:310), so the report predicts the rebuild decision
rather than guessing at it."
```

---

### Task 2: an index that will not answer stops being invisible

**Files:**
- Modify: `rust/src/db.rs:342-382`
- Modify: `rust/src/main.rs:1613` (`cmd_projects`), `rust/src/main.rs:1657` (`cmd_delete`)
- Test: `rust/tests/db.rs`

**Interfaces:**
- Consumes: `ProjectListRow` with the two fields from Task 1.
- Produces: `pub enum ProjectEntry { Indexed(ProjectListRow), Unreadable { db_path: String, reason: String } }` and `pub fn list_projects() -> Vec<ProjectEntry>`. Task 3 renders both variants.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/db.rs`:

```rust
/// A database file that exists and will not open is not an absent project. `list_projects` used to
/// `continue` past it, which is exactly the conflation `RootProbe::Unreadable` exists to prevent one
/// level down (`db.rs:559-562`) -- and it would have hidden the very indexes an upgrade must repair.
///
/// A file with no `projects` row is a different thing and must stay skipped: `ensure_schema` creates
/// that shape the first time anything opens a project, and `status_of` already reports it as not
/// indexed.
#[test]
fn an_unopenable_index_is_reported_rather_than_skipped() {
    let _g = env_guard();
    let cache = tempfile::tempdir().unwrap();
    let cache_s = cache.path().to_str().unwrap().to_string();
    with_var("CORT_CACHE_DIR", Some(&cache_s), || {
        // Not a database at all, but it ends in `.db`, which is the only thing the scan filters on.
        std::fs::write(cache.path().join("deadbeef.db"), b"this is not a sqlite file").unwrap();

        // A schema-only database: openable, no `projects` row. Must NOT be reported.
        let empty_root = tempfile::tempdir().unwrap();
        let empty_s = empty_root.path().to_str().unwrap();
        let db = open_db(db_path_for(empty_s)).unwrap();
        ensure_schema(&db).unwrap();
        drop(db);

        let entries = list_projects();
        let unreadable: Vec<_> = entries
            .iter()
            .filter_map(|e| match e {
                ProjectEntry::Unreadable { db_path, .. } => Some(db_path.clone()),
                ProjectEntry::Indexed(_) => None,
            })
            .collect();
        assert_eq!(unreadable.len(), 1, "entries: {entries:?}");
        assert!(unreadable[0].ends_with("deadbeef.db"), "{unreadable:?}");

        let indexed = entries
            .iter()
            .filter(|e| matches!(e, ProjectEntry::Indexed(_)))
            .count();
        assert_eq!(indexed, 0, "a schema-only database is not a project: {entries:?}");
    });
}
```

Add `ProjectEntry` to the `use cort::db::{...}` list in `rust/tests/db.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test db an_unopenable_index_is_reported`
Expected: FAIL to compile — `cannot find type 'ProjectEntry'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/db.rs`:

```rust
/// One entry from the cache directory scan. `Unreadable` is its own variant rather than an omission
/// so a caller cannot read a database that exists and will not answer as "nothing here" -- the same
/// discipline as `RootProbe::Unreadable`, one level up.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectEntry {
    Indexed(ProjectListRow),
    Unreadable { db_path: String, reason: String },
}
```

Change the signature to `pub fn list_projects() -> Vec<ProjectEntry>` and replace the two silent
skips:

```rust
        let db = match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(db) => db,
            Err(e) => {
                out.push(ProjectEntry::Unreadable {
                    db_path: db_path_str,
                    reason: e.to_string(),
                });
                continue;
            }
        };
```

and, after the `projects` query:

```rust
        match row {
            Ok(row) => out.push(ProjectEntry::Indexed(row)),
            // No row is not a failure: `ensure_schema` creates this shape before anything is
            // indexed, and a schema-only database is correctly not a project.
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => out.push(ProjectEntry::Unreadable {
                db_path: db_path_str,
                reason: e.to_string(),
            }),
        }
```

`db_path_str` is moved in two arms, so clone it once at the top of the loop body or borrow it into
each arm — the compiler will say which.

Update the two callers in `rust/src/main.rs`.

`cmd_delete` (around `main.rs:1657`) needs only readable rows:

```rust
            if let Some(row) = cort::db::list_projects()
                .into_iter()
                .filter_map(|e| match e {
                    cort::db::ProjectEntry::Indexed(r) => Some(r),
                    cort::db::ProjectEntry::Unreadable { .. } => None,
                })
                .find(|r| r.path.trim_end_matches('/') == want)
```

`cmd_projects` (around `main.rs:1613`) is rewritten in Task 3; for this task make it compile by
mapping `Unreadable` to a row of its own:

```rust
    let rows: Vec<Value> = list_projects()
        .into_iter()
        .map(|entry| match entry {
            ProjectEntry::Unreadable { db_path, reason } => json!({
                "db_path": db_path,
                "unreadable": reason,
            }),
            ProjectEntry::Indexed(r) => {
                // ... existing body unchanged ...
            }
        })
        .collect();
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test db an_unopenable_index_is_reported`
Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Temporarily restore `Err(_) => continue` on the open failure, re-run, and confirm the assertion on
`unreadable.len()` goes RED with `0`. Restore.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0. The existing `list_projects_enumerates_every_indexed_project_in_the_cache_dir`
test (`rust/tests/db.rs:233`) constructs no `ProjectEntry` but indexes `rows[0].path`; update it to
match on the variant.

- [ ] **Step 7: Commit**

```bash
git add rust/src/db.rs rust/src/main.rs rust/tests/db.rs
git commit -m "feat(db): an index that will not answer is reported, not skipped

list_projects dropped every database it could not open, so the indexes most
worth repairing were the ones the scan could not see. ProjectEntry gives
that failure a variant of its own -- the same discipline as
RootProbe::Unreadable one level down.

A database with no projects row stays skipped: ensure_schema creates that
shape before anything is indexed, so it is correctly not a project."
```

---

### Task 3: `cort projects` says whether each index drifted, and `--verdict` says it once

**Files:**
- Modify: `rust/src/main.rs:1611-1640` (`cmd_projects`)
- Test: `rust/tests/cli.rs`

**Interfaces:**
- Consumes: `ProjectEntry`, and `ProjectListRow.schema_version` / `.extractor_version` from Tasks 1-2.
- Produces: per-row JSON fields `schema_version`, `extractor_version`, `drifted` (bool), and rows of shape `{"db_path": ..., "unreadable": ...}`; plus `cort projects --verdict` printing one lean TSV line, either `indexes\tcurrent\t<n>` or `indexes\tdrifted\t<n>` (`<n>` is the count of projects not current). Plan 3's upgrader consumes both.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`:

```rust
/// The incident in one assertion: a project whose stored extractor is not the one this binary
/// would use must say so, and the one-line verdict must not say "current".
#[test]
fn projects_reports_extractor_drift_and_the_verdict_refuses_to_call_it_current() {
    let (_p, cwd, _c, cache) = sandbox();
    git_in_fixture(&cwd);
    let idx = run_cort(&["index"], &cwd, &cache);
    if idx.code != 0 {
        eprintln!("SKIP: index failed (ast-grep unavailable?): {}", idx.stderr);
        return;
    }
    // Move this index onto an extractor this binary does not have. Nothing else changes: the tree
    // is clean and the git head is unmoved, which is exactly the shape that read as fresh on
    // 2026-09-05.
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
    assert_eq!(
        row.get("drifted").and_then(Value::as_bool),
        Some(true),
        "a stored extractor this binary does not use is drift: {row}"
    );
    assert_eq!(
        row.get("stale").and_then(Value::as_bool),
        Some(false),
        "git head did not move -- this is the case the old check called `all current`"
    );

    let v = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(v.code, 0, "{}", v.stderr);
    assert!(
        v.stdout.contains("drifted"),
        "the one-line verdict must not report current: {:?}",
        v.stdout
    );
    assert!(
        !v.stdout.lines().any(|l| l.contains("\t\t")),
        "lean TSV must never carry an empty field: {:?}",
        v.stdout
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test cli projects_reports_extractor_drift`
Expected: FAIL — either `unexpected argument '--verdict'` or a missing `drifted` field.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/main.rs`, give `cmd_projects` a `--verdict` flag (a new `#[derive(Parser)]` args struct
beside `FormatOnlyArgs`, with `#[arg(long)] verdict: bool`), and compute drift against the running
binary:

```rust
    let want_schema = cort::db::SCHEMA_VERSION.to_string();
    let want_extractor = cort::pack::extractor_version();
```

For each `Indexed` row, `drifted` is true when either stored value is absent or differs:

```rust
            let drifted = r.schema_version.as_deref() != Some(want_schema.as_str())
                || r.extractor_version.as_deref() != Some(want_extractor.as_str());
```

Add `"schema_version"`, `"extractor_version"` and `"drifted"` to the emitted object. An `Unreadable`
entry emits `{"db_path": ..., "unreadable": <reason>}` and counts as not current.

For `--verdict`, emit a lean single line instead of the array. Count projects that are `drifted` or
`Unreadable`; a project whose directory is gone is **not** counted:

```rust
    if a.verdict {
        let word = if not_current == 0 { "current" } else { "drifted" };
        return Ok(Emit {
            render_command: Some("projects"),
            format: Format::Lean,
            payload: json!({ "indexes": word, "count": not_current }),
        });
    }
```

Render it as `indexes\t<word>\t<count>` — never an empty field, because `count` is always present.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test cli projects_reports_extractor_drift`
Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Temporarily hard-code `let drifted = false;`, re-run, and confirm the `drifted` assertion goes RED
**and** the `--verdict` assertion goes RED. Restore. If only one goes red, the verdict is not
derived from the same computation and must be.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: both exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/main.rs rust/tests/cli.rs
git commit -m "feat(projects): report extractor and schema drift, and say it once

stale answers whether the git head moved. On 2026-09-05 it had not moved in
any project tree, so every drifted index read as fresh. drifted answers the
other question -- was this index built by the extractor and schema this
binary uses -- and --verdict reduces both to one lean line so the decision
has one home rather than an awk filter in bash."
```

---

### Task 4: `install.sh --check` stops computing the answer and prints the one it is given

**Files:**
- Modify: `install.sh:873-899`
- Test: `tests/install-smoke.sh` is not the right level; assert instead in `rust/tests/cli.rs` that the verdict line is stable, and verify the installer change by running `./install.sh --check`.

**Interfaces:**
- Consumes: `cort projects --verdict` from Task 3.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/cli.rs`:

```rust
/// `install.sh --check` parses this line with `read`, so its shape is a contract: exactly three
/// tab-separated non-empty fields, and the second field is the word a human reads.
#[test]
fn the_projects_verdict_line_has_exactly_three_non_empty_fields() {
    let (_p, cwd, _c, cache) = sandbox();
    let r = run_cort(&["projects", "--verdict"], &cwd, &cache);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let line = r.stdout.lines().next().expect("one line").to_string();
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(fields.len(), 3, "line was {line:?}");
    assert!(fields.iter().all(|f| !f.is_empty()), "line was {line:?}");
    assert_eq!(fields[0], "indexes");
    assert!(
        fields[1] == "current" || fields[1] == "drifted",
        "line was {line:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test cli the_projects_verdict_line_has_exactly_three`
Expected: PASS if Task 3 rendered it correctly, FAIL otherwise. If it passes immediately, break the
renderer (emit two fields) and confirm it goes RED, then restore — the point of this test is that
the shape is pinned before bash depends on it.

- [ ] **Step 3: Write minimal implementation**

In `install.sh`, delete the `stale_names` `awk` pipeline (`install.sh:880-891`) and replace the
verdict with the binary's own:

```bash
  local verdict_line kind count
  if verdict_line="$("$managed_cort" projects --verdict 2>/dev/null)"; then
    IFS=$'\t' read -r _ kind count <<<"$verdict_line"
    if [ "$kind" = "drifted" ]; then
      echo "indexes: $count not current — run the upgrade"
      ok=0
    else
      echo "indexes: all current"
    fi
  else
    echo "indexes: could not query — installed cort predates \`cort projects --verdict\`"
  fi
```

Keep the existing `gone_names` block, which reports directories that no longer exist and does not
fail the check. Note that the fallback message at `install.sh:898` currently names a
`cort projects --stale` flag **that does not exist**; this replaces it with a flag that does.

- [ ] **Step 4: Run it and verify**

```bash
./install.sh --check
```
Expected: the `indexes:` line is printed. On a machine whose indexes are all current it reads
`indexes: all current`; drift a project (`sqlite3` is not available, so use
`cort projects` to confirm which) and it reads `indexes: N not current`.

- [ ] **Step 5: Verify the fallback branch**

`managed_cort` is not an environment variable — it is read from the manifest
(`install.sh:829-834`, `manifest_get cort_bin`, falling back to `$BIN_DIR/cort`). So exercise the
branch by pointing that key at a stub that does not know the flag, and put it back afterwards:

```bash
MANIFEST="${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung/manifest"
cp "$MANIFEST" /tmp/manifest.bak
STUB=$(mktemp); printf '#!/bin/sh\nexit 2\n' > "$STUB"; chmod 755 "$STUB"
sed -i "s|^cort_bin:.*|cort_bin:$STUB|" "$MANIFEST"
./install.sh --check 2>&1 | grep indexes
cp /tmp/manifest.bak "$MANIFEST"
```
Expected: the `could not query` line, and the check does not crash.

Note the surrounding guard: the whole block is inside `if [ -x "$managed_cort" ]`
(`install.sh:835`), so a **missing** managed binary skips the index report entirely and can still
reach `check: OK`. That is a real defect, it is not introduced by this task, and it is listed for
plan 3 — do not fix it here, and do not let this task's test appear to cover it.

- [ ] **Step 6: Run both crates and lint**

```bash
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
bash tests/install-smoke.sh
```
Expected: all exit 0.

- [ ] **Step 7: Commit**

```bash
git add install.sh rust/tests/cli.rs
git commit -m "fix(install): --check prints the verdict rather than computing one

The awk filter extracted only \"stale\": true, so a machine with seven
indexes on a superseded extractor was reported as \"all current\". The
decision now has one home in Rust and bash prints it. The fallback message
also stops naming cort projects --stale, a flag that never existed."
```

---

## Self-Review

**Spec coverage.** This plan implements the diagnosis half of spec §2 (rows 7 and the "unreadable ≠
absent" rule) and the §1 build-order decision. Spec §2 rows 1-6, 8 and 9 (shim, ast-grep pin,
manifest key-set, `usage.db`, `hook-gate`), §3, §4, §5 and §6 are explicitly deferred to plans 2 and
3 and are listed under **Scope** so nothing is silently dropped.

**Known gap, recorded rather than hidden:** `extractor_version` is stored twice — in
`projects.extractor_version` and in `_cortex_meta` — and `indexer.rs:416-419` writes both in one
transaction. This plan reads the `_cortex_meta` copy because that is the one `incremental_index`
compares against. The duplication itself is an anti-drift violation inside the database and belongs
in plan 3.

**Placeholders:** none. Every step carries the code it needs.

**Type consistency:** `ProjectEntry` and `ProjectListRow` field names are used identically in Tasks
1-3; `--verdict`'s three-field TSV shape is defined in Task 3 and asserted in Task 4.
