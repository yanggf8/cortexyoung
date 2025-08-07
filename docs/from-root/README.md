# Cortex V2.1 - Semantic Code Intelligence for Claude Code

A semantic code intelligence server designed to enhance Claude Code's context window efficiency through intelligent code analysis and multi-hop relationship traversal.

## Overview

Cortex V2.1 addresses Claude Code's primary limitation: **50-70% token waste** due to manual, text-based code discovery. Our system provides:

-   **80-90% token reduction** through semantic understanding
-   **Multi-hop relationship traversal** for complete context discovery
-   **MCP server architecture** for seamless Claude Code integration
-   **Adaptive context modes** balancing structure vs flexibility
-   **59% faster indexing** with intelligent process pool batching

## Key Features

-   **Advanced Relationship Traversal**: Multi-hop relationship discovery with complete context in single queries.
-   **ONNX Runtime Stability**: External Node.js processes with complete isolation and 10x parallelism.
-   **CPU + Memory Management**: Dual-resource monitoring preventing system overload (CPU: 69%/49%, Memory: 78%/69%).
-   **Signal Cascade System**: Reliable parent-child process cleanup with zero orphaned processes.
-   **Auto-sync Intelligence**: Eliminates manual storage commands with intelligent conflict resolution.

## Quick Start

```bash
# Install dependencies
npm install

# Compile TypeScript
npm run build

# Start the MCP server (intelligent mode - auto-detects indexing)
npm run server
```

## Available Scripts

### Build and Run
-   `npm run build` - Compile TypeScript to JavaScript in dist/
-   `npm run dev` - Run development server with ts-node
-   `npm run demo` - Run indexing demo
-   `npm run demo:reindex` - Run indexing demo with re-indexing
-   `npm run demo:full` - Run indexing demo with full indexing
-   `npm run server` - Start MCP server for Claude Code integration
-   `npm run server:rebuild` - Force complete rebuild with fresh embeddings
-   `npm start` - Run compiled server from dist/
-   `npm start:rebuild` - Force complete rebuild with fresh embeddings (compiled)
-   `npm start:full` - Force full repository indexing mode (compiled)
-   `npm start:incremental` - Force incremental indexing mode (compiled)
-   `npm start:cloudflare` - Start with Cloudflare embedder (compiled)
-   `npm run clean` - Remove dist directory

### Cache Management
-   `npm run cache:stats` - Show embedding cache statistics
-   `npm run cache:clear` - Clear embedding cache
-   `npm run cache:validate` - Validate cache integrity
-   `npm run cache:backup` - Backup embedding cache
-   `npm run cache:sync-global` - Sync local embeddings to global storage
-   `npm run cache:sync-local` - Sync global embeddings to local storage
-   `npm run cache:info` - Show storage paths and modification times
-   `npm run cache:clear-all` - Clear all caches (embeddings and relationships)

### Relationship Management
-   `npm run relationships:stats` - Show relationship store statistics
-   `npm run relationships:clear` - Clear relationship store
-   `npm run relationships:sync-global` - Sync local relationships to global storage
-   `npm run relationships:sync-local` - Sync global relationships to local storage
-   `npm run relationships:info` - Show relationship store paths and modification times

### Unified Storage
-   `npm run storage:status` - Show unified storage status
-   `npm run storage:stats` - Show unified storage statistics
-   `npm run storage:validate` - Validate unified storage integrity
-   `npm run storage:sync` - Sync unified storage
-   `npm run storage:clear` - Clear unified storage

### Health Checks
-   `npm run health` - Comprehensive health report
-   `npm run health:check` - Quick check if rebuild is recommended

### Testing
-   `npm run test:mcp` - Test MCP server functionality
-   `npm run test:worker-pool` - Test concurrent worker pool embedder
-   `npm run test:process-pool` - Test external process pool embedder
-   `npm run test:adaptive-features` - Test adaptive features
-   `npm run test:adaptive-pool` - Test adaptive pool
-   `npm run test:cleanup` - Test direct cleanup
-   `npm run test:orphan-prevention` - Test orphan prevention
-   `npm run test:final-cleanup` - Test final cleanup
-   `npm run test:cpu-memory` - Test CPU and memory scaling
-   `npm run test:signal-cascade` - Test signal cascade
-   `npm run test:auto-sync-staleness` - Test auto-sync staleness

