# Cortex v6 — Intelligence Upgrade Plan (v2.1)

> Skill + CLI only. No MCP, no server.
> Inspired by GitNexus competitive analysis + claudecat AST pattern work.
> Gemini-reviewed (two passes, 2026-04-09).

---

## P1 — Tree-sitter chunking (foundational)

**Why:** Regex chunking (`cli/src/chunker.ts`) is the weakest link. Better chunks improve vector recall, FTS5 hits, and relationship accuracy simultaneously. Everything downstream benefits.

**Scope:**
- Replace chunker internals with `web-tree-sitter` (WASM — no node-gyp, matches `@xenova/transformers` distribution model).
- Initial grammars: **TS, JS, TSX, Python**.
- Extract real AST nodes: function/method/class/interface declarations, exports, imports, call expressions, extends/implements.
- Keep `chunk_id` format `${projectId}:${filePath}:${startLine}` for backward compat.
- **Regex fallback** for unsupported languages or parse failures.
- **Scope guard:** "typed AST nodes" = syntactic kinds (FunctionDecl, CallExpr, ImportDecl). No type-checker. No symbol resolution beyond name matching. Ship the boring version.

**Grammar distribution:**
- **Lazy-download** to `~/.cortex/grammars/` on first `cortex index` (matches BGE model-download pattern, keeps npm package lean).
- `cortex grammars install <path>` subcommand for **offline/air-gapped** installs from a local tarball.
- Store `grammar_version` hash in `projects` table. Warn on mismatch to prevent silent kind-name drift breaking query filters (P5).

**Downstream wins:** relationship resolver gets typed edges (`calls`, `imports`, `extends`, `implements`) instead of name-matching heuristics; AMBIGUOUS rate drops.

**Risks:**
- Grammar bundle size (~2-4 MB each) — resolved by lazy-download.
- Grammar version drift — resolved by storing hash + warning.
- Quality gap between AST and regex fallback — fallback chunks should indicate source (`ast` vs `regex`) in metadata.

---

## P2 — Git HEAD staleness + incremental re-index

**Why:** Detecting staleness without fixing it = warning fatigue. Merged from two separate items because the plumbing (--watch per-file CRUD) already exists.

**Scope:**
- Store `git rev-parse HEAD` in `projects` table on every `cortex index` completion.
- `cortex search` / `cortex status` compares stored HEAD to current. If different, emit one-line stderr hint:
  ```
  [cortex] index is N commits behind HEAD, run: cortex index --incremental
  ```
- Suppressible via `--quiet` flag or `CORTEX_QUIET=1` env var.
- **`cortex index --incremental`**: diffs `git diff --name-only -M <stored_sha>..HEAD` (note `-M` for renames), reuses existing per-file CRUD path (`deleteStaleFileChunks` / `replaceFileRelationships`).
- **mtime fallback**: when git HEAD check is inconclusive (stash, reset, untracked edits), fall back to file mtime comparison against `last_indexed_at` timestamp stored alongside `git_head`.
- CORTEX_REPORT.md gains `Indexed at: <sha> (<date>)` line.

**Risks:**
- `git diff -M` rename detection must cascade: delete old path chunks, index new path.
- mtime fallback won't catch content changes with preserved mtime (rare, acceptable).

---

## P3 — Hybrid search with RRF fusion

**Why:** `cortex search` and `cortex search --keyword` are currently either/or. RRF fusion routinely beats both individually.

**Scope:**
- **New default**: `cortex search "q"` runs `vector_top_k` AND FTS5 in parallel, fuses via Reciprocal Rank Fusion (k=60).
- Flags:
  - `--rrf-k <n>` — tune fusion constant (default 60).
  - `--vector` — force vector-only mode.
  - `--keyword` — force FTS5-only mode.
- Each result row tagged with source: `[vec]` / `[fts]` / `[both]` so users understand ranking.
- Update skill file to describe new default behavior.

**Risks:**
- Latency of parallel search — benchmark before shipping as default; consider `--vector` as fast fallback.
- RRF on small corpora can bury high-quality FTS5 hits under mediocre vectors. Monitor and adjust k or expose to users.

---

## P4 — Evidence-weighted confidence scoring

**Why:** Current tiers (EXTRACTED/INFERRED/AMBIGUOUS) are coarse. claudecat's `confidence-scoring.ts` proves multi-factor scoring with reasoning is achievable and debuggable.

**Schema (ALTER TABLE, backward compat):**
```sql
ALTER TABLE relationships ADD COLUMN confidence_score REAL;
ALTER TABLE relationships ADD COLUMN confidence_reasoning TEXT;
```

