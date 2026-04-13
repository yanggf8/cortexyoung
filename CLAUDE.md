# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Cortex** is a semantic code intelligence tool for Claude Code. It chunks source files, embeds them locally (BGE-small-en-v1.5, 384-dim), and stores everything in Turso (vectors via F32_BLOB/DiskANN, content, relationships). One local process, one cloud database, no servers.

**Current version: V6** (P1–P5 + A0–A2 shipped). Design doc: `docs/plans/2026-04-09-cortex-v6-intelligence-upgrade.md`. V5 base: `docs/plans/2026-04-06-cortex-v5-direct-turso.md`. V3 legacy code archived on `archive/v3` branch.

## Commands

```bash
npm run build                          # Build CLI (TypeScript → dist/)
npm run dev -- <command>               # Run CLI in dev mode (tsx, no build needed)
npm run test                           # Build + phase 2 smoke tests
```

Monorepo layout: root `package.json` delegates to `"workspaces": ["cli"]`. All `npm run` commands at root forward to `cli/`.

### CLI Usage

```bash
cortex init                            # Create Turso DB, store config
cortex index [path]                    # Chunk → embed → upload to Turso
cortex index [path] --watch            # Index then watch for changes
cortex index [path] --incremental      # Reindex only files changed since last run
cortex search "query"                  # Hybrid search (vector + FTS, RRF)
cortex search "query kind:function lang:ts" # With P5 inline filters
cortex search "query" --vector         # Vector-only semantic search
cortex search "query" --keyword        # Keyword search (FTS5)
cortex search "query" --quiet          # Suppress staleness hint
cortex context "symbol-or-query"       # Minimal context pack (depth-1 neighbors)
cortex impact --symbol "name"          # Blast-radius analysis for a symbol
cortex impact --from-diff [sha]        # Impact analysis from git diff
cortex relationships "symbol"          # Recursive CTE traversal
cortex status                          # Project stats
cortex projects                        # List all indexed projects
cortex delete                          # Remove current project
cortex config                          # Show config
cortex grammars                        # Show grammar status
cortex grammars install <path>         # Install grammars from local dir
```

## Architecture

```
Claude Code → Skill → cortex CLI → web-tree-sitter (AST chunking, WASM)
                                   → @xenova/transformers (local embed)
                                   → @libsql/client → Turso (cloud DB)
```

### Source Files (`cli/src/`)

- `index.ts` — CLI entry point, 11 commands (including `context` and `impact`), import resolver, relationship resolution
- `turso.ts` — Turso client: schema, upsert, vector/FTS/hybrid search, relationships CTE, neighbor lookup, transitive impact, per-file CRUD for watch mode
- `chunker.ts` — Regex-based JS/TS/Python/Markdown chunking (fallback), chunk_id: `${projectId}:${filePath}:${startLine}`
- `ast-chunker.ts` — Tree-sitter AST-based chunking for TS/JS/TSX/Python (primary). Extracts real function/class/interface/import/export/call nodes.
- `grammars.ts` — Grammar management: resolves WASM files from npm packages (`tree-sitter-javascript`, etc.) or `~/.cortex/grammars/` cache. Supports offline install.
- `embedder.ts` — `@xenova/transformers` wrapper for BGE-small-en-v1.5
- `config.ts` — `~/.cortex/config.json` management

### Key Design Decisions

