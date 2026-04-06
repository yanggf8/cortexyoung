# Cortex V4.0 Cloud Architecture Design

**Date**: 2026-03-06
**Revised**: 2026-04-01
**Status**: OBSOLETE — Superseded by V5.0 (`2026-04-01-cortex-v5-skill-cloud-design.md`)

## 1. Problem Statement

The current Cortex V3.0 architecture is too complex:
- Local embedding generation (ProcessPool, memory management, ~200-400MB per worker)
- Local vector storage with sync issues (`.cortex/index.json.gz`)
- Heavy MCP server with complex state management (7 tools, fallback modes)
- Limited by local resources (CPU/memory thresholds, single-machine only)

**Goal**: Simplify to a lightweight, cloud-based architecture that reduces Claude Code's context window more effectively while eliminating local resource constraints.

## 2. Architecture Overview

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────────┐
│   Claude Code   │────▶│   MCP Client     │────▶│  Cloudflare Worker  │
│                 │     │   (thin proxy)   │     │  (API + logic)      │
└─────────────────┘     └──────────────────┘     └──────────┬──────────┘
                                                            │
                         ┌──────────────────────────────────┴──────────┐
                         │                                              │
                   ┌─────▼─────┐                                ┌──────▼──────┐
                   │Cloudflare │                                │Cloudflare AI│
                   │ Vectorize │                                │ Workers AI  │
                   └───────────┘                                └─────────────┘
```

## 3. Design Decisions (Resolved)

### 3.1 Single-User, API Key Auth
- **Decision**: Single-user personal tool. No multi-tenant.
- **Auth**: API key per installation, generated on first `cortex init`. Stored in `~/.cortex/config.json`.
- **Rationale**: Multi-tenant adds auth complexity (OAuth, RBAC, tenant isolation) with no immediate value. Single-user keeps it simple. Can add multi-tenant later if demand arises — Vectorize namespaces already support it.

### 3.2 No D1 — Vectorize Metadata Only
- **Decision**: Use Vectorize metadata fields for project info. No D1.
- **Rationale**: The only "relational" data is project name, file paths, and chunk metadata — all fit naturally as Vectorize metadata filters. Adding D1 means another service to manage, another billing line, and cross-service consistency concerns. If relational queries become necessary later, add D1 then.

### 3.3 Cloudflare Workers AI for Embeddings
- **Model**: `@cf/baai/bge-small-en-v1.5` (same as V3.0 local model — ensures embedding compatibility)
- **Free tier**: 10,000 neurons/day (~2,600 embeddings/day at 384 dimensions)
- **Paid**: $0.011 per 1,000 neurons beyond free tier
- **Estimated cost**: A 10K-chunk project costs ~$0.04 to fully index. Daily incremental updates negligible.

### 3.4 Vectorize for Storage
- **Dimensions**: 384 (matches BGE-small-en-v1.5)
- **Namespace strategy**: One namespace per project, named by project hash
- **Metadata per vector**: `file_path`, `symbol_name`, `chunk_type`, `start_line`, `end_line`, `content_hash`, `language`, `project_id`
- **Free tier**: 5M vector dimensions stored, 30M queried/month
- **Estimated capacity**: ~13,000 vectors free (5M ÷ 384)

## 4. Components

### 4.1 MCP Client (Lightweight)
- **Purpose**: Stateless HTTP proxy — no local state, no caching, no fallback modes
- **Responsibilities**:
  - Parse MCP protocol requests from Claude Code
  - Forward to Cloudflare Worker API
  - Return responses
- **Replaces**: `cortex-stdio-mcp.js`, `lightweight-handlers.ts`, `embedding-client.ts`, `cortex-mcp-client.ts`

### 4.2 Cloudflare Worker (Remote Server)
- **Purpose**: Single Worker handling all business logic
- **Routes**:
  - `POST /search` — Embed query via Workers AI → search Vectorize → return ranked chunks
  - `POST /index` — Receive chunks → embed via Workers AI → upsert to Vectorize
  - `POST /index/batch` — Bulk indexing for initial project setup
  - `GET /projects` — List indexed projects (query Vectorize metadata)
  - `GET /projects/:id/status` — Project indexing stats
  - `DELETE /projects/:id` — Remove project vectors
  - `GET /health` — Worker health check
- **Replaces**: `cortex-embedding-server.ts`, `centralized-handlers.ts`, `process-pool-embedder.ts`, `persistent-vector-store.ts`, `memory-mapped-cache.ts`

### 4.3 CLI Tool
- **Name**: `cortex`
- **Commands**:
  ```bash
  cortex init                # Generate API key, configure remote endpoint
  cortex index               # Index current project to cloud (chunk locally, embed+store remotely)
  cortex index --watch       # Watch filesystem and push incremental updates
  cortex search "query"      # CLI search (bypasses MCP, useful for debugging)
  cortex status              # Show project indexing status from cloud
  cortex projects            # List all cloud-indexed projects
  cortex config              # View/edit config (~/.cortex/config.json)
  cortex delete              # Remove project from cloud
  ```
- **Chunking stays local**: tree-sitter parsing happens in the CLI. Only chunk content is sent to the Worker for embedding. This avoids sending full source files to the cloud.

## 5. MCP Tools (Simplified)

5 tools, down from 7 (removed 2 deprecated + merged `code_intelligence` into `semantic_search`):

| Tool | Description |
|------|-------------|
| `semantic_search` | Vector search in Vectorize with relationship context |
| `contextual_read` | File read with cloud-sourced semantic context |
| `relationship_analysis` | Query relationship data from Vectorize metadata |
| `list_projects` | List cloud-indexed projects |
| `get_project_status` | Check indexing status and stats |

**Removed**:
- `trace_execution_path` — Removed in V3.0 cleanup (too complex, low value)
- `find_code_patterns` — Removed in V3.0 cleanup (over-engineered)
- `code_intelligence` — Merged into `semantic_search` (same underlying search, different prompt framing)
- `fetch_chunk` / `next_chunk` — Server handles response sizing; no client-side pagination needed

## 6. Data Flow

### 6.1 Indexing (Push-based, CLI-driven)
```
File changes → CLI (tree-sitter chunk) → POST /index/batch
                                              ↓
                                    Workers AI (embed) → Vectorize (store)
