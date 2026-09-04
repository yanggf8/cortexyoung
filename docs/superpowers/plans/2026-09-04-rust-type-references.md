# Rust Type References Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cort impact --symbol <RustType>` answer for a `struct`, `enum` or `trait` the way it
already answers for a function — a dependent set, each row carrying the line that names the type, and
a `--coverage` screen that can say what it missed.

**Architecture:** Three new Rust pack rules (a `chunk:class` on type declarations; two
`edge:references:type` rules that mirror the existing `bare`/`scoped` split of the call rules), a new
`rel_type` (`references`) and a new `call_form` (`type`) threaded through schema v5, and the four
hardcoded `rel_type = 'calls'` filters widened. No new resolution machinery: a **qualified** type
reference keeps its path and resolves through the module-suffix cascade that already handles
`crate::m::f()`, and a bare one resolves as a bare name.

**Tech Stack:** Rust (`rust/` and `evals/` crates), `ast-grep` 0.45.2 rule pack (`src/pack/rules/`), SQLite.

**Spec:** This document. §Evidence and §Design are the spec; there is no separate file.

**Revision:** v2.1, 2026-09-04. v1 was reviewed by Codex (`01a06ae4-966c-7d20-858c-0aafbc7bac68`, plus
a quota-truncated pass `01a06acb-a7ab-76e0-95b7-9af24f6458c5`) and independently corroborated against
source; it had one stop-ship defect and ten lesser ones. v2 was reviewed again
(`01a06b06-8a36-7be0-911f-638af0ddc623`, truncated by quota after confirming the pack rules) and the
remaining checks were completed by hand against the shipping binary. §Review record lists all
eighteen findings and where each is handled. **Do not execute v1.**

**Verification status of this revision.** The three pack rules, the two coverage fixtures, and the
`chunk_specificity` ordering were validated empirically (probes quoted inline at D2, D7 and Task 5).
`migrate_v5` was reviewed by Kimi against rusqlite and this schema and cleared on every axis it was
attacked on — manual transaction control through `execute_batch`, the RENAME rewrite, crash windows,
and the two-process race — with four improvements adopted (review record 19-22). The two residual
questions are closed: `evals/src/recall.rs` never opens the database (zero SQL in the file; it
re-scans source text), so the receiver-gate denominators cannot move; and no existing neighbour test
breaks, because every fixture that pins neighbour output indexes TypeScript while these rules are
`language: Rust`.

**What is still unproven, stated plainly.** Nothing has been executed. Every task below is a
prediction about what the tests will do, and Task 3 remains the one whose failure is not recoverable
by re-running.

---

## Evidence (why this and not something else)

Every number was produced on this machine on 2026-09-04 and is reproducible with the command shown.

1. `cort-evals hook-probe` over the real transcript tree — 57,158 commands, 4,821 searches, 468
   sessions, 22 projects — would fire 236 times. Of those, **148 (63%) are `rejected_not_a_function`**.
   Classified conservatively by name shape, **44 of the 148 are provably not functions**: 26 CamelCase
   (`FeedSpec`, `CdpError`, `D1ExecResult`, `HttpRequest`, `QuizSubmission`, `SkillStatus::Degraded`,
   `StellarHazard`, `UnitDestroyed`) and 18 SCREAMING_CASE (`TIMEOUT_S`, `NO_NEWS`, `COST_SERIES`).
   This plan addresses the CamelCase half.
2. The shipping hook has **no function gate**. `judge` (`rust/src/hook.rs:329-373`) checks symbol
   shape, context flags, indexed extensions and cross-file shape — nothing asks whether the symbol is
   callable. So in production the hook already fires on `FeedSpec` and suggests
   `cort impact --symbol FeedSpec`, which today returns `seeds=0`. `rejected_not_a_function` is the
   **eval harness's** post-hoc label (`evals/src/hook.rs:32`), not a production refusal — and
   hook-probe's own `index_check_reading` says the check is "reported, NOT applied".
3. Live confirmation, on two symbols `CLAUDE.md` itself calls load-bearing:

   ```
   $ cort impact --symbol CallForm --depth 1 --coverage -f lean
   # impact CallForm depth=1 seeds=0 dependents=0 stale=false
   coverage	no_seed_resolved	not a clean answer: nothing was looked at
   ```
   Cause: `src/pack/rules/rust.yml` has only `free-function`, `impl-method` and
   `trait-default-method` chunk rules. Rust has no type chunk at all.
4. Language breadth is **not** the bottleneck. Every project that fired is inside the three supported
   languages: GalaxyWarHero `rs:77`, travel-2026 `rs:269 ts:131 py:31`, gwebcdb `rs:150 py:54`,
   ft `rs:76`, finance-engineering `rs:72`, hesocial `ts:86 rs:86 tsx:37`.

**Why this is on the main line.** `CLAUDE.md` gates features on "a change that makes an answer cheaper
to verify is on the main line; a feature that only makes answers more numerous is not." A type
reference is the same caller-set question asked of a non-function symbol, carries the same
`call_site_line`, and is graded by the same `verify-impact`. It makes a currently-refused question
answerable *and* checkable.

## Design

### D1 — `chunk:class`, not a new `chunk_type`

`chunks.chunk_type` is CHECK-constrained to
`('function','class','method','config','documentation','unparsed')` (`rust/src/schema.sql:21`). Rust
struct/enum/trait declarations are emitted as **`chunk:class`**, the bucket JS/TS/Python type
declarations already use, so `chunks` and its external-content FTS mirror are never rebuilt.
`compose_symbol_name` (`rust/src/chunker.rs:346`) special-cases only `"method"`, so a `class` record
keeps its bare `$NAME` with no code change.

Verified compatible downstream: the CHECK accepts it (`schema.sql:21`), `context` carries arbitrary
`chunk_type` strings through to render (`context.rs:346`, `render.rs:308`), and FTS indexes
content/symbol/path but not `chunk_type` (`schema.sql:93-110`).

Cost: `cort struct` calls a Rust `trait` a `class`. Accepted — misleading metadata, not structural
corruption, and the alternative is rebuilding the largest table plus its FTS mirror for a label.

### D2 — two reference rules, mirroring the two call rules  ← **the v1 stop-ship**

v1 had a single rule capturing `type_identifier`. That silently discards the qualifier. Measured:

```
fn a(e: settings::SettingsError)              → captured 'SettingsError'
fn b(e: crate::settings_toml::SettingsError)  → captured 'SettingsError'
```

This repo really does declare that name twice — `rust/src/settings.rs:52` and
`rust/src/settings_toml.rs:74` — and `main.rs:1025`/`:1036` reference them by qualified path. With the
qualifier gone, `resolve_targets`'s module-suffix branch (`graph.rs:594`, which only runs when the
target contains `::`) never fires, both definitions match, and both are written as AMBIGUOUS
relationships (`graph.rs:701-733`).

That phantom is invisible to all three of this product's checks: `impact` does not emit confidence
(`impact.rs:232-247`), coverage treats any relationship to the seed as resolved (`coverage.rs:397`),
and `verify-impact` only asks whether the printed line contains the leaf word (`verify.rs:27-38`).
`CLAUDE.md`'s standard — a missing edge is a reported gap, a phantom edge is worse — forbids it.

The fix is already the shape of this file: the call rules are **two** rules, `edge:calls:bare` on
`identifier` and `edge:calls:scoped` on `scoped_identifier`. References get the same split. A
qualified target then keeps its path and resolves through the existing cascade —
`split_call_path("settings::SettingsError")` → `(["settings"], "SettingsError")` → matches only the
chunk whose module path ends in `settings`. No new resolution machinery, exactly as claimed, but only
*with* the scoped rule.

Both rules emit `call_form: type`. Resolution branches on whether the stored target contains `::`
(`graph.rs:594`), not on the form, so one form is correct for both.

**Probed against ast-grep 0.45.2, 2026-09-04.** `kind: scoped_type_identifier` with `pattern: $CALLEE`
captures the full text (`settings::SettingsError`, and `<Foo as Bar>::Baz` whole). `not: any: [...]`
is accepted. `Vec<settings::SettingsError>` behaves correctly: `Vec` emits bare, `SettingsError` is
suppressed by the second `not` clause, and the scoped rule emits the qualified target once — no
double emission.

