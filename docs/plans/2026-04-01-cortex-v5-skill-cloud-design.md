# Cortex V5.0 — Skill + Cloud Architecture Design

**Date**: 2026-04-01
**Revised**: 2026-04-06 (Turso-only architecture — eliminated Qdrant, KV; Turso handles vectors via native F32_BLOB/DiskANN)
**Status**: Draft (1 open decision remains)
**Supersedes**: V4.0 (`2026-03-06-cortex-v4-cloud-design.md`)

## 1. Why V5.0

V3.0 works but is too heavy — 34K LOC, local ProcessPool (200-400MB per worker), fragile process management, single-machine only.

V4.0 proposed Cloudflare-only cloud, but review uncovered blockers: embedding cost 1000x higher than estimated, Vectorize metadata too limited for relationships/content, unclear pricing.

V5.0 takes a different approach:
- **Skill instead of MCP** — no transport layer, no handler classes, skill scripts call cloud APIs directly
- **Local embeddings** — zero cost, no cloud embedding vendor lock-in, eliminates the ProcessPool
- **Turso-only cloud storage** — vectors (native F32_BLOB + DiskANN), content, relationships, API keys — all in one database
- **Cloudflare Worker** — authenticated API gateway: validates API keys, enforces rate limits, orchestrates Turso queries (vector search, FTS5, relationship traversal). No chunking or embedding — those run locally in CLI.

## 2. Architecture

```
LOCAL                                         CLOUD
─────                                         ─────
                                         Cloudflare Worker
Claude Code                              (API router + auth)
  ↓                                           │
Skill (loaded into context)                   │
  ↓ scripts                                   │
cortex CLI (absolute path from config)      Turso
  ├── SmartChunker (regex/heuristic)       (vectors F32_BLOB +
  ├── @xenova/transformers (embed)          content + relationships
  │   BGE-small-en-v1.5, 384-dim            + API keys)
  └── relationship analyzers
      (call-graph, data-flow)
      run at INDEX time, results
      uploaded to Turso
```

### What changed from V3.0

| V3.0 | V5.0 |
|------|------|
| MCP server + stdio transport | Skill with scripts |
| 7 MCP tool handlers + fallback modes | Skill scripts calling CLI → cloud API |
| ProcessPool (spawn external processes, 200-400MB each) | `@xenova/transformers` in-process (~200MB RSS) |
| Local `.cortex/index.json.gz` (file-based vectors) | Turso native vector search (F32_BLOB + DiskANN) |
| Local file for content + no relationship DB | Turso (SQLite edge DB — vectors, content, relationships, auth) |
| `cortex-embedding-server.ts` (HTTP server on port 8766) | Cloudflare Worker (serverless) |
| ~34K LOC, 97 files | Target: ~3K LOC |

### Skill vs MCP: What's Lost, What's Gained

| Lost (acceptable) | Gained |
|-------------------|--------|
| JSON Schema-validated tool inputs/outputs | Zero install friction (no `claude mcp add`) |
| Server-push notifications | No process management, no PID files |
| Stateful pagination (`fetch_chunk`/`next_chunk`) | Simpler architecture (~30K LOC removed) |
| Structured error handling per tool | CLI handles errors, skill gets clean output |

## 3. Decided

### 3.1 API Layer — Cloudflare Workers
- Free: 100K requests/day (personal use needs ~100-200/session)
- All cloud storage calls proxied through Worker for auth + orchestration
- Single Worker handles all routes
- **Rate limiting enabled** (Cloudflare built-in, free) — protects against leaked API keys
- **Worker must use WebSocket transport** for Turso (`@libsql/client`) to reuse connections across requests within the same isolate

### 3.2 Vector Storage — Turso Native (F32_BLOB + DiskANN)
- Vectors stored inline in `chunks` table as `embedding F32_BLOB(384)` column
- DiskANN index via `libsql_vector_idx(embedding)` — cosine distance by default
- ANN search via `vector_top_k('idx_chunks_embedding', vector(?), ?)` joined to chunks table
- No separate vector database — eliminates split-brain risk, simplifies deployment
- Over-fetch strategy (5x multiplier) compensates for lack of pre-filtering by project_id in ANN scan

