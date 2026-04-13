# Cortex v6 — Intelligence Upgrade Plan (v3, consolidated)

> Skill + CLI only. No MCP, no server.
> Inspired by GitNexus competitive analysis + claudecat AST pattern work.
> Original plan: 2026-04-09 (Gemini two-pass review).
> Agent-efficiency addendum: 2026-04-10 (`docs/plans/2026-04-10-cortex-v6-agent-efficiency-proposal.md`, now absorbed here).
> **Current codebase version: V6.1** — P1/P2/P3 + agent-first surface shipped.

---

## Status snapshot

| ID | Item | Status | Commit |
|---|---|---|---|
| P1 | Tree-sitter AST chunking | ✅ Shipped | `9c5c1d7` |
| P2 | Git HEAD staleness + incremental reindex | ✅ Shipped | `9c5c1d7` |
| P3 | Hybrid search (RRF fusion) as default | ✅ Shipped | `ed1c647` |
| A1 | Agent-first `context` command | ✅ Shipped | `ed1c647` |
| A2 | Agent-first `impact` command | ✅ Shipped | `ed1c647` |
| A0 | Eval baseline + trace comparison harness | ✅ Scaffolded | `ed1c647` |
| P4 | Evidence-weighted confidence scoring | ✅ Shipped | — |
| P5 | AST-aware query filters (`kind:`, `lang:`, `name:`, `file:`) | ✅ Shipped | — |
| P6 | Framework-idiom detection (5 detectors) | ⬜ Not started | — |
| P7 | Per-cluster skill snippets | ⏸ Deferred (experimental, behind eval) | — |

V6.1 = P1 + P2 + P3 + A0 + A1 + A2 landed.
V6 "completion" = V6.1 + P4 + P5 — **reached**.
V6.2 (stretch) = V6 + P6.
P7 only activates if eval data shows context/impact alone do not close the subsystem-orientation gap.

---

## Part I — Shipped (V6.1)

### ✅ P1 — Tree-sitter chunking  *(9c5c1d7)*

- `cli/src/ast-chunker.ts` + `cli/src/grammars.ts` added.
- Grammars: TS, TSX, JS, JSX, Python. WASM via npm packages (`tree-sitter-javascript`, `tree-sitter-typescript`, `tree-sitter-python`) or `~/.cortex/grammars/` cache.
- `chunkFileAST()` tries tree-sitter first, falls back to regex on failure or unsupported language. Each chunk carries `chunk_source: 'ast' | 'regex'`.
- Grammar version hash stored in `projects.grammar_version`.
- `cortex grammars` / `cortex grammars install <dir>` for offline installs.
- Post-processing (`splitOversized` / `mergeUndersized`) preserves AST metadata via `deriveChunk()` instead of re-extracting via regex.
- Function-valued declarations (`const foo = () => {}`) classified as `function` via `isFunctionValuedDeclaration()`.

### ✅ P2 — Git HEAD staleness + incremental reindex  *(9c5c1d7)*

- `projects` table gained `git_head TEXT` and `last_indexed_at INTEGER` via `ALTER TABLE`.
- `cortex index` stores `git rev-parse HEAD` + timestamp on completion.
- `cortex search` / `cortex status` compare stored HEAD to current; one-line stderr hint shown unless `--quiet` / `CORTEX_QUIET=1`.
- `cortex index --incremental`: `git diff --name-status -M -z <stored_sha>..HEAD` for tracked changes + `git ls-files --others --exclude-standard -z` for untracked files.
- Rename detection (`-M`) cascades: delete old path, index new path.
- **mtime fallback** when git is inconclusive (stash/reset/no repo): mtime vs `last_indexed_at` comparison. Cross-references on-disk files against `getIndexedFilePaths()` to detect deletions.
- Staleness hint uses `meta.path` from DB (not cwd) so it works from any subdirectory.
- Per-file CRUD (`deleteStaleFileChunks`, `replaceFileRelationships`) reused — no full rebuild on incremental updates.

### ✅ P3 — Hybrid search with RRF fusion  *(ed1c647)*

- `cortex search "q"` default now runs vector_top_k + FTS5 in parallel, fuses via Reciprocal Rank Fusion.
- Flags: `--vector` (legacy vector-only), `--keyword` (FTS-only), `--rrf-k <n>` (default 60), `--top-k <n>`, `--offset <n>`.
- Each result tagged `source: 'vec' | 'fts' | 'both'` + `vec_rank` / `fts_rank`.
- `sanitizeFtsQuery()` tokenizes on `[A-Za-z0-9_]+` and joins with `OR` phrases to survive punctuation in natural-language queries.
- Rank-based fusion only — no raw-score normalization between FTS and vector scores in v1 (distributions are not directly comparable).

