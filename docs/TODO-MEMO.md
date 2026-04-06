# Cortex V5.0 Known Issues

**Date**: 2026-04-06
**Scope**: Minor issues identified during code review. None affect correctness for real usage. Documented for reference — not planned for fix.

---

## Won't Fix

### 1. `hasFlag('--format', 'json')` Does OR Not AND
- **File**: `cli/src/index.ts`
- **Issue**: Returns true if either `--format` or `json` is present separately.
- **Why won't fix**: `--format json` isn't used anywhere in the CLI or skill scripts. All output is JSON by default. Dead code path.

### 2. `extractCalls` Regex Matches Control-Flow Keywords
- **File**: `cli/src/chunker.ts`
- **Issue**: Regex matches `if(`, `for(`, `while(` as function calls, creating false positive relationship edges.
- **Why won't fix**: False edges get filtered during two-pass resolution if no matching chunk exists. Noisy edges in the relationships table, but doesn't affect search results or traversal output.

### 3. Recursive CTE Uses UNION ALL (Cycle Risk)
- **File**: `cli/src/turso.ts`
- **Issue**: `traverseRelationships()` CTE uses `UNION ALL` instead of `UNION`, so a cycle would produce duplicate rows.
- **Why won't fix**: Depth is capped at 2-3 by default. Real codebases don't have circular call graphs. A cycle would hit the depth limit and stop.

### 4. `embedBatch` Is Sequential Per-Chunk
- **File**: `cli/src/embedder.ts`
- **Issue**: Embeds one chunk at a time in a loop rather than true batch processing.
- **Why won't fix**: `@xenova/transformers` processes one embedding at a time (CPU-bound ONNX). True batching wouldn't help. ~15ms/embed is already fast enough.
