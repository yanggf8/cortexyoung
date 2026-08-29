# Agent eval harness

## Protocol

- Same model, same task set, one full trace per arm.
- The harness is the `evals/` crate (`cort-evals`) — dev-only, never installed by `install.sh`.
  Two arms ship today (`run-agents`): `rg+Read` and `cort`, driven over the graph-required task set
  `evals/tasks-graph.json`. The third arm in `summary::ARMS` (`ast-grep+Read`) is the historical
  three-arm gate from rounds 1-3, kept so the archived rows still aggregate; the current runner
  compares against `rg+Read`.
- Layout: `src/stream.rs` transcript parsing, `src/grade.rs` answer contract and scoring,
  `src/summary.rs` the stop/go aggregate, `src/arms.rs` containment plus prompts and rows,
  `src/verify.rs` independent edge adjudication, `tests/harness.rs` the harness's own tests.
- **Containment cannot be enforced from the harness, so the harness measures it instead.** Two
  attempts at enforcement failed, and both failures are now recorded per cell rather than assumed:
  - `--allowedTools Bash(rg:*)` does not bind Bash in headless mode: the first live cell ran
    `grep -rn`/`sed -n` ten times with `permission_denials: []`.
  - A PATH jail does not hold either, because Claude Code normalises the Bash tool's `PATH`
    (observed: `/usr/local/bin:/usr/bin:/bin:~/.local/bin`). A jailed cell reached for `grep`,
    `ToolSearch` and an absolute `/usr/bin/grep` anyway — and, worse, cort could no longer find
    `ast-grep`, so the cort arm spent 20 turns on `ast_grep_missing` and scored zero. The jail is
    therefore opt-in (`--jail`) and is never treated as the control.
  - What *is* the control: every row carries `shells_used`, `arm_held` and `jailed`. A cell with
    `arm_held: false` did not stay inside its arm and must not be averaged into a comparison; the
    honest reading of the data so far is that no arm can be held on this driver, so the question
    "cort vs rg" cannot be answered here — only "cort vs an agent's whole shell", which is the
    adoption-realistic comparison anyway.
  - `build_env` still pins `CORT_AST_GREP_BIN` from the parent's PATH so the measured cort is
    bit-for-bit the one under test. It is no longer *required* for cort to work: audit F-13 is fixed,
    so cort probes its install locations instead of trusting the caller's PATH.
- Transcripts saved under `evals/runs/<date>/<arm>/<task>.json` (one JSON file per task per arm per date).
- `cort` is the Rust binary since the cutover. `CORT_BIN` resolves to
  `rust/target/release/cort` (override with the `CORT_BIN` env var), and `bin/cort.js` no longer exists.

## Metrics

Per-run fields produced by `arms::build_row` (older recorded cells use the same names):

```
{ success, total_tokens, tool_return_tokens, turns, read_calls, stale_reads }
```

Exported constants:

- `ARMS = ['rg+Read', 'ast-grep+Read', 'cort']`
- `METRICS = ['total_tokens', 'tool_return_tokens', 'turns', 'read_calls', 'stale_reads']`

The two metrics the archived V6 eval plan (`5a02c6e3:docs/agent-eval-plan.md:3-27`) was missing are `tool_return_tokens` and `stale_reads`. This harness adds them.

### stale_read (precise definition)

A **stale_read** is counted when the agent acted on file content that no longer matched disk at the moment it answered. The archived V6 eval plan never defined this — here it is counted explicitly as an incident per run.

### Micro metrics (spec section 1)

Four micro metrics are tracked outside the per-arm aggregates:

1. Index seconds for 10k files (`cort index` wall time)
2. `context` p95 latency (ms)
3. Actual emitted `context` tokens (measured on the emitted JSON, not a pre-truncation estimate, target ≤1500)
4. `impact` precision against the hand labels in `tasks.json` (`expected_symbols`)

`summarize(results)` computes per-arm aggregates (`success_rate`, `mean_total_tokens`, `mean_tool_return_tokens`, `mean_turns`, `mean_read_calls`, `stale_reads`) and the verdict.

## What the gate did not measure (2026-08-28)

Every recorded cell in `runs/2026-08-26/`, `runs/2026-08-26-cct/` and `runs/2026-08-26-cct-r3/` has
`tool_return_tokens: null` and `read_calls: null` — 30 of 30. The two metrics this harness exists to add
were never actually captured, so the three `STOP` verdicts rest entirely on `total_tokens`, which is
turns multiplied by an accumulating transcript. Treat those verdicts as "the agent talked more", not
"the tool cost more". See the Re-analysis section in `../README.md`.

