# Cortex V2.1 Technical Specification

## Overview

Cortex V2.1 is a semantic code intelligence server designed to provide AI agents with deep, contextual understanding of codebases. Unlike traditional Language Server Protocol (LSP) implementations that focus on syntactic analysis, Cortex prioritizes semantic meaning and intent, delivering structured "context packages" optimized for large language model consumption.

## Core Principles

### 1. Semantic-First Architecture
- **Purpose**: Answer semantic queries like "What code is related to 'authentication'?" rather than purely structural queries like "Where is `login_user` defined?"
- **Implementation**: Use semantic embeddings and metadata enrichment to understand code intent and relationships
- **Benefit**: Provides more relevant and contextual results for AI-driven code understanding

### 2. AI-Native Context Generation
- **Purpose**: Generate output specifically designed for LLM consumption
- **Implementation**: Synthesized, compressed, and structured context packages rather than raw file lists
- **Benefit**: Reduces token usage while maximizing contextual value for AI agents

### 3. Version-Aware Intelligence
- **Purpose**: Understand code evolution and historical context
- **Implementation**: Link code chunks to Git repository history and commit metadata
- **Benefit**: Prioritize recently modified code and understand change patterns

## System Architecture

The system consists of two primary components operating in tandem:

### A. Git-Aware Indexing Pipeline (Offline Processing)

The indexing pipeline runs as a background process, triggered on-demand or integrated into CI/CD workflows.

#### 1. Commit Scanner
**Purpose**: Identify changes in the codebase for incremental or full indexing

**Implementation**:
- **Incremental Mode**: Use `git diff` to identify modified files since last indexed commit
- **Full Mode**: Scan entire repository structure
- **Change Detection**: Track file additions, modifications, and deletions

**Output**: List of files requiring reprocessing

#### 2. Smart Chunking Engine
**Purpose**: Break down files into semantically meaningful units

**Implementation**:

**Code Files** (`.py`, `.js`, `.ts`, `.go`, etc.):
- Use Abstract Syntax Tree (AST) parsers (Tree-sitter recommended)
- Create chunks at logical boundaries:
  - Function definitions
  - Class definitions
  - Method implementations
  - Module-level constants and configurations

**Documentation Files** (`.md`, `.rst`, `.txt`):
- Parse based on structural elements:
  - Section headings (H1, H2, H3)
  - Code blocks
  - Lists and tables
- Maintain semantic coherence within chunks

**Configuration Files** (`.json`, `.yaml`, `.toml`):
- Chunk by logical configuration sections
- Preserve key-value relationships

**Output**: Semantically coherent code/text chunks with precise boundary information

#### 3. Embedding & Metadata Enrichment
**Purpose**: Generate vector representations and rich contextual metadata

**Embedding Generation**:
- **Model**: Use state-of-the-art embedding models (e.g., `nomic-embed-text`, OpenAI embeddings)
- **Input**: Raw chunk content with minimal preprocessing
- **Output**: High-dimensional vector representation

**Metadata Schema**:
```json
{
  "chunk_id": "string (UUID)",
  "file_path": "string (relative to repo root)",
  "symbol_name": "string (function/class name, optional)",
  "chunk_type": "enum (function|class|method|documentation|config)",
  "start_line": "integer",
  "end_line": "integer",
  "content_hash": "string (SHA-256 of content)",
  "embedding": "array<float> (vector representation)",
  "relationships": {
    "calls": "array<string> (functions this chunk calls)",
    "called_by": "array<string> (functions that call this chunk)",
    "imports": "array<string> (modules/symbols imported)",
    "exports": "array<string> (symbols exported)",
    "data_flow": "array<string> (variables/data passed between chunks)"
  },
  "git_metadata": {
    "last_modified_commit": "string (commit SHA)",
    "commit_author": "string (email)",
    "commit_message": "string",
    "commit_date": "ISO 8601 timestamp",
    "file_history_length": "integer (number of commits affecting this file)",
    "co_change_files": "array<string> (files often modified together)"
  },
  "language_metadata": {
    "language": "string (programming language)",
    "complexity_score": "float (cyclomatic complexity for code)",
    "dependencies": "array<string> (imported modules/packages)",
    "exports": "array<string> (exported symbols)"
  },
  "usage_patterns": {
    "access_frequency": "float (how often this code is accessed)",
    "task_contexts": "array<string> (types of tasks this code appears in)"
  }
}
```

