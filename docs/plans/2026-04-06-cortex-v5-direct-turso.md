# Cortex V5.0 — CLI + Turso Direct Architecture

**Date**: 2026-04-06
**Status**: Active
**Supersedes**: `2026-04-01-cortex-v5-skill-cloud-design.md` (Worker-based design)

## 1. Why This Revision

The previous V5.0 design routed everything through a Cloudflare Worker: CLI → Worker → Turso. But the Worker added nothing — it was proxying SQL queries to Turso with API key auth that Turso already provides via its own auth token. The Worker was a pointless middleman adding latency, complexity, and a third cloud dependency.

This revision eliminates the Worker entirely. The CLI connects to Turso directly.

## 2. Architecture

```
LOCAL                          CLOUD
─────                          ─────
Claude Code
  ↓
Skill (loaded into context)    Turso
  ↓ scripts                    (one database, everything:
cortex CLI                      vectors, content, relationships)
  ├── SmartChunker
  ├── @xenova/transformers
  │   BGE-small-en-v1.5
  ├── relationship analyzers
  └── @libsql/client ─────────→ libsql://cortex-v5-xxx.turso.io
```

**One local process. One cloud database. No servers.**

### What Changed from Previous V5.0 Design

| Previous V5.0 | This V5.0 |
|---------------|-----------|
| CLI → Cloudflare Worker → Turso | CLI → Turso direct |
| Worker handles auth, search, CRUD | CLI handles everything |
| 3 cloud dependencies (Worker + Turso + deploy pipeline) | 1 cloud dependency (Turso) |
| API key in `api_keys` table, validated per-request by Worker | Turso auth token in `~/.cortex/config.json` |
| `worker/` directory (~500 LOC) | Eliminated |
| Deploy requires `wrangler deploy` | Nothing to deploy — Turso DB is serverless |

### What Changed from V3.0

| V3.0 | V5.0 |
|------|------|
| MCP server + stdio transport | Skill with scripts |
| ProcessPool (200-400MB per worker) | `@xenova/transformers` in-process (~200MB RSS) |
| Local `.cortex/index.json.gz` | Turso (cloud, accessible anywhere) |
| ~34K LOC, 97 files | Target: ~2K LOC |

## 3. Components

### 3.1 Embedding — `@xenova/transformers`
- Model: BGE-small-en-v1.5, 384 dimensions
- Runtime: ONNX via `@xenova/transformers` v2.17.2
- ~200MB RSS, ~15ms per embedding, deterministic, L2-normalized
- Runs in-process in the CLI (no ProcessPool, no external processes)

### 3.2 Storage — Turso (Everything in One Database)

One Turso database stores:
- **Chunks**: source text, metadata, embeddings (`F32_BLOB(384)`)
- **Vectors**: DiskANN index via `libsql_vector_idx(embedding)`, searched with `vector_top_k()`
- **Relationships**: edges between chunks (calls, imports, data_flow, etc.)
- **FTS5**: keyword search fallback via external content table synced by triggers
- **Projects**: metadata (name, path, last indexed)

Schema highlights:
```sql
CREATE TABLE chunks (
  chunk_id TEXT PRIMARY KEY,    -- project_id:file_path:start_line
  project_id TEXT NOT NULL,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT,
  start_line INTEGER,
  end_line INTEGER,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  language TEXT,
  embedding F32_BLOB(384),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);

-- DiskANN vector index for semantic search
CREATE INDEX idx_chunks_embedding ON chunks(libsql_vector_idx(embedding));

-- FTS5 for keyword search
CREATE VIRTUAL TABLE chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid
);

-- Relationships with CASCADE delete
CREATE TABLE relationships (
  source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  rel_type TEXT NOT NULL,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
```

