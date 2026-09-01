# cortexyoung — cort, offline code-intelligence over ast-grep + SQLite

`cort` is an offline code-intelligence CLI built on `ast-grep` (the only parser, never `sg`) and SQLite. One repo checkout, one SQLite index per project, no embeddings, no cloud DB, no servers.

The repository is pure Rust: the product crate lives in [`rust/`](rust/), the dev-only agent-eval
harness in [`evals/`](evals/), and no JavaScript, TypeScript or Python exists as executable code. Bash
appears only in `install.sh` and `tests/install-smoke.sh`, where the platform requires a shell.

Six shipped commands (plus `status`/`projects`/`delete` utilities):

- `cort index [--incremental] [path]` — build or incrementally refresh the index (`ast-grep` + SQLite)
- `cort struct -p '<pattern>' --lang <lang>` — structural search joined to enclosing symbols + 3 neighbours
- `cort context <symbol-or-query>` — "what else deals with X" (exact symbol or FTS recall, depth-1 neighbours, ~1500-token budget; seed bodies are head-truncated to 12 lines, pass `--content full` for the whole body)
- `cort impact --symbol <name[,name2,...]>` — "what breaks if I change X": reverse dependents to
  depth 3, accepting a comma-separated batch. In `lean`, each row carries the dependent's definition
  line *and* the call site that ties it to the seed (`@626`), tagged with the call shape that attached
  it (`bare` / `scoped` / `receiver`)
- `cort read <file> [--start N] [--end N]` — read a file or line range and persist it as a reading note; unchanged repeats come from SQLite
- `cort recall <query>` — FTS lookup over previously read files/fragments (default 12-line heads; pass `--content full` for stored bodies)
- `cort usage [days]` — local per-machine usage stats (best-effort; 1–90 days, default 30)

All query/read verbs take `-f lean`: the same answer in a compact agent-oriented format, at about a
fifth of the tokens of the default JSON. Agents should pass it; see "Token cost" below.

Rust (`.rs`) is indexed through the pinned `ast-grep` 0.45.2 rule pack. Top-level functions and `impl`
methods are stored as symbol-scoped chunks, so `cort context <symbol> --content full -f lean` returns one
function body rather than forcing an agent to read a large source file.

Reading notes are content-addressed and project-local. `cort read` records the exact file/range on first
use and reports `source:"store"` on an unchanged repeat. Each entry carries file hash, size, and mtime;
`cort recall` validates them and removes entries whose source changed, so stale text is never returned as
a remembered reading. Run `cort index` once before using either command.

Routing for agents is in `skills/ast-grep/SKILL.md` — it states when to use `rg`, `ast-grep run`, `cort struct`/`context`/`impact`, and `xg`.

## What each language actually gets

Capability is per language, because the extractor pack is per language. Do not assume a verb works
just because the binary runs.

| Language | `struct` | `context` (symbol slice) | `read`/`recall` | `impact` (relationship graph) |
|---|---|---|---|---|
| TypeScript, TSX, JavaScript | yes | yes | yes | **yes** — `edge:calls` + `edge:imports` rules ship |
| Python | yes | yes | yes | **yes** — same |
| Rust | yes | **yes** (free functions, `impl`/trait methods as `Type::method`) | **yes** | **partial** — bare, qualified `Type::method()`, module-path `crate::m::f()` via suffix + `use` (`edge:calls` + `edge:imports`), and gated receiver `x.m()` |

For Rust, `cort context <symbol> --content full -f lean` is the supported use (that is the case the
27k→89-token measurement below covers). Rust `impact` covers the indexed call shapes and resolves
module paths by suffix — treat its output as candidates. For the precise caller list use `cargo check` /
`cargo build` errors — a compiler beats a name-resolved graph here. See `docs/2026-08-28-real-session-cost.md` §1.3.

## Local usage recording (opt-out by not existing: it is offline, plain SQLite)

Every cort command appends one row to a **central, local-only** SQLite database at
`$CORT_CACHE_DIR/usage.db` (default `~/.cache/cortex-ng/usage.db`). Nothing ever leaves the
machine; there is no network code in cort at all.

