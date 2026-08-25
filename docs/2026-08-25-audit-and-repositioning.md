# Cortex Audit & Repositioning Notes (2026-08-25)

Consolidated record of a three-input review of this repo: (A) full source review,
(B) engineering-health review, and (C) an external Codex consultation on how to
strengthen agent lexical workflows (Grep/Glob/Read) instead of competing with them.
All `file:line` references are against master @ e6d26b18.

## Strategic conclusion (from discussion + Codex)

1. Coding agents locate code primarily through **Grep/Glob/Read + iteration**, not
   auxiliary semantic-search tools. Vocabulary-gap queries get solved by iterating,
   which is cheap for the agent; indexes go stale exactly while the agent is editing.
   Competing head-on with grep is a losing battle.
2. The **relationship graph** (imports/calls, confidence metadata) plus the **AST
   chunker** are the only defensible cores. Embeddings/vector search have no proven
   value here — keep or drop pending a real eval (`agent-eval-plan.md`).
3. The pivot direction is to serve agents as **plain artifacts and transparent
   augmentation** (repo maps as files, worktree-fresh overlays, graph annotations
   attached to lexical hits), NOT as a command surface agents must remember to call.

---

## Part A — Source review (`cli/src/`, ~4,300 LOC)

File sizes: `index.ts` 1,932 / `turso.ts` 1,044 / `chunker.ts` 512 /
`ast-chunker.ts` 512 / `grammars.ts` 230 / `config.ts` 51 / `embedder.ts` 24.

### High severity

| Issue | Location | Fix direction |
|---|---|---|
| Serial embedding despite `embedBatch` name — hottest index loop | `embedder.ts:16-24` | Pass array input (batch 16–32) or migrate to `@huggingface/transformers`; likely 3–10× faster indexing |
| Every command (incl. read-only) runs 15 CREATEs + 7 guaranteed-to-throw ALTERs ≈ 1–3 s dead latency on Turso RTT | `turso.ts:118-166`, `index.ts:124-129` | Schema-version marker row, or batch DDL via `db.batch` |
| Multi-project over-fetch asks `vector_top_k` for k = entire DB row count → brute-force ANN scan every query | `turso.ts:500-506` | Cap fetch at `(topK+offset)*10`; per-project indexes if needed |
| Watch-mode lost-update race: per-path debounce allows concurrent `reindexOneFile` for same file; stale read can win | `index.ts:731-737` | Serialize via promise-chain keyed by path + content_hash compare before upsert |
| Hybrid FTS branch `.catch(() => empty)` silently degrades to vector-only on ANY failure (outage/auth) | `turso.ts:783` | Distinguish FTS-syntax errors from transport errors; warn loudly |
| `gitDiffAgainstWorkingTree` returning null (git failure) reported as "no changes detected" success | `index.ts:1107-1125` | Emit confidence note or exit non-zero |

### Medium severity

- Migration try/catch swallows ALL errors, not just duplicate-column — transient network
  error mid-migration leaves missing column and confusing later failures (`turso.ts:126-165`).
  Use `PRAGMA table_info` or message inspection.
- Grammar-version drift computed but never compared in incremental reindex — after a
  tree-sitter upgrade unchanged files keep old-grammar chunks forever (`index.ts:1483-1498`).
  Force full reindex on hash mismatch.
- `sanitizeFtsQuery` tokenizer `[A-Za-z0-9_]+` reduces non-ASCII queries (CJK!) to empty →
  FTS branch silently skipped; long prose queries become dozens of OR'd phrases
  (`turso.ts:752-758`). Unicode-aware tokenizer + cap OR breadth.
- Confidence-tier weights duplicated 5× (JS at `index.ts:919`; SQL CASE at `turso.ts:628-634`,
  `860-866`, `937-943`). Inject from one constant.
- `cmdImpactFromDiff` near-duplicates ~40 lines of `outputImpactResult`
  (`index.ts:1164-1265`). Route through shared path.
- Chunk-row building duplicated between full-index loop and `reindexOneFile`
  (`index.ts:296-322` vs `639-666`). Extract helper.
- `index.ts` is a god-file: git helpers (1370-1531), staleness (807-821, 1537-1583),
  import resolution (1850-1907), confidence scoring (1661-1735), report writer (488-592)
  all inline. Extract modules.
- SIGINT drops debounced changes mid-flight (`index.ts:717`); directory renames leave stale
  subtrees (inotify emits one dir event) (`index.ts:720`); backslash paths bypass ignore
  filter (`index.ts:729`).
- Memory: whole-project chunk rows + embeddings accumulate before single end-of-run upload;
  stream per-batch instead (`index.ts:266,330,339-340`).
