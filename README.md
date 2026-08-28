# cortexyoung — cort, offline code-intelligence over ast-grep + SQLite

`cort` is an offline code-intelligence CLI built on `ast-grep` (the only parser, never `sg`) and SQLite. One repo checkout, one SQLite index per project, no embeddings, no cloud DB, no servers.

Four shipped commands (plus `status`/`projects`/`delete` utilities):

- `cort index [--incremental] [path]` — build or incrementally refresh the index (`ast-grep` + SQLite)
- `cort struct -p '<pattern>' --lang <lang>` — structural search joined to enclosing symbols + 3 neighbours
- `cort context <symbol-or-query>` — "what else deals with X" (exact symbol or FTS recall, depth-1 neighbours, ~1500-token budget; seed bodies are head-truncated to 12 lines, pass `--content full` for the whole body)
- `cort impact --symbol <name[,name2,...]>` — reverse dependents (depth 3); accepts a comma-separated batch
- `cort impact --symbol <name>` — "what breaks if I change X" (reverse dependents, depth 3)

All three query verbs take `-f lean`: the same answer as one tab-separated row per result, at about a
fifth of the tokens of the default JSON. Agents should pass it; see "Token cost" below.

Routing for agents is in `skills/ast-grep/SKILL.md` — it states when to use `rg`, `ast-grep run`, `cort struct`/`context`/`impact`, and `xg`.

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
- Installs `cort` from this checkout to `~/.local/share/cortexyoung/cort` via `npm ci --omit=dev` (Node >= 22, `better-sqlite3` is the only runtime dependency) and shims `~/.cargo/bin/cort` or `~/.local/bin/cort`.
- Deploys `skills/ast-grep/SKILL.md` to `~/.claude/skills/ast-grep/SKILL.md` with a managed marker. Preflights collisions before mutating: skips if hash-equal, replaces if managed, refuses unmanaged collisions (use `--force` to backup and replace).
- Adds a single bounded idempotent `PATH` block to your shell profile (`.bashrc`/`.zshrc`/`.profile`) so `cort` and `ast-grep` are on `PATH`; removed on `--uninstall`.
- Records ownership in `~/.local/share/cortexyoung/manifest` (v2 `key:value` lines) — uninstall only removes what it installed, never a pre-existing binary.

**Requirements:** `curl` or `wget`, `tar`, `unzip`, `sha256sum`/`shasum`. Node >= 22. Rust only needed for the cargo fallback.

## Update

```bash
git pull --no-rebase
./install.sh              # idempotent — skips hash-equal skill, no duplicate PATH block
```

With xgrep (opt-in):

```bash
./install.sh --with-xgrep # idempotent xg install + xgrep skill deploy
```

## Uninstall

```bash
./install.sh --uninstall  # removes managed cort + ast-grep binaries, CORT_HOME, skills, PATH block, manifest
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

Measured on tasks that require a relationship walk (`evals/relation-cost.mjs`, deterministic, no model in
the loop; `cct`, 2,713 chunks; medians over 6 auto-picked multi-hop symbols):

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
across 5 chains (`evals/verify-impact.mjs`).

**Corrected positioning: `cort` is an agent tool for relationships, and `rg`/`xg` stay the right tool for
strings.** The claim "graph adds correctness nowhere" is withdrawn; it was only ever tested where the graph
could not apply.

What is still unproven, and the prerequisites for the end-to-end run that would settle it, are recorded in
`docs/2026-08-28-end-to-end-eval-wip.md`: the metric-capture path exists (`claude -p --output-format
stream-json` exposes every `tool_result` payload and `permission_denials`), the agent config must be isolated
or 16,067 tokens of hooks/plugin text enter every cell and swamp a ~1.1k lean payload, and `projectId` is
derived from `cwd` — running `impact` for the `cct` venue from this repo silently returns `seeds=0`. No cell
has been run yet.

## Documented limitations (contracts, not apologies)

1. **Index staleness:** the index lags unsaved edits and brand-new untracked files. Every `cort` command returns JSON with `index_is_stale`; when `true`, run `cort index --incremental` before trusting the answer, or fall back to `rg`. A brand-new untracked file is invisible to `cort` until the next index. `cort index` must have been run once for the project.
2. **`chunk_id` stability:** `chunk_id` is stable only while a symbol's first line does not move — inserting lines above a symbol changes its id.
3. **`context` is FTS-only:** `cort context` uses SQLite FTS keyword recall, not semantic search or embeddings. No vector, no RRF, no reranking.
4. **Const-bound functions are chunks; plain aliases and collection transforms are not.** `const f = (x) => ...`,
   `const f = function () {...}` and `const f = factory(async () => {...})` are indexed (`src/pack/rules/*.yml`,
   `cort-*-chunk-const-function`). `const rows = xs.map(x => x)` stays data — the wrapper form only counts when
   the callee is a bare identifier, so method-call transforms never become symbols. Rebuilding an index written
   before this rule adds ~1% chunks and ~2% relationships; `cort index` (full) is required once.
5. **Name-based target resolution:** relationship targets are resolved by symbol name. A same-named symbol in an unimported file can still surface as `AMBIGUOUS`, even if it is not actually imported.
6. **`--lang` is required on `struct`:** `cort struct -p '<pattern>' --lang <lang>` fails with `{"error":"missing_lang"}` if `--lang` is absent. It also drives the pattern pre-flight that turns a malformed pattern into `{"error":"parse_failed"}` instead of a silent empty result. The binary is `ast-grep`, never `sg`.
7. **FTS tokenizer is bare `unicode61`:** the design calls for `unicode61 "remove_diacritics 1" "tokenchars ._$"`, but the bundled SQLite (3.49.2 via `better-sqlite3` 11.10.0) rejects every parameterised `unicode61` form. Consequence: `cort context` keyword recall splits identifiers on `.`, `_` and `$` — searching `foo.bar` matches `foo` and `bar` separately, and diacritics are not folded. CJK still tokenizes. `src/schema.sql` carries a `NOTE` and reverting is one line once a SQLite build accepts the parameters.

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
- [`better-sqlite3`](https://github.com/WiseLibs/better-sqlite3) — MIT, the only runtime npm dependency of `cort`.
- [`xgrep` (momokun7/xgrep)](https://github.com/momokun7/xgrep) — MIT/Apache-2.0, optional `--with-xgrep` extra (`xg` v0.7.0, `xgrep-search` on crates.io).
- [`ripgrep`](https://github.com/BurntSushi/ripgrep) — MIT OR Unlicense, not installed by this repo; expected on the host.

## License

MIT — see [LICENSE](LICENSE). Third-party notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