#### 4. Vector Storage (Cloudflare Vectorize)
**Purpose**: Store embeddings and metadata for high-speed similarity search

**Storage Strategy**:
- **Primary Index**: Vector embeddings for similarity search
- **Secondary Indexes**: File path, symbol name, commit date for filtering
- **Metadata Storage**: Full metadata objects linked to vectors

**Performance Requirements**:
- Sub-100ms query response time
- Support for 1M+ code chunks
- Concurrent query handling

### B. Cortex MCP Server (Real-Time Query Engine)

The MCP server provides the live interface for AI agents to retrieve contextual information.

#### 1. Query Planner
**Purpose**: Orchestrate retrieval based on natural language queries

**Intent Deconstruction**:
- Parse natural language task descriptions
- Extract key semantic concepts using NLP techniques
- Generate search terms and filters

**Multi-Hop Vector Search Strategy**:
- **Initial Semantic Search**: Embedding similarity using query vector
- **Relationship Traversal**: Follow code connections (calls, imports, data flow)
- **Multi-Hop Expansion**: Explore connected code up to configurable depth
- **Keyword Search**: Exact matches for technical terms
- **Metadata Filtering**: File type, recency, author constraints

**Enhanced Ranking Algorithm**:
```python
score = (
    semantic_similarity * 0.3 +
    relationship_relevance * 0.3 +
    recency_score * 0.2 +
    file_importance * 0.1 +
    exact_match_bonus * 0.1
)
```

#### 2. Context Synthesizer
**Purpose**: Transform search results into coherent, structured context packages

**Grouping Strategy**:
- Group related chunks by file
- Group by feature/module when spanning multiple files
- Maintain logical flow and dependencies

**Summary Generation**:
- Generate concise descriptions for each group
- Highlight key functionality and recent changes
- Provide navigation hints (line numbers, file paths)

**Output Formatting**:
- **Markdown**: Human-readable format for documentation
- **JSON**: Structured format for programmatic consumption
- **Context-Aware**: Optimized for LLM context windows

## API Specifications

### MCP Server Interface

#### Query Context
```typescript
interface QueryRequest {
  task: string;                    // Natural language task description
  max_chunks?: number;            // Maximum chunks to return (default: 20)
  file_filters?: string[];        // File path patterns to include/exclude
  recency_weight?: number;        // Weight for recent changes (0-1, default: 0.3)
  include_tests?: boolean;        // Include test files (default: false)
  multi_hop?: {
    enabled: boolean;             // Enable multi-hop relationship traversal
    max_hops: number;            // Maximum relationship depth (default: 3)
    relationship_types: string[]; // ["calls", "imports", "data_flow", "co_change"]
    hop_decay: number;           // Relevance decay per hop (default: 0.8)
  };
  context_mode?: "minimal" | "structured" | "adaptive"; // Orchestration level
}

interface QueryResponse {
  context_package: ContextPackage;
  metadata: {
    total_chunks_found: number;
    query_time_ms: number;
    chunks_returned: number;
  };
}

interface ContextPackage {
  summary: string;                // High-level summary of relevant code
  groups: ContextGroup[];         // Grouped code chunks
  related_files: string[];        // Additional files that might be relevant
}

interface ContextGroup {
  title: string;                  // Human-readable group title
  description: string;            // What this group contains
  chunks: CodeChunk[];           // Individual code chunks
  importance_score: number;       // Relevance score (0-1)
}

interface CodeChunk {
  file_path: string;
  start_line: number;
  end_line: number;
  content: string;
  symbol_name?: string;
  chunk_type: ChunkType;
  last_modified: string;          // ISO 8601 timestamp
  relevance_score: number;        // How relevant to the query (0-1)
}
```

#### Repository Management
```typescript
interface IndexRequest {
  repository_path: string;
  mode: 'full' | 'incremental';
  since_commit?: string;          // For incremental mode
}

interface IndexResponse {
  status: 'success' | 'error';
  chunks_processed: number;
  time_taken_ms: number;
  error_message?: string;
}
```

### Configuration