### ✅ A0 — Evaluation baseline & telemetry  *(ed1c647)*

- `docs/agent-eval-plan.md` defines task set, pass criteria, metrics, judging protocol.
- `cli/tests/compare-agent-evals.mjs` reduces result rows into median/mean deltas + per-task breakdowns + paired success comparison.
- Minimum benchmark size: 10 tasks with ≥3 off-repo controls (no correctness regression required before a hard percentage token-saving bar).

### ✅ A1 — `cortex context` command  *(ed1c647)*

- `cortex context "<symbol-or-query>"` — one command replacing search + relationships + file reads for depth-1 orientation.
- Backed by `hybridSearch()` + symbol-name lookup + `getDirectNeighbors()`.
- Output JSON: `primary_matches[]`, `neighbor_chunks[]` (with relationship metadata), `key_files[]`, `confidence_notes[]`, `suggested_next_queries[]`, `metadata` (shared confidence schema).
- **Hard budget ~2000 tokens.** Low-confidence neighbors trimmed first; `metadata.truncated = true` when hard cap hits; `suggested_next_queries` guides the next call.
- Inherits git-staleness via `emitStalenessHint()` — staleness appears in `metadata.index_is_stale` / `metadata.index_staleness_reason`.
- Boundary: depth-1, answer-the-question-now. Transitive analysis lives in `impact`.

### ✅ A2 — `cortex impact` command  *(ed1c647)*

- Two entry modes on the same backend:
  - `cortex impact --symbol "<name>"` — transitive dependents via recursive CTE, depth ≤ 3.
  - `cortex impact --from-diff [base-sha]` — maps `git diff` changed files → indexed chunks → `getTransitiveDependents()`. Defaults base SHA to stored `git_head`.
- Output JSON: `affected_files[]`, `affected_symbols[]`, `edges[]` (with confidence + depth), `changed_files[]` (diff mode), `depth_reached`, `confidence_notes[]`, `metadata` (shared schema).
- Boundary: transitive + aggregated. Compact "best next read" packs stay in `context`.

### Shared confidence schema

All agent-first commands reuse a common metadata block:

```
index_is_stale            boolean
index_staleness_reason    string?
truncated                 boolean
budget_tokens             number
confidence_score?         number
confidence_tier?          'EXTRACTED' | 'INFERRED' | 'AMBIGUOUS'
is_ambiguous?             boolean
ambiguity_reason?         string
```

`confidence_score` / `is_ambiguous` / `ambiguity_reason` remain optional placeholders until P4 lands.

---

## Part II — Remaining (V6 → V6.2)

> P4 and P5 are both shipped; sections kept for historical reference and scoring math reference.

### ✅ P4 — Evidence-weighted confidence scoring *(shipped)*

**Why:** Current tiers (EXTRACTED/INFERRED/AMBIGUOUS) are coarse. claudecat proves multi-factor scoring with reasoning is debuggable. Shared confidence schema already exposes `confidence_score` fields — they just need to be populated.

**Schema (ALTER TABLE, backward compat):**
```sql
ALTER TABLE relationships ADD COLUMN confidence_score REAL;
ALTER TABLE relationships ADD COLUMN confidence_reasoning TEXT;
```

**Scoring function v1 — two factors only:**
```
score = resolution_quality × source_multiplier
```

| Factor | Value |
|---|---|
| **Resolution quality** | |
| Exact symbol + file match | 1.0 |
| Single name match | 0.7 |
| Multi-candidate (N matches) | 1/N |
| Unresolved | 0.2 |
| **Source multiplier** | |
| EXTRACTED (imports/exports from AST) | ×1.0 |
| INFERRED (calls) | ×0.8 |
| AMBIGUOUS | ×0.5 |

- Clamp [0, 1].
- `cortex relationships` orders by `confidence_score DESC`.
- `--verbose` shows `confidence_reasoning` per edge.
- `context` / `impact` surface `confidence_score` in their shared metadata block and use it for ranking and for neighbor-trim ordering (currently both use tier order only).
- Keep text tier column — if float scoring miscalibrates, revert without schema churn.
- **No context bonus in v1** (same-file, same-package deferred — "package boundary" underspecified for monorepos).

**Risks:** weights are heuristic; reasoning column makes miscalibration visible. Calibrate in point releases.