What is recorded per invocation: command name, status (ok/error) and error code only, the
project id, allowlisted arguments (symbol, project-relative path, line range), for reads the
requested vs effective content mode and `source`, whether the index was evaluated as stale, the
exact rendered output size (`bytes_out`), and `saved_bytes` — on a receipt cache hit, the number
of raw body bytes the receipt omitted from the response.

What is never recorded: file contents, `recall` queries, `struct` patterns, unresolved free-text
`context` queries, clap/error messages, absolute home paths. The recorder is best-effort: it
never retries, never blocks beyond 25 ms, and a logging failure can never change a command's
output or exit code — which also means the report can only ever under-count.

Read the report with `cort usage [days]` (1–90, default 30; retention is 90 days, pruned at
most once per day). Two fields deserve care: `receipt_hit_rate` counts only successful
`auto`-mode reads, so toggling `--content full/receipt` does not distort it; and `saved_bytes`
means raw body bytes omitted from receipts — it is **not** a claim about total output, network
or cost savings. The report itself is written to the log after it is rendered, so an invocation
never appears in its own report.

## Install

```bash
./install.sh              # cort v0.1.0 + ast-grep v0.45.2 + skill to ~/.claude/skills/ast-grep/
./install.sh --check      # verify without mutating
./install.sh --uninstall  # remove managed artifacts only (reads manifest v2)
./install.sh --force      # on unmanaged skill collision: backup and replace
./install.sh --with-rustup # bootstrap rustup if cargo is missing
./install.sh --with-xgrep  # opt-in: also install xg v0.7.0 (xgrep-search crate) + xgrep skill
```

**What it does (default, without `--with-xgrep`):**

- Downloads pinned `ast-grep` v0.45.2 prebuilt `app-<target>.zip` for your platform (Linux x86_64/aarch64, macOS x86_64/arm64) from GitHub Releases and verifies SHA-256 (repo-maintained; upstream publishes no checksums) — fail-closed: an empty or mismatched checksum refuses to install. Falls back to `cargo install ast-grep --version 0.45.2 --locked` (requires Rust 1.88+).
- Builds `cort` from `rust/` with `cargo build --release --locked` on **every** run and installs the binary plus its ast-grep pack (`src/pack`, located at runtime via `CORT_PACK_DIR`) to `~/.local/share/cortexyoung/cort`, shimming `~/.cargo/bin/cort` or `~/.local/bin/cort`.
- Deploys `skills/ast-grep/SKILL.md` to **both** agent homes — `~/.claude/skills/ast-grep/SKILL.md` and `~/.codex/skills/ast-grep/SKILL.md` (honouring `CODEX_HOME`) — **byte-for-byte the repo file**. The installer writes nothing inside the document: the frontmatter block holds keys only (`name`, `description`, and nothing of ours), both loaders anchor that fence to line 1, and `rust/tests/skill_format.rs` fails the build if a source stops parsing. Ownership lives in `.cortexyoung-managed` beside the skill, which records the SHA-256 of the bytes we deployed — so a hand-edit of a deployed `SKILL.md` reads as someone else's file and is refused, not silently overwritten. Edit `skills/<name>/SKILL.md` in this repo instead; one source feeds both homes. Preflights collisions before mutating: skips if hash-equal, replaces what it owns, refuses what it does not (use `--force` to backup and replace). Uninstall removes the document and its stamp, and nothing else.
- Adds a single bounded idempotent `PATH` block to your shell profile (`.bashrc`/`.zshrc`/`.profile`) so `cort` and `ast-grep` are on `PATH`; removed on `--uninstall`.
- Records ownership in `~/.local/share/cortexyoung/manifest` (v2 `key:value` lines) — uninstall only removes what it installed, never a pre-existing binary.

**Requirements:** `curl` or `wget`, `tar`, `unzip`, `sha256sum`/`shasum`. Rust (cargo) — cort is the Rust binary; `--with-rustup` bootstraps rustup if cargo is missing.

## Update