## Graph-required tasks

`tasks.json` is single-hop or literal-answerable — it cannot exercise a transitive walk. Use
`tasks-graph.json`: 3-hop blast-radius questions whose labels were produced by the shipped pack and then
independently confirmed against file text (`cort-evals verify-impact`, precision 1.0 on all five chains).
`createSimplifiedEnhancedDAL` was measured and **dropped** for scoring 0.965, not 1.0 — labels that do not
verify do not go in the file.

## Deterministic cost probe (no model in the loop)

Agent arms are dominated by behavioural variance (one cell ran 171 turns). To price the tool itself:

```bash
CORT_CACHE_DIR=/tmp/cort ./rust/target/release/cort index /path/to/repo
cargo run --manifest-path evals/Cargo.toml --release -- verify-impact \
  --repo /path/to/repo --symbols A,B --depth 3
```

`verify-impact` is current and runnable — Rust now, like everything else here — and checks each
reported dependent against the file text, so a label file cannot grade itself. Its word matching is
textual, so a mention inside a comment also "confirms" an edge: it is a soundness screen against
fabricated dependents, not proof of a call.

`relation-cost` — the no-model cost probe behind the hop-ratio table in `../README.md` — existed only
as `evals/relation-cost.mjs`, was deleted by the cutover (`1a4052cc`), and imported `better-sqlite3`
plus `src/db.js`, so it cannot run against this tree as-is. Two consequences, both stated rather than
papered over:

- The table it produced (968/1,022/1,136 cort tokens vs 16,584/86,949/127,531 for `rg`) is
  **historical evidence**, reproducible only with the archived script:
  `git show 1a4052cc^:evals/relation-cost.mjs`.
- Re-pricing the graph needs it re-implemented as a `cort-evals` subcommand (open follow-up: the hop
  walk plus per-hit enclosing-symbol pricing; `rusqlite` is already vendored in this repo), and it is only worth writing if
  the graph is still in scope after `docs/2026-08-29-finance-cli-measurement.md` and
  `docs/2026-08-28-real-session-cost.md`.

## Stop/go gate (spec section 8)

This is a gate, not a report. The rule verbatim:

> If `cort struct` + `cort context` does not beat `ast-grep` + manual `Read` on **both** token count and success rate, stop adding features.

Concretely `summarize` returns:

```js
{
  by_arm: { 'rg+Read': {...}, 'ast-grep+Read': {...}, cort: {...} },
  verdict: {
    cort_beats_ast_grep: boolean,  // cort.mean_total_tokens < ast-grep.mean_total_tokens && cort.success_rate >= ast-grep.success_rate
    next_action: 'continue to deferred features' | 'STOP: do not add features until cort wins'
  }
}
```

If `cort_beats_ast_grep` is `false`, deferred features (`cort rewrite`, `cort modules` Louvain Phase-1, `--watch`, `cort impact --from-diff`, `cort search`, `CORTEX_REPORT.md`, and the embedding/RRF slots in spec section 11) must not be built until `cort` wins.

## Running

```bash
cargo test --manifest-path evals/Cargo.toml --all-targets        # the harness's own tests (16)

cargo run --manifest-path evals/Cargo.toml --release -- run-agents \
  --only <task-id> --arms rg+Read,cort --max-turns 40 --concurrency 2 \
  --config-dir /tmp/cc-eval --cache-dir /tmp/cort-exp --out /tmp/cort-eval-runs/<date>

cargo run --manifest-path evals/Cargo.toml --release -- summarize \
  evals/runs/2026-08-26/rows.json          # add --strict to refuse unmeasured metrics
```

`run-agents` needs an isolated `CLAUDE_CONFIG_DIR` (default `/tmp/cc-eval`) or ~16k tokens of
hooks/plugin text enter every cell and swamp a ~1.1k lean payload; it refuses to run when that
directory is missing. It also builds one PATH jail per arm (default `/tmp/cc-jails`, disable with
`--no-jail`), because `--allowedTools` does not bind Bash in headless mode — see Containment above.

`summarize` counts unmeasured metrics instead of averaging nulls, and `--strict` refuses the whole
aggregate when anything was not measured — the failure mode that let 30 null cells decide a STOP for
three rounds. It does not drive agents; only `run-agents` does, and only against real spend.
The `-f lean` contract lives in `rust/tests/render.rs` since the cutover.
