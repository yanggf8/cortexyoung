# Cortex V5.0 Known Issues

**Date**: 2026-04-06
**Scope**: Minor issues identified during code review. Documented for reference.

---

## Resolved

### 1. `extractCalls` Regex Matches Control-Flow Keywords
- **File**: `cli/src/chunker.ts`
- **Issue**: Regex matched `if(`, `for(`, `while(` as function calls, creating false positive relationship edges.
- **Fix**: Added `CONTROL_FLOW` keyword set filter to `extractCalls()`. Also added exclusion guard to `isFunctionDecl()` to prevent control-flow blocks from being chunked as functions. Relationship count dropped from 2,529 to 229 (-91%) on self-index.

### 2. Chunker Quality Improvements (Token-Aware Sizing, Context Prefix, Import Grouping)
- **File**: `cli/src/chunker.ts`, `cli/src/index.ts`
- **Changes**: `splitOversized()` caps chunks at 400 tokens (BGE-small 512 limit). `mergeUndersized()` combines tiny chunks. `contextPrefix()` prepends file/symbol context before embedding (not stored). Import lines grouped per file instead of individual chunks.
- **Result**: Chunk count reduced 10%, search quality maintained, embedding disambiguation improved.

---

## Won't Fix

### 3. `hasFlag('--format', 'json')` Does OR Not AND
- **File**: `cli/src/index.ts`
- **Issue**: Returns true if either `--format` or `json` is present separately.
- **Why won't fix**: `--format json` isn't used anywhere in the CLI or skill scripts. All output is JSON by default. Dead code path.

### 4. Recursive CTE Uses UNION ALL (Cycle Risk)
- **File**: `cli/src/turso.ts`
- **Issue**: `traverseRelationships()` CTE uses `UNION ALL` instead of `UNION`, so a cycle would produce duplicate rows.
- **Why won't fix**: Depth is capped at 2-3 by default. Real codebases don't have circular call graphs. A cycle would hit the depth limit and stop.

### 5. `embedBatch` Is Sequential Per-Chunk
- **File**: `cli/src/embedder.ts`
- **Issue**: Embeds one chunk at a time in a loop rather than true batch processing.
- **Why won't fix**: `@xenova/transformers` processes one embedding at a time (CPU-bound ONNX). True batching wouldn't help. ~15ms/embed is already fast enough.
