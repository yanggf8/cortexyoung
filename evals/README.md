# Agent eval harness

## Protocol

- Same model, same task set (`evals/tasks.json`), one full trace per arm.
- Arms: `rg+Read`, `ast-grep+Read`, `cort` (`cort struct` + `cort context`).
- Transcripts saved under `evals/runs/<date>/<arm>/<task>.json` (one JSON file per task per arm per date).

## Metrics

Per-run fields returned by `runEval({arm, task})`:

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
CORT_CACHE_DIR=/tmp/cort node bin/cort.js index /path/to/repo
node evals/relation-cost.mjs --repo /path/to/repo --depth 3 --pick 6   # per-hop payload accounting
node evals/verify-impact.mjs --repo /path/to/repo --symbols A,B --depth 3
```

`relation-cost.mjs` compares, for the same answer set: `cort impact` JSON tokens, `cort impact -f lean`
tokens, and what `rg` must ingest to reach that set — one batched alternation grep per hop (a best case
for `rg`: oracle names, one call, no wasted turns) plus the enclosing-symbol text for every hit. It also
reports `rg_precision`, the fraction of grep hits that are actually on a dependency path.

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
node evals/run-eval.mjs          # exports summarize() over recorded rows; see caveat above
node --test tests/eval-harness.test.js
node --test tests/render.test.js # the -f lean contract
```

`run-eval.mjs` exports `summarize()` and the arm/metric constants; it does not itself drive an agent.
Arms are run by dispatching fresh subagents and recording one JSON file per cell under `runs/<date>/`.
