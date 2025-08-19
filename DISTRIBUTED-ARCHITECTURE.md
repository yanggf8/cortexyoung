# Cortex Distributed Multi-Project Architecture

## Overview

Cortex V3.0 introduces a revolutionary distributed architecture enabling seamless multi-project semantic code intelligence for Claude Code. This design addresses the fundamental limitation of monolithic servers that can only analyze one project at a time.

## Architecture Principles

### Core Goals
- **Multi-Project Concurrency**: Claude Code can work with unlimited projects simultaneously
- **Resource Efficiency**: Centralized heavy computation with lightweight local interfaces
- **Real-Time Performance**: Sub-second response times for interactive code analysis
- **Fault Tolerance**: Graceful degradation when central services are unavailable
- **Developer Experience**: Zero-friction project onboarding and management

### Design Philosophy
> "Lightweight locally, powerful centrally, resilient everywhere"

## System Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Claude Code   │    │   Claude Code   │    │   Claude Code   │
│    Project A    │    │    Project B    │    │    Project N    │
│   (Frontend)    │    │   (Backend)     │    │   (Mobile)      │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         │ MCP (stdio)           │ MCP (stdio)           │ MCP (stdio)
         ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ Local MCP Server│    │ Local MCP Server│    │ Local MCP Server│
│     (stdio)     │    │     (stdio)     │    │     (stdio)     │
│  Git Operations │    │  Git Operations │    │  Git Operations │
│ Project Context │    │ Project Context │    │ Project Context │
│   Health Check  │    │   Health Check  │    │   Health Check  │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         │ gRPC/HTTP API         │ gRPC/HTTP API         │ gRPC/HTTP API
         └───────────────────────┼───────────────────────┘
                                 ▼
                    ┌─────────────────────────────┐
                    │    Centralized Search       │
                    │        Server               │
                    │      (Port 9000)            │
                    │                             │
                    │ 🧠 Multi-Project Index       │
                    │ 🚀 BGE Embedding Generation  │
                    │ 🔍 Vector Storage & Search   │
                    │ 🕸️  Relationship Analysis    │
                    │ ⚡ Job Queue & Worker Pool   │
                    │ 💾 Shared Cache & State      │
                    └─────────────────────────────┘
```

## Component Architecture

### 1. Local MCP Server (Per Project)

**File**: `cortex-local-server.js`
**Transport**: stdio (Claude Code native)
**Installation**: 
```bash
claude mcp add cortex-projectA stdio node cortex-local-server.js /path/to/projectA
claude mcp add cortex-projectB stdio node cortex-local-server.js /path/to/projectB
```

**Responsibilities**:
- **🏠 Project-Specific Operations**
  - Git repository monitoring and file operations
  - Real-time file change detection
  - Project metadata and context management
  - Local configuration and preferences

- **🔌 MCP Protocol Interface**
  - stdio-based communication with Claude Code
  - MCP tool implementation (semantic_search, code_intelligence, etc.)
  - Parameter validation and request routing
  - Response formatting and optimization

- **🌐 Search Server Communication**
  - gRPC client for performance-critical operations
  - HTTP client for management operations
  - Request batching and connection pooling
  - Automatic retry and circuit breaker patterns

- **💡 Intelligence & Fallbacks**
  - Health monitoring of centralized server
  - Local result caching for resilience
  - Degraded mode with keyword-based fallback
  - Progressive enhancement when server is available

**Key Features**:
```javascript
// Ultra-lightweight design
const localServer = {
  startup: '<100ms',
  memory: '<50MB',
  dependencies: 'minimal',
  reliability: '99.9%'
}
```

### 2. Centralized Search Server

**File**: `cortex-search-server.ts`
**Transport**: Hybrid gRPC + HTTP
**Port**: 9000

**Architecture**:
```
┌─────────────────────────────────────┐
│        API Gateway Layer            │
│  • gRPC Server (performance)       │
│  • HTTP Server (management)        │
│  • Authentication & Rate Limiting  │
└─────────────────────────────────────┘
┌─────────────────────────────────────┐
│       Job Queue & Orchestration     │
│  • Priority Queue (interactive >    │
│    background)                      │
│  • Async Job Processing            │
│  • Worker Pool Management          │
└─────────────────────────────────────┘
┌─────────────────────────────────────┐
│      Core Intelligence Engine       │
│  • Multi-Project Vector Store      │
│  • BGE-small-en-v1.5 Embeddings   │
│  • Semantic Search & MMR          │
│  • Relationship Graph Analysis     │
└─────────────────────────────────────┘
┌─────────────────────────────────────┐
│         Resource Management         │
│  • Memory-Mapped Shared Cache     │
│  • Process Pool with Batch Safety │
│  • CPU/Memory Adaptive Scaling    │
│  • Cross-Project Resource Isolation│
└─────────────────────────────────────┘
```

**Responsibilities**:
- **🧠 Semantic Intelligence**
  - BGE-small-en-v1.5 embedding generation
  - Vector storage with multi-project isolation
  - Advanced semantic search with MMR optimization
  - Cross-project relationship analysis

- **⚡ Performance & Scalability**
  - Asynchronous job queue for heavy operations
  - Worker pools for parallel embedding generation
  - Priority-based request handling
  - Resource isolation per project

- **🔄 State Management**
  - Multi-project index coordination
  - Incremental update propagation
  - Cache coherence across projects
  - Persistent storage and recovery

- **🛡️ Reliability & Operations**
  - Health check endpoints
  - Metrics and monitoring
  - Graceful shutdown with state preservation
  - Auto-recovery and fault tolerance

## API Specification

### gRPC API (Performance-Critical)

**Service**: `CortexSearchService`

```protobuf
service CortexSearchService {
  // High-frequency, low-latency operations
  rpc SemanticSearch(SearchRequest) returns (SearchResponse);
  rpc CodeIntelligence(AnalysisRequest) returns (AnalysisResponse);
  rpc RelationshipAnalysis(RelationshipRequest) returns (RelationshipResponse);
  rpc RealTimeSearch(stream SearchRequest) returns (stream SearchResponse);
}

