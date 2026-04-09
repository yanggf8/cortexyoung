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

Semantic search across the indexed codebase. Returns chunks ranked by relevance.

```bash
cortex search "$QUERY"
```

For keyword/exact match search:

```bash
cortex search "$QUERY" --keyword
```

Options:
- `--top-k N` — number of results (default: 15)
- `--offset N` — skip first N results for pagination
- `--project ID` — search a specific project (default: current project)
- `--keyword` — use FTS5 keyword search instead of semantic

Output is JSON with `chunks[]` (chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, language, score), `total_matches`, and `has_more`.

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