- `PRAGMA foreign_keys=ON` sent 9× per connection; CASCADE behavior across pooled remote
  sessions unverified — relationships deletion may silently not fire (`turso.ts:120,198,...1040`).
- Mixed stdout discipline breaks `--format json` machine consumption (`index.ts:332,1919`).
- mtime fallback strict-greater comparison misses equal timestamps; WSL2 clock-skew risk
  (`index.ts:1511`).

### Low / hygiene

- 34 `any` occurrences concentrated in turso.ts/index.ts data layer — define row/node interfaces.
- Dead code: `canASTChunk`, `resolveGrammarWasm`, config fields `model`/`dimensions`/`binary_path`;
  dead ternary `c.status === 'R' ? c.path : c.path` (`index.ts:1490`).
- Divergent `LANG_MAP` copies (ast-chunker vs chunker); duplicate `hashContent`,
  `CONTROL_FLOW`, `estimateTokens`, `hasFilters`.
- Top-level catch drops stack traces (`index.ts:1927-1932`) — honor `CORTEX_DEBUG`.
- `chunk_id = projectId:path:startLine` same-start-sibling invariant enforced nowhere.
- Glob semantics: `src/*` and `src/**` both → `src/%` (no single-segment distinction).
- SQL injection surface: clean (bind params everywhere, escaped globs). Token storage: mode 0600, masked in output. Shell-outs: argv-only. OK.

### Perf quick wins summary

Batched embedding + schema-version marker would cut index time 3–10× and remove ~1–3 s
from every read command. Cap vector over-fetch; fetch metadata-only in hybrid branches then
hydrate content for final page only; drop pre-search COUNTs.

---

## Part B — Engineering health

### Tests — weakest pillar

- Only test: `cli/tests/phase2-smoke.mjs` (157 lines, hand-rolled asserts), imports from
  `../dist/index.js` (needs successful build first). Covers parseQueryFilters, project
  selection, import resolution, relationship scoring — i.e. ~10% of surface.
- **Zero coverage**: both 512-line chunkers, RRF fusion, `sanitizeFtsQuery`, staleness/
  incremental logic, grammars resolution. No vitest/jest configured anywhere.
- Fragile assertion: exact float `0.5599999999999999` via deepEqual (`phase2-smoke.mjs:100`).

### CI — absent

No `.github/`, no pipelines. Remote: github.com/yanggf8/cortexyoung.

### Dependencies

| Package | Installed | Latest | Note |
|---|---|---|---|
| `@xenova/transformers` | 2.17.2 | superseded by `@huggingface/transformers` 4.x | pins 2023-era onnxruntime-node 1.14; npm does NOT flag deprecated |
| `@libsql/client` | ^0.14.0 | 0.17.4 | cloud-facing client, stale 3 minors |
| `web-tree-sitter` | ^0.26.8 | 0.26.13 | fine |
| tree-sitter-{js,ts,py} | current | current | npm-tarball WASM strategy verified sound |

- No `engines` field despite documented Node 20+ requirement (recursive fs.watch needs it).
- Two lockfiles (root + cli) can drift; consolidate to root workspace install only.

### Repo hygiene

- **`.claude/settings.local.json` is git-tracked** — contains sweeping allow rules
  (`Bash(rm:*)`, `git push:*`, `curl:*`, `kill:*`), stale cross-project MCP permissions,
  personal absolute paths. Untrack it, gitignore it, prune dangerous grants. Worst finding.
- `.gitignore` misses `CORTEX_REPORT.md` (runtime-generated into indexed project roots).
- `cortex-nudge.sh` reads `CLAUDE_TOOL_NAME` env, but PreToolUse input arrives as stdin JSON —
  hook is likely inert (never matches). Also adds python3 startup cost per Grep. Verify & fix.
- CLAUDE.md claims "V3 legacy archived on `archive/v3` branch" — no such branch exists.
- README.md stuck at V5 (regex chunking, vector-only search, missing context/impact/watch/
  incremental/grammars commands). 7 orphaned docs describe nonexistent systems
  (mmr-deployment-guide, dual-mode-file-processing, git-first-approach, file-watching-*,
  advanced-smart-filtering) — mark OBSOLETE like plans/ does.
- Versioning: 5 major versions shipped, only tag ever created is `v2.1.6-dual-mode`.
  Tag v6.0.0 onward. A0 eval checkpoint never run (no baseline JSON exists).

### Docs that ARE accurate

`docs/plans/*` carry honest status markers and match implemented state (hybrid RRF default,
filters, context/impact, incremental/watch all verified in code).