message SearchRequest {
  string project_id = 1;
  string query = 2;
  int32 max_chunks = 3;
  repeated string file_filters = 4;
  MultiHopConfig multi_hop = 5;
}
```

### HTTP REST API (Management Operations)

**Base URL**: `http://localhost:9000/api/v1`

```yaml
# Project Management
POST   /projects/{projectId}/index        # Index project files
GET    /projects/{projectId}/status       # Project status and stats
DELETE /projects/{projectId}              # Remove project
POST   /projects/{projectId}/files        # Update specific files
GET    /projects/{projectId}/health       # Project-specific health

# System Management  
GET    /health                           # Overall system health
GET    /metrics                          # Performance metrics
POST   /admin/gc                         # Force garbage collection
GET    /admin/cache/stats                # Cache statistics
```

## Implementation Strategy

### Phase 1: Centralized Search Server
**Duration**: 2-3 days
**Priority**: High

```typescript
// Key components to implement
├── cortex-search-server.ts     # Main server entry point
├── grpc/
│   ├── search.proto            # gRPC service definition
│   ├── search-service.ts       # gRPC service implementation
│   └── client-utils.ts         # gRPC client utilities
├── http/
│   ├── api-routes.ts           # REST API routes
│   ├── middleware.ts           # Auth, validation, rate limiting
│   └── health-checks.ts        # Health and monitoring endpoints
├── queue/
│   ├── job-queue.ts            # Priority-based job queue
│   ├── workers.ts              # Background job workers
│   └── scheduler.ts            # Job scheduling and retry logic
└── storage/
    ├── multi-project-store.ts  # Multi-tenant vector storage
    ├── project-manager.ts      # Project lifecycle management
    └── cache-coordinator.ts    # Cross-project cache management
```

### Phase 2: Local MCP Server
**Duration**: 1-2 days  
**Priority**: High

```javascript
// cortex-local-server.js - Lightweight and focused
├── mcp/
│   ├── stdio-server.js         # MCP stdio transport
│   ├── tool-handlers.js        # MCP tool implementations
│   └── response-optimizer.js   # Ultra-minimal responses
├── client/
│   ├── grpc-client.js          # gRPC client for search server
│   ├── http-client.js          # HTTP client for management
│   └── connection-pool.js      # Connection management
├── project/
│   ├── git-operations.js       # Project-specific git ops
│   ├── file-watcher.js         # Real-time file monitoring
│   └── context-manager.js      # Project context and metadata
└── resilience/
    ├── health-monitor.js       # Central server health checks
    ├── local-cache.js          # Local result caching
    ├── fallback-search.js      # Degraded mode with ripgrep
    └── circuit-breaker.js      # Fault tolerance patterns
```

### Phase 3: Integration & Testing
**Duration**: 1 day
**Priority**: Medium

- Multi-project testing scenarios
- Performance benchmarking
- Fault tolerance validation
- Documentation and examples

## Performance Targets

### Response Times
- **Semantic Search**: <200ms (gRPC)
- **Code Intelligence**: <500ms (gRPC)
- **Project Status**: <50ms (HTTP)
- **File Updates**: <100ms (HTTP)

### Throughput
- **Concurrent Projects**: 20+ active projects
- **Search Queries**: 100+ queries/second
- **Embedding Generation**: 1000+ chunks/minute
- **Memory Usage**: <2GB for 10 active projects

### Reliability
- **Uptime**: 99.9% availability
- **Fault Recovery**: <30s automatic recovery
- **Degraded Mode**: <100ms fallback activation
- **Data Durability**: Zero data loss on graceful shutdown

## Resource Management

