# Cloudflare Vectorize Migration Plan

## Overview

Migrate cortexyoung's vector storage from local file-based storage (`index.json`, `embedding-cache.json.gz`) to Cloudflare Vectorize, while keeping all local pipeline work (file watching, chunking, change detection, git scanning) unchanged.

---

## 1. Architecture Summary

### Current State

| Component | Location | Role |
|---|---|---|
| `VectorStore` | `src/vector-store.ts` | In-memory Map\<chunk_id, CodeChunk\> |
| `PersistentVectorStore` | `src/persistent-vector-store.ts` | Extends VectorStore, writes to `index.json[.gz]` |
| `UnifiedStorageCoordinator` | `src/unified-storage-coordinator.ts` | Manages both local `.cortex/` and global `~/.claude/cortex-embeddings/<repoHash>/` |
| `CachedEmbedder` | `src/cached-embedder.ts` | Maintains `embedding-cache.json` keyed by `content_hash` |
| `CloudflareAIEmbedder` | `src/cloudflare-ai-embedder.ts` | Posts to worker at `https://cortex-embedder.yanggf.workers.dev` |
| `cloudflare-worker.js` | root | Single embed endpoint only |

### Target State

| Component | Location | Role |
|---|---|---|
| `CloudflareVectorStore` | `src/cloudflare-vector-store.ts` (new) | Implements VectorStore interface, calls worker REST endpoints |
| Updated worker | `cloudflare-worker.js` | Adds `/upsert`, `/search`, `/delete`, `/deleteByFile`, `/embedAndUpsert` endpoints |
| `wrangler.toml` | root | Adds `[[vectorize]]` binding |
| `UnifiedStorageCoordinator` | `src/unified-storage-coordinator.ts` | Updated to instantiate `CloudflareVectorStore` |
| Local files | eliminated | `index.json`, `embedding-cache.json.gz` removed |

**What stays local:** file watching, chunking, code parsing, change detection, git scanning

**What moves to Cloudflare:** vector storage, similarity search, metadata index

**What disappears:** `index.json`, `embedding-cache.json.gz`, `~/.claude/cortex-embeddings/`

---

## 2. Metadata Schema

Every vector stored in Vectorize carries this metadata namespace. Vectorize metadata values must be strings, numbers, or booleans.

```
VectorizeMetadata {
  repoHash:      string   // e.g. "cortexyoung-3a7f9b2c1e4d8f6a"  (StoragePaths.getRepositoryHash())
  chunkId:       string   // CodeChunk.chunk_id
  filePath:      string   // CodeChunk.file_path  (relative)
  contentHash:   string   // CodeChunk.content_hash  (for delta detection without loading vectors)
  chunkType:     string   // CodeChunk.chunk_type: 'function'|'class'|'method'|'documentation'|'config'
  language:      string   // CodeChunk.language_metadata.language
  symbolName:    string   // CodeChunk.symbol_name ?? ''
  startLine:     number   // CodeChunk.start_line
  endLine:       number   // CodeChunk.end_line
  lastModified:  string   // CodeChunk.last_modified  (ISO string)
  gitCommit:     string   // CodeChunk.git_metadata.last_modified_commit ?? ''
}
```

**Filtering pattern:**
`repoHash == "cortexyoung-3a7f9b2c1e4d8f6a"`

### Local chunk-metadata store

Full `CodeChunk` content (relationships, full source text, etc.) cannot fit in Vectorize metadata (32-field limit, 1024-byte-per-value limit). Solution: keep a lightweight local file `chunk-metadata.json` that maps `chunkId -> full CodeChunk minus embedding field`. This file replaces the bloated `index.json` — no float arrays means ~5-20 KB vs the current ~3-8 MB.

```
ChunkMetadataStore {
  version: string
  repoHash: string
  chunks: Record<chunkId, CodeChunk>   // WITHOUT embedding field
  fileHashes: Record<filePath, contentHash>
  metadata: { totalChunks, lastIndexed, embeddingModel, gitCommitHash, gitBranchName }
}
```

---

## 3. Step 1 — Vectorize Index Setup

### wrangler.toml changes

```toml
name = "cortex-embedder"
main = "cloudflare-worker.js"
compatibility_date = "2023-12-01"

[ai]
binding = "AI"

[[vectorize]]
binding = "VECTORIZE"
index_name = "cortex-vectors"
```

