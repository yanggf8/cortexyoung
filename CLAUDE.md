# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Cortex** is a semantic code intelligence MCP server for Claude Code. It solves Claude Code's context window problem by automatically discovering and delivering architectural context (dependencies, patterns, structures) relevant to each query.

**V3.0 (Current)**: Centralized architecture — a single HTTP embedding server (port 8766) shared by all Claude Code instances, with lightweight stdio MCP clients connecting to it.

**V4.0 (Obsolete)**: Cloudflare-only design, superseded by V5.0. See `docs/plans/2026-03-06-cortex-v4-cloud-design.md`

**V5.0 (Planned)**: Skill + Cloud architecture — local embeddings (@xenova/transformers BGE-small-en-v1.5) + Cloudflare Worker API + Turso (content, vectors via native F32_BLOB/DiskANN, relationships). Replaces MCP with Claude Code skill. Design doc: `docs/plans/2026-04-01-cortex-v5-skill-cloud-design.md`

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