```bash
git pull --no-rebase
./install.sh              # idempotent — cargo decides build freshness (0.04s when nothing changed), skips hash-equal skill, no duplicate PATH block
```

With xgrep (opt-in):

```bash
./install.sh --with-xgrep # idempotent xg install + xgrep skill deploy
```

## Upgrade note — index schema v3 and v4

Schema v3 adds a `raw_edges` table: the unresolved call/import matches that the relationship graph
is derived from. It exists because resolution spans files — re-indexing one file used to delete its
target chunks, `ON DELETE CASCADE` took the edges pointing at them with it, and the unchanged
caller's edge was never rebuilt (audit F-01, `docs/2026-08-29-project-audit-root-causes-and-remediation.md`).

Schema v4 adds two columns: `raw_edges.call_form` / `relationships.call_form` (`bare` | `receiver` |
`scoped`), and `relationships.call_site_line` — the line inside the caller that names the callee. The
form exists because a receiver call (`tally.add()`) is resolved by a different rule than a bare one,
and the line exists so that one edge can be checked by reading one line. Both are carried into
`impact` output (`@626 receiver`), and both are recorded for every edge, not only receiver ones.

Nothing to do by hand in either case. An index written by an older cort is detected on first use,
reported as stale, and rebuilt in full by the next `cort index --incremental` (which falls back to a
full index while the graph is pending, then clears the marker). An older database is upgraded in
place with `ALTER TABLE` — `CREATE TABLE IF NOT EXISTS` never adds a column to a table that already
exists — and the added columns stay `CHECK`-constrained, so a form this build does not know cannot be
stored. Until that rebuild runs, `impact` results come from the pre-upgrade graph (with `@-` where no
call site was recorded) and `index_is_stale` is `true`.

## Uninstall

```bash
./install.sh --uninstall  # removes managed cort + ast-grep binaries, CORT_HOME, skills (claude + codex), PATH block, manifest
```

Pre-existing binaries and unmanaged skills are never removed. With `--with-xgrep`, the managed `xg` binary and `xgrep` skill are also removed if owned.

## The ast-grep 0.45.2 pin — why fail-closed

`ast-grep` is the only parser (never add an in-process parser, never call `sg` — on Linux `/usr/bin/sg` is `setgroups(1)`). The pin is `0.45.2` exactly: the installer verifies the download's SHA-256 against repo-maintained hashes for `app-<target>.zip` before extracting. An empty expected hash is fatal (`no checksum on record`), and a mismatch is fatal (`refusing to install an unverified binary`). This is fail-closed because installing an unverified binary would silently change parse behaviour — `parse_failed` detection, pattern validation, and struct/context/impact all depend on the same parser version.

Alternative install (same pin, same fail-closed version check):

```bash
cargo install ast-grep --version 0.45.2 --locked  # requires Rust 1.88+
```

## Eval results (2026-08-26)

Three three-arm agent evaluations were run with the shipped harness (`rg+Read` vs `ast-grep+Read` vs `cort`,
fresh subagent per task, strict tool policies, hand-verified labels):

| venue | success (rg / ag / cort) | mean tokens (rg / ag / cort) | verdict |
|---|---|---|---|
| this repo (76 chunks) | .80 / .80 / .80 | 186k / 200k / 388k | STOP |
| cct (2,531 chunks) | 1.00 / 1.00 / 1.00 | 180k / 486k / 656k | STOP |
| cct, heaviest 3 cells after ergonomics fixes | 1.00 / 1.00 | 392k / 2.19M* | STOP |

\* one cort run spiralled into a 171-turn verification loop — single-seed cells are dominated by agent
behavioural variance; excluding it entirely still leaves cort above baseline.

Reading at the time: answer quality is identical across arms once labels are clean; `rg` + Read is the
cheapest arm in every round because current models are grep-native. Cort's graph adds correctness nowhere
and costs 2-3.6x in total context. **Full evidence: `evals/runs/2026-08-26{,-cct,-cct-r3}/`.**

## Re-analysis (2026-08-28): the gate was measured on the wrong case