One shared index for all projects. Namespace isolation via `repoHash` metadata filter at query time.

### Create the index

```bash
# BGE-small-en-v1.5 produces 384-dimensional L2-normalized embeddings
npx wrangler vectorize create cortex-vectors \
  --dimensions=384 \
  --metric=cosine

# Deploy the updated worker with VECTORIZE binding
npx wrangler deploy
```

---

## 4. Step 2 — Worker Changes (`cloudflare-worker.js`)

Add routing layer and five new endpoints. Existing embed behaviour is unchanged.

### New endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/` or `/embed` | Existing embed (unchanged) |
| POST | `/upsert` | Upsert vectors to Vectorize |
| POST | `/search` | Similarity search with repoHash filter |
| POST | `/delete` | Delete by chunk IDs |
| POST | `/deleteByFile` | Delete file's chunks by IDs (semantic alias for /delete) |
| POST | `/embedAndUpsert` | Embed texts + upsert in one round-trip |

### `/embedAndUpsert` (key optimization)

```javascript
// Request: { texts: string[], metadatas: VectorizeMetadata[], ids: string[] }
async function handleEmbedAndUpsert(request, env) {
  const { texts, metadatas, ids } = await request.json();
  const response = await env.AI.run('@cf/baai/bge-small-en-v1.5', { text: texts });
  const vectors = response.data.map((values, i) => ({
    id: ids[i], values, metadata: metadatas[i]
  }));
  await env.VECTORIZE.upsert(vectors);
  return new Response(JSON.stringify({ upserted: vectors.length }), {
    headers: { 'Content-Type': 'application/json' }
  });
}
```

### `/search`

```javascript
// Request: { queryVector: number[], repoHash: string, limit?: number }
async function handleSearch(request, env) {
  const { queryVector, repoHash, limit = 20 } = await request.json();
  const results = await env.VECTORIZE.query(queryVector, {
    topK: limit,
    filter: { repoHash },
    returnMetadata: 'all',
    returnValues: false,
  });
  return new Response(JSON.stringify({ matches: results.matches }), {
    headers: { 'Content-Type': 'application/json' }
  });
}
```

### `/upsert`

```javascript
// Request: { vectors: Array<{ id, values, metadata }> }
// Batches at 1000 (Vectorize limit)
async function handleUpsert(request, env) {
  const { vectors } = await request.json();
  const BATCH = 1000;
  for (let i = 0; i < vectors.length; i += BATCH) {
    await env.VECTORIZE.upsert(vectors.slice(i, i + BATCH));
  }
  return new Response(JSON.stringify({ upserted: vectors.length }), {
    headers: { 'Content-Type': 'application/json' }
  });
}
```

### `/delete`

```javascript
// Request: { ids: string[] }
async function handleDelete(request, env) {
  const { ids } = await request.json();
  await env.VECTORIZE.deleteByIds(ids);
  return new Response(JSON.stringify({ deleted: ids.length }), {
    headers: { 'Content-Type': 'application/json' }
  });
}
```

---

## 5. Step 3 — New `CloudflareVectorStore` (`src/cloudflare-vector-store.ts`)

Replaces `PersistentVectorStore` as the concrete storage implementation. Extends `VectorStore` to satisfy existing constructor signatures, overrides all I/O methods.

### Public API (mirrors PersistentVectorStore exactly)

```typescript
export class CloudflareVectorStore extends VectorStore {
  constructor(repositoryPath: string, indexDir?: string)

  async initialize(): Promise<void>
  async upsertChunks(chunks: CodeChunk[]): Promise<void>      // embed + upsert via /embedAndUpsert
  async similaritySearch(queryEmbedding: number[], limit?: number): Promise<CodeChunk[]>
  async getChunk(chunkId: string): Promise<CodeChunk | undefined>   // local metadataStore
  async deleteChunk(chunkId: string): Promise<void>
  async clear(): Promise<void>
  async getStats(): Promise<{ total_chunks, totalFiles, indexSize, lastUpdated }>
  async findByFilePath(filePath: string): Promise<CodeChunk[]>      // local metadataStore
  async removeChunksForFile(filePath: string): Promise<void>        // delete from Vectorize + local
  async calculateFileDelta(...): Promise<IndexDelta>                // local metadataStore
  async applyDelta(delta: IndexDelta): Promise<void>
  setFileHash(filePath: string, contentHash: string): void
  async savePersistedIndex(modelInfo?: ModelInfo): Promise<void>    // saves chunk-metadata.json
  async hasValidIndex(): Promise<boolean>
  async clearIndex(): Promise<void>
  getAllChunks(): CodeChunk[]
  getChunkCount(): number
  getChunksByFile(filePath: string): CodeChunk[]
  getChunksForFile(filePath: string): CodeChunk[]
  compareChunks(oldChunks, newChunks): { toAdd, toKeep, toRemove }
}
```