### Memory Allocation
```typescript
const resourceLimits = {
  // Per project limits
  maxChunksPerProject: 100_000,
  maxMemoryPerProject: '500MB',
  
  // Global limits
  totalMemoryLimit: '4GB',
  embeddingCacheSize: '1GB',
  
  // Worker pool limits
  maxWorkers: Math.min(16, os.cpus().length),
  workerMemoryLimit: '200MB'
}
```

### Priority System
1. **P0 - Interactive**: Real-time search queries from active Claude Code sessions
2. **P1 - Background**: File updates and incremental indexing
3. **P2 - Batch**: Full project reindexing and optimization

### Scaling Strategy
- **Vertical**: Scale worker pools based on CPU cores and memory
- **Horizontal**: Multiple search server instances with load balancing (future)
- **Adaptive**: Dynamic resource allocation based on project activity

## Security & Reliability

### Authentication
```typescript
// Simple token-based auth for local network
const authConfig = {
  apiKey: process.env.CORTEX_API_KEY || generateSecureKey(),
  tokenExpiry: '24h',
  rateLimits: {
    perProject: '1000/hour',
    global: '10000/hour'
  }
}
```

### Health Monitoring
```typescript
const healthChecks = {
  // System health
  memory: () => checkMemoryUsage(),
  cpu: () => checkCPUUsage(),
  disk: () => checkDiskSpace(),
  
  // Service health  
  vectorStore: () => testVectorOperations(),
  embeddings: () => testEmbeddingGeneration(),
  jobQueue: () => checkQueueHealth(),
  
  // Per-project health
  projectIndex: (projectId) => validateProjectIndex(projectId),
  lastActivity: (projectId) => getLastActivityTime(projectId)
}
```

### Fallback Strategies
1. **Graceful Degradation**: Local servers switch to keyword-based search
2. **Local Caching**: Cache recent results for offline functionality  
3. **Retry Logic**: Exponential backoff with circuit breaker
4. **Health Recovery**: Automatic reconnection when service restored

## Migration Path

### From Current Monolithic Architecture
1. **Phase 1**: Deploy centralized search server alongside existing server
2. **Phase 2**: Migrate one project to local MCP server for testing
3. **Phase 3**: Gradual migration of remaining projects
4. **Phase 4**: Deprecate monolithic server

### Backward Compatibility
- Existing HTTP MCP server remains functional during transition
- Gradual migration project by project
- No disruption to existing Claude Code workflows

## Monitoring & Observability

### Key Metrics
```typescript
const metrics = {
  // Performance
  searchLatency: histogram('search_request_duration'),
  embeddingThroughput: counter('embeddings_generated_total'),
  cacheHitRate: gauge('cache_hit_rate'),
  
  // Reliability
  serverUptime: gauge('server_uptime_seconds'),
  errorRate: rate('errors_per_second'),
  failoverEvents: counter('failover_events_total'),
  
  // Resource utilization
  memoryUsage: gauge('memory_usage_bytes'),
  cpuUsage: gauge('cpu_usage_percent'),
  activeProjects: gauge('active_projects_count')
}
```

### Alerting
- **Critical**: Server down, memory exhaustion, embedding failures
- **Warning**: High latency, elevated error rate, cache misses
- **Info**: New project onboarding, successful recovery events

## Developer Experience

### Project Onboarding
```bash
# One command to add any project
cd /path/to/my-awesome-project
claude mcp add cortex-awesome stdio node cortex-local-server.js

# Automatic project discovery and indexing
# Ready for semantic analysis in <30 seconds
```

### IDE Integration
```bash
# Works seamlessly with Claude Code
@cortex-semantic_search "JWT authentication logic"
@cortex-code_intelligence "understand the payment flow"
@cortex-relationship_analysis --starting_symbols "UserService.login"
```

### Troubleshooting
```bash
# Built-in diagnostics
cortex-cli status                    # Overall system health
cortex-cli project status awesome    # Project-specific status  
cortex-cli logs --follow            # Real-time log streaming
cortex-cli metrics                  # Performance metrics
```

## Future Enhancements

### V3.1 - Cross-Project Intelligence
- Semantic search across multiple projects
- Cross-project relationship analysis
- Shared library and dependency mapping

### V3.2 - Advanced Features  
- Code completion and suggestions
- Automated refactoring recommendations
- Security vulnerability detection
- Performance optimization insights

### V3.3 - Enterprise Features
- Multi-user support with RBAC
- Audit logging and compliance
- High availability with clustering
- Custom embedding models

## Conclusion

This distributed architecture transforms Cortex from a single-project tool into a true multi-project semantic intelligence platform. By separating lightweight local interfaces from powerful centralized computation, we achieve:

- **Unlimited project support** without resource conflicts
- **Sub-second response times** for interactive analysis
- **99.9% reliability** with intelligent fallbacks
- **Zero-friction onboarding** for new projects
- **Resource efficiency** through intelligent sharing

The architecture is production-ready, scalable, and designed for the real-world needs of developers working across multiple codebases simultaneously.

---

**Status**: Architecture design complete, ready for implementation  
**Next Steps**: Begin Phase 1 implementation of centralized search server