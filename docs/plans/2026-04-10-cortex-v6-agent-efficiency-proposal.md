# Cortex V6.1 — Agent Efficiency Proposal

**Date**: 2026-04-10
**Status**: ✅ Absorbed into `2026-04-09-cortex-v6-intelligence-upgrade.md` (v3). P1 (hybrid RRF), P2 (`context`), P3 (`impact`), and P0 (eval harness) all shipped in commit `ed1c647`. P5 (cluster summaries) remains deferred per the consolidated plan. This document is retained as historical context; see the consolidated v6 plan for current status and remaining work.
**Review Target**: Gemini architecture review
**Depends on**: `2026-04-09-cortex-v6-intelligence-upgrade.md`
**Companion docs**:
- `docs/agent-eval-plan.md`
- `CORTEX_REPORT.md`

## 1. Objective

The KPI for this revision is narrow:

**Reduce total agent context-token consumption on real code understanding tasks without reducing answer correctness.**

This is not a generic "make search better" project. It is a practical effort to make Cortex more useful for Claude Code, Codex, and similar coding agents working inside a repo.

Success means:

- fewer raw file reads per task
- fewer exploratory tool calls per task
- lower total model tokens on passed tasks
- equal or better task success rate

## 2. Current State

Cortex already has a good base:

- local embeddings via `@xenova/transformers`
- Turso-backed chunks, vectors, relationships, and FTS5
- AST-first chunking with regex fallback
- incremental indexing and git staleness hints
- zero-tool-call orientation via `CORTEX_REPORT.md`

The current architecture is best described as:

**searchable semantic code memory**

That is useful, but it is still one layer short of the actual KPI. Right now the agent mostly gets:

- `search`
- `relationships`
- `status`
- report summary

Those are low-level primitives. The agent still has to compose them into a task answer by itself.

## 3. Core Gap

GitNexus appears stronger not because it has "a graph" in the abstract, but because it exposes more task-shaped operations such as context, impact, and change analysis.

Cortex today is optimized for:

- retrieving chunks
- traversing direct relationships
- summarizing a project at a coarse level

It is not yet optimized for:

- "give me the minimal context pack for this symbol"
- "show blast radius before I refactor"
- "map this diff to likely affected functions and files"
- "show subsystem context without forcing me to read 8 files"

That is the real product gap.

## 4. Design Goal

Turn Cortex from a retrieval layer into a lightweight **agent intelligence layer** while preserving current constraints:

- CLI-first
- no MCP server
- no browser app
- no custom graph database
- no always-on local daemon

The target is not to clone GitNexus. The target is to capture the highest-ROI parts of the "precomputed intelligence for agents" model while keeping Cortex operationally light.

## 5. Proposal Summary

Ship the smallest credible set of changes:

1. **Task-based eval and telemetry to prove token savings**
2. **Hybrid retrieval as the default path**
3. **Agent-first commands: `context`, `impact`**

Defer subsystem summaries in `.cortex/clusters/` to a later experimental phase unless the first three items fail to move the KPI.

This is the smallest set of changes that can plausibly move the KPI without turning Cortex into a heavier platform.

## 6. Proposed Work

### P0 — Evaluation Baseline First

Before shipping new intelligence features, lock the measurement method.

Use `docs/agent-eval-plan.md` as the source of truth. That document defines:

- the task set
- pass criteria
- metrics
- comparison rules

Add result capture as a first-class workflow for all future changes.

**Why first:** Without this, "better for agents" becomes opinion-driven.

**Existing support:**
- `cli/tests/compare-agent-evals.mjs`

**Acceptance criteria:**
- one completed baseline run for `Cortex-current`
- benchmark contains at least `10` tasks total
- benchmark includes at least one off-repo control corpus
- judging protocol is written down and executable
- trace capture produces the JSON rows consumed by `compare-agent-evals.mjs`
- result JSON format stable enough for repeated use

### P1 — Hybrid Retrieval as the Default

Current search is split:

- `cortex search "q"` => vector-only
- `cortex search "q" --keyword` => FTS-only

That pushes ranking and fallback strategy back onto the agent.

#### Change

Make default search run:

- vector search
- FTS search
- Reciprocal Rank Fusion

Flags remain:

- `--vector`
- `--keyword`
- `--rrf-k <n>`

Each result should also include:

- source: `vec` | `fts` | `both`
- rank positions from each branch when available

#### Ranking contract

Use **rank-based fusion only** in v1:

- compute vector rank
- compute FTS rank
- fuse by Reciprocal Rank Fusion

Do **not** attempt raw-score normalization between FTS and vector scores in this phase. Their score distributions are not directly comparable, and a bad normalization layer would create ranking instability without helping the KPI.