- **vector_distance_cos()**: Turso's `vector_top_k()` returns only `id`, not `distance`. Must compute distance explicitly via `vector_distance_cos(c.embedding, vector(?))`.
- **ON CONFLICT DO UPDATE**: Preserves rowid for FTS5 + vector index integrity (INSERT OR REPLACE would churn rowid).
- **splitStatements()**: SQL parser respecting BEGIN...END trigger blocks for schema application.
- **PRAGMA foreign_keys = ON**: Required per-connection for CASCADE behavior in libSQL.
- **Over-fetch strategy**: When multi-project, fetch all chunks globally from vector_top_k then filter by project_id post-ANN.
- **Two-pass relationship resolution**: First pass builds symbolIndex/fileIndex, second pass resolves pending relationships to real chunk_ids.
- **Per-file watch operations**: Watch mode uses file-scoped delete/upsert (`deleteStaleFileChunks`, `replaceFileRelationships`) to avoid full-project rebuilds. Stale chunks are pruned by diffing current chunk_ids against DB; CASCADE handles relationship cleanup on file deletion.
- **Edge confidence tags**: Relationships carry a `confidence` column — `EXTRACTED` (deterministic: imports/exports), `INFERRED` (single symbol-name match for `calls`), `AMBIGUOUS` (multi-target name collision or unresolved). `applySchema` runs ALTER TABLE for existing DBs and is invoked at the start of every `cortex index` to migrate forward.
- **Evidence-weighted confidence scoring (P4)**: Relationships also carry `confidence_score REAL` (0–1) and `confidence_reasoning TEXT`. Score = `resolution_quality × source_multiplier`: file-index resolution → 1.0, single name match → 0.7, N-way collision → 1/N; multiplied by EXTRACTED ×1.0, INFERRED ×0.8, AMBIGUOUS ×0.5. `context` sorts neighbors by score (highest first) for budget trimming. `relationships` orders edges by score DESC. `impact` returns scores per edge. `--verbose` on `cortex relationships` shows reasoning strings. Text tier kept as fallback.
- **CORTEX_REPORT.md**: `cortex index` writes a one-page summary (god files, languages, chunk types, top symbols) to the project root for zero-tool-call orientation post-compact. Excluded from indexing via `IGNORE_FILES`. Best-effort write — read-only trees emit a warning instead of failing. Stale after `--watch` updates.
- **PreToolUse nudge hook**: `.claude/hooks/cortex-nudge.sh` suggests `cortex search` over Grep when an index exists. Soft hint only.
- **AST-first chunking (v6)**: `chunkFileAST()` tries tree-sitter first, falls back to regex on failure or unsupported language. Grammar WASM files resolved from npm packages (`tree-sitter-javascript`, `tree-sitter-typescript`, `tree-sitter-python`) at zero download cost. Offline installs via `cortex grammars install <dir>` to `~/.cortex/grammars/`. Grammar version hash stored in `projects.grammar_version` for staleness detection. Each chunk carries `chunk_source` ('ast' or 'regex').
- **Git HEAD staleness + incremental reindex (P2)**: `cortex index` stores `git_head` (SHA) and `last_indexed_at` (epoch ms) in the `projects` table. `cortex search` / `cortex status` compare stored HEAD to current and emit a one-line stderr hint when behind (suppressible via `--quiet` or `CORTEX_QUIET=1`). Staleness check resolves against the project's stored path (not cwd) so default-project cross-directory lookups compare the right repo. `cortex index --incremental` diffs `git diff --name-status -M` against stored SHA plus `git ls-files --others` for untracked files (or mtime fallback against `last_indexed_at` with deletion detection via DB file-path cross-reference). Replays only changed files through `reindexOneFile()` (shared with watch mode). Renames cascade: delete old-path chunks, index new path. Falls back to full index when no prior state exists.
- **Hybrid search (V6.1)**: Default `cortex search` runs both vector and FTS5 in parallel, fusing results via Reciprocal Rank Fusion (rank-based only, no raw-score normalization). Each result tagged with `source: vec|fts|both` and rank positions from each branch. Pagination (`--offset`, `has_more`) preserved on the hybrid path via over-fetch + slice. Escape hatches: `--vector` for vector-only, `--keyword` for FTS-only, `--rrf-k N` to tune the smoothing constant (default 60).
- **FTS query sanitization (V6.1)**: Raw user queries routinely contain FTS5 syntax characters (`.`, `(`, `-`, `/`) that would otherwise throw a parse error and tear down the whole hybrid search inside `Promise.all`. `sanitizeFtsQuery()` tokenizes on `[A-Za-z0-9_]+`, wraps each token as a phrase, and OR-joins for recall (`useEffect(` → `"useEffect"`; `foo.bar` → `"foo" OR "bar"`). The FTS branch is also wrapped in a `.catch()` fallback so any remaining edge case degrades gracefully to vector-only instead of failing the call.
- **Agent-first `context` command (V6.1)**: `cortex context "query"` returns a compact context pack (budget: <2000 tokens). Tries exact symbol match first, falls back to hybrid search. Fetches depth-1 relationship neighbors via `getDirectNeighbors()`. Trims low-confidence (AMBIGUOUS) neighbors first when over budget, then returns `truncated: true` with `suggested_next_queries`. Inherits git-staleness contract from search. Shared `ConfidenceMetadata` schema with `impact`.
- **Agent-first `impact` command (V6.1)**: Two entry modes — `--symbol` (transitive dependents via reverse CTE, depth 3) and `--from-diff [sha]` (git diff → chunk lookup → transitive impact). Uses `getTransitiveDependents()` for reverse-edge traversal. For `--from-diff`, seeds are looked up under the path they had at index time: `D` uses `change.path` (the DB still holds the chunks about to disappear — that's the blast-radius case), `R` uses `change.oldPath` (new path not yet indexed), `A`/`M` use `change.path`. Shares `ConfidenceMetadata` output schema with `context`. `context` is depth-1 "answer now"; `impact` is transitive "what else breaks".
- **AST-aware query filters (P5)**: `cortex search` and `cortex context` accept structured filter tokens inline with the query: `kind:function|class|config`, `lang:ts|tsx|js|py|...`, `name:parse*` (glob on `chunks.symbol_name`), `file:src/auth/**` (glob on `chunks.file_path`). `parseQueryFilters()` strips filter tokens before embedding/FTS; remaining text is the ranking signal. `lang:` aliases (`ts→typescript`, `py→python`, etc.) and `kind:` aliases (`method→function`, `interface|type|enum→config`, since the AST chunker stores them as `config`) live in `index.ts`. Globs convert via `globToLike()` — existing SQL `%`/`_` are backslash-escaped, then `*`/`**` map to `%`. Filters are AND-combined and applied as `WHERE` clauses inside `vectorSearch` / `keywordSearch` / `hybridSearch` (see `buildFilterClause()` in `turso.ts`); `vectorSearch` over-fetches 10× from `vector_top_k` when filters are active so post-ANN trimming doesn't return a near-empty page. When `kind:` or `lang:` filters are used, `emitGrammarVersionWarning()` compares the project's stored `grammar_version` to the current bundle and prints a one-line stderr warning on drift (suppressible via `--quiet` / `CORTEX_QUIET=1`). The `--keyword` path now sanitizes free-form queries with the same FTS-safe tokenizer used by hybrid search so punctuation or filter-heavy queries do not throw parse errors.

## Development Notes

- TypeScript strict mode, ES2022 target, ESM modules
- Embedding model: BGE-small-en-v1.5 (384 dimensions) via `@xenova/transformers` (~200MB RSS, ~15ms/embed)
- AST parsing: `web-tree-sitter` (WASM) + grammar packages for TS/JS/TSX/Python
- Config: `~/.cortex/config.json` (Turso URL + auth token, 600 perms)
- Turso: F32_BLOB(384) column, DiskANN index, FTS5 for keyword search, relationships with CASCADE delete
- Skill file: `cli/cortex.skill.md` — loaded into Claude Code context
- **Schema migration contract**: new columns added via `ALTER TABLE` try/catch in `applySchema()` (`turso.ts`). All read/write commands call `ensureSchema()` (`index.ts`) — a memoized wrapper — so existing user databases migrate forward automatically. When adding a new column, add the ALTER TABLE to `applySchema`, and ensure any SQL that SELECTs it uses `COALESCE` for pre-migration rows with NULL values.
- **Test export convention**: functions exported as `*ForTest` (e.g., `resolveRelationshipsForTest`, `parseQueryFilters`) are wired in `cli/src/index.ts` for import by `cli/tests/phase2-smoke.mjs`. The smoke test imports from `../dist/index.js` (the built output), so `npm run build` must precede test runs (the `test:phase2-smoke` script handles this).
- **Chunk type values**: `function | class | method | documentation | config` — the AST chunker stores `interface`, `type`, and `enum` declarations as `config`.
- **Requirements**: Node.js 20+, Turso CLI (`turso`), Turso account (free tier: 9GB, 500M reads/mo)