### 3.3 Content + Relationships + Auth — Turso
- Free: 500 databases, 9GB storage, 500M rows read/month
- libSQL (SQLite-compatible), CLI-accessible via `turso db shell`
- Stores chunk source text, relationships, project metadata
- Database provisioned automatically by `cortex init`
- Solves the V4.0 blocker: relationships are SQL queries, not vector metadata hacks

```sql
-- Chunks: source text + metadata
-- IMPORTANT: chunk_id must be globally unique across projects.
-- V3.0 uses `${filePath}:${startLine}` which collides across repos.
-- V5.0 format: `${project_id}:${filePath}:${startLine}`
-- This requires a change to chunker.ts generateChunkId() in Phase 2.
CREATE TABLE chunks (
  chunk_id TEXT PRIMARY KEY,  -- format: project_id:file_path:start_line
  project_id TEXT NOT NULL,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT,        -- function, class, method, config, documentation
  start_line INTEGER,
  end_line INTEGER,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  language TEXT,
  embedding F32_BLOB(384),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX idx_chunks_project ON chunks(project_id);
CREATE INDEX idx_chunks_file ON chunks(project_id, file_path);
CREATE INDEX idx_chunks_language ON chunks(project_id, language);
CREATE INDEX idx_chunks_embedding ON chunks(libsql_vector_idx(embedding));

-- Auto-update updated_at (libSQL has no ON UPDATE default)
CREATE TRIGGER chunks_updated_at
  AFTER UPDATE ON chunks
  FOR EACH ROW
BEGIN
  UPDATE chunks SET updated_at = datetime('now') WHERE chunk_id = NEW.chunk_id;
END;

-- Relationships: edges between chunks
CREATE TABLE relationships (
  source_chunk_id TEXT NOT NULL,
  target_chunk_id TEXT NOT NULL,
  rel_type TEXT NOT NULL,  -- calls, called_by, imports, exports, data_flow
  FOREIGN KEY (source_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  FOREIGN KEY (target_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX idx_rel_target ON relationships(target_chunk_id);
CREATE INDEX idx_rel_type_source ON relationships(rel_type, source_chunk_id);
CREATE INDEX idx_rel_type_target ON relationships(rel_type, target_chunk_id);

-- Projects: metadata (chunk_count computed on demand, not denormalized)
CREATE TABLE projects (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT,
  last_indexed TEXT,
  created_at TEXT DEFAULT (datetime('now'))
);

-- FTS5 for keyword search fallback when vector search misses
-- External content table: requires manual sync triggers (SQLite does not auto-sync)
-- See: https://sqlite.org/fts5.html#external_content_tables
CREATE VIRTUAL TABLE chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid
);

-- FTS5 sync triggers (required for external content tables)
CREATE TRIGGER chunks_fts_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;

CREATE TRIGGER chunks_fts_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES ('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
END;

CREATE TRIGGER chunks_fts_update AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES ('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;
```

### 3.4 Embedding Model — `@xenova/transformers` (Resolved)

Replacing the ProcessPool with in-process embedding via Hugging Face Transformers.js:

- **Model**: BGE-small-en-v1.5 (same as V3.0 — existing indexes compatible if migrating)
- **Runtime**: ONNX Runtime via `@xenova/transformers` npm package
- **Memory**: ~200MB RSS (vs 200-400MB per ProcessPool worker)
- **No external dependency**: no Ollama daemon, no TVM compilation
- **Single process**: runs in the same Node.js process as the CLI

Why not the alternatives:
- **TVM**: Over-engineered for personal tool. Per-platform compilation is maintenance burden.
- **Ollama**: Adds mandatory 500MB daemon dependency. Dimension mismatch — `ollama-embedder.ts` uses `nomic-embed-text` at 768-dim, not 384-dim BGE-small.
- **fastembed (current)**: The library itself is fine — the problem was the ProcessPool wrapper. But `@xenova/transformers` is more actively maintained and lighter.

### 3.5 Skill Scripts — CLI Wrapper (Resolved)

**Decision**: Option B — Skill scripts call `cortex` CLI, not raw API.