The STOP verdict is real but its scope was overstated. Three defects made it unavoidable, and none of them
are "the graph does not help". Full write-up (method, numbers, what remains unproven):
`docs/2026-08-28-graph-cost-reanalysis.md`.

1. **The deciding metric was never recorded.** All 30 cells across all three rounds have
   `tool_return_tokens: null` and `read_calls: null`. The only number ever compared is `total_tokens`,
   which is `input + cache_read + output` — i.e. turns multiplied by an accumulating transcript. It measures
   how much the agent talked, not what the tool cost. `evals/runs/2026-08-26/token-raw.json` (the only round
   with a token breakdown, 15 cells) shows it directly: `cache_read` is 69-94% of every cell, median 88%.
2. **No task needed the graph.** Of the 10 tasks, every `expected_symbols` label is depth 1 or a literal
   string / config lookup. Three of them (`where-is-confidence-set`, `where-market-open-decided`,
   `do-bindings-trace`) are answered by one `rg`; `rg` solved `where-market-open-decided` in 1 turn and
   `cort` took 20. The transitive closure in `getTransitiveDependents` — the actual product — was never
   asked to run.
3. **The graph was blind to the venue's most important symbols.** `cct`'s route handlers are
   `export const handleX = createHandler('...', async (req) => {...})`. The chunker matched only
   `function_declaration`, `class_declaration` and `method_definition`, so those bindings produced **no
   chunk**, calls inside them had `source_symbol: null`, and `relationshipRowsForFile` dropped those edges.
   Round-2's own hand labels (`handlePreMarketBriefing`, `handleEndOfDaySummary`) were therefore
   unreachable by `cort impact` at any depth — the cort arm scored 1.00 only because the agent went and
   read files, which is precisely the turn inflation that produced the STOP.

Measured on tasks that require a relationship walk (deterministic probe, no model in the loop; `cct`,
2,713 chunks; medians over 6 auto-picked multi-hop symbols). **Provenance:** that probe was
`evals/relation-cost.mjs`, which lived in the JS tree and was deleted by the Rust cutover
(`1a4052cc`), so these numbers are historical evidence, not a reproducible command; recover it with
`git show 1a4052cc^:evals/relation-cost.mjs`, or re-price it as a `cort-evals` subcommand (the
harness is Rust now, so that probe belongs in `evals/`):

| hops | `cort impact -f lean` | `rg` + reads to reach the same set | ratio | share of rg cost that is reads | rg hit precision |
|---|---|---|---|---|---|
| 1 | 968 tok | 16,584 tok | 14.8x | 83% | 0.67 |
| 2 | 1,022 tok | 86,949 tok | 67x | 86% | 0.42 |
| 3 | 1,136 tok | 127,531 tok | 62x | 87% | 0.57 |

`rg`'s cost is dominated by **reads, not greps** (83-87% at every depth), and it grows with hop depth
because each hop hands back names that must be searched again — and every hit must be opened to learn which
symbol encloses it. Of the hits `rg` returns, the share actually on a dependency path ranged 0.04-0.90
across the six symbols (medians in the table); a common name like `logInfo` or `createHandler` is the
expensive case, because the name is everywhere and most of that "everywhere" is irrelevant. `cort`'s cost
is nearly flat in depth because the walk is one recursive SQL query.

Soundness of those same answers, checked independently against file text: 100/100 dependents confirmed
across 5 chains (`cort-evals verify-impact`, re-run on the current binary — see
[`evals/README.md`](evals/README.md)). Since schema v4 the same checker also grades the *single line*
each row reports: 117/117 at `line_precision` 1.0 on those cct chains, and 64/64 on four chains in this
repo — where the body-level check scores 0.667 on `Tally::add` because the body contains `tally.add` and
the seed was asked for as `Tally::add`. That is the same 5 chains re-indexed with the previous build
(23/4/4/20/66 dependents) against the v4 index (identical counts): the upgrade moved no TypeScript
baseline, because no TypeScript rule changed.