Key queries:
```sql
-- Semantic search: vector_top_k + JOIN for content in one query
SELECT c.*, v.distance
FROM vector_top_k('idx_chunks_embedding', vector(?), ?) v
JOIN chunks c ON c.rowid = v.id
WHERE c.project_id = ?
ORDER BY v.distance;

-- Keyword search: FTS5
SELECT c.*, rank FROM chunks_fts f
JOIN chunks c ON c.rowid = f.rowid
WHERE f.content MATCH ? AND c.project_id = ?;

-- Relationship traversal: recursive CTE
WITH RECURSIVE graph AS (
  SELECT ... FROM relationships WHERE source_chunk_id IN (?)
  UNION ALL
  SELECT ... FROM relationships r JOIN graph g ON r.source_chunk_id = g.target_chunk_id
  WHERE g.depth < ?
) SELECT DISTINCT * FROM graph;
```

### 3.3 Auth — Turso Auth Token
- Turso provides its own auth: URL + auth token per database
- Stored in `~/.cortex/config.json` (set during `cortex init`)
- No API key management, no key rotation — Turso handles this via `turso db tokens create/revoke`
- Single-user by design

### 3.4 Chunking — Local Heuristic Parser
- `SmartChunker` from V3.0 (regex/brace-counting, not tree-sitter)
- chunk_id format: `${project_id}:${filePath}:${startLine}`
- Unchanged from V3.0 logic

### 3.5 Relationship Analysis — CLI-Side
- `call-graph-analyzer.ts`, `data-flow-analyzer.ts`, `dependency-mapper.ts`
- Run at index time, upload edges to Turso `relationships` table
- Query-time traversal via recursive CTE in Turso

### 3.6 Delivery — Claude Code Skill
- Skill file loaded into Claude's context
- 3 scripts calling `cortex` CLI via absolute path from `~/.cortex/config.json`:
  - `cortex search "query"` — embed locally → vector search in Turso → return chunks
  - `cortex relationships "symbol"` — recursive CTE in Turso
  - `cortex status` — project stats from Turso
- CLI output is JSON (`--format json`)

## 4. CLI Commands

```bash
cortex init                  # create Turso DB, store URL+token in config, consent prompt
cortex index                 # chunk → analyze → embed → upload to Turso
cortex index --watch         # watch + incremental sync
cortex search "query"        # embed locally → vector_top_k in Turso → print results
cortex search "q" --keyword  # FTS5 fallback
cortex relationships "sym"   # recursive CTE traversal
cortex status                # project stats
cortex projects              # list all indexed projects
cortex delete                # remove current project from Turso
cortex config                # view/edit ~/.cortex/config.json
```

## 5. Data Flow

### 5.1 Index
```
cortex index .
  ├── SmartChunker → chunks[]
  ├── call-graph/data-flow analyzers → relationships[]
  ├── @xenova/transformers → embeddings[]
  └── @libsql/client → Turso
      INSERT INTO chunks ... ON CONFLICT DO UPDATE (preserves rowid)
      INSERT INTO relationships ...
```

### 5.2 Search
```
cortex search "query"
  ├── @xenova/transformers → query vector (single embed, ~15ms)
  └── @libsql/client → Turso
      vector_top_k() JOIN chunks → chunks[] with scores
  → print JSON results
```

**Latency**: ~15ms embed + ~30-80ms Turso query = **~50-100ms total**

### 5.3 Relationships
```
cortex relationships "functionName"
  └── @libsql/client → Turso
      recursive CTE on relationships table
  → print JSON graph
```

## 6. Config File

`~/.cortex/config.json`:
```json
{
  "turso_url": "libsql://cortex-v5-xxx.turso.io",
  "turso_auth_token": "eyJ...",
  "binary_path": "/usr/local/bin/cortex",
  "consent_given": true,
  "model": "Xenova/bge-small-en-v1.5",
  "dimensions": 384
}
```

## 7. Migration Plan

## 7A. Current Implementation Status

As of 2026-04-06, the codebase has moved beyond the original "next step: Phase 1" note.

- **Phase 1: CLI Core**
  - Status: implemented
  - Present in `cli/`: standalone CLI entry point, Turso client, schema application, config handling, local embeddings, and indexing flow
- **Phase 2: Search + Relationships**
  - Status: mostly implemented
  - Present in `cli/`: semantic search, keyword search, relationships traversal, project status, project listing, deletion, config display
  - Known caveat: project-scoped vector correctness currently relies on widening the Turso `vector_top_k()` fetch size when multiple projects share the same database