#### Files likely affected

- `cli/src/index.ts`
- `cli/src/turso.ts`
- `cli/cortex.skill.md`

#### Why this matters for tokens

If the default search gives the agent better first-pass recall, the agent needs fewer retries, fewer alternate queries, and fewer raw file reads.

#### Acceptance criteria

- hybrid search becomes the default path
- search output remains machine-readable
- agent-eval median token use improves on search-heavy tasks

### P2 — Agent-First `context` Command

This is the most important change.

Today the agent asks for a symbol, then has to manually combine:

- search results
- relationship traversal
- report context

#### Change

Add:

```bash
cortex context "symbol-or-query"
```

This command should return a **minimal context pack** for answering a code-understanding question. It should combine:

- best-matching primary chunks
- direct callers/callees/import/export neighbors
- file-level summary
- confidence and relationship source
- optional cluster summary if available

The output must be compact and intentionally shaped for agent consumption.

#### Command boundary

`context` is the **local, depth-1, answer-the-question-now** command.

It should answer:

- what is the most relevant symbol or chunk
- what are its direct neighbors
- which one or two files are most likely worth reading next

It should **not** attempt:

- transitive blast radius
- diff-rooted impact analysis
- broad subsystem inventory

#### Hard output budget

The context command must not become a disguised mega-read.

Initial budget:

- target output size: `< 2000 tokens`
- soft cap: trim low-confidence neighbors first
- hard cap: if still too large, return truncation metadata and suggest one follow-up command

#### Output shape

Suggested sections:

- `primary_matches`
- `neighbor_chunks`
- `key_files`
- `confidence_notes`
- `suggested_next_queries`

#### Shared output schema

All new agent-first commands should reuse the same core machine-readable confidence schema.

Suggested fields:

- `confidence_score`
- `confidence_tier`
- `is_ambiguous`
- `ambiguity_reason`
- `truncated`
- `budget_tokens`
- `index_is_stale`
- `index_staleness_reason`

This should be defined once and reused across `context` and `impact`.

#### Staleness contract

`context` must inherit the existing git-staleness behavior from `search` / `status`.

At minimum:

- emit structured staleness metadata in the output
- warn when the index is behind HEAD
- never silently present stale context as fresh context

#### Files likely affected

- `cli/src/index.ts`
- `cli/src/turso.ts`
- possibly `cli/src/chunker.ts` or metadata helpers if more fields are needed
- `cli/cortex.skill.md`

#### Why this matters for tokens

This removes agent-side orchestration. One command can replace:

- one search
- one retry search
- one relationships call
- two or three raw file reads

#### Acceptance criteria

- at least 3 of 5 eval tasks can be answered from `context` output plus at most one follow-up call
- median raw file reads drop versus current Cortex

### P3 — Agent-First `impact` Command

The next high-value task is blast-radius reasoning.

#### Change

Add two entry modes on the same command:

```bash
cortex impact --symbol "symbol-or-file"
cortex impact --from-diff [base-sha]
```

This command should provide an **aggregated or transitive impact view**:

- direct dependents
- import fan-in / fan-out
- callers and likely affected files
- confidence weighting
- ambiguity warnings
- changed files and rename/delete/add status when rooted from diff

#### Files likely affected

- `cli/src/index.ts`
- `cli/src/turso.ts`
- `cli/cortex.skill.md`

#### Command boundary

`impact` is the **aggregated, blast-radius, or diff-rooted** command.

It should answer:

- what else might break
- what files or symbols are affected beyond the direct local neighborhood
- what changed structurally in the current diff

It should **not** try to be a compact "best next read" pack. That remains `context`.

#### Why this matters for tokens

Without this, the agent has to simulate impact analysis by repeatedly calling search and relationships, then reading files to verify.

#### Acceptance criteria

- `impact` can answer the `chunk_id` blast-radius eval task with fewer tool calls than current Cortex
- diff-oriented tasks require fewer raw file reads
- output explicitly marks confidence and ambiguity

### P5 — Cluster Summaries as Experimental Follow-On

`CORTEX_REPORT.md` is global and coarse. The next missing layer is subsystem context.

This item is explicitly **not part of the minimum KPI-proof scope** for V6.1. It should only start after `P1` to `P4` are implemented and measured.

#### Change

Add optional generation of:

```text
.cortex/clusters/<name>.md
```

Start with simple grouping:

- top-level directory

Each cluster file should include:

- top symbols
- entry points
- dominant file types
- framework idioms if available later
- high-confidence relationships

#### Files likely affected

- `cli/src/index.ts`
- new helper module if needed
- `cli/cortex.skill.md`