**What the two new columns cost.** The `lean` dependent row grew from four fields to six, so the same
answer is bigger: `logInfo --depth 3` 4,411 → 5,069 bytes, `getCurrentTimeET` 1,641 → 1,874,
`handleReportsStatus` 835 → 873, `createBacktestingStorage` 1,414 → 1,619 (cct, same two binaries, same
index contents, ~15% average). The 7.7x payload advantage over the shell arm becomes ~6.7x. It buys the
removal of the read that used to be required to check a row, which is the half of the goal sentence that
was still open.

**Corrected positioning: `cort` is an agent tool for relationships, and `rg`/`xg` stay the right tool for
strings.** The claim "graph adds correctness nowhere" is withdrawn; it was only ever tested where the graph
could not apply.

What that section left unproven has since been measured. Two rounds of the end-to-end two-arm eval
(`cort-evals run-agents`, 5 graph-required tasks x 2 arms x 2 rounds = 20 cells, metrics-only under
`evals/runs/2026-08-30-graph{,-sample2}/`) give the `cort` arm 10/10 success at a mean 992
tool-return tokens against the baseline arm's 4/10 at 7,642, and the spec section 8 gate returns
`cort_beats_ast_grep=true` for the first time with the metrics actually recorded. Two readings are
load-bearing and are spelled out in
[`docs/2026-08-29-project-audit-root-causes-and-remediation.md`](docs/2026-08-29-project-audit-root-causes-and-remediation.md)
sections 13f and 13n: the comparator is **an agent's whole shell**, not `rg` — headless
`--allowedTools` does not bind Bash (audit F-11), so most cells carry `arm_held: false` and must not be
averaged as a tool-vs-tool A/B — and the labels are the graph's own output confirmed against file text,
which rules out fabrication but is not compiler-grade truth.

The demand side points the other way and is measured separately:
[`docs/2026-08-28-real-session-cost.md`](docs/2026-08-28-real-session-cost.md) goes through 1,565 real
prompts from the two heaviest repos and finds relational questions rare in daily use. `cort` is cheap
when a relationship walk is actually asked for; it is not asked for often. `docs/2026-08-28-end-to-end-eval-wip.md`
is the historical prerequisite note behind that eval run, marked superseded, including the one claim in it
that measurement falsified.

### Demand, re-measured (2026-08-31)

The 2026-08-28 note concluded "0 of 1,565 real prompts asked about code relationships" — and its
transcripts have since been deleted by Claude Code's 30-day retention, so it can no longer be recomputed.
[`docs/2026-08-31-demand-recheck.md`](docs/2026-08-31-demand-recheck.md) re-measures it with a committed
tool (`cort-evals demand`) over the 301 transcripts that still exist on this machine (95 Claude Code + 206 Codex): 1,214 genuine user
instructions contain **one** relationship question (0.08%), and 4-7 (0.33-0.58%) are instructions that
cannot be done correctly without a call-site set — all of them delete, refactor or review. 42% of what
arrives as a "user message" is a pasted agent report being fact-checked. Every hit is listed with the
needles that fired, and the hand verdicts are committed beside them.

So the goal is stated where it can be argued with, and in [`AGENTS.md`](AGENTS.md) as the project's
long-term direction: **make the caller-set enumeration an agent already performs — and often gets
wrong — both cheap and checkable.** Cost per use is settled (7.7x smaller tool payload at ~4x fewer
turns, 10/10 vs 6-of-10-wrong on the same tasks). Checkability moved on 2026-08-31: schema v4 records
the line that names the callee and the shape the call arrived as, `impact` prints both, and
`cort-evals verify-impact` now grades an edge against that single line — 117 of 117 dependents
confirmed across the 5 cct chains at `line_precision` 1.0, and 64 of 64 across 4 chains in this repo,
where the older whole-body check scores 0.667 on `Tally::add` because the body says `tally.add` and the
seed was asked as `Tally::add`. What is still open is stated in
[`docs/2026-08-31-recall-wip.md`](docs/2026-08-31-recall-wip.md). The boolean half of it landed the same
day (coverage-v2): `enumeration_may_be_incomplete` now has two causes — a named gap row, or a file the
screen never read — and `unparsed` is advisory, after two chunk-less files in this repo were enough to
set the flag for every one of 60 sampled seeds. What remains is the receiver gate's recall: it attaches
9 of 4,833 receiver call sites here.


