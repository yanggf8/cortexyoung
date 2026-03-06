# Cortex V4.0 Cloud Architecture Design

**Date**: 2026-03-06
**Status**: Draft for Approval

## 1. Problem Statement

The current Cortex architecture is too complex:
- Local embedding generation (ProcessPool, memory management)
- Local vector storage with sync issues
- Heavy MCP server with complex state management
- Limited by local resources

**Goal**: Simplify to a lightweight, cloud-based architecture that reduces Claude Code's context window more effectively.

## 2. Architecture Overview

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────────┐
│   Claude Code   │────▶│   MCP Client     │────▶│   Remote Server     │
│                 │     │   (thin proxy)   │     │   (API handler)     │
└─────────────────┘     └──────────────────┘     └──────────┬──────────┘
                                                            │
                         ┌──────────────────────────────────┴──────────┐
                         │                                              │
                   ┌─────▼─────┐                                ┌──────▼──────┐
                   │Cloudflare │                                │Cloudflare AI│
                   │ Vector DB │                                │ (embeddings)│
                   └───────────┘                                └─────────────┘
```

## 3. Components

### 3.1 MCP Client (Lightweight)
- **Purpose**: Thin HTTP client that proxies requests to remote server
- **Responsibilities**:
  - Parse MCP protocol requests
  - Forward to remote API
  - Return responses to Claude Code
- **No local state** - stateless operation

### 3.2 Remote Server
- **Purpose**: Handles all business logic
- **Responsibilities**:
  - Receive search requests
  - Query Cloudflare Vector DB
  - Generate embeddings via Cloudflare AI
  - Optimize response for Claude Code context
  - Manage project metadata

### 3.3 Cloudflare
- **Vector DB**: Store embeddings and metadata
- **Workers AI**: Generate embeddings (`@cf/baai/bge-small-en-v1.5`)
- **D1** (optional): Project metadata, user configs

### 3.4 CLI (Standalone Binary)
- **Name**: `cortex`
- **Commands**:
  ```bash
  cortex index              # Index project to cloud
  cortex index --watch     # Watch and sync
  cortex search "query"    # CLI search
  cortex status            # Cloud index status
  cortex projects          # List projects
  cortex config            # Manage config
  cortex delete            # Remove from cloud
  ```

## 4. MCP Tools

Simplified toolset focused on what actually helps Claude Code:

| Tool | Description |
|------|-------------|
| `semantic_search` | Vector search in Cloudflare Vector DB |
| `contextual_read` | File read with cloud context |
| `relationship_analysis` | Query relationship data from cloud |
| `list_projects` | List cloud-indexed projects |
| `get_project_status` | Check indexing status |

**Removed from MCP**:
- `trace_execution_path` - Too complex, low value
- `find_code_patterns` - Over-engineered
- `code_intelligence` - Can use semantic_search instead
- Chunking tools - Server handles response sizing

## 5. Data Flow

### 5.1 Indexing (Push-based)
```
File changes → CLI watch → Chunk files → Cloudflare AI (embed)
                                    ↓
                            Cloudflare Vector DB
```

### 5.2 Search
```
Claude Code → MCP semantic_search → Remote Server
                                         ↓
                                  Cloudflare Vector DB
                                         ↓
                                  Claude Code (optimized)
```

## 6. Migration Plan

### Phase 1: Build Remote Server
- Create Cloudflare Workers app
- Implement Vector DB CRUD
- Implement embedding generation
- Deploy to Cloudflare

### Phase 2: Build MCP Client
- Replace complex MCP with simple HTTP proxy
- Keep tool signatures compatible
- Test with remote server

### Phase 3: Build CLI
- Create standalone `cortex` binary
- Implement file watching + push to cloud
- Implement management commands

### Phase 4: Decommission
- Remove local embedding code
- Remove local vector store
- Archive V3.0 code

## 7. Benefits

| Aspect | Current (V3) | New (V4) |
|--------|--------------|----------|
| Context reduction | ~80-90% | ~90-95% |
| Index size | Local disk | Unlimited |
| Response speed | Local | <200ms |
| Complexity | High | Low |
| Maintenance | Complex | Simple |

## 8. Open Questions

1. **D1 for metadata?** - Use Cloudflare D1 for project metadata?
2. **Authentication** - How to secure remote API?
3. **Multi-tenant** - Support multiple users/orgs?
4. **Pricing** - Cloudflare billing considerations

---

**Next**: Get approval, then create implementation plan.