- **Phase 3: Skill**
  - Status: not started in this repository
  - The planned Claude Code skill/scripts are not yet present
- **Phase 4: Decommission V3.0**
  - Status: not started
  - Root MCP/server code and the `worker/` directory are still present

This means the repository is currently hybrid:
- `cli/` is the active V5 implementation
- root `src/` remains the legacy V3 runtime

### Phase 1: CLI Core — 2 weeks
1. Create `cortex` CLI entry point (Node.js)
2. Implement Turso client module (`@libsql/client` direct connection)
3. Implement schema application (statement splitter respecting BEGIN...END triggers)
4. Port `chunker.ts` (update `generateChunkId()` for project_id prefix)
5. Integrate `@xenova/transformers` for in-process embedding
6. Implement `cortex init` (create Turso DB via `turso` CLI, store config, consent prompt)
7. Implement `cortex index` (chunk → analyze → embed → batch upsert to Turso)
8. **Exit criteria**: Can `cortex init && cortex index .` a real project

### Phase 2: Search + Relationships — 1 week
1. Implement `cortex search` (embed query → `vector_top_k()` → print results)
2. Implement `cortex search --keyword` (FTS5 fallback)
3. Implement `cortex relationships` (recursive CTE)
4. Implement `cortex status`, `cortex projects`, `cortex delete`
5. **Exit criteria**: Full CLI round-trip works from terminal

### Phase 3: Skill — 1 week
1. Create Cortex skill file with description and trigger conditions
2. Write 3 skill scripts calling `cortex` CLI
3. Test end-to-end: Claude Code → skill → search → context
4. **Exit criteria**: Claude Code can semantically search a project via skill

### Phase 4: Decommission V3.0 — 1 week
1. Archive V3.0 code to `archive/v3` branch
2. Remove: MCP layer, ProcessPool, local vector store, embedding server
3. Keep and port: chunker, types, analyzers, semantic-watcher
4. Delete `worker/` directory (no longer needed)
5. **Exit criteria**: `npm run build` succeeds with only V5.0 code

## 8. Cost

| Component | Free Tier | Usage | Cost |
|-----------|-----------|-------|------|
| Turso | 9GB, 500M reads/mo | ~5-10MB/project | $0 |
| Local embeddings | N/A | ~200MB RAM | $0 |
| **Total** | | | **$0/mo** |

No Cloudflare account needed. No Worker deployment. No KV. No Qdrant.

## 9. Risk

| Risk | Mitigation |
|------|------------|
| Turso free tier changes | Data exportable (SQLite-compatible). Self-host via `sqld` |
| Source code in cloud | Chunks only (not full files). `cortex init` requires consent |
| Turso latency | ~30-80ms per query. Acceptable for dev tool |
| Auth token leak | Stored in `~/.cortex/config.json` (600 perms). Revoke via `turso db tokens revoke` |
| vector_top_k global scan (no per-project pre-filter) | Over-fetch 5x, filter by project_id post-ANN |

## 10. What Gets Deleted

**Entire `worker/` directory** — no longer needed:
- `worker/src/index.ts`, `turso-client.ts`, `auth.ts`, `types.ts`
- `worker/schema.sql`, `wrangler.toml`, `package.json`

**V3.0 MCP + server layer** (~13K LOC):
- All MCP handlers, servers, transports
- ProcessPool, embedding server, memory-mapped cache
- Local vector store, searcher, indexer

**Kept and ported to CLI**:
- `chunker.ts` + `git-scanner.ts`
- `types.ts` (trimmed)
- `call-graph-analyzer.ts`, `data-flow-analyzer.ts`, `dependency-mapper.ts`
- `semantic-watcher.ts` (for `--watch`)

---

**Open decisions**: 1
- V3→V5 migration: Leaning "no, re-index only"

**Next steps**:
1. Finish Phase 3 by adding the Claude Code skill/scripts for the V5 CLI.
2. Decide whether MCP remains as a compatibility layer or is removed entirely.
3. Start Phase 4 by archiving or removing the legacy V3 MCP/server runtime once V5 is the primary path.
