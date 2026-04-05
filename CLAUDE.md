# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Cortex** is a semantic code intelligence tool for Claude Code. It solves Claude Code's context window problem by automatically discovering and delivering architectural context (dependencies, patterns, structures) relevant to each query.

**Repo state (hybrid, April 2026)**: The repository currently contains both the legacy V3 MCP/server stack and the new V5 CLI implementation. The V5 code lives under `cli/` and is the active migration target; the root `src/` tree still contains the V3 server/MCP runtime and has not been decommissioned yet.

**V3.0 (Legacy, still present)**: Local MCP server — ProcessPool embeddings, local vector store, ~34K LOC. Still shipped in the root package and still documented in parts of the repo, but intended to be removed once V5 is complete.

**V5.0 (Feature-complete through Phase 2 surface)**: CLI + Turso direct. Local embeddings (@xenova/transformers BGE-small-en-v1.5) → Turso (vectors via F32_BLOB/DiskANN + content + relationships). No Workers. The standalone package is `cli/`, with commands including `init`, `index`, `search`, `relationships`, `status`, `projects`, `delete`, and `config`. Claude Code skill delivery, full end-to-end terminal validation, and V3 decommissioning are not done yet. Design doc: `docs/plans/2026-04-06-cortex-v5-direct-turso.md`

## Commands

### Build & Run
```bash
npm run build                          # Compile TypeScript (required before deploy)
npm run start:centralized              # Start centralized embedding server (port 8766)
npm run start:centralized -- 8777      # Custom port
npm run startup                        # Start server with health checks
npm run shutdown                       # Clean shutdown with process cleanup
npm run health                         # HTTP-based health check
npm run status                         # Check server status
```

### V5 CLI
```bash
cd cli
npm run build                          # Build standalone cortex CLI
npm run dev -- init                    # Initialize Turso-backed V5 config/database
npm run dev -- index .                 # Index current project into Turso
npm run dev -- search "query"          # Semantic search via Turso vector_top_k()
```

### MCP Servers
```bash
npm run server                         # Lightweight MCP server (HTTP transport)
npm run lightweight-server             # Alternative lightweight server
# stdio MCP entry point: cortex-stdio-mcp.js (registered via claude mcp add)
```

### Testing
```bash
npm run test:lightweight               # Test lightweight MCP clients
npm run test:cleanup                   # ProcessPoolEmbedder cleanup
npm run test:cpu-memory                # CPU + memory adaptive scaling
npm run test:signal-cascade            # Parent→child signal cascade
npm run test:final-cleanup             # Comprehensive validation suite
npm run benchmark                      # Full benchmark suite
npm run benchmark:quick                # Quick validation
npm run validate:performance           # Critical improvements validation
npm run demo                           # Run indexing demo
```

### Storage
```bash
npm run storage:status                 # Complete status report
npm run storage:validate               # Consistency check
npm run cache:clear-all                # Clear all storage
```

## Architecture

### Data Flow
```
File Changes → SemanticWatcher → Change Queue → Delta Analysis
     ↓               ↓               ↓              ↓
Claude Code ← MCP Server ← Vector Store ← ProcessPool → Incremental Updates
```

### V3.0 Two-Tier Architecture
1. **Centralized Embedding Server** (`cortex-embedding-server.ts`) — HTTP server on port 8766 with ProcessPool, memory-mapped cache, and PersistentVectorStore. Singleton-enforced via PID file at `~/.cortex/centralized-server.pid`.
2. **Lightweight MCP Clients** (`cortex-stdio-mcp.js`, `server.ts`, `stdio-server.ts`) — Thin clients that forward requests to the centralized server via HTTP. The stdio client auto-starts the server if not running.

### V5.0 Direct CLI Architecture
1. **Standalone CLI package** (`cli/src/index.ts`) — Node.js entry point for `init`, `index`, `search`, `relationships`, `status`, `projects`, `delete`, and `config`.
2. **Direct Turso client** (`cli/src/turso.ts`) — schema management, chunk upsert, project metadata, FTS, relationships, and vector search via `@libsql/client`.
3. **In-process embedding/chunking** (`cli/src/embedder.ts`, `cli/src/chunker.ts`) — local `@xenova/transformers` embeddings and local chunk generation.

### Key Source Files

**Centralized Server Layer:**
- `cortex-embedding-server.ts` — Main HTTP server with ProcessPool, routes, client registration
- `centralized-handlers.ts` — Server-side MCP tool implementations (semantic search, code intelligence, etc.)
- `context-enhancement-layer.ts` — Project type detection and context injection into search results
- `start-centralized-server.ts` — Startup script with singleton enforcement and PID management
- `embedding-client.ts` — HTTP client with circuit breaker for centralized server communication

