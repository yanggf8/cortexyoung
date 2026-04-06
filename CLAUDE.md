# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Cortex** is a semantic code intelligence tool for Claude Code. It chunks source files, embeds them locally (BGE-small-en-v1.5, 384-dim), and stores everything in Turso (vectors via F32_BLOB/DiskANN, content, relationships). One local process, one cloud database, no servers.

**V5.0**: CLI + Turso direct. Design doc: `docs/plans/2026-04-06-cortex-v5-direct-turso.md`. V3 legacy code archived on `archive/v3` branch.

## Commands

```bash
npm run build                          # Build CLI (TypeScript → dist/)
npm run dev -- <command>               # Run CLI in dev mode
npm run test                           # Phase 2 smoke tests
```

### CLI Usage

```bash
cortex init                            # Create Turso DB, store config
cortex index [path]                    # Chunk → embed → upload to Turso
cortex index [path] --watch            # Index then watch for changes
cortex search "query"                  # Semantic search (vector_top_k)
cortex search "query" --keyword        # Keyword search (FTS5)
cortex relationships "symbol"          # Recursive CTE traversal
cortex status                          # Project stats
cortex projects                        # List all indexed projects
cortex delete                          # Remove current project
cortex config                          # Show config
```

## Architecture

```
Claude Code → Skill → cortex CLI → @xenova/transformers (local embed)
                                   → @libsql/client → Turso (cloud DB)
```

### Source Files (`cli/src/`)

- `index.ts` — CLI entry point, 8 commands, import resolver, relationship resolution
- `turso.ts` — Turso client: schema, upsert, vector search, FTS5, relationships CTE
- `chunker.ts` — JS/TS/Python/Markdown chunking, chunk_id: `${projectId}:${filePath}:${startLine}`
- `embedder.ts` — `@xenova/transformers` wrapper for BGE-small-en-v1.5
- `config.ts` — `~/.cortex/config.json` management

### Key Design Decisions

- **vector_distance_cos()**: Turso's `vector_top_k()` returns only `id`, not `distance`. Must compute distance explicitly via `vector_distance_cos(c.embedding, vector(?))`.
- **ON CONFLICT DO UPDATE**: Preserves rowid for FTS5 + vector index integrity (INSERT OR REPLACE would churn rowid).
- **splitStatements()**: SQL parser respecting BEGIN...END trigger blocks for schema application.
- **PRAGMA foreign_keys = ON**: Required per-connection for CASCADE behavior in libSQL.
- **Over-fetch strategy**: When multi-project, fetch all chunks globally from vector_top_k then filter by project_id post-ANN.
- **Two-pass relationship resolution**: First pass builds symbolIndex/fileIndex, second pass resolves pending relationships to real chunk_ids.

## Development Notes

- TypeScript strict mode, ES2022 target, ESM modules
- Embedding model: BGE-small-en-v1.5 (384 dimensions) via `@xenova/transformers` (~200MB RSS, ~15ms/embed)
- Config: `~/.cortex/config.json` (Turso URL + auth token, 600 perms)
- Turso: F32_BLOB(384) column, DiskANN index, FTS5 for keyword search, relationships with CASCADE delete
- Skill file: `cli/cortex.skill.md` — loaded into Claude Code context