The CLI handles auth, local embedding, API calls. The skill is a thin wrapper.

**Constraints**:
- Skill resolves `cortex` binary via **absolute path from `~/.cortex/config.json`**, not `$PATH` (Claude Code's execution environment may differ from user's shell)
- Skill exposes exactly 3 scripts (minimal surface area — no destructive commands):
  - `cortex search "query"` — embed query + vector search + content retrieval
  - `cortex relationships "symbol"` — relationship graph traversal
  - `cortex status` — project stats (used by Claude to decide whether to search)
- CLI output is JSON (`--format json`) for reliable parsing

### 3.6 Response Sizing (Resolved)

**Decision**: Worker enforces hard chunk limit; CLI supports offset/limit for pagination.

- Worker returns top N chunks (default 15, configurable via `top_k` param)
- Response includes `total_matches` field so skill knows if more exist
- CLI supports `--offset` and `--limit` params for cursor-style pagination
- No server-side session state required (stateless Worker compatible)
- Skill includes "X more results available" hint for Claude to request next page

### 3.7 Delivery — Claude Code Skill (not MCP)
- Skill file loaded into Claude's context, with scripts calling CLI
- No MCP transport, no stdio, no handler classes
- Skill teaches Claude when/how to use Cortex searches

### 3.8 Chunking — Local heuristic parser (unchanged from V3.0)
- `chunker.ts` uses regex/brace-counting heuristics (`SmartChunker` class), **not tree-sitter AST parsing** despite `tree-sitter` being a package.json dependency. The chunker detects functions, classes, and methods via line-by-line pattern matching.
- `chunker.ts` stays as-is for V5.0 — proven, fast, no cloud dependency. A tree-sitter rewrite is a potential future improvement but out of V5.0 scope.
- Note: `chunker.ts` imports `FileChange` from `git-scanner.ts` — must trace and keep this dependency
- **chunk_id format change required**: V3.0 generates `${filePath}:${startLine}` (collides across projects). V5.0 must change `generateChunkId()` to `${project_id}:${filePath}:${startLine}`. This is a small but breaking change in Phase 2.
- Source code chunks (function/class bodies) are uploaded to Turso — see privacy note in Section 3.9

### 3.9 Auth — API Key in Turso
- Generated on first `cortex init`, stored locally in `~/.cortex/config.json`
- Sent as `Authorization: Bearer <key>` to Worker
- **Server-side key store**: `api_keys` table in Turso. Worker queries `SELECT 1 FROM api_keys WHERE api_key = ? AND active = 1` per request.
- `cortex init` uploads the generated key via Worker's `/admin/register-key` route (one-time bootstrap, uses a setup token from Wrangler secrets)
- **Key rotation**: `cortex config --rotate-key` generates new key locally, calls `/admin/rotate-key` to revoke old + register new in Turso
- Single-user, no multi-tenant
- `cortex init` includes **explicit consent prompt**: "Cortex will store source code chunks (function bodies, class definitions) in Turso cloud database. Continue? [y/N]" (non-skippable, stored as boolean in config)

### 3.10 Relationship Analysis — CLI-Side (Resolved)

**Important architectural split**: Relationship analysis has two phases:

1. **Graph building** (index time, runs in CLI): `call-graph-analyzer.ts`, `data-flow-analyzer.ts`, `dependency-mapper.ts` (~2.5K LOC) analyze source code and produce relationship edges. These run locally during `cortex index` and upload results to Turso.

2. **Graph traversal** (query time, runs in Worker): Recursive CTE queries on the `relationships` table in Turso. The Worker handles this, not the CLI.

This means the CLI is not a thin wrapper — it carries the analysis logic. This drives the CLI's dependency footprint (chunker + analyzers + transformers.js).

## 4. Undecided

### 4.1 V3.0 → V5.0 Index Migration

Should users be able to migrate their existing `.cortex/index.json.gz` to cloud?