**Scoring function (v1 — minimal, two factors only):**
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
- **No context bonus in v1** (same-file, same-package bonuses deferred — "package boundary" is underspecified for monorepos).
- `cortex relationships` orders by `confidence_score DESC`.
- `--verbose` shows `confidence_reasoning` for each edge.
- Keep `confidence` text tier column — if float scoring miscalibrates, revert to tiers without schema churn.

**Risks:**
- Weights are heuristic. Reasoning column makes miscalibration visible and debuggable.
- calibration: observe in practice, adjust in point releases.

---

## P5 — AST-aware query filters

**Why:** Low effort, high UX win once tree-sitter lands. Directly improves skill usability for Claude Code agents.

**Scope:**
- Structured filters for `cortex search`:
  - `kind:function|class|method|interface|route|hook`
  - `lang:ts|py|tsx|js`
  - `name:parse*` (glob on symbol name)
  - `file:src/auth/**` (glob on file path)
- Parsed client-side, applied as SQL `WHERE` clauses before vector/FTS search.
- `kind:` values sourced from tree-sitter node kinds stored at index time.
- **Grammar version guard**: if stored `grammar_version` in projects table doesn't match current, warn that `kind:` filters may be inaccurate.

**Risks:**
- AST-query mismatch if grammar version drifts (mitigated by P1 grammar version hash).

---

## P6 — Framework-idiom detection

**Why:** Tree-sitter gives syntax; claudecat-style detection gives semantics. `app.use(passport.initialize())` is meaningfully different from `foo(bar())`. Tagging framework idioms as first-class extracted relationships is a real differentiator.

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
- Detected idioms become enriched chunks with `framework_kind` field stored in chunks table (ALTER TABLE).
- High-confidence idioms generate synthetic relationships (e.g., `route → handler`).
- Searchable via P5 filters (`kind:route`, `framework:express` — add `framework:` filter).

**Explicitly NOT:**
- Cross-file pattern conflict resolution (claudecat's problem space).
- Evidence aggregation across files.
- More than 5 detectors.

**Risks:**
- Detector registry is scope-creep territory. Hard cap enforced.
- Framework detection is speculative ROI — validate that hybrid search (P3) alone doesn't solve the recall problem first.

---

## P7 — Per-cluster skill snippets

**Why:** GitNexus's per-community SKILL.md is a context win. Lighter version without Leiden: group by directory + import clusters.

**Scope:**
- `cortex index --clusters` groups chunks by top-level directory.
- Writes `.cortex/clusters/<name>.md` per group — each file summarizes: top symbols, entry points, framework idioms (from P6).
- `cli/cortex.skill.md` points Claude at `.cortex/clusters/` for subsystem-level orientation.
- **Lifecycle rules:**
  - Regenerated on full `cortex index`.
  - Stale-marked (header warning) on `cortex index --incremental`.
  - Deleted when parent project is deleted via `cortex delete`.
- Gated behind `--clusters` flag initially; promote to default after validation.

**Risks:**
- Quality depends heavily on P1 chunk quality and P6 idiom detection.
- `.cortex/clusters/` can rot if lifecycle rules aren't enforced.

---

## Future candidates (not in v2.1)

| ID | Item | Notes |
|---|---|---|
| P8 | Import-graph transitive traversal | Fan-out metrics, transitive dep analysis beyond per-edge resolution |
| P9 | Orphan project GC / bloat control | Auto-prune project rows whose directories no longer exist, node_modules/dist guard |

---

## Execution order

```
P1 (tree-sitter) → P2 (staleness + incremental) → P3 (hybrid RRF)
                                                  → P4 (confidence) [parallel-safe with P3]
P5 (query filters) — depends on P1
P6 (framework idioms) — depends on P1, benefits from P3+P4 validation
P7 (cluster snippets) — depends on P6
```

**Suggested sequence:** P1 → P2 → P3 → P4 → P5 → P6 → P7

P3 and P4 can run in parallel after P2 if bandwidth allows.

---

## Out of scope

- MCP server
- Multi-repo / monorepo support
- Web UI
- Custom graph database (Turso stays)
- Leiden community detection
- Type-checker integration
- Cross-file pattern conflict resolution
- IDE features (rename, go-to-def, hover)

---

## Review history

| Date | Reviewer | Version | Key feedback |
|---|---|---|---|
| 2026-04-09 | Gemini (pass 1) | v1 | Incremental indexing gap, WASM distribution, RRF needs tuning knob, P5 cap at 5 |
| 2026-04-09 | Gemini (pass 2) | v2 | mtime fallback, confidence reasoning > float, grammar version hash, swap P5/P6, air-gap support |
| 2026-04-09 | — | v2.1 | Incorporated all Gemini feedback, ported claudecat confidence model, added P5 filters + P6 idioms |