Two knowingly-accepted losses from this shape:
- In `<Foo as Bar>::Baz`, `Foo` and `Bar` sit inside the `scoped_type_identifier`, so the bare rule
  suppresses them and the scoped target (`<Foo as Bar>::Baz`, containing `<`) resolves to nothing.
  A qualified-associated-type projection therefore contributes no edges. It appears zero times in
  this repo; if it ever matters it is a third rule, not a change to these two.
- Primitives (`u8`, `u16`) are `primitive_type`, a different node, so they never enter the graph at
  all. That is free correctness, not a rule that has to be maintained.

### D3 — `rel_type = 'references'` and `call_form = 'type'`, together, in schema v5

Storing a type reference as `calls` would corrupt `verify-impact` and the `@line form` column, both of
which mean *a call happened here*. So `rel_type` widens.

`call_form` widens in the same migration so a reader can tell the rows apart **without the row
changing shape**. `render.rs:90-92` states the six columns are fixed because "a row that changed shape
would be read as a different claim". A type reference renders as
`h1  rust/src/main.rs  map_json_settings_err  1025  @1025  type`. `bare` was rejected: literally
accurate, but it tells the reader nothing.

### D4 — the migration enumerates columns, runs in a transaction, and is tested against a real v4 file

Three separate v1 defects, all in `migrate_v5`:

**Column order.** `migrate_v4` (`db.rs:154`) adds columns with `ALTER TABLE ADD COLUMN`, which
appends. Proven against real SQLite:

```
v3→v4 migrated : [source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score,
                  confidence_reasoning, call_site_line, call_form]
fresh from schema.sql : [source_chunk_id, target_chunk_id, rel_type, call_site_line, call_form,
                  confidence, confidence_score, confidence_reasoning]
```

Five of eight columns misalign, so `INSERT INTO x__v5 SELECT * FROM x` is wrong. **Every column is
named explicitly on both sides.**

**Atomicity.** v1 ran CREATE/INSERT/DROP/RENAME through `execute_batch` with no `BEGIN`/`COMMIT`. A
mid-batch failure leaves `__v5` behind and the promised retry then fails on the next open. v2 wraps
each table's rebuild in an explicit transaction and drops any stale `__v5` first.

**`PRAGMA foreign_keys` is a no-op inside a transaction.** v1 put it inside the batch. v2 sets it
outside, before the transaction opens, and restores it after.

**The test.** v1's test built a fresh current-schema database and rewrote its metadata to `"4"` — it
could never have caught the column-order defect, because the physical order was already correct. v2's
test builds the actual v3 table shape, runs the real `migrate_v4` path, and only then migrates to v5.

### D5 — std types stay unresolved, and the growth estimate

`type_identifier` matches 2,763 times in `rust/src evals/src`; top names are `String` 667, `Vec` 273,
`Option` 243, `Result` 183. None names a project chunk, so `resolve_candidates` returns nothing and no
relationship is written — as `graph.rs:191-194` requires. They do land in `raw_edges`.

v1 said this "roughly doubles `raw_edges`" by comparing against 2,198 *relationships*. Wrong
denominator: this repo holds roughly **17,000** raw edges (`incremental.rs:402`), so the upper bound is
about **+16%**, less after dedup.

They do not pollute the coverage screen: `extracted_but_unresolved` filters on the seed's own name
(`coverage.rs:379`), so `String` rows never appear under a `FeedSpec` query.

### D6 — generic parameters and associated types: measured, not special-cased

`type_identifier` also matches generic parameters (`T`, `E`) at both declaration and use, and an
associated type's own name — `associated_type` is a distinct node from `type_item`, confirmed by
probe, so `trait X { type Assoc; }` emits a reference to its own `Assoc`. The `associated_type`
exclusion is cheap and is in Task 1.

The generic-parameter phantom is **deliberately not special-cased in v2**. `T` resolves to nothing
unless the project also declares a real type named `T`; ast-grep cannot do the scope analysis that
would settle it, and a name-shape hack (`^[A-Z][A-Z0-9]?$`) would be a naming convention masquerading
as semantics. Task 8 measures the actual phantom rate and that number, not this paragraph, decides
whether a second round is needed. **Shipping with a known, quantified phantom rate is the accepted
risk; shipping with an unmeasured one is not.**

### D7 — same-line chunk collision, made deterministic

`chunk_id_for` is `project:file:start_line` (`chunker.rs:146`) — no chunk type. `pub trait T { fn f(&self) {} }`
on one line would produce a `class` chunk and a `method` chunk with the same id, and the dedup at
`chunker.rs:589-595` keeps whichever ast-grep emitted first, i.e. rule order in a directory listing.

v2 does not change `chunk_id` (it is a primary key with two foreign keys pointing at it). It makes the
loss **deterministic and documented**: the dedup sort gains a final tie-break that prefers the more
specific chunk, so the method survives and the type chunk is the one dropped. A type declared on the
same line as one of its own methods is a documented limitation, not an ordering accident.

The tie-break fires in exactly one case, and it is worth stating why. A method is nested inside its
trait, so `method.end_line <= trait.end_line` always. The existing `(start_line, end_line)` sort
therefore already puts the method first whenever the trait spans more lines — which is every
ordinary multi-line trait. The only shape where both keys tie is a trait whose entire body is on the
declaration line, and the third key exists solely for that. This means the change is inert on every
real trait in this repo and cannot reorder anything else.

### D8 — the measuring instrument must move too

`evals/src/hook.rs:32` (`declares_callable_in`) recognises only `fn`/`function`/`def`, and
`evals/tests/hook.rs:25` pins `pub struct Confidence;` to be rejected. That is the **eval** classifier,
not the shipping gate (§Evidence 2), so it does not block the feature — but if it is left alone, the
63% number cannot move and the win is unmeasurable. Task 7 teaches it type declarations and reports
them as a distinct verdict rather than folding them into the function count.

### D9 — out of scope, with the reason

`const`/`static` references (`TIMEOUT_S`, the 18 SCREAMING_CASE fires) are **deferred to a separate
plan**. In the Rust grammar a const use is a plain `identifier`, indistinguishable without scope
analysis from every local, parameter and function name. `type_identifier` has no such problem, which
is why types go first.

## Review record — every v1 defect and where it is handled