## Documented limitations (contracts, not apologies)

1. **Index staleness:** the index lags unsaved edits and brand-new untracked files. Every `cort` command returns JSON with `index_is_stale`; when `true`, run `cort index --incremental` before trusting the answer, or fall back to `rg`. A brand-new untracked file is invisible to `cort` until the next index. `cort index` must have been run once for the project. Staleness covers two things, not one: a file whose indexed content no longer matches disk, and a relationship graph that has not been recomputed since its chunks changed (the `graph_pending` marker — set by an interrupted index or an older-schema database — makes every graph answer stale instead of quietly wrong).
2. **`chunk_id` stability:** `chunk_id` is stable only while a symbol's first line does not move — inserting lines above a symbol changes its id.
3. **`context` is FTS-only:** `cort context` uses SQLite FTS keyword recall, not semantic search or embeddings. No vector, no RRF, no reranking.
4. **Const-bound functions are chunks; plain aliases and collection transforms are not.** `const f = (x) => ...`,
   `const f = function () {...}` and `const f = factory(async () => {...})` are indexed (`src/pack/rules/*.yml`,
   `cort-*-chunk-const-function`). `const rows = xs.map(x => x)` stays data — the wrapper form only counts when
   the callee is a bare identifier, so method-call transforms never become symbols. Rebuilding an index written
   before this rule adds ~1% chunks and ~2% relationships; `cort index` (full) is required once.
5. **Name-based target resolution:** relationship targets are resolved by symbol name. A same-named symbol in an unimported file can still surface as `AMBIGUOUS`, even if it is not actually imported.
6. **`--lang` is required on `struct`:** `cort struct -p '<pattern>' --lang <lang>` fails with `{"error":"missing_lang"}` if `--lang` is absent. It also drives the pattern pre-flight that turns a malformed pattern into `{"error":"parse_failed"}` instead of a silent empty result. The binary is `ast-grep`, never `sg`.
7. **FTS tokenizer is bare `unicode61`:** the design calls for `unicode61 "remove_diacritics 1" "tokenchars ._$"`, but the bundled SQLite that ships with rusqlite 0.32 rejects every parameterised `unicode61` form (the JS reference via `better-sqlite3` had the same limit). Consequence: `cort context` keyword recall splits identifiers on `.`, `_` and `$` — searching `foo.bar` matches `foo` and `bar` separately, and diacritics are not folded. CJK still tokenizes. `src/schema.sql` carries a `NOTE` and reverting is one line once a SQLite build accepts the parameters.
8. **`impact` is only as good as a language's edge rules:** the relationship graph indexes the call
   shapes the language's `edge:calls`/`edge:imports` rules in `src/pack/rules/` capture. For Rust that
   is three shapes — bare (`foo()`), qualified (`Type::method()`) and receiver (`x.method()`) — plus
   module-path calls (`crate::m::f()`) resolved by name and module-path suffix with `use` statements
   feeding the same map; only the internal prefixes `crate::`, `self::`, `super::`, `::` are rescued,
   so a dependency call like `Vec::new` stays unresolved and visible instead of inventing an edge
   (`docs/2026-08-31-rust-qualified-call-resolution.md`). Measured on this repo: the rule added 18
   module-path edges, all real (`usage::now_ms()`, `arms::resolve_binary()`, `coverage::attach()`), and
   narrowed six previously-`AMBIGUOUS` bare calls onto the module their `use` names; `Vec::new` (75
   sites), `fs::write` (44) and `String::new` (28) still attach nothing and stay visible as gaps.
   **What it cannot see:** a qualifier that is a *dependency* module while a local module carries the
   same name. `use std::fs;` + `fs::write(..)` in a project that ships `src/fs.rs::write` attaches to
   the local one as `INFERRED` -- not `AMBIGUOUS`, because the external crate is not indexed and has no
   candidate to disagree with. Separating them needs the crate's own name or `mod` declarations (the
   undecided half of "B"); the behaviour is pinned by
   `a_std_module_qualifier_that_matches_a_local_module_file_still_attaches`, so a relaxation cannot
   arrive unnoticed. Use `cargo check`/`rg` for the precise Rust caller list (see the capability table
   above).

   The Rust receiver shape is gated, and the gate is a name test, not a type test: `x.m()` becomes an
   edge only when `m` belongs to exactly one symbol in the project *and* `x` can be that symbol's owner
   (`self` inside the same `impl`, or a receiver whose last segment equals/prefixes/suffixes the type
   name after normalising case and underscores). Measured at `a0269cda`: 4,833 receiver sites, 9
   attached edges, all 9 correct; uniqueness alone would have attached 25, of which 13 were correct and
   12 invented -- `e.kind()` onto a test fixture's `FailFs::kind`, `status.code()` onto a helper named
   `code`, `.chain()` onto a function named `chain` (row by row, with the source line each one claims:
   `evals/runs/2026-08-31-schema-v4/receiver-gate-counterfactual.json`). The same query on the current
   tree gives 5,212 sites and 12 attached, 12 correct; re-derive either side with
   `cort-evals recall-exp --venue .` (text-side population) and `cort status` plus
   `SELECT ... WHERE call_form='receiver'` (what got attached) rather than trusting a number that was
   written down on a different day. The gate also refuses true calls whose variable
   carries no trace of its type: `b.problem()` onto `BatchRead::problem` (3 sites), `err.to_json()` onto
   `CortError::to_json`. Every refusal is reported by `--coverage` as an `extracted_but_unresolved` row,
   so a refused edge is visible as a gap and never silent. TypeScript/TSX/JavaScript keep their single
   `call_expression` rule (no form suffix, no gate): their recorded eval labels depend on the existing
   recall, and putting the gate there would move a baseline nobody has measured yet.
