# Cortex — Semantic Code Intelligence

Cortex gives you semantic search, relationship traversal, and project status for indexed codebases. It chunks source files, embeds them locally (BGE-small-en-v1.5, 384-dim), and stores everything in Turso (vectors, content, relationships).

## When to use Cortex

- You need to find code by meaning, not just text matching (e.g., "error handling in the auth flow")
- You want to understand how a function connects to the rest of the codebase (callers, imports, data flow)
- You want a quick overview of what's been indexed

## Scripts

### search

Semantic search across the indexed codebase. Returns chunks ranked by relevance.

```bash
node ~/a/cortexyoung/cli/dist/index.js search "$QUERY"
```

For keyword/exact match search:

```bash
node ~/a/cortexyoung/cli/dist/index.js search "$QUERY" --keyword
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
node ~/a/cortexyoung/cli/dist/index.js relationships "$SYMBOL"
```

Options:
- `--depth N` — traversal depth (default: 2)
- `--project ID` — specific project

Output is JSON with `nodes[]` (chunk_id, file_path, symbol_name, chunk_type) and `edges[]` (source, target, rel_type).

### status

Show project stats: chunk count, relationship count, languages, last indexed time.

```bash
node ~/a/cortexyoung/cli/dist/index.js status
```

Options:
- `--project ID` — specific project

Output is JSON with project_id, name, chunk_count, relationship_count, last_indexed, languages.

### index (with --watch)

Index then watch for file changes, reindexing incrementally per-file. Generates `CORTEX_REPORT.md` in the project root on full index (not on watch updates).

```bash
node ~/a/cortexyoung/cli/dist/index.js index . --watch
```

Watch mode uses per-file delete/upsert — no full rebuild on each save. Ctrl+C to stop.

## Setup

The project must be indexed first:

```bash
cd /path/to/project
node ~/a/cortexyoung/cli/dist/index.js init      # one-time: creates Turso DB
node ~/a/cortexyoung/cli/dist/index.js index .    # index current directory
```

Config is stored in `~/.cortex/config.json`.