#### Why this matters for tokens

Agents often need "just enough subsystem orientation." A per-cluster note is much cheaper than reading 5 to 10 source files.

#### Why this is deferred

This is the highest-risk item for:

- stale artifact management
- token bloat
- maintenance complexity without measurable KPI improvement

Start with directory-level aggregation only if this phase is activated. Do not begin with graph-derived clustering heuristics.

#### Acceptance criteria

- generated summaries remain under a reasonable size budget
- agents use cluster summaries instead of broad raw-file exploration on subsystem tasks

## 7. Explicit Non-Goals

This proposal does **not** include:

- MCP server
- browser UI
- multi-repo registry
- custom graph database
- type-checker integration
- rename/go-to-definition IDE features
- multimodal documents/images/PDF ingestion

Those may be valid later, but they are not required to improve the token-efficiency KPI for this repo.

## 8. Implementation Order

Recommended order:

1. `P0` evaluation baseline
2. `P1` hybrid retrieval default
3. `P2` context command
4. `P3` impact command (--symbol and --from-diff)
5. `P5` cluster summaries (experimental only)

Reasoning:

- `P1` improves current behavior with low conceptual risk
- `P2` is the highest-ROI intelligence feature
- `P3` covers both symbol-rooted and diff-rooted blast radius on one backend and one public command surface
- `P5` should come only after command semantics stabilize and measurable gains are already proven

## 9. Main Risks

### Risk 1 — Better-looking output without real token savings

Mitigation:

- do not accept features without running the eval plan

### Risk 2 — Overbuilding a platform

Mitigation:

- keep everything CLI-first
- reject MCP/server/web work in this phase

### Risk 3 — Relationship noise makes impact/context unreliable

Mitigation:

- preserve confidence tags
- expose ambiguity instead of hiding it
- prefer compact "likely impact" wording over false certainty

### Risk 4 — Cluster summaries rot after incremental updates

Mitigation:

- mark cluster summaries stale after `--incremental` and `--watch`
- only promise freshness after full index or targeted regeneration

### Risk 5 — Hybrid search increases latency enough to reduce practical utility

Mitigation:

- keep `--vector` escape hatch
- record latency in eval outputs

### Risk 6 — `context` output becomes too large to help

Mitigation:

- enforce a hard token budget
- prefer dropping low-confidence neighbors before dropping primary matches
- return explicit truncation metadata so the agent knows when to escalate

## 10. Success Criteria

This proposal should be considered successful only if all of the following hold:

- task success rate is equal or better than `Cortex-current`
- median raw file reads on passed tasks decreases
- at least one of `context` or `impact` becomes a clearly preferred entry point over raw search for the eval tasks
- `context` remains within its output budget on the majority of runs
- benchmark size is large enough to make the reported delta credible

For early benchmarks with fewer than `15` tasks, use:

- no correctness regression
- positive median token delta
- directional improvement on the majority of tasks

Only introduce a hard percentage bar such as `15%` once the benchmark set is larger and includes off-repo controls.

If token use does not improve materially, the proposal failed even if the feature set looks richer.

## 11. Why This Proposal Instead of a Full GitNexus Clone

GitNexus appears to win by packaging precomputed intelligence into agent-oriented operations. That insight is worth copying.

The rest is not required for this repo right now.

A full GitNexus-style platform would add:

- operational weight
- maintenance surface
- more state and interfaces
- more implementation risk before KPI proof

This proposal intentionally aims for the smallest architecture that can still test the same core thesis:

**precomputed task-shaped context beats repeated exploratory retrieval**

## 12. Questions for Gemini Review

Please review this proposal as an architecture and product-design document, not as a generic brainstorming note.

Focus questions:

1. Is the problem statement sharp enough, or is the KPI still too broad?
2. Is the proposed scope the smallest credible scope for proving token savings?
3. Are `context` and `impact --from-diff` the right minimum public command surface, or is one of them still premature?
4. Is the implementation order correct, or should cluster summaries come earlier?
5. Are there missing failure modes around relationship ambiguity, retrieval fusion, or stale artifacts?
6. Is the `15%` token-improvement threshold reasonable for this class of change?
7. Which part of the proposal is most likely to produce complexity without measurable KPI improvement?
8. If one item should be cut to reduce scope while preserving the thesis, which one should be cut?

Please be specific. Prefer criticism tied to:

- expected user workflow
- architectural coupling
- retrieval quality
- failure modes
- operational maintenance cost

## 13. Proposed Review Outcome Format

Ask Gemini to respond in four sections:

1. `What is strong`
2. `What is weak or underspecified`
3. `What should be cut or reordered`
4. `Go / No-go recommendation`