**Leaning toward: No, re-index only.**
- Re-indexing 1K chunks with in-process Transformers.js: ~30-90 seconds
- Re-indexing 10K chunks: ~5-15 minutes (acceptable with progress bar)
- Migration code is one-time throwaway that adds test surface
- V3.0 index has fields (`git_metadata`, `usage_patterns`) that V5.0 schema drops — requires mapping logic
- If we keep BGE-small-en-v1.5 (we are), vectors are compatible in theory, but extraction from `.cortex/index.json.gz` is messy

**Decision needed before**: Phase 2 (CLI) — if yes, implement `cortex migrate`; if no, just make `cortex index` fast.

## 5. Data Flow

### 5.1 Index (CLI-driven, local embed + analyze)
```
cortex index .
  ├── SmartChunker (regex/heuristic) → chunks[]
  ├── call-graph/data-flow analyzers → relationships[]
  ├── @xenova/transformers → vectors[]
  └── POST /index/batch
        → Worker
           └── Turso.upsert(chunks content + embeddings + relationships)
               (ON CONFLICT DO UPDATE to preserve rowid + FTS integrity)
```

### 5.2 Search (Skill-driven)
```
Claude Code → Skill script → cortex search "query"
  ├── @xenova/transformers → query vector
  └── POST /search {vector, project_id, top_k, offset}
        → Worker
           └── Turso: vector_top_k() JOIN chunks (single query, vectors + content)
        → response {chunks[], total_matches, has_more}
  → Skill formats result into Claude's context
```

**Latency budget** (warm Worker):
- Receive POST: ~5ms
- Turso vector_top_k + JOIN: ~30-80ms (single query returns vectors + content)
- Assemble + return: ~5ms
- **Total: ~40-90ms** (target <500ms)

**Cold start** (first request after idle): add ~200-400ms for TCP+TLS to Turso. Expected on spaced-out sessions.

### 5.3 Relationships (Skill-driven)
```
Claude Code → Skill script → cortex relationships "functionName"
  └── POST /relationships {symbol, project_id, depth, rel_types}
        → Worker
           └── Turso: recursive CTE on relationships table
               (uses idx_rel_type_source/target composite indexes)
        → response (relationship graph)
```

### 5.4 File-Level Deletion (on file rename/delete)
```
cortex index --watch
  detects file deleted/renamed
  └── POST /delete-by-file {project_id, file_path}
        → Worker:
           └── Turso.delete(WHERE file_path = ?) — CASCADE deletes relationships,
               DiskANN index auto-updated, FTS5 triggers fire on delete
```

### 5.5 Keyword Search Fallback
```
cortex search "functionName" --keyword
  └── POST /search/keyword {query, project_id}
        → Worker
           └── Turso: FTS5 query on chunks_fts
        → response (keyword-matched chunks)
```

## 6. Worker API Routes

```
POST   /index/batch          — bulk upsert chunks + embeddings (client-side batch ≤100 chunks)
POST   /index/delete-file    — remove all chunks for a file (CASCADE handles relationships)
POST   /search               — vector similarity via Turso vector_top_k() + JOIN
POST   /search/keyword       — FTS5 keyword search fallback
POST   /relationships        — recursive CTE traversal on Turso
GET    /projects              — list projects (chunk_count computed via COUNT(*), not cached)
GET    /projects/:id/status   — project stats from Turso
DELETE /projects/:id          — remove entire project from Turso
GET    /health                — ping Turso
POST   /admin/init-schema     — apply schema (setup token required)
POST   /admin/register-key    — register API key (setup token required)
POST   /admin/rotate-key      — rotate API key (setup token required)
```

All routes require `Authorization: Bearer <api-key>` (validated against `api_keys` table in Turso).
Rate limiting: Cloudflare built-in, per-location (sufficient for personal use, not globally consistent).
Worker validates `chunk_id` format (`project_id:file_path:start_line`) before querying.

**CPU time budget**: Cloudflare Workers free tier has 10ms CPU limit per invocation. Paid Workers Standard allows up to 30 seconds CPU per invocation (not 50ms as previously stated). `/index/batch` must be ≤100 chunks per request on free tier. CLI batches larger projects into multiple requests. Note: Cloudflare rate limiting is per-location, not globally consistent — sufficient for personal use but not a strong abuse prevention mechanism.

## 7. CLI Commands

