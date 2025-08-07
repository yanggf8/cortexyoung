# Intelligent Caching Strategy

## The Challenge: AST Chunking vs. Caching

The codebase uses a sophisticated, AST-aware chunking strategy to create semantically meaningful code chunks. This is crucial for the quality of embeddings and the effectiveness of semantic search.

However, this approach presents a caching challenge. Even a minor, single-line change (like adding a comment) can alter the AST, causing all subsequent chunks in the file to be considered "new." A naive caching strategy based on file modification times or file hashes would result in:

- **Frequent Cache Invalidation:** All chunks for a modified file are discarded.
- **Low Hit Rate:** Embeddings are constantly recomputed, even for code that hasn't changed.
- **High Computational Cost:** Unnecessary processing cycles are spent on re-embedding identical content.

## The Solution: Content-Aware Chunk Caching

To address this, we have implemented an intelligent, content-aware caching mechanism directly into the incremental indexing process. This strategy is designed to work *with* the AST chunker, not against it.

### How It Works

When a file is modified, the incremental indexer now performs the following steps:

1.  **Re-chunk the File:** The modified file is processed by the `SmartChunker` to generate a set of `new` chunks.
2.  **Retrieve Old Chunks:** The indexer fetches the `old` chunks associated with that specific file from the `PersistentVectorStore`.
3.  **Compare Content Hashes:** It then compares the `content_hash` of each `new` chunk against the `old` chunks.
    - **Cache Hit:** If a new chunk has a content hash that matches an old chunk, its existing embedding is preserved. The chunk is marked as `toKeep`.
    - **Cache Miss (New/Modified):** If a new chunk's hash is not found in the old set, it is marked as `toAdd` and sent to the embedding model.
    - **Cache Invalidation (Deleted):** Any old chunks that do not have a matching hash in the new set are marked as `toRemove`.
4.  **Apply Granular Delta:** A precise "delta" containing only the chunks to be added, updated (kept), and removed is applied to the vector store.

### Benefits

This approach provides the best of both worlds:

-   **High-Quality Embeddings:** We retain the superior semantic chunking provided by the AST-aware process.
-   **Efficient Caching:** We eliminate redundant embedding computations by reusing embeddings for unchanged code blocks.
-   **Improved Performance:** Incremental indexing is significantly faster, especially for large files with small changes.
