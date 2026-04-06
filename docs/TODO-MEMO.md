# Cortex V3.0 Technical Debt Memo

**Date**: 2026-04-01
**Scope**: All known TODOs extracted from source code. Most are non-blocking diagnostic/monitoring gaps, not core functionality issues. Categorized by priority for V4.0 migration planning.

---

## Will Be Resolved by V4.0 Migration

These items exist in code that will be replaced or removed during V4.0 cloud migration. No action needed — they go away with the V3.0 decommission.

### 1. Placeholder `/semantic-search` Route
- **File**: `cortex-embedding-server.ts:262`
- **Issue**: The `POST /semantic-search` route returns a dummy response. Real semantic search is handled by `centralized-handlers.ts`.
- **Why it exists**: Route was scaffolded early, never wired to ProcessPool. The `/mcp/semantic_search` route (via centralized handlers) is the real implementation.
- **V4.0**: Entire embedding server is replaced by Cloudflare Worker.

### 2. Hardcoded ProcessPool Status Values
- **File**: `cortex-embedding-server.ts:450-459`
- **Issue**: `activeProcesses: 1` (mock), `activeBatches: []` (mock), `cpuUsage: 0` (not monitored).
- **Why it exists**: Status endpoint was built before ProcessPool exposed these metrics. ProcessPool works correctly — only the status reporting is incomplete.
- **V4.0**: No local ProcessPool in V4.0. Cloud Worker has its own health endpoint.

### 3. Last Indexed Time Not Tracked
- **File**: `indexer.ts:565`
- **Issue**: `getIndexStats()` returns `new Date().toISOString()` instead of the actual last indexing timestamp.
- **Why it exists**: No persistent timestamp storage was added. The vector store tracks chunks but not indexing metadata.
- **V4.0**: Cloud Vectorize tracks metadata natively.

### 4. Fallback Mode Not Initialized
- **File**: `cortex-mcp-client.ts:220, 271`
- **Issue**: `ensureFallbackComponents()` sets a flag but doesn't initialize actual fallback search/indexing. `cleanup()` has no fallback components to clean.
- **Why it exists**: Fallback was designed as a graceful degradation path but never needed — the centralized server is reliable enough locally.
- **V4.0**: No fallback mode in V4.0. Cloud is either available or returns an error.

### 5. Cache Hit Rate Not Tracked
- **File**: `project-context-detector.ts:404`
- **Issue**: `getCacheStats()` returns `hitRate: 0` always.
- **Why it exists**: Cache was added for performance but hit rate tracking was deferred.
- **V4.0**: No local caching in V4.0.

### 6. Cache Size Not Reported
- **File**: `centralized-handlers.ts:548`
- **Issue**: Context enhancer `cacheSize` always returns 0 in health check.
- **Why it exists**: Same as #5 — cache exists but stats not wired.
- **V4.0**: Removed with local server.

### 9. Startup Banner Advertises Removed HTTP Routes
- **File**: `start-centralized-server.ts:268-269`
- **Issue**: Startup banner still prints `POST /trace-execution-path` and `POST /find-code-patterns` as supported API endpoints, but both routes were removed from `cortex-embedding-server.ts`. Following the banner yields 404s.
- **Why it exists**: Routes were removed during a tool consolidation but the startup banner was not updated in the same change.
- **V5.0**: Entire V3 server layer is decommissioned in Phase 4.

---

## Worth Fixing if V4.0 Is Delayed

These would improve V3.0 if it remains in production longer than expected.

### 7. Contextual File Reading Delegates to Search
- **File**: `mcp-handlers.ts:163`
- **Issue**: `ContextualReadHandler` delegates to `semantic_search` with file filters instead of reading the file and augmenting with semantic context.
- **Impact**: Works but suboptimal — searches for chunks in a file rather than reading the file and adding related context.
- **Fix**: Read file directly via `fs.readFileSync`, then use searcher to find related chunks from other files. ~30min fix.

### 8. SharedArrayBuffer Transfer for Embedding Workers
- **File**: `external-embedding-process.js:301`
- **Issue**: Embedding results are sent via JSON serialization instead of SharedArrayBuffer.
- **Impact**: Performance — JSON serialization adds overhead for large embedding batches. Currently acceptable because batches are 400 chunks max.
- **Fix**: Would require parent/child SharedArrayBuffer coordination. Significant refactor. Not worth it if V4.0 eliminates local embeddings.
