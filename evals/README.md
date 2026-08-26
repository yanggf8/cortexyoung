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
node evals/run-eval.mjs          # runs the three arms over tasks.json
node --test tests/eval-harness.test.js
```