### Performance Benchmarking
-   `npm run benchmark` - Full benchmark suite
-   `npm run benchmark:startup` - Startup performance benchmarks only
-   `npm run benchmark:search` - Search performance benchmarks only
-   `npm run benchmark:storage` - Storage operations benchmarks only
-   `npm run benchmark:full` - Full suite with 3 iterations and detailed analysis
-   `npm run validate:performance` - Comprehensive validation of critical improvements
-   `npm run test:performance` - Alias for performance validation
-   `npm run benchmark:adaptive` - Benchmark adaptive pool
-   `npm run benchmark:quick` - Quick startup validation

### Process Management
-   `npm run shutdown` - Clean shutdown with process cleanup
-   `npm run startup` - Start server with health checks
-   `npm run status` - Check server status

## Architecture

Cortex V2.1 uses a hybrid local + cloud architecture.

-   **Local:** `ProcessPoolEmbedder` with CPU + memory adaptive management.
-   **Cloud:** Cloudflare Workers AI option.
-   **Storage:** Dual persistence (local `.cortex/` + global `~/.claude/`).
-   **Auto-sync:** Intelligent conflict resolution and missing data recovery.

### Data Flow
```
Claude Code ← MCP Server ← Vector Store ← Dual Storage + Auto-sync
     ↓             ↓              ↓              ↓
User Query → ProcessPool → Embeddings → Local + Global Storage
```

### Key Components
-   **server.ts:** MCP server with HTTP transport and startup tracking.
-   **indexer.ts:** Repository indexing with incremental support.
-   **process-pool-embedder.ts:** CPU + memory adaptive embedding generation.
-   **unified-storage-coordinator.ts:** Auto-sync dual storage management.

## Configuration

### Environment Variables
-   `EMBEDDER_TYPE`: `local` (default) or `cloudflare`.
-   `PORT`: Server port (default: 8765).
-   `LOG_FILE`: Custom log file path (default: `logs/cortex-server.log`).
-   `DEBUG`: Enable debug logging (set to `true`).
-   `INDEX_MODE`: Override intelligent mode: `full` or `incremental`.
-   `FORCE_REBUILD`: Force complete rebuild: `true` (clears existing embeddings).

## MCP Server Integration

**Configuration for Claude Code** (`~/.claude/mcp_servers.json`):
```json
{
  "mcpServers": {
    "cortex": {
      "command": "npm",
      "args": ["run", "server"],
      "cwd": "/home/yanggf/a/cortexyoung",
      "env": {
        "PORT": "8765",
        "EMBEDDER_TYPE": "local"
      },
      "transport": {
        "type": "http",
        "url": "http://localhost:8765"
      }
    }
  }
}
```

### Available MCP Tools
1.  **semantic_search** - Advanced semantic code search with relationship traversal
2.  **contextual_read** - File reading with semantic context awareness
3.  **code_intelligence** - High-level semantic codebase analysis
4.  **relationship_analysis** - Code relationship analysis and traversal
5.  **trace_execution_path** - Execution path tracing
6.  **find_code_patterns** - Complex code pattern discovery

## Storage Management

### Automatic Storage (Recommended)
-   **Auto-sync on startup**: Resolves missing data and staleness automatically.
-   **Dual persistence**: Local (`.cortex/`) + Global (`~/.claude/`).
-   **Smart conflict resolution**: Newer timestamp wins.
-   **Zero manual intervention**: System handles synchronization intelligently.

### Manual Commands (Rarely Needed)
```bash
npm run storage:status    # Complete status report
npm run storage:validate  # Consistency check
npm run cache:clear-all   # Clear all storage (nuclear option)
```

## Process Management

### Always Clean Up Interrupted Processes
When interrupting any command (Ctrl+C, timeout, kill):
```bash
# Clean up both parent and child processes
pkill -f "npm.*demo|ts-node.*index|node.*external-embedding-process"
```

**Or use the automated cleanup:**
```bash
npm run shutdown  # Comprehensive cleanup script
```