### Key implementation notes

1. **`similaritySearch`**: Receives pre-computed `queryEmbedding` from `SemanticSearcher`. POST to `/search` with `repoHash` filter. Hydrate full `CodeChunk` from local `metadataStore` using `match.id`. Set `relevance_score = match.score`.

2. **`upsertChunks`**: Batch into groups of 100 (CF AI limit). Call `/embedAndUpsert` with texts + metadatas + ids. After success, strip `embedding` field from chunks and save to local `metadataStore`.

3. **Fast local methods** (`getChunk`, `getAllChunks`, `findByFilePath`, `getChunksByFile`): work entirely from `metadataStore` — no network call. These are called frequently by `indexer.ts` for delta calculation.

4. **`removeChunksForFile`**: Fetch chunk IDs from `metadataStore.getByFile(filePath)`, then call `/delete` on Vectorize, then remove from `metadataStore`.

### `LocalChunkMetadataStore` (inner class)

Manages `chunk-metadata.json[.gz]` in `.cortex/` and `~/.claude/cortex-embeddings/<repoHash>/`. Uses `CompressionUtils` from `storage-constants.ts`.

```typescript
class LocalChunkMetadataStore {
  private chunks: Map<string, CodeChunk>    // WITHOUT embedding field
  private fileHashes: Map<string, string>

  async load(): Promise<boolean>
  async save(modelInfo?: ModelInfo): Promise<void>
  get(chunkId: string): CodeChunk | undefined
  set(chunk: CodeChunk): void
  delete(chunkId: string): void
  getByFile(filePath: string): CodeChunk[]
  getAll(): CodeChunk[]
  setFileHash(path: string, hash: string): void
  getFileHash(path: string): string | undefined
  get size(): number
}
```

---

## 6. Step 4 — Integration Points

### `src/unified-storage-coordinator.ts`

Switch on `CORTEX_VECTOR_BACKEND` env var:

```typescript
const backend = process.env.CORTEX_VECTOR_BACKEND || 'local';
if (backend === 'cloudflare') {
  this.vectorStore = new CloudflareVectorStore(repositoryPath, indexDir);
} else {
  this.vectorStore = new PersistentVectorStore(repositoryPath, indexDir);
}
```

### `src/indexer.ts`

Two `instanceof PersistentVectorStore` guards (lines ~258, ~380) gate `setFileHash()` calls. Replace with a duck-type check: `if ('setFileHash' in this.vectorStore)`.

### `src/searcher.ts`

No changes. `SemanticSearcher` takes `VectorStore` — `CloudflareVectorStore extends VectorStore`.

### `src/embedding-strategy.ts`

No changes. When `CORTEX_VECTOR_BACKEND=cloudflare`, `CloudflareVectorStore.upsertChunks` detects non-empty `embedding` fields and upserts directly to Vectorize without re-embedding.

### `src/cached-embedder.ts`

`embedding-cache.json` is unnecessary in Cloudflare mode. Skip cache initialization when `CORTEX_VECTOR_BACKEND=cloudflare`.

### `src/storage-constants.ts`

```typescript
export const STORAGE_FILENAMES = {
  INDEX: 'index.json',                      // legacy
  CHUNK_METADATA: 'chunk-metadata.json',    // new: chunks without embeddings
  RELATIONSHIPS: 'relationships.json',
  DELTAS: 'deltas',
  EMBEDDING_CACHE: 'embedding-cache.json'
} as const;
```

### `src/env-config.ts`

```typescript
// Add to CortexConfig:
vectorBackend: 'local' | 'cloudflare';   // default 'local'
workerUrl: string;                        // default 'https://cortex-embedder.yanggf.workers.dev'
```

