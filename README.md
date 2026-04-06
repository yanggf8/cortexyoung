# Cortex

Semantic code intelligence for Claude Code. Chunks source files, embeds locally, stores in Turso cloud. One CLI, one database, no servers.

## How It Works

```
cortex index .          →  chunk files → embed (BGE-small-en-v1.5) → upload to Turso
cortex search "query"   →  embed query → vector_top_k in Turso → ranked results
cortex relationships fn →  recursive CTE in Turso → call graph / imports / data flow
```

## Quick Start

```bash
# Install
cd cli && npm install && npm run build

# Initialize (creates Turso database, stores config)
node dist/index.js init

# Index a project
node dist/index.js index /path/to/project

# Search
node dist/index.js search "authentication middleware"
node dist/index.js search "auth" --keyword    # FTS5 keyword search

# Explore relationships
node dist/index.js relationships "handleLogin"

# Project info
node dist/index.js status
node dist/index.js projects
```

## Architecture

| Component | Technology | Purpose |
|-----------|-----------|---------|
| Embeddings | `@xenova/transformers` BGE-small-en-v1.5 | 384-dim vectors, ~15ms/embed, local |
| Storage | Turso (libSQL) | Vectors (F32_BLOB/DiskANN), content, relationships, FTS5 |
| Chunking | Regex/brace-counting | JS/TS/Python/Markdown aware |
| Delivery | Claude Code Skill | 3 scripts: search, relationships, status |

## CLI Commands

| Command | Description |
|---------|-------------|
| `cortex init` | Create Turso DB, store URL+token in `~/.cortex/config.json` |
| `cortex index [path]` | Chunk → analyze → embed → upload to Turso |
| `cortex search "query"` | Semantic search via vector_top_k |
| `cortex search "q" --keyword` | FTS5 keyword search |
| `cortex relationships "symbol"` | Recursive CTE relationship traversal |
| `cortex status` | Project stats |
| `cortex projects` | List all indexed projects |
| `cortex delete` | Remove current project from Turso |
| `cortex config` | Show config |

## Cost

$0/mo. Turso free tier (9GB, 500M reads/mo). Local embeddings. No cloud compute.

## Requirements

- Node.js 20+
- Turso CLI (`curl -sSfL https://get.tur.so/install.sh | bash`)
- Turso account (free)