#### Server Configuration
```json
{
  "server": {
    "port": 3000,
    "host": "localhost",
    "max_concurrent_requests": 10
  },
  "vectorize": {
    "account_id": "cloudflare_account_id",
    "api_token": "cloudflare_api_token",
    "index_name": "cortex_embeddings"
  },
  "embedding": {
    "model": "nomic-embed-text",
    "dimensions": 768,
    "batch_size": 100
  },
  "chunking": {
    "max_chunk_size": 2000,
    "overlap_size": 200,
    "languages": ["python", "javascript", "typescript", "go", "rust"]
  }
}
```

## Performance Requirements

### Indexing Pipeline
- **Throughput**: Process 1000+ files per minute
- **Memory Usage**: <2GB RAM for repositories up to 100k files
- **Storage**: Efficient vector storage with <100MB per 10k code chunks

### Query Server
- **Response Time**: <200ms for 95% of queries
- **Concurrent Users**: Support 50+ simultaneous queries
- **Availability**: 99.9% uptime with graceful degradation

## Security Considerations

### Access Control
- **Repository Access**: Validate user permissions before indexing
- **Query Filtering**: Respect file-level access controls
- **API Authentication**: Secure MCP server endpoints

### Data Privacy
- **Local Processing**: Keep sensitive code local during indexing
- **Metadata Scrubbing**: Remove sensitive information from embeddings
- **Audit Logging**: Track all query and indexing operations

## Deployment Architecture

### Development Environment
```
[Local Git Repo] → [Indexing Pipeline] → [Local Vector Store] → [MCP Server] → [AI Agent]
```

### Production Environment
```
[Git Repository] → [CI/CD Pipeline] → [Indexing Service] → [Cloudflare Vectorize] → [MCP Server Cluster] → [AI Agents]
```

### Scalability Strategy
- **Horizontal Scaling**: Multiple MCP server instances behind load balancer
- **Caching**: Redis cache for frequent queries
- **Background Processing**: Async indexing with job queues

## Implementation Phases

### Phase 1: Core Functionality (MVP)
- Basic chunking for Python and JavaScript
- Simple vector search with embeddings
- MCP server with basic query interface
- Local vector storage (SQLite + extensions)

### Phase 2: Enhanced Intelligence
- Multi-language AST parsing
- Git metadata integration
- Cloudflare Vectorize integration
- Advanced ranking algorithms

### Phase 3: Production Features
- Incremental indexing
- Performance optimization
- Comprehensive test coverage
- Documentation and examples

### Phase 4: Advanced Features
- Cross-repository search
- Team collaboration features
- Analytics and usage metrics
- Custom embedding models

## Success Metrics

### Technical Metrics
- **Query Accuracy**: >90% relevant results in top 5
- **Response Time**: <200ms average query time
- **Index Freshness**: <5 minute lag for incremental updates

### User Experience Metrics
- **AI Agent Efficiency**: Reduced context gathering time
- **Code Understanding**: Improved accuracy of AI-generated code
- **Developer Productivity**: Faster debugging and feature development

## Future Considerations

### Extensibility
- Plugin architecture for custom chunking strategies
- Support for additional version control systems
- Integration with popular IDEs and development tools

### Advanced Features
- Semantic code search across multiple repositories
- Automated code documentation generation
- Intelligent code review assistance
- Cross-team knowledge sharing

---

## Appendix

### Supported File Types
- **Code**: `.py`, `.js`, `.ts`, `.jsx`, `.tsx`, `.go`, `.rs`, `.java`, `.cpp`, `.c`, `.h`
- **Documentation**: `.md`, `.rst`, `.txt`, `.adoc`
- **Configuration**: `.json`, `.yaml`, `.yml`, `.toml`, `.ini`
- **Markup**: `.html`, `.xml`, `.svg`

### Dependencies
- **Core**: Node.js 18+, TypeScript 5+
- **Parsing**: Tree-sitter, unified/remark
- **Vector Search**: Cloudflare Vectorize API
- **Embeddings**: Configurable (OpenAI, Cohere, local models)
- **Git Integration**: isomorphic-git or native git CLI

### License and Contributing
- Open source under MIT license
- Community contributions welcome
- Comprehensive contributor guidelines and code of conduct