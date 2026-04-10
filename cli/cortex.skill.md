# Cortex — Semantic Code Intelligence

Cortex gives you semantic search, relationship traversal, and project status for indexed codebases. It chunks source files (one function/class/method/top-level block per chunk — not line ranges or whole files), embeds them locally (BGE-small-en-v1.5, 384-dim), and stores everything in Turso (vectors, content, relationships).

## When to use Cortex

- **Read `CORTEX_REPORT.md` in the project root first** for a zero-tool-call orientation: god files, top symbols, language breakdown, relationship quality.
- You need to find code by meaning, not just text matching (e.g., "error handling in the auth flow")
- You want to understand how a function connects to the rest of the codebase (callers, imports, data flow)
- You want a quick overview of what's been indexed

All commands below assume `cortex` is on PATH (`npm install -g` from the repo). If not, fall back to `node <repo>/cli/dist/index.js`.

## Scripts

### search

Hybrid search (vector + FTS with Reciprocal Rank Fusion) across the indexed codebase. Returns chunks ranked by fused relevance.

```bash
cortex search "$QUERY"
```

For vector-only or keyword-only search:

```bash
cortex search "$QUERY" --vector    # vector-only (legacy)
cortex search "$QUERY" --keyword   # FTS5 keyword-only
```

Options:
- `--top-k N` — number of results (default: 15)
- `--offset N` — skip first N results for pagination
- `--project ID` — search a specific project (default: current project)
- `--vector` — vector-only semantic search
- `--keyword` — FTS5 keyword-only search
- `--rrf-k N` — RRF smoothing constant (default: 60)

Default output is JSON with `chunks[]` (chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, language, rrf_score, source: `vec`|`fts`|`both`, vec_rank, fts_rank), `total_matches`.

### context

**Agent-first command.** Returns a minimal context pack for answering a code-understanding question. Combines symbol lookup, hybrid search, and depth-1 relationship neighbors into a single compact response.

```bash
cortex context "$SYMBOL_OR_QUERY"
```

Use this instead of composing search + relationships + file reads manually. One command replaces multiple tool calls.

Output is JSON with:
- `primary_matches[]` — best-matching chunks (symbol match preferred, hybrid search fallback)
- `neighbor_chunks[]` — direct callers/callees/import/export neighbors with relationship metadata
- `key_files[]` — unique files worth reading next
- `confidence_notes[]` — ambiguity warnings, match type, staleness
- `suggested_next_queries[]` — follow-up commands when output was truncated
- `metadata` — `index_is_stale`, `index_staleness_reason`, `truncated`, `budget_tokens`

**Output budget:** Capped at ~2000 tokens. Low-confidence neighbors are trimmed first. When truncated, `metadata.truncated` is true and `suggested_next_queries` suggests the next step.

**Boundary:** `context` is depth-1, answer-the-question-now. For transitive blast radius or diff-rooted analysis, use `impact`.

### impact

**Agent-first command.** Aggregated blast-radius analysis. Two entry modes:

```bash
cortex impact --symbol "$NAME"       # what breaks if this symbol changes?
cortex impact --from-diff [base-sha] # what's affected by the current diff?
```

`--symbol` mode: finds all transitive dependents (callers, importers) up to depth 3.
`--from-diff` mode: identifies changed files via `git diff`, maps them to indexed chunks, then computes transitive impact. Uses stored `git_head` as default base SHA.

Output is JSON with:
- `affected_files[]` — files in the blast radius
- `affected_symbols[]` — symbols with file and chunk_type
- `edges[]` — relationship edges with confidence and depth
- `changed_files[]` — (from-diff mode only) files changed in the diff with status
- `depth_reached` — max traversal depth actually used
- `confidence_notes[]` — ambiguity warnings, staleness
- `metadata` — shared confidence schema

**Boundary:** `impact` is transitive and aggregated. For a compact "best next read" pack, use `context`.

### relationships

Traverse call graphs, imports, data flow, and other relationships for a symbol.

```bash
cortex relationships "$SYMBOL"
```

Options:
- `--depth N` — traversal depth (default: 2). Use `--depth 1` when `cortex status` shows a high AMBIGUOUS edge count — deeper traversal through ambiguous edges explodes noise.
- `--project ID` — specific project

Output is JSON with `nodes[]` (chunk_id, file_path, symbol_name, chunk_type) and `edges[]` (source, target, rel_type, confidence).

**Edge confidence**: `EXTRACTED` (deterministic, e.g. imports/exports), `INFERRED` (single symbol-name match for `calls`), `AMBIGUOUS` (multi-target name collision or unresolved). Prefer EXTRACTED edges; treat AMBIGUOUS as noisy and filter them out unless you need a broad graph view.

### status

Show project stats: chunk count, relationship count, languages, last indexed time, relationship confidence breakdown.

```bash
cortex status
```

Options:
- `--project ID` — specific project

Output is JSON with project_id, name, chunk_count, relationship_count, last_indexed, languages, confidence_breakdown (`{EXTRACTED, INFERRED, AMBIGUOUS}` edge counts).

### index (with --watch)

Index then watch for file changes, reindexing incrementally per-file. Generates `CORTEX_REPORT.md` in the project root on full index (not on watch updates — report goes stale during watch).

```bash
cortex index . --watch
```

Watch mode uses per-file delete/upsert — no full rebuild on each save. Ctrl+C to stop.

### index --incremental

Reindex only files changed since the last full or incremental run. Uses `git diff` when a stored SHA exists, falls back to file mtime comparison. Handles renames, deletions, and untracked (not yet `git add`'d) files.

```bash
cortex index . --incremental
```

Falls back to full index automatically when no prior state exists. Staleness hints appear on `cortex search`/`cortex status` when the index is behind HEAD (suppress with `--quiet` or `CORTEX_QUIET=1`).

## Setup

The project must be indexed first:

```bash
cd /path/to/project
cortex init      # one-time: creates Turso DB
cortex index .   # index current directory
```

Config is stored in `~/.cortex/config.json`.