**Acceptance:** `cortex impact` neighbor ordering should visibly improve on at least one eval task versus tier-only ordering.

### ✅ P5 — AST-aware query filters *(shipped)*

Structured filter tokens parsed inline with `cortex search` / `cortex context` queries and applied as SQL WHERE clauses on `chunks`:

- `kind:function|class|config` — `chunks.chunk_type`. `method` aliases to `function`; `interface`/`type`/`enum` alias to `config` (the AST chunker's bucket for them).
- `lang:ts|tsx|js|py|go|rust|...` — `chunks.language`. Aliases short codes to full names (`ts→typescript`, `py→python`, etc.).
- `name:parse*` — glob on `chunks.symbol_name`. `*`/`**` → SQL `%`; existing `%`/`_` are backslash-escaped via `LIKE ? ESCAPE '\\'`.
- `file:src/auth/**` — glob on `chunks.file_path`, same escaping rules.

**Implementation:**
- `parseQueryFilters()` (`cli/src/index.ts`) tokenizes the query (preserving quoted phrases), strips filter tokens, returns `{ textQuery, filters }`. The remaining text is what gets embedded / FTS-matched.
- `buildFilterClause()` (`cli/src/turso.ts`) builds the trailing `AND ...` clause and arg list. `vectorSearch`, `keywordSearch`, `hybridSearch` all accept an optional `SearchFilters` parameter.
- `vectorSearch` over-fetches `(topK + offset) × 10` from `vector_top_k` when filters are active so post-ANN trimming doesn't return a near-empty page.
- `cmdContext` skips the exact-symbol short-circuit when filters are present (the user wants filtered hits, not the literal-name match).
- **Grammar version guard:** when `kind:` or `lang:` filters are used, `emitGrammarVersionWarning()` compares the project's stored `grammar_version` against `computeGrammarVersionHash()` and prints a one-line stderr warning on drift. Suppressible via `--quiet` / `CORTEX_QUIET=1`.

**Tests:** `cli/tests/phase2-smoke.mjs` covers basic parsing, glob conversion, alias mapping, SQL-LIKE wildcard escaping, and the no-filter pass-through.

### ⬜ P6 — Framework-idiom detection

**Why:** Tree-sitter gives syntax; idiom detection gives semantics. `app.use(passport.initialize())` is meaningfully different from `foo(bar())`. First-class idiom tagging is a real differentiator — but **only pursue after P4/P5 ship and eval data shows remaining recall gaps**.

**Scope:**
- Pluggable detector registry:
  ```typescript
  interface FrameworkDetector {
    name: string;
    languages: string[];
    match(node: ASTNode, context: FileContext): DetectedIdiom | null;
  }
  interface DetectedIdiom {
    kind: string;        // 'route' | 'hook' | 'middleware' | 'handler' | 'decorator'
    confidence: number;
    evidence: string;
    framework: string;   // 'express' | 'react' | 'hono' | 'nextjs' | 'fastapi'
  }
  ```
- **Ship 5 detectors (hard cap for v1):**
  1. Express routes (`app.get/post/put/delete/use`)
  2. React hooks (`useState`, `useEffect`, custom `use*`)
  3. Hono routes (`app.get/post/...`, `c.json/text`)
  4. Next.js route handlers (`export GET/POST` in route files)
  5. Python FastAPI decorators (`@app.get`, `@router.post`)
- Detected idioms enrich chunks with `framework_kind` field (ALTER TABLE chunks).
- High-confidence idioms generate synthetic relationships (e.g., `route → handler`).
- Searchable via P5 filters (`kind:route`, new `framework:express` filter).

**Explicitly NOT:**
- Cross-file pattern conflict resolution (claudecat's problem space).
- Evidence aggregation across files.
- More than 5 detectors.

**Risks:** scope-creep territory. Hard cap enforced. Validate ROI against eval data before starting.

### ⏸ P7 — Per-cluster skill snippets *(deferred / experimental)*

**Status:** Deferred per 2026-04-10 proposal. `CORTEX_REPORT.md` (global) + `cortex context` (local, depth-1) already cover the orientation workflow. Subsystem-level summarization is the next credible gap only if eval data shows agents still over-read subsystem files despite having `context` / `impact`.

**Activation condition:** start P7 only if eval results demonstrate that `context` + `impact` alone do not close the subsystem-orientation token gap.

**Scope (when activated):**
- `cortex index --clusters` groups chunks by top-level directory (no Leiden, no graph-derived clustering).
- Writes `.cortex/clusters/<name>.md` — top symbols, entry points, framework idioms (if P6 shipped), high-confidence relationships.
- `cli/cortex.skill.md` points Claude at `.cortex/clusters/` for subsystem orientation.
- **Lifecycle:**
  - Regenerated on full `cortex index`.
  - Stale-marked (header warning) on `cortex index --incremental` and `--watch`.
  - Deleted when project is deleted via `cortex delete`.
- Hard budget per cluster file. Gated behind `--clusters` flag until validated.

**Risks:** stale artifact management, token bloat, maintenance complexity without measurable KPI improvement.

---

## Part III — Execution order (remaining)

```
[P4 ✅] + [P5 ✅] ─▶ eval checkpoint ─▶ decide P6 vs P7
```

**Status:** P4 and P5 both landed. V6 is feature-complete.

**Next step — eval checkpoint:**
1. Run `cli/tests/compare-agent-evals.mjs` against a pre-P4 baseline (10 tasks, ≥3 off-repo controls).
2. Inspect:
   - `context` / `impact` neighbor ordering with real confidence scores
   - `kind:` / `lang:` filter usage on naturally-filterable tasks (e.g., "find the Hono route that handles X")
   - `context` staying within its 2000-token budget
3. Decide P6 (framework idioms) vs P7 (cluster summaries) based on where the measured gap actually is. Do not ship both speculatively.

---

## Part IV — KPI + acceptance (from 2026-04-10 proposal)

**Primary KPI:** reduce total agent context-token consumption on real code-understanding tasks without reducing answer correctness.

Success criteria for each remaining priority:
- task success rate ≥ `Cortex-current`
- median raw file reads on passed tasks decreases
- `context` stays within its ~2000-token budget on the majority of runs
- eval benchmark includes ≥10 tasks with ≥3 off-repo controls before claiming a hard percentage improvement (e.g., 15%)
- directional improvement on the majority of tasks before that threshold is set

If a shipped feature does not move the KPI, revisit scope before continuing down the list.

---

## Out of scope (unchanged)

- MCP server
- Multi-repo / monorepo registry
- Web UI / browser app
- Custom graph database (Turso stays)
- Leiden community detection
- Type-checker integration
- Cross-file pattern conflict resolution
- IDE features (rename, go-to-def, hover)
- Multimodal ingestion (images, PDFs)

---

## Future candidates (not in v6)

| ID | Item | Notes |
|---|---|---|
| P8 | Import-graph transitive traversal metrics | Fan-out metrics, transitive dep analysis beyond `impact`'s depth cap |
| P9 | Orphan project GC / bloat control | Auto-prune project rows whose directories no longer exist, node_modules/dist guard |

---

## Review history

| Date | Reviewer | Version | Key feedback |
|---|---|---|---|
| 2026-04-09 | Gemini (pass 1) | v1 | Incremental indexing gap, WASM distribution, RRF needs tuning knob, P5 cap at 5 |
| 2026-04-09 | Gemini (pass 2) | v2 | mtime fallback, confidence reasoning > float, grammar version hash, swap P5/P6, air-gap support |
| 2026-04-09 | — | v2.1 | Incorporated Gemini feedback, ported claudecat confidence model, added P5 filters + P6 idioms |
| 2026-04-10 | — | — | Agent-efficiency proposal (`2026-04-10-cortex-v6-agent-efficiency-proposal.md`) — narrowed scope to eval + hybrid + `context` + `impact`, deferred P7 |
| 2026-04-10 | Codex | v2.1 P1 | Fixed metadata loss in post-process + function-valued declaration misclassification |
| 2026-04-10 | Codex | v2.1 P2 | Fixed untracked-file omission, staleness repo path, mtime deletion detection |
| 2026-04-11 | — | v3 (this doc) | Consolidated 2026-04-09 + 2026-04-10 plans; marked P1/P2/P3/A0/A1/A2 shipped; reframed remaining work around V6 completion + V6.2 stretch |
| 2026-04-12 | — | v3 | Shipped P4 (evidence-weighted confidence scoring); `confidence_score` / `confidence_reasoning` surfaced in `context` / `impact` / `relationships` |
| 2026-04-12 | Codex | v3 P4 | Fixed schema migration gap on read commands; COALESCE legacy-null ordering so mixed-state DBs don't lose EXTRACTED edges in neighbor trim |
| 2026-04-13 | — | v3 | Shipped P5 (AST-aware `kind:`/`lang:`/`name:`/`file:` filters, grammar-drift warning); V6 feature-complete, next step is eval checkpoint |