```bash
cortex init                  # generate API key, provision Turso DB,
                             # consent prompt for cloud storage, store config + binary path
cortex index                 # index current directory → chunk → analyze → embed → upload
cortex index --watch         # watch + incremental sync (chokidar, debounced)
cortex search "query"        # embed query locally → search cloud → print results
cortex search "q" --keyword  # FTS5 keyword fallback
cortex search "q" --offset N # pagination
cortex relationships "sym"   # query relationship graph from Turso
cortex status                # project stats from cloud
cortex projects              # list all indexed projects
cortex delete                # remove current project from cloud
cortex config                # view/edit ~/.cortex/config.json
cortex config --rotate-key   # generate new API key
```

## 8. Migration Plan

### Phase 0: Embedding Model Validation (1-2 days)
1. Install `@xenova/transformers`, load BGE-small-en-v1.5
2. Embed 1K chunks from V3.0's test corpus
3. Run 10-20 representative search queries, compare top-5 results with V3.0
4. Measure: time, memory, result quality
5. **Exit criteria**: Result quality matches V3.0; embedding time < 100ms per chunk

### Phase 1: Cloud Backend (Worker + Turso) — 2 weeks
1. Scaffold Cloudflare Worker project with Wrangler
2. Set up Turso database, apply schema (chunks with F32_BLOB embedding, DiskANN index, FTS5, triggers, relationships, api_keys)
3. Implement Worker routes: `/index/batch`, `/search`, `/search/keyword`, `/relationships`, `/health`, admin routes
4. Implement Turso-based auth middleware (api_keys table lookup per request)
5. Use WebSocket transport for Turso (`@libsql/client`) in Worker
6. Test with curl: index a small project, search it, query relationships
7. **Exit criteria**: Can round-trip index→search→relationships via curl with auth

### Phase 2: CLI — 2 weeks
1. Create `cortex` CLI (Node.js, single entry point)
2. Port `chunker.ts` (heuristic chunking — trace `git-scanner.ts` dependency, change `generateChunkId()` to include project_id prefix)
3. Port relationship analyzers (`call-graph-analyzer.ts`, `data-flow-analyzer.ts`, `dependency-mapper.ts`) — run at index time, upload to Turso
4. Integrate `@xenova/transformers` for in-process embedding
5. Implement `cortex init` (generate API key, register in Turso, consent prompt, store absolute binary path in config)
6. Implement `cortex index` (chunk → analyze → embed → batch POST, ≤100 chunks per request)
7. Implement `cortex index --watch` (port `semantic-watcher.ts` debouncing + git-tracked/untracked logic)
8. Implement remaining commands (`search`, `relationships`, `status`, `projects`, `delete`, `config --rotate-key`)
9. **Exit criteria**: Can `cortex index .` a real project and `cortex search` from terminal

### Phase 3: Skill — 1 week
1. Create Cortex skill file with description and trigger conditions
2. Write 3 skill scripts calling `cortex` CLI via absolute path from config
3. Ensure CLI output is JSON (`--format json`) for reliable parsing by skill
4. Package as Claude Code plugin (skill + CLI)
5. Test end-to-end: Claude Code loads skill → runs search → gets code context
6. **Exit criteria**: Claude Code can semantically search a cloud-indexed project via skill

### Phase 4: Decommission V3.0 — 1 week
1. Archive V3.0 code to `archive/v3` branch
2. Remove from `src/`: ProcessPool, local vector store, embedding server, MCP handlers, all transport code
3. Keep and port:
   - `chunker.ts` + `git-scanner.ts` — heuristic chunking (generateChunkId updated for project_id prefix)
   - `types.ts` — core types (trimmed)
   - `call-graph-analyzer.ts`, `data-flow-analyzer.ts`, `dependency-mapper.ts` — relationship analysis (runs in CLI at index time)
   - `semantic-watcher.ts` — file watching (ported for `cortex index --watch`)
4. Remove `searcher.ts`, `indexer.ts` (replaced by CLI→Worker→Turso flow)
5. Update CLAUDE.md, package.json, docs
6. **Exit criteria**: `npm run build` succeeds with only V5.0 code

## 9. Cost

