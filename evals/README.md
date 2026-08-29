# Agent eval harness

## Protocol

- Same model, same task set, one full trace per arm.
- Two arms ship today (`run-agents.mjs`): `rg+Read` and `cort`, driven over the graph-required task
  set `evals/tasks-graph.json`. The third arm in `run-eval.mjs`'s constants (`ast-grep+Read`) is the
  historical three-arm gate from rounds 1-3; the newer runner compares against `rg+Read`.
- Transcripts saved under `evals/runs/<date>/<arm>/<task>.json` (one JSON file per task per arm per date).
- `cort` is the Rust binary since the cutover. `CORT_BIN` resolves to
  `rust/target/release/cort` (override with the `CORT_BIN` env var), and `bin/cort.js` no longer exists.

## Metrics

Per-run fields produced by `buildRow()` in `run-agents.mjs` (older recorded cells use the same names):

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
independently confirmed against file text (`verify-impact.mjs`, precision 1.0 on all five chains).
`createSimplifiedEnhancedDAL` was measured and **dropped** for scoring 0.965, not 1.0 — labels that do not
verify do not go in the file.

## Deterministic cost probe (no model in the loop)

Agent arms are dominated by behavioural variance (one cell ran 171 turns). To price the tool itself:

```bash
CORT_CACHE_DIR=/tmp/cort ./rust/target/release/cort index /path/to/repo
node evals/verify-impact.mjs --repo /path/to/repo --symbols A,B --depth 3
```

`verify-impact.mjs` is current and runnable: it checks each reported dependent against the file text.

`relation-cost.mjs` — the no-model cost probe behind the hop-ratio table in `../README.md` — was part
of the JS tree and was deleted by the cutover (`1a4052cc`). It imported `better-sqlite3` and
`src/db.js`, so it cannot run against this tree as-is. Two consequences, both stated rather than
papered over:

- The table it produced (968/1,022/1,136 cort tokens vs 16,584/86,949/127,531 for `rg`) is
  **historical evidence**, reproducible only with the archived script:
  `git show 1a4052cc^:evals/relation-cost.mjs`.
- Re-pricing the graph needs a Rust-native probe (open follow-up), and it is only worth writing if
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
node --test evals/harness.test.mjs                     # the harness's own tests (no npm install needed)
node evals/run-agents.mjs --only <task-id> --arms rg+Read,cort   # drives real agent cells
node -e "import('./evals/run-eval.mjs').then(m=>console.log(m.summarize(JSON.parse(require('fs').readFileSync('evals/runs/2026-08-26/rows.json')))))"
```

`run-agents.mjs` needs an isolated `CLAUDE_CONFIG_DIR` (default `/tmp/cc-eval`) or ~16k tokens of
hooks/plugin text enter every cell and swamp a ~1.1k lean payload; it refuses to run when that
directory is missing. `-f lean` contract tests moved to `rust/tests/render.rs` with the product.

`run-eval.mjs` exports `summarize()` and the arm/metric constants; it does not drive an agent.
`summarize()` now counts unmeasured metrics instead of averaging nulls, and `summarize(rows, {strict: true})`
throws on any missing metric — the failure mode that let 30 null cells pass for three rounds.
The `-f lean` contract lives in `rust/tests/render.rs` since the cutover.