9. **`status.unparsed` also counts files with no symbols:** `extract_file` degrades to a single
   FTS-only `unparsed` chunk on timeout, on a non-zero `ast-grep` exit, **and** when the scan returns
   zero records. A file that legitimately declares no functions (e.g. `rust/src/lib.rs`, which is only
   `pub mod` lines) is therefore included in that count. It does not mean the parser failed.
10. **Reading recall is lexical and source-validated:** `cort recall` searches only fragments previously
   captured by `cort read`; it is FTS5, not semantic memory. Changed or deleted source files invalidate
   their stored readings. Reading notes survive full and incremental re-indexing when the source is unchanged.

What landed on the recall line, and what is still open (`unparsed` and the gap boolean, the wording
that has to change with it, trait declarations labelled `call`, the ±2-line tolerance), is recorded in
[`docs/2026-08-31-recall-wip.md`](docs/2026-08-31-recall-wip.md), together with the measured numbers
that killed the alternative (Rust `use`/`mod` import edges).

11. **`impact --coverage` is a recall *screen*, not a completeness proof, and `incomplete` is a warning
   light, not the answer.** It compares the graph with `raw_edges` and with the text of indexed files,
   and reports three layers: mentions that produced no edge, edges dropped during resolution, and blind
   files. **Read the rows**: `true` has exactly two causes and `why` names them -- a row in either of the
   first two layers, or a file this screen never read (`unindexed`, or `scan_skipped` for a file over
   2 MB or unreadable). `false` means only "every file it read produced no gap signal for that seed",
   never "verified none". Known holes, each one measured rather than assumed: a caller in a file the
   indexer does not read at all (`.sh`, `.txt`, config, or anything under `dist/`, `build/`, `target/`,
   `node_modules/`) is invisible to every layer and does not flip the flag either; a re-export chain
   (`export { x as y }`, then called as `y`) is flagged at the barrel line, not at the eventual caller; a
   mention within 2 lines of an extracted call is treated as covered, so nearby prose can be swallowed;
   name-in-string and name-in-comment matching is lexical, so a common word (`get`, `add`, `new`) yields
   candidates that must be triaged, and Rust lifetimes before a name can be labelled `quoted`. A call
   that exists only after **macro expansion** is silent too: no file on disk names the callee, so no
   layer of the screen can fire and `false` means nothing there — that one is inherent to a text
   comparison and is disclosed rather than fixed (`cargo expand`/`cargo check` is the tool for it).
   **`unparsed` is advisory (coverage-v2, 2026-08-31):** a file with no chunks -- barrel, types-only,
   `pub mod`-only `lib.rs` -- has no edges by construction but *is* text-scanned, so its callers arrive as
   rows. It used to flip every seed: two such files in this repo (four in cct) made the boolean true for
   every symbol in the project, which is not a warning light but noise, and noise is what agents ignore
   when the real warning comes. The paths are still listed under `blind_files.unparsed_example`, `why`
   still says `unparsed_files`, and the `lean` output carries the sentence
   `blind unparsed advisory: text-scanned, no edges; does not flip incomplete`.

   **The printed list is capped at 20 rows, and lean says when it cut them (2026-09-01).** `gap_count`
   in JSON has always been every row found, but `lean` printed the length of the capped array as
   `no_edge=`, so a reader of the format the routing skill mandates saw `no_edge=20` over 51 rows and
   had no way to know. `no_edge=` is now the full count and a `miss	truncated	shown=N	of=M` row
   precedes the rows themselves. Rows are ordered by cause severity, so a cut always drops the least
   severe first; `-f json` returns the remainder. Pinned by
   `a_truncated_gap_list_says_how_many_rows_it_dropped` (`rust/tests/render.rs`).