| Component | Free Tier | Personal Usage Estimate | Cost |
|-----------|-----------|------------------------|------|
| Cloudflare Workers | 100K req/day | ~200/session | $0 |
| Turso | 9GB, 500M reads/mo | ~5-10MB/project (content + vectors + relationships) | $0 |
| Local embeddings (@xenova/transformers) | N/A | ~200MB RSS RAM | $0 |
| **Total** | | | **$0/mo** |

**Capacity at free tier**: ~50 projects at 2K chunks each = 100K chunks. Turso stores vectors inline (384-dim F32 = 1.5KB/vector), so 100K chunks ≈ 150MB vectors + ~200MB content. Well within 9GB limit.

**Turso row reads**: 50 searches/day x 15 rows = 750 reads/day. Budget = 16.7M reads/day. Negligible.

**Paid tier triggers**: >9GB content+vectors (Turso) or >100K API calls/day (Workers). Neither realistic for personal use.

## 10. Risk

| Risk | Mitigation |
|------|------------|
| Turso free tier changes | Data exportable (SQLite-compatible). Can self-host via open-source `sqld` server. Vectors are reproducible — re-embed if needed |
| Embedding model mismatch (CLI vs query) | Config stores model name + dimension. CLI refuses to search if mismatch detected |
| Source code chunks in cloud | Chunk content (function bodies, class definitions) is stored verbatim in Turso — these are raw source excerpts for compliance purposes. Full files are never uploaded, but chunks are real code. `cortex init` requires explicit consent. Turso is shared infra — orgs prohibiting cloud source code need self-hosted `sqld` |
| Latency (local → Workers → Turso) | Target <500ms warm, <800ms cold. Single-query vector+content retrieval via vector_top_k JOIN |
| Worker cold starts | First request ~400-600ms (TCP+TLS to Turso). Subsequent requests fast |
| API key leak | Rate limiting on Worker. `cortex config --rotate-key` for rotation. Key not in env/source, only in `~/.cortex/config.json` |
| `cortex` binary not on PATH in Claude Code | Skill uses absolute path from `~/.cortex/config.json`, set during `cortex init` |
| Relationship CTE explosion on hub nodes | Limit depth to 3, add per-query result cap. Consider adding `strength` column to relationships for filtering |

## 11. What V5.0 Deletes from V3.0

**Entire MCP layer** (~8K LOC):
- `cortex-stdio-mcp.js`, `mcp-tools.ts`, `mcp-handlers.ts`
- `lightweight-handlers.ts`, `lightweight-mcp-server.ts`
- `server.ts`, `stdio-server.ts`, `simple-stdio-server.ts`
- `cortex-mcp-client.ts`, `embedding-client.ts`

**Local embedding infrastructure** (~5K LOC):
- `process-pool-embedder.ts` (3.3K LOC), `external-embedding-process.js`
- `cortex-embedding-server.ts`, `start-centralized-server.ts`
- `memory-mapped-cache.ts`, `shared-embedding-cache.ts`
- `embedding-strategy.ts`, `cached-embedder.ts`

**Local storage** (~3K LOC):
- `persistent-vector-store.ts`, `unified-storage-coordinator.ts`
- `persistent-relationship-store.ts`
- `searcher.ts`, `indexer.ts` (replaced by CLI→cloud flow)

**Kept and ported to CLI**:
- `chunker.ts` + `git-scanner.ts` — heuristic chunking (unchanged logic, but `generateChunkId()` must add project_id prefix)
- `types.ts` — core types (trimmed)
- `call-graph-analyzer.ts`, `data-flow-analyzer.ts`, `dependency-mapper.ts` — run at index time
- `semantic-watcher.ts` — file watching for `--watch` mode

---

**Open decisions**: 1 remaining
1. ~~Embedding model~~ → `@xenova/transformers` with BGE-small-en-v1.5
2. ~~Skill script design~~ → CLI wrapper with absolute path
3. V3→V5 migration → Leaning "no, re-index only" (decide before Phase 2)
4. ~~Response sizing~~ → Worker chunk limit + CLI offset/limit pagination

**Next step**: Phase 0 — validate `@xenova/transformers` embedding quality against V3.0