**Core Intelligence:**
- `indexer.ts` — Repository indexing with incremental support, produces CodeChunks
- `searcher.ts` — Semantic search with MMR optimization, the main search engine
- `process-pool-embedder.ts` — Manages external Node.js processes for BGE-small-en-v1.5 embeddings with CPU/memory adaptive scaling
- `persistent-vector-store.ts` — Vector storage with cosine similarity search, persisted to `.cortex/index.json.gz`
- `chunker.ts` — Splits source files into semantic CodeChunks using tree-sitter (functions, classes, methods)

**Relationship Analysis:**
- `relationship-traversal-engine.ts` — Multi-hop relationship discovery across code chunks
- `call-graph-analyzer.ts` / `data-flow-analyzer.ts` — Static analysis for call graphs and data flow
- `smart-dependency-chain.ts` — Automatic dependency context inclusion

**File Watching:**
- `semantic-watcher.ts` — chokidar-based file monitoring with semantic change detection
- `context-invalidator.ts` — Chunk invalidation and reindexing triggers
- `staging-manager.ts` — Dual-mode tracking (git-tracked vs untracked files)

**MCP Interface:**
- `mcp-tools.ts` — Tool definitions and schemas (5 core tools: semantic_search, code_intelligence, relationship_analysis, real_time_status, multi_instance_health; plus fetch_chunk, next_chunk for pagination)
- `mcp-handlers.ts` — HTTP transport MCP handlers
- `lightweight-handlers.ts` — Lightweight handlers that proxy to centralized server
- `cortex-stdio-mcp.js` — Production stdio MCP entry point (compiled JS, registered with Claude Code)

**Infrastructure:**
- `memory-mapped-cache.ts` — Zero-copy cross-process embedding cache via mmap
- `unified-storage-coordinator.ts` — Dual persistence: local `.cortex/` + global `~/.claude/`
- `types.ts` — Core types (CodeChunk, ChunkType, etc.)
- `env-config.ts` — Environment variable configuration (all `CORTEX_` prefixed vars with unprefixed fallback)
- `scripts/` — Operational scripts: startup, shutdown, health checks, storage management

**V5 CLI Package:**
- `cli/src/index.ts` — standalone CLI entry point
- `cli/src/turso.ts` — Turso schema and query layer
- `cli/src/chunker.ts` — V5 chunking logic
- `cli/src/embedder.ts` — local embedding runtime
- `cli/src/config.ts` — `~/.cortex/config.json` management for V5

### MCP Integration
```bash
# Install (one-time)
npm run start:centralized              # Start backend
claude mcp add cortex "$(pwd)/cortex-stdio-mcp.js" --scope user

# Verify
claude mcp list                        # Should show cortex: ✓ Connected
```

5 core MCP tools: `semantic_search`, `code_intelligence`, `relationship_analysis`, `real_time_status`, `multi_instance_health` (plus `fetch_chunk`, `next_chunk` for response pagination)

## Development Notes

- TypeScript strict mode, ES2020 target, CommonJS modules
- No external database — in-memory + file persistence (`.cortex/index.json.gz`)
- `.cortex/` directory is gitignored — contains local index, cache, and PID files
- Embedding model: BGE-small-en-v1.5 (384 dimensions, 400-chunk batches) via `fastembed`
- Code parsing uses `tree-sitter` with TypeScript and JavaScript grammars
- ProcessPool spawns external Node.js processes (`src/external-embedding-process.js`) each using ~200-400MB
- Resource thresholds: Memory stop at 78%/resume at 69%, CPU stop at 69%/resume at 49%
- Auto-shutdown: Server stops when no MCP clients connected (configurable via `CORTEX_AUTO_SHUTDOWN`, `CORTEX_NO_CLIENTS_TIMEOUT`, `CORTEX_IDLE_TIMEOUT`)
- Process cleanup: Always run `npm run shutdown` or `pkill -f "npm.*demo\|ts-node.*index\|node.*external-embedding-process"` after interrupts
- Multi-instance logs: `~/.cortex/multi-instance-logs/`
- Embedding strategies: <500 chunks uses cached strategy, >=500 uses ProcessPool directly
- Cloudflare AI embedder available as alternative (`cloudflare-ai-embedder.ts`) with circuit breaker pattern
- Several plain `.js` files exist in `src/` (worker processes, embedding bridge) — these are intentionally not TypeScript

### Environment Variables

All env vars support `CORTEX_` prefix (preferred) with unprefixed fallback. Full list in `env-config.ts`.

| Variable | Purpose | Default |
|----------|---------|---------|
| `CORTEX_EMBEDDING_SERVER_PORT` | Centralized server port | 8766 |
| `CORTEX_AUTO_SHUTDOWN` | Enable auto-shutdown | true |
| `CORTEX_NO_CLIENTS_TIMEOUT` | No-clients shutdown delay (ms) | 300000 |
| `CORTEX_IDLE_TIMEOUT` | Idle shutdown delay (ms) | 1800000 |
| `DISABLE_REAL_TIME` | Disable file watching | false |
| `FORCE_REBUILD` | Force full reindex on startup | false |
| `MCP_MULTI_INSTANCE` | Multi-instance compatibility | false |