```

### 6.2 Search (MCP-driven)
```
Claude Code → MCP Client → POST /search → Workers AI (embed query)
                                               ↓
                                        Vectorize (similarity search)
                                               ↓
                                        Claude Code (optimized response)
```

### 6.3 Relationship Data
Relationships (`calls`, `imports`, `data_flow`) are stored as Vectorize metadata on each vector. The Worker reconstructs relationship graphs by querying metadata filters — no separate relationship store needed.

## 7. Migration Plan

### Phase 1: Cloudflare Worker + Vectorize (2 weeks)
1. Scaffold Cloudflare Worker project with Wrangler
2. Create Vectorize index (384 dimensions, cosine metric)
3. Implement `/index/batch` — accept chunks, embed via Workers AI, upsert to Vectorize
4. Implement `/search` — embed query, search Vectorize, return ranked results
5. Implement `/projects`, `/projects/:id/status`, `/health`
6. Deploy and validate with manual curl tests
7. **Exit criteria**: Can index a project and search it via curl

### Phase 2: CLI Tool (1 week)
1. Create `cortex` CLI (Node.js, single binary via `pkg` or `esbuild`)
2. Port tree-sitter chunking from V3.0 `chunker.ts` (keep as-is)
3. Implement `cortex init` (generate API key, store config)
4. Implement `cortex index` (chunk → POST /index/batch)
5. Implement `cortex index --watch` (chokidar → incremental POST /index)
6. Implement remaining commands (`status`, `projects`, `delete`, `search`)
7. **Exit criteria**: Can `cortex index` a real project and `cortex search` it

### Phase 3: MCP Client (1 week)
1. Create new thin MCP client (stdio transport, HTTP proxy to Worker)
2. Implement 5 tool handlers (stateless, no caching)
3. Register with `claude mcp add`
4. Test end-to-end: Claude Code → MCP → Worker → Vectorize → response
5. **Exit criteria**: Claude Code can use all 5 tools against cloud backend

### Phase 4: Decommission V3.0 (1 week)
1. Archive V3.0 source to `archive/v3/` branch (not deleted)
2. Remove from `src/`: ProcessPool, local vector store, embedding server, memory-mapped cache, fallback handlers
3. Update `CLAUDE.md` and docs for V4.0
4. Update `package.json` scripts
5. **Exit criteria**: `npm run build` succeeds with only V4.0 code

## 8. Benefits

| Aspect | V3.0 (Current) | V4.0 (Cloud) |
|--------|----------------|---------------|
| Context reduction | ~80-90% | ~90-95% (server-optimized) |
| Index size | Local disk limited | ~13K vectors free, unlimited paid |
| Response latency | Local (fast but resource-heavy) | <200ms (Cloudflare edge) |
| Codebase complexity | ~34K LOC, 97 files | ~3K LOC estimated |
| Memory usage | 200-400MB per worker process | Near zero (cloud-hosted) |
| Setup | `npm run start:centralized` + PID management | `cortex init` once |
| Cross-machine | Not supported | Works everywhere (cloud storage) |

## 9. Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Cloudflare outage | MCP client returns clear "service unavailable" error; no silent degradation |
| Free tier limits exceeded | CLI warns on `cortex status`; paid tier is cheap ($0.04/10K chunks) |
| Embedding model mismatch | V4.0 uses same BGE-small-en-v1.5 as V3.0; can re-index if model changes |
| Latency regression | Cloudflare edge network; benchmark in Phase 1 before proceeding |
| Source code privacy | Chunk content stored in Vectorize metadata — user must accept cloud storage. Add `cortex init` consent prompt |

---

**Next step**: Begin Phase 1 — scaffold Cloudflare Worker project.