## What is deliberately not built

These stay deferred and must not appear (spec section 8). Nothing below was built in the 2026-08-28 pass:
that pass only fixed recall and payload inside the four verbs that already ship, and re-specified the gate.

- `rewrite` (`cort rewrite` / `ast-grep --rewrite` wiring, dry-run, `--interactive`, `--update-all`)
- `modules` (`cort modules` Louvain Phase-1 greedy community detection)
- `--watch` (file-watcher with `inFlight` serialization)
- `impact --from-diff` (diff-aware blast radius)
- `search` as a first-class verb (`cort search` — use `cort struct` / `cort context` instead)
- `embeddings` / `cort embed --backfill` (`ALTER TABLE chunks ADD COLUMN embedding BLOB`, BGE, dense search, three-arm RRF)

## Archival access to V6

Cortex V6 (AST chunking, embeddings, Turso, relationship graph, `cortex` CLI) is archived at tag `v6-final`:

```bash
git show v6-final:docs/2026-08-25-audit-and-repositioning.md
git show v6-final:CLAUDE.md
git show v6-final:cli/src/index.ts
git show v6-final:README.md
```

Any path can be inspected via `git show v6-final:<path>` without checking out the tag.

## How the skill is used

The `ast-grep` skill teaches agents when to use `rg` vs `ast-grep` vs `cort` — see `skills/ast-grep/SKILL.md` (also installed to `~/.claude/skills/ast-grep/`). `rg` for fresh/short/small and for finding strings, `ast-grep run` for one shape, `cort struct` for shape +
neighbours, `cort context` for neighbourhood, `cort impact` for multi-hop blast radius, `xg` only when
`command -v xg` succeeds and the task is repeated literal-string search. The skill states the measured
break-even so an agent does not route a string question into a graph tool.

## Upstream credits

- [`ast-grep`](https://github.com/ast-grep/ast-grep) v0.45.2 — MIT, installed from GitHub Releases `app-<target>.zip` (repo-maintained SHA-256) or `cargo install ast-grep --version 0.45.2 --locked`.

- [`xgrep` (momokun7/xgrep)](https://github.com/momokun7/xgrep) — MIT/Apache-2.0, optional `--with-xgrep` extra (`xg` v0.7.0, `xgrep-search` on crates.io).
- [`ripgrep`](https://github.com/BurntSushi/ripgrep) — MIT OR Unlicense, not installed by this repo; expected on the host.

## License

MIT — see [LICENSE](LICENSE). Third-party notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