---

## Part C — Codex consultation (verbatim)

> Forwarded question: IF we accept that agents will keep using Grep/Glob/Read as their
> primary information-finding primitives, how can we strengthen THOSE workflows?
> Deliverable: prioritized buildable ideas; which fit this repo's assets; be critical.

## Evidence boundary

- **Observed:** Default `cortex search` returns 15 results with full chunk bodies; chunks are capped at 400 estimated tokens, so content alone can approach 6,000 tokens before JSON metadata. index.ts:763, chunker.ts:23
- **Observed:** `context` targets 2,000 tokens, but budgets against full bodies and later emits 300-character previews. It can discard graph neighbors based on tokens never actually returned. index.ts:909
- **Observed:** Staleness compares stored Git HEAD with current HEAD. Dirty worktree edits do not change HEAD and therefore are not detected. index.ts:807
- **Observed:** Incremental/watch updates explicitly leave `CORTEX_REPORT.md` stale. The report contains global counts/hotspots, not a navigable file or subsystem map. index.ts:484
- **Observed:** The only configured hook is `PreToolUse` for Grep; it always suggests another command when any default Cortex project exists and does not validate the current repo or inspect the query. settings.json:1, cortex-nudge.sh:13
- **Harness assumption:** Grep/Glob/Read results normally enter the model context as tool-result text. Compression must happen before insertion to save tokens; a `PostToolUse` annotation added after the raw result cannot recover those tokens.
- **Harness caveat:** Hook names and whether hooks can replace results vary. The repository proves only `PreToolUse`; ideas involving `PostToolUse`, pre/post-compact, stop, or resume events are conditional on documented harness support.

### Prioritized ideas

1. **Budgeted, symbol-aware Grep result envelope** — Impact: High · Effort: M. Consume `rg --json` before context insertion; group by enclosing symbol, rank definition/export → imports/callers → other references → tests, cap hits per symbol/file, enforce 1,200–2,000-token budget with omitted counts. Compact records such as `DEF 0.94 cli/src/index.ts:745 cmdSearch — 3 hits`. Must transparently wrap/adapt Grep — a post-result summary saves nothing. Reuses AST chunker + graph as tie-breaker.
2. **Worktree freshness overlay** — Impact: High · Effort: M/L. HEAD-based staleness misses uncommitted changes while agents reason about the dirty worktree. Before Grep/Glob/Read, reconcile `git diff --name-only`, untracked files, stored content hashes; locally reparse touched files, overlay symbols/imports/calls on the base graph, invalidate edges targeting moved/deleted chunks. Mark paths STALE when refresh exceeds latency budget.
3. **Plain, continuously refreshed repository map** — Impact: High · Effort: M. Evolve report generation into `CORTEX_MAP.md` plus sidecars (`files.tsv`, `symbols.tsv`, `imports.tsv`): file role, principal symbols, line spans, fan-in/out, test/config/generated classification, dirty/fresh status. Human Markdown + stable TSV; consumed by ordinary Read/Grep/Glob.
4. **Graph annotations attached to lexical hits** — Impact: High · Effort: M. After finding a definition, the expensive loop is "which caller/importer should I read next?". Append ≤3 fresh high-confidence neighbors per top symbol group: defining file, strongest importer/caller, nearest test; direction, confidence, freshness — no neighbor bodies. Prefer EXTRACTED edges; INFERRED only above measured threshold; never pad with AMBIGUOUS.
5. **Intent-aware Read shaping** — Impact: High · Effort: M. For line-targeted Reads return imports/type context, enclosing declaration signature, bounded window, referenced symbol names; expose omitted ranges and continuation windows. Budget ~1,000–1,500 tokens; preserve literal source/line numbers; never reshape explicit whole-file requests invisibly.
6. **Semantic Glob summaries** — Impact: Medium · Effort: S/M. Group by directory/role, counts + representative/high-fan-in files, exact total preserved, full listing only below threshold, suggested narrower glob when truncated.
7. **Deterministic post-compact orientation artifact** — Impact: High · Effort: S/M. Untracked `CORTEX_SESSION.md`: dirty files, successful and zero-hit queries with scopes, files/ranges read, last test result, fresh map hash, unresolved truncations. Inject short pointer on resume; else skill text says reread after compaction.
8. **Replace the Grep nudge with adaptive lexical hints** — Impact: Medium · Effort: S. Current hook emits identical "use Cortex" for every Grep whenever any default project exists. Instead emit nothing for scoped searches; for broad/repeated queries one line like `Likely definition: cli/src/index.ts; exclude dist/generated; map freshness: dirty 2 files`.
9. **Explicit failure metadata for zero/truncated results** — Impact: Medium · Effort: S. Report searched roots, file count, ignore policy, case mode, truncation point, omitted hits, map freshness; suggest one bounded relaxation. Prevents false "symbol does not exist" conclusions.
10. **Tiny lexical-search playbook in harness instructions** — Impact: Medium · Effort: S. Replace most of cortex.skill.md with 15–25 lines teaching definition → imports/references → nearest tests → targeted Read ladder, scoping, generated/test exclusions, rereading CORTEX_SESSION.md after compaction.
11. **Embedding reranking only inside lexical candidates** — Impact: Low/Medium · Effort: M. Lexical retrieval first; embeddings reorder symbol groups only when candidates exceed budget, pinning exact definitions above semantic scores. Kill if evals don't improve. (Narrow role for embeddings without competing with exact search.)
12. **Agent-invoked "smart grep" command** — Impact: Low · Effort: M. Contradicts adoption premise; at most a human debugging interface. Do not prioritize.