---

## 7. Step 5 — Migration Path

### Option A: Full reindex (simplest)

```bash
CORTEX_VECTOR_BACKEND=cloudflare CORTEX_INDEX_MODE=reindex node cortex-mcp.js
```

Re-chunks and re-embeds all source files. Clean slate in Vectorize.

### Option B: Migrate existing embeddings (no re-embedding)

Script at `scripts/migrate-to-vectorize.ts`:

```
For each project in ~/.claude/cortex-embeddings/:
  1. Load existing index.json.gz -> PersistedIndex
  2. For each CodeChunk:
     a. Build VectorizeMetadata from chunk fields + repoHash
     b. Collect { id: chunk.chunk_id, values: chunk.embedding, metadata }
  3. POST /upsert in batches of 1000
  4. Write chunk-metadata.json (chunks without embeddings)
  5. Log progress
```

Preserves existing embeddings — useful for large projects.

### Post-migration cleanup

```bash
rm ~/.claude/cortex-embeddings/*/index.json.gz
rm ~/.claude/cortex-embeddings/*/embedding-cache.json.gz
rm -rf <project>/.cortex/index.json.gz
```

---

## 8. Worker Authentication

The worker is currently open. With Vectorize storing all project vectors, add a shared secret:

- Worker checks `Authorization: Bearer <token>` on all endpoints
- Set via: `npx wrangler secret put WORKER_AUTH_TOKEN`
- Client reads from: `CORTEX_WORKER_AUTH_TOKEN` env var

---

## 9. Error Handling

`CloudflareVectorStore` implements circuit-breaker matching `CloudflareAIEmbedder`:

- Vectorize failures: retry up to 3× with exponential backoff, then log warning and return empty results
- `CORTEX_VECTOR_BACKEND_FALLBACK=local`: silently fall back to `PersistentVectorStore` if Cloudflare calls fail

---

## 10. Implementation Sequence

| # | Task | File |
|---|------|------|
| 1 | Add `[[vectorize]]` binding | `wrangler.toml` |
| 2 | Create Vectorize index | CLI: `wrangler vectorize create` |
| 3 | Add routing + new endpoints | `cloudflare-worker.js` |
| 4 | Deploy and verify | `wrangler deploy` + curl tests |
| 5 | Add `CHUNK_METADATA` constant | `src/storage-constants.ts` |
| 6 | Add `vectorBackend`, `workerUrl` | `src/env-config.ts` |
| 7 | Implement `CloudflareVectorStore` | `src/cloudflare-vector-store.ts` (new) |
| 8 | Add backend switching | `src/unified-storage-coordinator.ts` |
| 9 | Fix `instanceof` guards | `src/indexer.ts` |
| 10 | Integration test on small repo | — |
| 11 | Write + run migration script | `scripts/migrate-to-vectorize.ts` |
| 12 | Delete old `index.json.gz` files | — |

---

## 11. Vectorize Limits Reference

| Limit | Value |
|---|---|
| Max vectors per index | 5,000,000 |
| BGE-small dimensions | 384 |
| Max vectors per upsert call | 1,000 |
| Max metadata fields per vector | 32 |
| Max metadata value length | 1,024 bytes |
| Query topK max | 20 (free) / 100 (paid) |
| Metadata filter operators | `$eq`, `$ne`, `$in`, `$lt`, `$lte`, `$gt`, `$gte` |

This plan uses 11 metadata fields — well within limits.

---

## Critical Files

| File | Change |
|------|--------|
| `cloudflare-worker.js` | Add routing + upsert/search/delete/embedAndUpsert handlers |
| `wrangler.toml` | Add `[[vectorize]]` binding |
| `src/cloudflare-vector-store.ts` | New file — core implementation |
| `src/unified-storage-coordinator.ts` | Backend switching on `CORTEX_VECTOR_BACKEND` |
| `src/indexer.ts` | Replace `instanceof PersistentVectorStore` with duck-type check |
| `src/storage-constants.ts` | Add `CHUNK_METADATA` filename |
| `src/env-config.ts` | Add `vectorBackend`, `workerUrl` config fields |
| `src/cached-embedder.ts` | Skip cache init when Cloudflare backend active |