| # | Defect | Found by | Handled |
|---|---|---|---|
| 1 | Qualified type paths lose their module path → undetectable phantoms | Codex | D2, Task 1 |
| 2 | `edge:references` parses to `CallForm::Bare`; nothing supplies `Type` | Codex | D3, Task 1 |
| 3 | `migrate_v5` `SELECT *` misaligns columns on a real v4 file | Codex + independent SQLite probe | D4, Task 4 |
| 4 | Migration non-atomic; `PRAGMA foreign_keys` no-op in transaction | Codex | D4, Task 4 |
| 5 | Migration test never builds a real v4 physical schema | Codex | D4, Task 4 |
| 6 | Fourth `'calls'` filter missed: `coverage.rs:308/315` `extracted_calls` | corroboration | Task 6 |
| 7 | Task 5's fixture cannot reach `extracted_but_unresolved` (multi-candidate attaches AMBIGUOUS) | corroboration | Task 6 |
| 8 | Same-line trait/method chunk collision, order-dependent | Codex | D7, Task 2 |
| 9 | `rust/tests/incremental.rs:524` hardcodes `Some("4")` | Codex | Task 4 |
| 10 | Task 6 used `--symbol`; the CLI takes `--symbols` (`evals/src/main.rs:104`) | Codex | Task 8 |
| 11 | Growth estimate used the wrong denominator (2,198 vs ~17,000) | Codex | D5 |
| 12 | Eval hook classifier pins structs as rejected → win unmeasurable | Codex (scope corrected) | D8, Task 7 |
| 13 | Stale contracts: SKILL.md, README ×3, `chunker.rs:16`, `graph.rs:30`, `impact.rs:19`, `schema.sql` | Codex | Task 8 |
| 14 | Generic params and associated types emit references | Codex | D6, Task 1 + Task 8 |
| 15 | `union_item` missing from the exclusion list — a union cites itself | Codex r2 | Task 1 |
| 16 | Task 1's expected vector included `u8`; primitives are `primitive_type`, not `type_identifier` | Codex r2 | Task 1 |
| 17 | `chunk_specificity`'s tie-break could be pre-empted by the `end_line` key | corroboration | D7 (shown inert: a method is nested, so it always sorts first when the spans differ) |
| 18 | Rename sites and the loss of comparability with the 63% baseline | corroboration | Task 6 |
| 19 | `PRAGMA foreign_keys` restore was best-effort; the pragma is unnecessary — nothing references these tables | Kimi | Task 3 (both pragmas deleted) |
| 20 | Error path dropped `__v5`, which after a failed ROLLBACK could be the only surviving copy | Kimi | Task 3 (cleanup removed; the retry's own `DROP IF EXISTS` handles it) |
| 21 | First write transaction on the open path — `hook-refresh` now blocks up to 5s instead of failing fast | Kimi | Task 3 (named in the doc comment; not a defect) |
| 22 | The v5 consts are a third copy of the table bodies with nothing checking them against `schema.sql` | Kimi | Task 3 (column-**order** equality against a fresh database) |
| 23 | Ordering hazard **inside** the plan: rows could exist before `impact.rs:37` widened, printing `@- -` | Kimi | Restructured — every gate opens in Task 2, the pack rules are Task 4 |
| 24 | `rust/tests/scan_backend.rs:23-33` is the one Rust fixture on the real pack and becomes a canary | Kimi | Task 4 Step 4c |

**One v1 claim Codex got wrong, corrected here:** it reported that the eval classifier "invalidates the
plan's claim that this feature addresses the measured CamelCase rejection population". It does not —
the shipping `judge` has no function gate (§Evidence 2), so production already fires on these symbols.
What it invalidates is the *measurability* of the improvement, which is why D8 and Task 7 exist.

**One nuance that changes a task:** `impact.rs:37` is not the graph walk. The recursive walk
(`graph.rs:781-787`) has no `rel_type` predicate and already traverses every relation, so reference
dependents appear as soon as the rows exist. Widening `impact.rs:37` is still required, but only so
those rows carry their line and form instead of rendering `@- -`.

## Global Constraints

- The repo is pure Rust plus `install.sh` and `tests/install-smoke.sh`. No new script of any kind.
- No absolute developer paths and no Node-toolchain paths anywhere, fixtures included. A test that
  cannot find `ast-grep` prints `SKIP:` (see `resolve_ast_grep_bin` in `rust/tests/pack.rs`).
- `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` in **both** crates before every
  commit. CI runs fmt first and a fmt failure hides every gate behind it.
- Storage failures are returned as `CortError`, never `panic!`/`expect`.
- A `skills/<name>/SKILL.md` is deployed byte-for-byte; its frontmatter key set is closed.
- Commit messages end with:
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- The user gates every commit. Do not push.

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/pack/rules/rust.yml` | modify | `cort-rust-chunk-type`, `cort-rust-edge-references-bare`, `-scoped` |
| `rust/src/chunker.rs` | modify `:33-79`, `:584-595`, `:146` doc | `CallForm::Type`; `references`; deterministic dedup |
| `rust/src/schema.sql` | modify `:46`, `:56-57`, `:76`, `:84-85` | widen four CHECKs |
| `rust/src/db.rs` | modify `:10`, add `migrate_v5` | `SCHEMA_VERSION = 5`; transactional rebuild |
| `rust/src/impact.rs` | modify `:37`, doc `:19-28` | reference rows carry line and form |
| `rust/src/coverage.rs` | modify `:315`, `:379`, doc `:17` | both filters widen |
| `rust/src/graph.rs` | doc `:30-36` | `call_form` doc admits a non-call form |
| `evals/src/hook.rs` | modify `:32` | type declarations are a seed the screen recognises |
| `evals/tests/hook.rs` | modify `:25` | the pinned expectation moves with it |
| `rust/tests/incremental.rs` | modify `:524` | schema version 5 |
| `rust/tests/{pack,chunker,db,impact,coverage}.rs` | add tests | one per task |
| `skills/ast-grep/SKILL.md`, `README.md` | modify | stated contracts |

---

### Task 1: `CallForm::Type`, the `references` rel type, and a deterministic dedup

**Files:**
- Modify: `rust/src/chunker.rs:33-71` (`CallForm`), `:79` (`EDGE_REL_TYPES`), `:584-595` (dedup sort)
- Test: `rust/tests/chunker.rs`

**Interfaces:**
- Consumes: Task 1's `edge:references:type` message.
- Produces: `CallForm::Type` (`as_str() == "type"`, `parse("type") == Some(Type)`,
  `insertion_rank() == 3`); `EDGE_REL_TYPES` containing `"references"`; a dedup that prefers the more
  specific chunk on an id collision. Tasks 3-6 rely on all three.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/chunker.rs`:

```rust
/// `type` is a fourth call form, not a fourth rel type in disguise. It ranks last because
/// `insertion_rank` decides which row survives a duplicate key, and a call is a stronger claim about
/// a line than a type mention on the same line.
#[test]
fn a_type_reference_parses_as_its_own_form_and_rel_type() {
    assert_eq!(CallForm::Type.as_str(), "type");
    assert_eq!(CallForm::parse("type"), Some(CallForm::Type));
    assert_eq!(CallForm::Type.insertion_rank(), 3);
    assert!(CallForm::Type.insertion_rank() > CallForm::Bare.insertion_rank());

    assert!(EDGE_REL_TYPES.contains(&"references"));
    assert_eq!(
        parse_edge_tag("references:type"),
        Some(("references".to_string(), CallForm::Type)),
        "the pack rule's own message is the only channel that can supply the form"
    );
}

/// `chunk_id` is project:file:start_line with no chunk type (`chunker.rs:146`), so a type declared on
/// the same line as one of its own methods collides. Which one survived used to depend on the order
/// ast-grep emitted records in, i.e. on a directory listing. The loss is accepted, but it must be the
/// same loss every time: the method is the chunk `impact` can hold a seed for, so it wins.
#[test]
fn a_type_sharing_a_line_with_its_method_loses_deterministically() {
    let source = "pub trait T { fn f(&self) {} }\n";
    let (_dir, abs) = tmp_file("t.rs", source);
    let r = extract_real(&abs, "t.rs", source);
    let kinds: Vec<&str> = r.chunks.iter().map(|c| c.chunk_type.as_str()).collect();
    assert_eq!(
        kinds,
        ["method"],
        "the method survives the id collision, every time: {kinds:?}"
    );
}
```

Uses the helpers already in `rust/tests/chunker.rs` (verified 2026-09-04):
`tmp_file(name, body) -> (TempDir, PathBuf)` and
`extract_real(abs, file_path, source) -> ExtractResult`, which resolves the binary itself. Ensure the
file's `use cort::chunker::{...}` imports `CallForm`, `EDGE_REL_TYPES` and `parse_edge_tag`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test chunker a_type_reference_parses_as_its_own_form_and_rel_type`
Expected: FAIL to compile — "no variant named `Type` found for enum `CallForm`".

- [ ] **Step 3: Write minimal implementation**

In `rust/src/chunker.rs`, add to `CallForm` after `Scoped`:

```rust
    /// `FeedSpec` in a type position -- not a call at all. Carried as a form so an `impact` row can
    /// say what kind of edge it is inside the six fixed columns, instead of the row changing shape.
    Type,
```

Add `Self::Type => "type",` to `as_str`, `"type" => Some(Self::Type),` to `parse`, and
`Self::Type => 3,` to `insertion_rank`.

Widen the allowlist at `:79`:

```rust
pub const EDGE_REL_TYPES: &[&str] = &["imports", "exports", "calls", "references"];
```

Give the dedup at `:584` a deterministic tie-break. Add a rank helper and extend the sort:

```rust
/// Which chunk wins when two share a `chunk_id` (project:file:start_line, `chunk_id_for`). Only
/// reachable when a type is declared on the same line as one of its own methods. The method wins
/// because it is the chunk a caller-set question can hold a seed for; without this the survivor was
/// whichever record ast-grep emitted first, i.e. a directory listing order.
fn chunk_specificity(chunk_type: &str) -> u8 {
    match chunk_type {
        "method" => 0,
        "function" => 1,
        _ => 2,
    }
}
```

and in the existing `chunks.sort_by`, append `.then(chunk_specificity(&a.chunk_type).cmp(&chunk_specificity(&b.chunk_type)))`.

Update the doc comment on `chunk_id_for` (`:146`) to state that the id does not include the chunk
type and that `chunk_specificity` is what resolves the resulting collision.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test chunker`
Expected: PASS. Any non-exhaustive `match` on `CallForm` is a compile error — handle `Type`
explicitly, never with a `_` arm.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/chunker.rs rust/tests/chunker.rs
git commit -m "feat(chunker): add the type call form, the references rel type, deterministic dedup

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: Widen every gate for the `references` kind — schema, impact, coverage

**Files:**
- Modify: `rust/src/schema.sql:46`, `:56-57`, `:76`, `:84-85`
- Modify: `rust/src/impact.rs:37` and the doc at `:19-28`
- Modify: `rust/src/coverage.rs:315`, `:379`, doc `:17`
- Test: covered by Task 4's migration test and Tasks 5-6's end-to-end tests; no separate test.

**Interfaces:**
- Consumes: `CallForm::Type` (Task 2).
- Produces: a **fresh** database that accepts `rel_type = 'references'` and `call_form = 'type'`, and
  every reader already prepared for those rows. Task 4 handles existing databases.

**Why all four widenings are one commit, and why they land before the pack rules.** Widening a filter
for a `rel_type` that has no rows yet is inert — it cannot change any output, so it needs no test of
its own and can never regress anything. The reverse order can: the moment the pack rules ship (Task
5), resolved reference rows become `impact` dependents automatically, because the recursive walk
(`graph.rs:781-793`) has **no `rel_type` predicate**. If `call_sites_by_source` (`impact.rs:37`) is
still filtering `'calls'` at that moment, those dependents print `@- -` — a row a reader cannot check
against one line, which is precisely the property schema v4 exists to restore. The same argument
applies to `coverage.rs:315`: an un-widened suppression map would report every real reference edge as
a gap. **The pack rules are therefore the last mechanism change in this plan** (Task 5), and every
consumer is made ready before a single row can exist.

- [ ] **Step 1: Change both `rel_type` CHECKs (lines 46 and 76)**

```sql
  rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls','references')),
```

- [ ] **Step 2: Change both `call_form` CHECKs (lines 56-57 and 84-85)**

```sql
  call_form TEXT NOT NULL DEFAULT 'bare'
    CHECK(call_form IN ('bare','receiver','scoped','type')),
```

- [ ] **Step 3: Update the two column comments so they stop claiming call-only semantics**

The `call_site_line` comment (`:47-50`) and both `call_form` comments (`:51-55`, `:77-83`) say "call"
throughout. State that the line is the line naming the **target**, and that `type` is a form the
column now carries for an edge that is not a call.

- [ ] **Step 4: Widen `impact.rs:37` so reference rows will carry their line and form**

```rust
          WHERE r.rel_type IN ('calls', 'references')
            AND r.call_site_line IS NOT NULL AND c.project_id = ?1
```

Update the doc at `:19-28` and on `call_sites_by_source`/`call_site_for`: the stored line is the line
that names the **target**, whether it is called or named in a type position — not "call site".

- [ ] **Step 5: Widen both coverage filters**

`coverage.rs:315` (inside `extracted_calls`) and `coverage.rs:379` (inside
`extracted_but_unresolved`), both to:

```sql
          WHERE project_id = ?1 AND rel_type IN ('calls', 'references')
```

Rename `extracted_calls` to `extracted_edges` and update its doc (`:308`) — it no longer maps calls
only. Update the module doc at `:17` so `extracted_but_unresolved` is described as covering both edge
kinds.

- [ ] **Step 6: Verify nothing changed yet**

Run: `cd rust && cargo test`
Expected: PASS, with **no** test output differing — the pack emits no `references` records until Task
5, so every widening in this commit is inert by construction. A failure here means something else
already produces that rel type, which would need explaining before continuing.

- [ ] **Step 7: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/schema.sql rust/src/impact.rs rust/src/coverage.rs
git commit -m "feat: widen schema, impact and coverage for the references rel type

Every gate is opened before the extractor can emit a single row, so no
commit in this series can print a dependent without its line and form.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Schema v5 migration — column-explicit, transactional, tested against a real v4 file

**Files:**
- Modify: `rust/src/db.rs:10` (`SCHEMA_VERSION`), add `migrate_v5` after `migrate_v4:154`, call it in `ensure_schema:205`
- Modify: `rust/tests/incremental.rs:524` (`Some("4")` → `Some("5")`)
- Test: `rust/tests/db.rs`

**Interfaces:**
- Consumes: Task 3's widened `SCHEMA_SQL`.
- Produces: an existing v4 database upgraded in place at `SCHEMA_VERSION = 5`, every row preserved
  and correctly aligned, `graph_pending` set.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/db.rs`:

```rust
/// A REAL v4 file, not a fresh one with its metadata rewritten. `migrate_v4` adds columns with
/// `ALTER TABLE ADD COLUMN`, which APPENDS, so a v3→v4 database has
/// [... rel_type, confidence, confidence_score, confidence_reasoning, call_site_line, call_form]
/// while a fresh one from schema.sql has
/// [... rel_type, call_site_line, call_form, confidence, confidence_score, confidence_reasoning].
/// Five of eight columns differ. A `SELECT *` rebuild would silently misalign them, and a test that
/// starts from a fresh schema can never see it.
#[test]
fn migrating_a_real_v4_database_to_v5_preserves_and_aligns_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.db");
    let db = Db::open(&path).unwrap();

    // The v3 shape: schema.sql's tables minus the two v4 columns.
    db.execute_batch(
        "CREATE TABLE projects (project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
           git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL,
           created_at TEXT DEFAULT (datetime('now')));
         CREATE TABLE chunks (chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
           file_path TEXT NOT NULL, symbol_name TEXT, chunk_type TEXT, start_line INTEGER NOT NULL,
           end_line INTEGER NOT NULL, content TEXT NOT NULL, content_hash TEXT NOT NULL,
           language TEXT, chunk_source TEXT NOT NULL);
         CREATE TABLE relationships (
           source_chunk_id TEXT NOT NULL, target_chunk_id TEXT NOT NULL,
           rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
           confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
           confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
           confidence_reasoning TEXT,
           PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type));
         CREATE TABLE raw_edges (
           project_id TEXT NOT NULL, file_path TEXT NOT NULL,
           source_symbol TEXT NOT NULL DEFAULT '', raw_target TEXT NOT NULL,
           rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
           start_line INTEGER NOT NULL,
           PRIMARY KEY (project_id, file_path, rel_type, raw_target, source_symbol, start_line));
         INSERT INTO projects (project_id, name, path, extractor_version)
           VALUES ('p','n','/tmp/p','v');
         INSERT INTO chunks VALUES ('c1','p','a.rs','caller','function',1,3,'x','h','rust','ast');
         INSERT INTO chunks VALUES ('c2','p','b.rs','callee','function',1,3,'y','h','rust','ast');
         INSERT INTO relationships VALUES ('c1','c2','calls','INFERRED',0.7,'resolved: callee');
         INSERT INTO raw_edges VALUES ('p','a.rs','caller','callee','calls',7);",
    )
    .unwrap();
    cort::db::set_meta(&db, "SCHEMA_VERSION", "3").unwrap();
    drop(db);

    // v3 -> v4 through the real migration path, then v4 -> v5.
    let db = Db::open(&path).unwrap();
    cort::db::ensure_schema(&db).unwrap();

    assert_eq!(
        cort::db::get_meta(&db, "SCHEMA_VERSION").unwrap().as_deref(),
        Some("5")
    );
    assert_eq!(
        cort::db::get_meta(&db, "graph_pending").unwrap().as_deref(),
        Some("1"),
        "a widened graph must be rebuilt before it is trusted"
    );

    // The alignment assertion: read the row back BY NAME and check every field landed where it
    // belongs. A `SELECT *` rebuild puts `confidence` into `call_site_line`.
    let (rel, conf, score, reasoning): (String, String, f64, Option<String>) = db
        .query_row(
            "SELECT rel_type, confidence, confidence_score, confidence_reasoning
               FROM relationships WHERE source_chunk_id = 'c1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(rel, "calls");
    assert_eq!(conf, "INFERRED");
    assert_eq!(score, 0.7);
    assert_eq!(reasoning.as_deref(), Some("resolved: callee"));

    let (target, line): (String, i64) = db
        .query_row(
            "SELECT raw_target, start_line FROM raw_edges WHERE source_symbol = 'caller'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((target.as_str(), line), ("callee", 7));

    // And the point of the whole migration.
    db.execute(
        "INSERT INTO raw_edges (project_id, file_path, source_symbol, raw_target, rel_type, call_form, start_line)
         VALUES ('p','a.rs','caller','settings::SettingsError','references','type',9)",
        [],
    )
    .expect("v5 accepts a qualified type reference");

    // The anti-drift assertion, and the one that actually pins what this migration exists for.
    // `V5_RELATIONSHIPS`/`V5_RAW_EDGES` are a THIRD copy of these table bodies -- schema.sql plus two
    // consts -- and nothing else checks that they agree. The v4 misalignment this migration fixes WAS
    // that kind of drift, one level worse. A migrated database must end up with the same column ORDER
    // as a fresh one, not merely the same column set, or the next `SELECT *` anywhere silently lies.
    let fresh_path = dir.path().join("fresh.db");
    let fresh = Db::open(&fresh_path).unwrap();
    cort::db::ensure_schema(&fresh).unwrap();
    for table in ["relationships", "raw_edges"] {
        assert_eq!(
            columns(&db, table),
            columns(&fresh, table),
            "{table}: a migrated database must match a fresh one column for column, in order"
        );
    }
}

/// A failed rebuild must leave the database retryable, not half-migrated with a stray table.
#[test]
fn a_v5_rebuild_that_fails_leaves_no_temporary_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.db");
    let db = Db::open(&path).unwrap();
    cort::db::ensure_schema(&db).unwrap();
    db.execute_batch("CREATE TABLE relationships__v5 (bogus TEXT);")
        .unwrap();
    cort::db::set_meta(&db, "SCHEMA_VERSION", "4").unwrap();
    drop(db);

    let db = Db::open(&path).unwrap();
    cort::db::ensure_schema(&db).expect("a stale temporary table is cleared, not fatal");
    let stale: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'relationships__v5'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stale, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test db migrating_a_real_v4_database_to_v5`
Expected: FAIL — `SCHEMA_VERSION` is `Some("4")`.

- [ ] **Step 3: Write minimal implementation**

Set `pub const SCHEMA_VERSION: i64 = 5;` in `rust/src/db.rs:10`.

Add the two rebuild bodies as consts beside `V4_ADDED_COLUMNS`. Each is that table's body from
`schema.sql`, name suffixed `__v5`, `IF NOT EXISTS` removed — and **every column named explicitly** in
the copy:

```rust
/// v5 rebuild for `relationships`. The column list is written out on both sides of the INSERT
/// because a v3->v4 database has a different PHYSICAL column order than a fresh one: `migrate_v4`
/// appends with ALTER TABLE ADD COLUMN, so `call_site_line`/`call_form` sit at the END there and in
/// the MIDDLE here. `SELECT *` would put `confidence` into `call_site_line`.
const V5_RELATIONSHIPS: (&str, &str, &str) = (
    "relationships",
    "CREATE TABLE relationships__v5 (
       source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
       target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
       rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls','references')),
       call_site_line INTEGER,
       call_form TEXT NOT NULL DEFAULT 'bare'
         CHECK(call_form IN ('bare','receiver','scoped','type')),
       confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
       confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
       confidence_reasoning TEXT,
       PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type))",
    "source_chunk_id, target_chunk_id, rel_type, call_site_line, call_form,
     confidence, confidence_score, confidence_reasoning",
);

const V5_RAW_EDGES: (&str, &str, &str) = (
    "raw_edges",
    "CREATE TABLE raw_edges__v5 (
       project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
       file_path TEXT NOT NULL,
       source_symbol TEXT NOT NULL DEFAULT '',
       raw_target TEXT NOT NULL,
       rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls','references')),
       call_form TEXT NOT NULL DEFAULT 'bare'
         CHECK(call_form IN ('bare','receiver','scoped','type')),
       start_line INTEGER NOT NULL,
       PRIMARY KEY (project_id, file_path, rel_type, raw_target, source_symbol, start_line))",
    "project_id, file_path, source_symbol, raw_target, rel_type, call_form, start_line",
);
```

Add the migration:

```rust
/// Widen the v5 CHECK constraints on `relationships` and `raw_edges`.
///
/// SQLite cannot alter a CHECK in place, so this is the documented rebuild. Four properties, each of
/// which was wrong in a draft and is now a test:
///
/// * Columns are named on both sides of the INSERT. `SELECT *` is wrong here -- see V5_RELATIONSHIPS.
/// * Each table's rebuild runs inside an explicit transaction, so a mid-rebuild failure rolls back
///   instead of leaving a `__v5` table behind and making the promised retry fail.
/// * No `PRAGMA foreign_keys` dance. Nothing in `schema.sql` references `relationships` or
///   `raw_edges` -- only their own indexes do (`:63`, `:64`, `:89`), and those drop and recreate with
///   the table -- and dropping a table is never an FK violation. A draft turned enforcement off and
///   restored it with `let _ =`, which would have left foreign keys silently OFF for the rest of the
///   process if the restore failed. The pragma protected against a hazard this schema does not have.
/// * The error path does not delete `{table}__v5`. The only state where a populated temporary table
///   outlives this batch is one where the transaction never committed, and if the ROLLBACK itself
///   failed -- the transient IOERR class that already killed eight tests on a CI runner -- that table
///   may hold the only surviving copy of the rows. The retry clears it, because the batch opens with
///   `DROP TABLE IF EXISTS`. Cleaning up here can only destroy data, never save any.
///
/// Rows are copied rather than re-derived because README's upgrade note promises that `impact` keeps
/// answering from the pre-upgrade graph until the forced re-index runs. `chunks` is untouched: Rust
/// type declarations are stored as `chunk:class`, a value its CHECK already allows.
///
/// Runs before `SCHEMA_VERSION` is written, so a failure leaves the database at its old version and
/// the next open retries. Every sqlite error is returned, never panicked on -- `hook-refresh` reaches
/// this path on every edit and promises to be silent and exit 0.
///
/// One behaviour worth knowing before diagnosing it twice: this is the first *write transaction* on
/// the open path (v4's were quick ALTERs). Between deploying this binary and the first successful
/// migration, a concurrent open serialises on `BEGIN IMMEDIATE` for up to the 5s `busy_timeout` set
/// at `db.rs:86`. A racing `hook-refresh` blocks rather than failing fast, then lands on its quiet
/// `db_unavailable` path. The race itself is safe: the loser re-runs the rebuild, which is idempotent
/// and lossless because both sides of the INSERT name their columns.
fn migrate_v5(db: &Db) -> Result<(), CortError> {
    let fail = |stage: &str, e: rusqlite::Error| {
        CortError::new(
            "schema_migration_failed",
            json!({ "version": 5, "stage": stage, "message": e.to_string() }),
        )
    };
    for (table, create, columns) in [V5_RELATIONSHIPS, V5_RAW_EDGES] {
        db.execute_batch(&format!(
            "BEGIN IMMEDIATE;
             DROP TABLE IF EXISTS {table}__v5;
             {create};
             INSERT INTO {table}__v5 ({columns}) SELECT {columns} FROM {table};
             DROP TABLE {table};
             ALTER TABLE {table}__v5 RENAME TO {table};
             COMMIT;"
        ))
        .map_err(|e| {
            let _ = db.execute_batch("ROLLBACK");
            fail(table, e)
        })?;
    }
    // The indexes named in SCHEMA_SQL went with the dropped tables.
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
         CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);
         CREATE INDEX IF NOT EXISTS idx_raw_edges_file ON raw_edges(project_id, file_path);",
    )
    .map_err(|e| fail("indexes", e))
}
```

In `ensure_schema`, call it after `migrate_v4(db)?;`:

```rust
        migrate_v4(db)?;
        migrate_v5(db)?;
```

Change `rust/tests/incremental.rs:524-528` from `Some("4")` to `Some("5")`, and update its assertion
message if it names v4 specifically.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test db && cargo test --test incremental`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/db.rs rust/tests/db.rs rust/tests/incremental.rs
git commit -m "feat(db): schema v5 migration, column-explicit and transactional

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: The three pack rules — the last mechanism change, driven by both tests

**Files:**
- Modify: `src/pack/rules/rust.yml` (append)
- Test: `rust/tests/pack.rs` (rule shapes), `rust/tests/impact.rs` (end to end)

**Interfaces:**
- Consumes: Tasks 1-3 — every gate is already open, so the moment these rules land the rows flow
  all the way through to a rendered dependent with its line and form.
- Produces: ast-grep records with `message: "chunk:class"` capturing `$NAME`, and
  `message: "edge:references:type"` capturing `$CALLEE` — the **full** target, qualified where the
  source qualified it. Task 2 parses the tag; Task 5 resolves the target.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/pack.rs`:

```rust
/// A Rust type declaration must produce a chunk, and a use of that type must produce a reference
/// edge. Two properties are load-bearing and are asserted separately below:
///   * a declaration's own name is not a reference to itself, or every type would be its own
///     dependent;
///   * a qualified use keeps its qualifier, because `settings::SettingsError` and
///     `settings_toml::SettingsError` are two different types in this very repository and the
///     module-suffix resolver cannot tell them apart once the path is gone.
#[test]
fn the_pack_extracts_rust_type_chunks_and_reference_edges() {
    let _g = pack_guard();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("k.rs");
    fs::write(
        &file,
        [
            "pub struct FeedSpec { pub url: String }",
            "pub enum SkillStatus { Ok, Degraded }",
            "pub trait Emit { type Sink; fn emit(&self); }",
            "pub union Raw { a: u8, b: u16 }",
            "type Alias = u8;",
            "pub fn take(s: FeedSpec) -> SkillStatus { SkillStatus::Ok }",
            "pub fn qualified(e: settings::SettingsError) -> u8 { 1 }",
        ]
        .join("\n"),
    )
    .unwrap();
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    let sg = sgconfig();
    let r = exec_ast_grep(
        &bin,
        &[
            "scan",
            "--json=stream",
            "--config",
            sg.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
        ExecOpts::default(),
    )
    .unwrap();
    assert_eq!(r.code, 0);
    let records: Vec<serde_json::Value> = r
        .stdout
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    let mut chunks: Vec<String> = records
        .iter()
        .filter(|x| x["message"] == "chunk:class")
        .map(|x| x["metaVariables"]["single"]["NAME"]["text"].as_str().unwrap().to_string())
        .collect();
    chunks.sort();
    assert_eq!(
        chunks,
        ["Emit", "FeedSpec", "SkillStatus"],
        "struct, enum and trait become class chunks; a type alias does not"
    );

    let mut refs: Vec<String> = records
        .iter()
        .filter(|x| x["message"] == "edge:references:type")
        .map(|x| x["metaVariables"]["single"]["CALLEE"]["text"].as_str().unwrap().to_string())
        .collect();
    refs.sort();
    assert_eq!(
        refs,
        ["FeedSpec", "SkillStatus", "String", "settings::SettingsError"],
        "uses are references, declarations are not, and a qualified use keeps its path"
    );

    // Primitives are `primitive_type` nodes in this grammar, not `type_identifier` (probed
    // 2026-09-04), so `u8`/`u16` cost nothing and need no suppression rule.
    assert!(
        !refs.iter().any(|r| r == "u8" || r == "u16"),
        "a primitive is not a type_identifier and must not appear: {refs:?}"
    );
    for own_name in ["Sink", "Alias", "Raw"] {
        assert!(
            !refs.iter().any(|r| r == own_name),
            "{own_name} is a declaration's own name, not a reference to itself: {refs:?}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test --test pack the_pack_extracts_rust_type_chunks_and_reference_edges -- --nocapture`
Expected: FAIL — `chunks` is empty.

- [ ] **Step 3: Write minimal implementation**

Append to `src/pack/rules/rust.yml`:

```yaml
---
# struct / enum / trait become `chunk:class`, the bucket JS/TS/Python type declarations already use.
# A dedicated `chunk_type` would mean widening the CHECK on `chunks` and rebuilding its
# external-content FTS mirror for a label the chunk's own content already shows.
#
# `type_item` (a plain alias) is deliberately NOT a chunk: an alias has no body to slice and no
# members, so a seed on it would return the alias line and nothing else.
id: cort-rust-chunk-type
language: Rust
severity: hint
message: chunk:class
rule:
  any:
    - { kind: struct_item, has: { field: name, pattern: $NAME } }
    - { kind: enum_item, has: { field: name, pattern: $NAME } }
    - { kind: trait_item, has: { field: name, pattern: $NAME } }
---
# Two reference rules, for the same reason there are two call rules: a qualified target must keep its
# path. `settings::SettingsError` and `settings_toml::SettingsError` are both live in this repo, and
# the module-suffix branch of `graph::resolve_targets` only runs when the stored target contains
# `::`. Capture the leaf alone and both definitions match, both are written AMBIGUOUS, and no screen
# this product has can see the phantom -- `impact` drops confidence, coverage counts any edge to the
# seed as resolved, and `verify-impact` only asks whether the line contains the word.
#
# The `not` clause drops each declaration's OWN name. `associated_type` is a distinct node from
# `type_item` in this grammar, so `trait X { type Assoc; }` needs it named explicitly or the trait
# cites itself.
#
# Generic parameters (`T`, `E`) are knowingly NOT excluded: ast-grep cannot do the scope analysis, a
# single-uppercase-letter regex is a naming convention pretending to be semantics, and `T` resolves
# to nothing unless the project declares a real type by that name. The rate is measured in Task 8.
id: cort-rust-edge-references-scoped
language: Rust
severity: hint
message: edge:references:type
rule:
  kind: scoped_type_identifier
  pattern: $CALLEE
---
id: cort-rust-edge-references-bare
language: Rust
severity: hint
message: edge:references:type
rule:
  kind: type_identifier
  pattern: $CALLEE
  not:
    any:
      - inside:
          any:
            - { kind: struct_item }
            - { kind: enum_item }
            - { kind: union_item }
            - { kind: trait_item }
            - { kind: type_item }
            - { kind: associated_type }
          field: name
      - inside: { kind: scoped_type_identifier }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust && cargo test --test pack`
Expected: PASS (the whole file — `extractor_version` hashes every pack file and one test asserts it changes).

If the `refs` vector disagrees, adjust it to what the grammar actually emits rather than adding a
suppression rule: an unresolvable name is already free (D5), and a rule that hides one would also
hide real misses.

- [ ] **Step 4b: Add the end-to-end tests — these are the real gate**

The rule-shape test above proves ast-grep emits the right records. These two prove the records
survive parsing, storage, resolution and rendering. Append to `rust/tests/impact.rs`:

```rust
/// The measured gap this whole change exists for: before it, this returned `seeds=0`.
#[test]
fn a_struct_reports_the_functions_that_name_it() {
    let (_dir, root, db, project_id, bin) = indexed(&[(
        "src/lib.rs",
        "pub struct FeedSpec { pub url: String }\npub fn take(s: FeedSpec) -> u8 { 1 }\n",
    )]);
    let out = impact_command(&db, &bin, &root, &project_id, "FeedSpec", 1).unwrap();
    assert_eq!(out["seeds"].as_i64(), Some(1), "the struct resolves as a seed");
    let deps = out["dependents"].as_array().unwrap();
    assert_eq!(deps.len(), 1, "one function names the type: {deps:?}");
    assert_eq!(deps[0]["symbol_name"].as_str(), Some("take"));
    assert_eq!(deps[0]["call_form"].as_str(), Some("type"));
    assert_eq!(
        deps[0]["call_site_line"].as_i64(),
        Some(2),
        "the line that names the type, so one read checks the edge"
    );
}

/// The v1 stop-ship, as a test. Two same-named types in different modules, each used by qualified
/// path: the qualifier is the only thing that can tell them apart, and a leaf-only capture would
/// attach BOTH to BOTH -- a phantom no screen in this product can see.
#[test]
fn a_qualified_type_reference_resolves_to_the_module_it_names() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/settings.rs", "pub enum SettingsError { Io }\n"),
        ("src/settings_toml.rs", "pub enum SettingsError { Io }\n"),
        (
            "src/main.rs",
            "pub fn from_json(e: settings::SettingsError) -> u8 { 1 }\n",
        ),
    ]);
    let out = impact_command(&db, &bin, &root, &project_id, "SettingsError", 1).unwrap();
    let deps = out["dependents"].as_array().unwrap();
    assert_eq!(
        deps.len(),
        1,
        "exactly one dependent -- attaching to both definitions is the phantom: {deps:?}"
    );
    assert_eq!(deps[0]["symbol_name"].as_str(), Some("from_json"));
}
```

Both use the helpers already in `rust/tests/impact.rs` (verified 2026-09-04):
`indexed(&[(path, source)]) -> (TempDir, PathBuf, Connection, String, String)` returning
`(dir, root, db, project_id, bin)`, then
`impact_command(&db, &bin, &root, &project_id, symbol, depth) -> Result<Value, _>`. `indexed` takes
file contents as one string with real newlines, not a joined vector.

Run: `cd rust && cargo test --test impact`
Expected: PASS. If `a_qualified_type_reference_resolves_to_the_module_it_names` reports 2
dependents, the scoped rule is not matching and the qualifier is being discarded — fix the rule,
never the test. That assertion is the v1 stop-ship, pinned.

- [ ] **Step 4c: Run the whole suite — one fixture is a canary**

Run: `cd rust && cargo test`

`rust/tests/scan_backend.rs:23-33` is the only fixture with **Rust** source that exercises the real
pack (`pub struct T;` and `let t = T;`), so it starts producing `references` records the moment this
task lands. It compares the two scan backends against each other rather than a golden list, so it
stays green **only if `rust/src/scan.rs` implements the same rules in this same commit**. If it fails,
the crate-side scanner has drifted from the pack — fix it here, not later. Note its `>= 3` assertion
is a floor, not a ceiling: it cannot detect a rule that never fires, so a green run there is not
evidence the new rules work. Step 4b is that evidence.

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
git add src/pack/rules/rust.yml rust/tests/pack.rs rust/tests/impact.rs
git commit -m "feat(pack): extract Rust type declarations and qualified type references

A qualified path keeps its module, so settings::SettingsError and
settings_toml::SettingsError resolve to one definition each instead of
both attaching to both -- a phantom no screen in this product can see.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: End-to-end — the coverage screen neither floods nor hides

**Files:**
- Modify: `rust/src/coverage.rs:315` (`extracted_calls`), `:379` (`extracted_but_unresolved`), doc `:17`
- Test: `rust/tests/coverage.rs`

**Interfaces:**
- Consumes: Tasks 1-5.
- Produces: a coverage screen for a type seed that neither over- nor under-reports.

**Why both, and why together.** They fail in opposite directions and shipping one without the other is
worse than shipping neither:
- `:379` (`extracted_but_unresolved`) too narrow ⇒ a type seed is **blind to its own dropped
  resolutions**, and `enumeration_may_be_incomplete: false` becomes a lie.
- `:315` (`extracted_calls`) too narrow ⇒ every successfully-built reference edge is missing from the
  suppression map, so every type mention is reported as a gap and the boolean is **stuck true**.

**Both fixtures below were validated against the shipping binary before this task was written**, using
the equivalent `calls` shape (the resolution path is shared, so a scoped call proves the scoped
reference). v1 got this wrong once by assuming a drop where the code attaches AMBIGUOUS; that is why
these were run rather than reasoned:

```
$ cort impact --symbol widget --depth 1 --coverage -f lean     # src/user.rs: nowhere::widget()
# impact widget depth=1 seeds=1 dependents=0
seed	widget	mentions=1	no_edge=0	dropped=1	incomplete=true
drop	src/user.rs:1	take -> nowhere::widget

$ cort impact --symbol helper --depth 1 --coverage -f lean     # one file, edge resolves
seed	helper	mentions=1	no_edge=0	dropped=0	incomplete=false
```

A path naming a module the project does not contain is genuinely dropped and named — it does **not**
degrade to the bare leaf and attach. And a clean single-file fixture does reach `incomplete=false`,
so the first test's expectation is reachable.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/coverage.rs`:

```rust
/// The suppression half. A reference edge that WAS built must not also be reported as a missing one,
/// or a type seed's screen is pure noise and the boolean is permanently true.
#[test]
fn a_resolved_type_reference_is_not_also_reported_as_a_mention_gap() {
    let (_dir, root, db, project_id, bin) = indexed(&[(
        "src/lib.rs",
        "pub struct FeedSpec { pub url: String }\npub fn take(s: FeedSpec) -> u8 { 1 }\n",
    )]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "FeedSpec");
    let mentions = cov["mentions_without_edge"].as_array().unwrap();
    assert!(
        !mentions.iter().any(|m| m["line"].as_i64() == Some(2)),
        "line 2 has an edge; it is not a gap: {mentions:?}"
    );
    assert_eq!(
        cov["enumeration_may_be_incomplete"].as_bool(),
        Some(false),
        "a clean, fully-resolved type answer says so"
    );
}

/// The disclosure half. A reference the extractor SAW but resolution could not place must be named.
/// The fixture uses a qualified path to a module that does not exist in the project: the suffix
/// resolver finds no chunk whose module path ends in `nowhere`, so the edge is dropped -- unlike a
/// bare multi-candidate name, which attaches as AMBIGUOUS rather than being dropped (`graph.rs:536`).
#[test]
fn a_dropped_type_reference_is_reported_as_an_unresolved_extraction() {
    let (_dir, root, db, project_id, bin) = indexed(&[
        ("src/real.rs", "pub struct Widget { pub x: u8 }\n"),
        ("src/user.rs", "pub fn take(w: nowhere::Widget) -> u8 { 1 }\n"),
    ]);
    let cov = coverage_of(&db, &project_id, &root, &bin, "Widget");
    let unresolved = cov["extracted_but_unresolved"].as_array().unwrap();
    assert!(
        unresolved
            .iter()
            .any(|r| r["raw_target"].as_str() == Some("nowhere::Widget")
                && r["file_path"].as_str() == Some("src/user.rs")),
        "the dropped reference is named, not silently absent: {unresolved:?}"
    );
    assert_eq!(
        cov["enumeration_may_be_incomplete"].as_bool(),
        Some(true),
        "a named gap flips the boolean"
    );
}
```

Both tests use the helpers already in `rust/tests/coverage.rs` (verified 2026-09-04): `indexed(...)`
with the same signature as `impact.rs`'s, and
`coverage_of(&db, &project_id, &root, &bin, symbol) -> Value`. **`coverage_of` already returns the
`coverage` subtree**, so index into it directly (`cov["mentions_without_edge"]`), never
`cov["coverage"][...]`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test coverage a_resolved_type_reference a_dropped_type_reference`
Expected: both FAIL — the first because line 2 is reported as a gap, the second because
`extracted_but_unresolved` is empty.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/coverage.rs`, widen **both** queries:

`:315` (inside `extracted_calls`):
```sql
          WHERE project_id = ?1 AND rel_type IN ('calls', 'references')
```

`:379` (inside `extracted_but_unresolved`):
```sql
          WHERE project_id = ?1 AND rel_type IN ('calls', 'references')
```

Rename `extracted_calls` to `extracted_edges` and update its doc (`:308`) — it no longer maps calls
only. Update the module doc at `:17` so `extracted_but_unresolved` is described as covering both edge
kinds.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test coverage`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
git add rust/src/coverage.rs rust/tests/coverage.rs
git commit -m "feat(coverage): both screens cover type references

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: Move the measuring instrument

**Files:**
- Modify: `evals/src/hook.rs:32` (`declares_callable_in`), `:87-97` (verdict keys)
- Modify: `evals/tests/hook.rs:25`
- Test: `evals/tests/hook.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks — this is the eval harness, not the product.
- Produces: `hook-probe`'s `index_check` distinguishing a type declaration from a non-declaration, so
  the 63% number can move.

**Scope note.** This does **not** gate anything in production: `judge` (`rust/src/hook.rs:329-373`)
has no function check and already fires on these symbols. Without this task the feature works and the
improvement is simply invisible to `hook-probe`.

- [ ] **Step 1: Write the failing test**

Modify `evals/tests/hook.rs`. The existing assertion at `:25`

```rust
    assert!(declares_callable_in("pub struct Confidence;", "Confidence").is_none());
```

is now wrong — a struct IS a seed `impact` can hold. Replace it and add coverage:

```rust
    // A struct is now a seed `impact` can hold, so the screen must stop calling it a non-declaration.
    // A const and a struct field still are not: `cort` indexes no chunk for either (plan D9).
    assert_eq!(declares_callable_in("pub struct Confidence;", "Confidence"), Some("struct"));
    assert_eq!(declares_callable_in("pub enum CallForm {", "CallForm"), Some("enum"));
    assert_eq!(declares_callable_in("pub trait Emit {", "Emit"), Some("trait"));
    assert!(declares_callable_in("pub const TIMEOUT_S: u64 = 30;", "TIMEOUT_S").is_none());
    assert!(declares_callable_in("    trace_file: PathBuf,", "trace_file").is_none());
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd evals && cargo test --test hook`
Expected: FAIL — `declares_callable_in("pub struct Confidence;", ...)` returns `None`.

- [ ] **Step 3: Write minimal implementation**

In `evals/src/hook.rs`, add the three type keywords to the table in `declares_callable_in`:

```rust
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
```

Update the function's doc comment (`:25-31`): it currently justifies excluding non-callables. It must
now say that a type declaration is a seed because `cort` chunks it, while a `const`, `static`, `let`
or struct field still is not — the distinction is "does the pack chunk it", not "is it callable".

Its name no longer fits. Rename it `declares_seedable_in`, and rename the two verdict strings
`confirmed_function` → `confirmed_seed` and `rejected_not_a_function` → `rejected_not_a_seed`. The
keyword that decided the verdict is already returned and must keep being carried into the row, so a
reader can still tell a `fn` hit from a `struct` hit.

**The complete set of sites, enumerated** (searched 2026-09-04; nothing outside the `evals` crate
consumes these names, and no stored report under `evals/runs/` contains either key):

| File:line | What |
|---|---|
| `evals/src/hook.rs:32` | the function definition |
| `evals/src/hook.rs:113` | its only call site |
| `evals/src/hook.rs:92-93` | `DeclCheck::verdict()` — both strings |
| `evals/src/hook.rs:452-453` | the two JSON summary counters |
| `evals/src/hook.rs:457` | `confirmed_callable_in_searched_tree` → `confirmed_seeds_in_searched_tree` |
| `evals/tests/hook.rs:5` | the `use` |
| `evals/tests/hook.rs:25-54` | the assertions |
| `evals/tests/hook.rs:98-99` | the pinned verdict strings |

**Comparability caveat, which must reach the report text.** The 63% baseline in §Evidence was measured
under the old key. After this rename a `struct` fire moves from the rejected bucket to the confirmed
one, so post-change runs are **not** directly comparable to it — the population is the same, the
classifier is not. Task 8 Step 3's re-measurement is the comparable number, and `hook-probe`'s
`index_check_reading` must say so in the same sentence it explains the buckets.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd evals && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/yanggf/a/cortexyoung
cargo fmt --all --manifest-path evals/Cargo.toml
cargo clippy --manifest-path evals/Cargo.toml --all-targets -- -D warnings
git add evals/src/hook.rs evals/tests/hook.rs
git commit -m "feat(evals): the index check asks what cort chunks, not what is callable

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: Measure it, then write down what is true

**Files:**
- Modify: `README.md` (`:16-20`, `:28-30`, the v3/v4 upgrade note, `What each language actually gets`, limitation 8 at `:355-364`)
- Modify: `skills/ast-grep/SKILL.md:16`, `:22`

**Interfaces:**
- Consumes: Tasks 1-7, all committed.
- Produces: measured numbers, and documentation that matches the code.

**This task is allowed to fail the plan.** Stop and report rather than documenting a regression if:
index time regresses more than 2x, `verify-impact` grades below 100%, or the phantom rate in Step 4 is
anything other than zero.

- [ ] **Step 1: Record the before numbers**

```bash
cd /home/yanggf/a/cortexyoung
git stash list                      # confirm nothing pending
time cort index . 2>&1 | tail -5    # the OLD binary, still on PATH
```
Record `chunks`, `relationships`, `unparsed` and wall time. Baseline measured 2026-09-04:
`relationships: 2198`, and roughly 17,000 raw edges (`incremental.rs:402`).

- [ ] **Step 2: Rebuild, re-index, compare**

```bash
cd rust && cargo build --release && cd ..
time ./rust/target/release/cort index . 2>&1 | tail -5
```
Record the same four numbers. Expected raw-edge growth is about +16% (D5), not a doubling.

- [ ] **Step 3: Verify the two symbols that motivated the plan**

```bash
./rust/target/release/cort impact --symbol CallForm --depth 1 --coverage -f lean | head -20
./rust/target/release/cort impact --symbol HOOK_TARGETS --depth 1 --coverage -f lean | head -5
```
Expected: `CallForm` gives `seeds=1` and non-zero dependents with `type` in the sixth column.
`HOOK_TARGETS` is a `const` and **must still** report `seeds=0` — it is out of scope (D9), and a hit
there means the reference rule is matching something it should not.

- [ ] **Step 4: Measure the phantom rate — the number D6 defers to**

```bash
cd evals && cargo build --release
./target/release/cort-evals verify-impact --repo /home/yanggf/a/cortexyoung \
  --symbols CallForm,Chunk,Edge,SettingsError,CortError,Db --depth 1
```
Note the flag is `--symbols`, not `--symbol` (`evals/src/main.rs:104`).

Then adjudicate by hand, because `verify-impact` cannot see this class of error (D2): for each
dependent of `SettingsError`, open the cited line and confirm the module it names matches the module
the seed lives in. Record `phantoms / total`. **Any non-zero count stops the plan** and reopens D6.

- [ ] **Step 5: Measure coverage latency on a hub type**

```bash
time ./rust/target/release/cort impact --symbol CortError --depth 1 --coverage -f lean | tail -3
```
`extracted_but_unresolved` runs one prepared query per candidate row (`coverage.rs:397-411`), and
`CortError` has ~100 `type_identifier` matches. Record the wall time; if it exceeds 2s, report it as
a finding rather than documenting it as acceptable.

- [ ] **Step 6: Update the stated contracts and commit**

`README.md`:
- `:16-20` — the command overview lists only three call forms; add `type`.
- `:28-30` — the Rust overview says only functions and `impl` methods are chunks; add struct/enum/trait.
- The `Upgrade note` heading becomes `v3, v4 and v5`, with a paragraph: v5 widens `rel_type` to include
  `references` and `call_form` to include `type`; both tables are rebuilt in place with every column
  named explicitly and every row preserved; `graph_pending` forces the next incremental index to fall
  back to a full one.
- `What each language actually gets` — the Rust `impact` cell states that struct/enum/trait references
  are covered, that `const`/`static` are not, and that generic parameters are extracted and normally
  resolve to nothing.
- Limitation 8 (`:355-364`) describes only call and import rules; add the reference rules, the
  same-line chunk collision (D7) and the measured phantom rate from Step 4.

`skills/ast-grep/SKILL.md:16` and `:22` — state that Rust `impact` covers type references and list
`type` beside `bare/scoped/receiver`. Change nothing else in that file: it is deployed byte-for-byte
and `rust/tests/skill_format.rs` gates its shape.

```bash
cd /home/yanggf/a/cortexyoung
cargo test --manifest-path rust/Cargo.toml --test skill_format
git add README.md skills/ast-grep/SKILL.md
git commit -m "docs: record schema v5, type references, and the measured numbers

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage.** D1 → Task 1. D2 → Task 1 + Task 5's qualified test. D3 → Tasks 2, 3. D4 → Task 4.
D5 → Task 8 Steps 1-2. D6 → Task 1 comment + Task 8 Step 4. D7 → Task 2. D8 → Task 7. D9 → Task 8
Step 3's negative assertion. All fourteen review-record rows map to a task.

**Placeholder scan.** The three "use the existing helper" soft spots of v2 are gone: every test helper
is now named with its real signature, read from the test files on 2026-09-04 —
`indexed`/`impact_command` (`rust/tests/impact.rs:43,70`), `indexed`/`coverage_of`
(`rust/tests/coverage.rs:11,38`, and `coverage_of` returns the coverage subtree, not the whole
payload), `tmp_file`/`extract_real` (`rust/tests/chunker.rs:74,92`). Two named soft spots remain, both
deliberate: Task 1 Step 4 permits adjusting the expected `refs` vector to what the grammar actually
emits, and Task 8 Step 6's README edits are described rather than pasted because they are prose edits
to sections whose current wording must be read at the time.

**Type consistency.** `CallForm::Type` / `"type"` / rank 3 is identical across Tasks 2, 3, 4, 5, 8.
`references` is identical across Tasks 1-6. `$CALLEE` is the capture name in Task 1 and is what
`chunker` already reads for every edge rule. `declares_seedable_in` is introduced in Task 7 and used
nowhere earlier.

**Remaining known risk, stated rather than hidden.** Generic parameters produce reference edges that
resolve to nothing in this repo but could produce phantoms in a project that declares a real type
named `T`. This ships unmeasured on other people's repositories and measured on this one; Task 8
Step 4 is the gate, and D6 is the reopening condition.