### What actually breaks lexical loops

Result breadth allocated by filesystem order rather than evidence value; definitions/tests/generated mixed together; tool-side truncation removes the tail silently; Reads repeatedly reload imports/boilerplate; dirty edits invalidate anchors without changing HEAD; zero-hit searches indistinguishable from bad scope; compaction loses negative searches; generic hook reminders consume attention without improving the call underway.

### Keep Cortex alive, or kill it?

**Blunt recommendation: kill Cortex as an agent-facing semantic-search CLI unless a lexical-augmentation eval reverses the premise.** `search`, `context`, the BGE runtime, Turso dependency, and the Grep-replacement skill are not justified if agents do not invoke them.

Keep these parts:

- The AST chunker as a local symbol/range extractor.
- The high-confidence import/relationship graph as a sidecar generator.
- Report generation, redesigned as fresh repo/session maps.
- Incremental per-file parsing, triggered transparently by hooks/provider integration.
- Confidence and ambiguity metadata.
- The existing evaluation protocol, changed to compare **plain lexical** versus **augmented lexical**, measuring correctness, total tokens, tool-result tokens, turns, repeated queries, truncations, and stale-read incidents.

Absorb them into plain files/hooks/harness adapters. The surviving executable, if one is needed, should be an internal artifact builder or watcher — not a command surface the agent must remember.

The graph is the only plausible reason to keep a Cortex core, and only if automatically attached to lexical hits with worktree freshness. If that does not reduce successful-task tokens or turns on the repository's existing controlled evals, kill the graph service too.

*(Codex session 01a03735-b954-7f51-a472-f602e2a2dceb, task-mt86bxyi-57zs0q)*

---

## Consolidated roadmap (proposal, not yet agreed)

**P0 — hygiene & security (do regardless of direction)**
1. Untrack `.claude/settings.local.json`, add to `.gitignore`, prune dangerous allows.
2. Add `CORTEX_REPORT.md` to `.gitignore`. Fix or delete the inert nudge hook.
3. Fix CLAUDE.md's false `archive/v3` claim; mark orphaned docs OBSOLETE; rewrite README to V6 reality; start tagging releases.

**P1 — perf & correctness quick wins (valuable under any direction)**
1. Batched embedding (or skip entirely if vectors get dropped — see P2).
2. Schema-version marker replacing per-command applySchema.
3. Cap vector over-fetch; slim hybrid payloads.
4. Serialize watch-mode per-file jobs; fix SIGINT flush.
5. Stop hiding infrastructure failures (FTS catch, git-null-as-empty, blanket ALTER catches).
6. Force full reindex on grammar-version drift; unicode-aware FTS tokenizer.

**P2 — decide the product's fate with data, not vibes**
1. Run the A0 eval protocol comparing plain-lexical vs augmented-lexical workflows
   (correctness, total/tool-result tokens, turns, repeated queries, stale-read incidents).
2. Decision gates: embeddings survive only as rank-11 rerank-if-eval-wins; the graph
   survives only as transparent annotation/sidecar (ideas #2/#3/#4), never as a
   must-remember command.
3. If pivot confirmed: build `CORTEX_MAP.md` + TSV sidecars and the worktree-freshness
   overlay first (they make everything else honest), then adaptive hints replacing the nudge.

**Open questions**
- Can this harness family replace/compress Grep output pre-insertion via hooks, or only annotate? Determines feasibility of ideas #1/#4/#5 as built.
- Is `@huggingface/transformers` migration worth it at all if vectors are likely dropped?
- Single fixed DB name `cortex-v5` collides across machines sharing a Turso account — resolve if multi-project use continues.
