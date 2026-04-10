# Agent Eval Plan for `cortexyoung`

This document defines a task-based evaluation to compare:

- `GitNexus`
- `Cortex (current or improved)`

The goal is not "which output looks smarter." The goal is:

- lower total model token usage
- fewer raw file reads
- fewer tool calls
- no drop in task success rate

## What counts as proof

For this repo, "GitNexus or Cortex saves context" is only proven if all of the following are true:

1. Both systems run the same task set on the same repo commit.
2. Both systems use the same base model.
3. Both systems keep a full execution trace.
4. Task correctness is judged against the same rubric.
5. Token savings are only counted on tasks that pass correctness review.

If one system uses fewer tokens but fails more tasks, that is not a win.

This document is a **protocol**, not just a reducer. `cli/tests/compare-agent-evals.mjs` only compares result rows after the run. This document defines how those rows must be produced and judged.

## Required controls

- Same repo commit SHA
- Same model
- Same temperature
- Same max-turn limit
- Same time budget per task
- Same user prompt text
- Same judging protocol
- Same trace format
- Each system may use its native tool surface without additional restriction inside that system

Recommended fixed settings:

- model: one model only, for example `claude-sonnet` or `gpt-5.4`
- temperature: `0`
- max turns: `8`
- timeout per task: `10 min`

## Trace capture protocol

Each task run must emit a trace artifact that is sufficient to derive:

- total tool calls
- raw file reads
- latency
- final answer

Minimum required trace fields:

- `task_id`
- `system`
- `repo`
- `repo_commit`
- `model`
- `started_at`
- `completed_at`
- `events[]`
- `final_answer`
- `token_usage`

Minimum event shape:

```json
{
  "t_ms": 1820,
  "kind": "tool_call",
  "tool": "search",
  "target": "cli/src/index.ts",
  "notes": "optional"
}
```

Accepted event kinds:

- `tool_call`
- `raw_file_read`
- `system_message`
- `final_answer`

## Counting rules

### Tool calls

Count every native system operation that the agent invokes as one tool call, including:

- GitNexus MCP or CLI operations
- Cortex CLI operations
- repo search helpers
- relationship or graph queries

### Raw file reads

Count only operations that return raw source, raw markdown, or raw config file content.

Do **not** count:

- `search`
- `context`
- `impact`
- graph traversal outputs
- precomputed summaries

The point of this metric is to measure how often the agent must fall back to uncompressed repo material.

## Metrics to capture

Per task:

- `task_id`
- `system`
- `success`
- `judge_notes`
- `input_tokens`
- `output_tokens`
- `total_tokens`
- `tool_calls`
- `raw_file_reads`
- `latency_ms`
- `final_answer_chars`

Optional but useful:

- `search_calls`
- `graph_calls`
- `relationship_calls`
- `fallback_raw_read_ratio`

## Judging protocol

Preferred protocol:

1. Blind the system name from the judge.
2. Judge against the task rubric and gold evidence only.
3. Record both pass/fail and short reasoning.

Recommended judge order:

1. **Primary**: human reviewer, blinded to system name
2. **Fallback**: LLM-as-judge, blinded to system name, using the same rubric
3. **Audit**: human spot-check at least 20% of judged tasks if using LLM-as-judge

The evaluation is invalid if pass/fail is assigned without a documented rubric.

## Comparison rule

Primary metric:

- `median total_tokens on passed tasks`

Secondary metrics:

- `mean total_tokens on passed tasks`
- `success rate`
- `median raw_file_reads on passed tasks`
- `median tool_calls on passed tasks`
- `tokens per successful task`

Recommended reporting:

- absolute token delta
- percentage token delta
- pass/fail counts

## Minimum benchmark size

Do not use a 5-task benchmark to claim a stable percentage improvement.

Minimum acceptable benchmark:

- `10+` total tasks
- at least `3` off-repo control tasks

Until the benchmark grows to `15+` tasks, treat token deltas as directional rather than statistically strong.

## Task set for this repo

These tasks are chosen because they map to real `cortexyoung` work and force structure understanding rather than simple grep.

### T1: Incremental indexing behavior

Prompt:

`Explain exactly how incremental indexing decides which files to reindex, including rename handling, deleted files, untracked files, and fallback behavior. Cite the code paths.`

Gold evidence:

- `cli/src/index.ts`
- `cli/src/turso.ts`

Pass criteria:

- mentions `--incremental`
- mentions git diff path
- mentions rename handling
- mentions deletion handling
- mentions untracked file handling
- mentions fallback when incremental is not possible

### T2: Blast radius of changing `chunk_id`

Prompt:

`If we change chunk_id format away from ${projectId}:${filePath}:${startLine}, what code paths and behaviors break or need updates? Give a concrete impact analysis for this repo.`

Gold evidence:

- `cli/src/chunker.ts`
- `cli/src/index.ts`
- `cli/src/turso.ts`
- `cli/tests/phase2-smoke.mjs`
- `docs/plans/2026-04-09-cortex-v6-intelligence-upgrade.md`

Pass criteria:

- identifies chunk creation sites
- identifies storage/update dependence
- identifies stale-delete or replace behavior impact
- identifies relationship references impact
- identifies test and backward-compat risk

### T3: Relationship noise and duplication risk

Prompt:

`Why can relationship traversal become noisy or duplicate-heavy in this project? Point to the exact implementation and current safeguards.`

Gold evidence:

- `cli/src/turso.ts`
- `cli/cortex.skill.md`
- `docs/TODO-MEMO.md`

Pass criteria:

- identifies recursive traversal implementation
- identifies `UNION ALL` duplication risk
- identifies ambiguity/confidence issue
- identifies depth limit safeguard

### T4: Minimum change set for hybrid search

Prompt:

`What is the minimum code change set needed to make hybrid search (vector + FTS with fusion) the default in this repo?`

Gold evidence:

- `cli/src/index.ts`
- `cli/src/turso.ts`
- `docs/plans/2026-04-09-cortex-v6-intelligence-upgrade.md`

Pass criteria:

- identifies current vector-only and keyword-only split
- identifies need for fusion logic
- identifies CLI flag implications
- identifies output shape implications

### T5: Offline AST grammar install path

Prompt:

`How does this repo support offline or air-gapped AST grammar installation, and what code path should an agent inspect first if AST chunking is unavailable?`

Gold evidence:

- `cli/src/index.ts`
- `cli/src/ast-chunker.ts`
- `cli/src/grammars.ts`
- `docs/plans/2026-04-09-cortex-v6-intelligence-upgrade.md`

Pass criteria:

- identifies grammar install command
- identifies runtime availability check
- identifies fallback to regex
- identifies first debugging path

### T6: Project selection and fallback behavior

Prompt:

`Explain how this repo decides which project_id to use when the current working directory is not indexed but a default project exists. Cite the code paths and tests.`

Gold evidence:

- `cli/src/index.ts`
- `cli/tests/phase2-smoke.mjs`

Pass criteria:

- identifies cwd project vs default project behavior
- identifies fallback behavior when neither is available
- cites test coverage or exported helpers

### T7: Staleness semantics across search, status, and reports

Prompt:

`Explain how index staleness is surfaced in this repo and where the behavior differs between search, status, incremental index, watch mode, and CORTEX_REPORT.md.`

Gold evidence:

- `cli/src/index.ts`
- `cli/cortex.skill.md`
- `CORTEX_REPORT.md`

Pass criteria:

- identifies git-head based staleness hinting
- identifies report staleness after incremental or watch updates
- distinguishes command behavior from artifact freshness

### T8: Schema migration and backward compatibility

Prompt:

`What schema migration strategy does this repo use for newly added columns, and what backward-compatibility risks remain?`

Gold evidence:

- `cli/src/turso.ts`
- `CLAUDE.md`

Pass criteria:

- identifies `applySchema` migration behavior
- identifies ALTER TABLE best-effort additions
- identifies at least one remaining compatibility risk

### T9: Rowid integrity and write semantics

Prompt:

`Why does this repo prefer ON CONFLICT DO UPDATE over INSERT OR REPLACE for chunks, and what would break if that changed?`

Gold evidence:

- `cli/src/turso.ts`
- `CLAUDE.md`

Pass criteria:

- identifies rowid preservation
- identifies FTS or vector-index integrity implications
- identifies downstream correctness risk

### T10: AST-first fallback behavior

Prompt:

`Walk through the AST-first indexing path for a supported file and explain exactly when the repo falls back to regex chunking.`

Gold evidence:

- `cli/src/index.ts`
- `cli/src/ast-chunker.ts`
- `cli/src/grammars.ts`

Pass criteria:

- identifies parser init path
- identifies grammar availability checks
- identifies fallback conditions
- distinguishes supported-language vs parse-failure behavior

## Off-repo control tasks

At least one unrelated control repo must be included before claiming KPI wins.

Recommended control characteristics:

- mid-size repo
- different language or framework than `cortexyoung`
- enough files that retrieval and impact reasoning matter

Minimum off-repo control pack:

### C1: Change-impact task

Ask for the blast radius of a shared symbol, config key, or API shape change.

### C2: Diff-rooted task

Ask for a structural summary of changed files, likely affected symbols, and next files to inspect.

### C3: Fallback or error-path task

Ask for the control flow of an error-handling or fallback path that crosses multiple files.

The same judging and trace rules apply.

## Execution protocol

For each task:

1. Reset to the same repo commit.
2. Start a fresh agent session.
3. Give only the task prompt.
4. Let the agent finish or hit timeout.
5. Save the full trace.
6. Record metrics in a JSON file.
7. Judge pass/fail against the rubric above.

Run the full set twice:

- once with `GitNexus`
- once with `Cortex`

If Cortex is improved later, run a third set:

- `Cortex-improved`

## Result file format

Each run should be a JSON array. Example:

```json
[
  {
    "task_id": "T1",
    "system": "gitnexus",
    "success": true,
    "judge_notes": "Covered rename, delete, untracked, fallback.",
    "input_tokens": 820,
    "output_tokens": 260,
    "total_tokens": 1080,
    "tool_calls": 4,
    "raw_file_reads": 1,
    "latency_ms": 18300,
    "final_answer_chars": 1412
  }
]
```

Use the comparison script:

```bash
node cli/tests/compare-agent-evals.mjs gitnexus-results.json cortex-results.json
```

## How to interpret outcomes

Strong proof:

- higher or equal success rate
- benchmark has `15+` tasks including off-repo controls
- lower median tokens on passed tasks
- fewer raw file reads on most tasks

Weak proof:

- lower tokens but lower success rate
- savings only on easy search tasks
- benchmark too small for a strong percentage claim

No proof:

- prompts differ
- model differs
- traces missing
- correctness not judged
