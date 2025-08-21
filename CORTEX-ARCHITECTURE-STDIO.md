# Cortex Proactive Context Engine - Architecture Overview (stdio Transport)

## System Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        CORTEX ARCHITECTURE                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────┐    ┌──────────────────────────────────┐   │
│  │   Claude Code   │◄──►│        MCP Server                │   │
│  │   (Client)      │    │    (stdio Transport)             │   │
│  └─────────────────┘    │  Protocol: JSON-RPC 2.0          │   │
│                         │  Communication: stdin/stdout     │   │
│                         └──────────────────────────────────┘   │
│                                       │                        │
│  ┌─────────────────────────────────────▼─────────────────────┐  │
│  │              PROACTIVE CONTEXT ENGINE                   │  │
│  │                                                         │  │
│  │  ┌─────────────────┐  ┌─────────────────────────────┐   │  │
│  │  │ ContextWatcher  │  │  Enhanced Project Detector  │   │  │
│  │  │                 │  │                             │   │  │
│  │  │ • File Monitor  │  │ • Auth Pattern Detection   │   │  │
│  │  │ • Change Queue  │  │ • API Response Detection   │   │  │
│  │  │ • Debouncing    │  │ • Error Handling Detection │   │  │
│  │  └─────────────────┘  │ • Confidence Scoring       │   │  │
│  │                       │ • Evidence Collection      │   │  │
│  │                       └─────────────────────────────┘   │  │
│  │                                       │                 │  │
│  │  ┌─────────────────────────────────────▼─────────────┐   │  │
│  │  │           CLAUDE.md Maintainer                   │   │  │
│  │  │                                                  │   │  │
│  │  │ • Boot-time Context Generation                  │   │  │
│  │  │ • Marker-based Safe Updates                     │   │  │
│  │  │ • Atomic File Operations                        │   │  │
│  │  │ • Critical Guardrails Generation                │   │  │
│  │  │ • Confidence Indicators                         │   │  │
│  │  └──────────────────────────────────────────────────┘   │  │
│  └─────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## Production Deployment Architecture (stdio)

```
┌─────────────────────────────────────────────────────────┐
│                    DEPLOYMENT                           │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Developer Machine:                                     │
│  ┌─────────────────────────────────────────────────────┐ │
│  │  1. npm run server:stdio                            │ │
│  │     ├─ Starts MCP Server (stdio mode)               │ │
│  │     ├─ Initializes Proactive Context Engine        │ │
│  │     └─ Begins file watching                         │ │
│  │                                                     │ │
│  │  2. claude mcp add cortex ./mcp-server-stdio.js     │ │
│  │     └─ Registers MCP stdio integration              │ │
│  │                                                     │ │
│  │  3. claude chat --mcp                               │ │
│  │     └─ Starts Claude Code with MCP tools            │ │
│  └─────────────────────────────────────────────────────┘ │
│                                                         │
│  Communication Flow:                                    │
│  ┌─────────────────────────────────────────────────────┐ │
│  │  Claude Code ←→ stdio ←→ MCP Server ←→ Cortex       │ │
│  │                                                     │ │
│  │  JSON-RPC 2.0 messages via stdin/stdout pipes      │ │
│  └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

## MCP Workflow with stdio Transport

### Boot-time Proactive Context Generation

```
┌─────────────────────────────────────────────────────────────┐
│                    BOOT SEQUENCE                            │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. Developer starts MCP server:                            │
│     npm run server:stdio                                    │
│                    │                                        │
│                    ▼                                        │
│  2. Proactive Context Engine initializes:                  │
│     ┌─────────────────────────────────────────────────┐     │
│     │ • Enhanced Project Detector starts              │     │
│     │ • Scans authentication patterns                 │     │
│     │ • Analyzes API response formats                 │     │
│     │ • Detects error handling patterns               │     │
│     │ • Generates confidence scores                   │     │
│     │ • Collects evidence citations                   │     │
│     └─────────────────────────────────────────────────┘     │
│                    │                                        │
│                    ▼                                        │
│  3. CLAUDE.md Maintainer activates:                        │
│     ┌─────────────────────────────────────────────────┐     │
│     │ • Generates implementation patterns section     │     │
│     │ • Creates critical guardrails                   │     │
│     │ • Updates with confidence indicators            │     │
│     │ • Uses atomic marker-based updates              │     │
│     └─────────────────────────────────────────────────┘     │
│                    │                                        │
│                    ▼                                        │
│  4. MCP Server ready for stdio connections                 │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Runtime Pattern Updates

```
┌─────────────────────────────────────────────────────────────┐
│                   RUNTIME UPDATES                           │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  File Change Detection:                                     │
│  ┌─────────────────────────────────────────────────────┐     │
│  │ 1. ContextWatcher detects file changes              │     │
│  │    ├─ Authentication files modified                 │     │
│  │    ├─ API endpoint changes                          │     │
│  │    └─ Error handling updates                        │     │
│  │                                                     │     │
│  │ 2. Change queue processes updates                   │     │
│  │    ├─ Debouncing (500ms)                           │     │
│  │    ├─ Pattern re-analysis                          │     │
│  │    └─ Confidence re-scoring                        │     │
│  │                                                     │     │
│  │ 3. CLAUDE.md Maintainer updates                    │     │
│  │    ├─ Atomic marker-based updates                  │     │
│  │    ├─ Preserves existing content                   │     │
│  │    └─ Updates only changed patterns                │     │
│  └─────────────────────────────────────────────────────┘     │
│                                                             │
│  Claude Code stays automatically informed via CLAUDE.md    │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## stdio Integration Details

### MCP Server Configuration

```javascript
// mcp-server-stdio.js
const { ProactiveContextEngine } = require('./src/proactive-context-engine');

class StdioMCPServer {
  constructor() {
    this.contextEngine = new ProactiveContextEngine();
    this.setupStdioTransport();
  }

  setupStdioTransport() {
    // JSON-RPC 2.0 over stdin/stdout
    process.stdin.on('data', this.handleMCPMessage.bind(this));
    this.contextEngine.on('claudemd-updated', this.notifyContextUpdate.bind(this));
  }

  async handleMCPMessage(data) {
    const message = JSON.parse(data.toString());
    // Handle MCP protocol messages
    const response = await this.processRequest(message);
    process.stdout.write(JSON.stringify(response) + '\n');
  }
}
```

### Claude Code Integration Commands

```bash
# Register stdio MCP server
claude mcp add cortex /path/to/mcp-server-stdio.js

# Verify registration
claude mcp list
# Expected output: cortex: /path/to/mcp-server-stdio.js (stdio)

# Start Claude Code with MCP tools
claude chat --mcp
```

## Data Flow Sequence (stdio)

```
┌─────────────┐    ┌──────────────┐    ┌──────────────────┐
│ Claude Code │    │ MCP Server   │    │ Proactive Engine │
│             │    │ (stdio)      │    │                  │
└─────────────┘    └──────────────┘    └──────────────────┘
       │                   │                      │
       │ JSON-RPC Request  │                      │
       ├──────────────────►│                      │
       │   (via stdin)     │                      │
       │                   │  Query Patterns      │
       │                   ├─────────────────────►│
       │                   │                      │
       │                   │  Implementation      │
       │                   │  Patterns + Evidence │
       │                   │◄─────────────────────┤
       │                   │                      │
       │ JSON-RPC Response │                      │
       │◄──────────────────┤                      │
       │   (via stdout)    │                      │
       │                   │                      │
       │                   │   File Change Event  │
       │                   │◄─────────────────────┤
       │                   │                      │
       │                   │   Update CLAUDE.md   │
       │                   ├─────────────────────►│
       │                   │                      │
```

## Implementation Benefits (stdio vs HTTP)

### Advantages of stdio Transport

1. **Lower Latency**: Direct pipe communication vs network stack
2. **No Port Management**: No need to manage HTTP ports or conflicts
3. **Process Lifecycle**: Automatic cleanup when parent process exits
4. **Security**: No network exposure, local process communication only
5. **Simplicity**: Standard JSON-RPC 2.0 over pipes

### Performance Comparison

```
Transport Type    │ Latency    │ Overhead │ Security │ Complexity
─────────────────┼───────────┼──────────┼──────────┼───────────
stdio             │ < 1ms      │ Minimal  │ High     │ Low
HTTP (localhost)  │ 2-5ms      │ Higher   │ Medium   │ Medium
```

## CLAUDE.md Auto-Generated Section Example

```markdown
<!-- cortex:auto:begin:implementation-patterns -->
## Implementation Patterns (Auto-detected)

### Authentication (Confidence: 95%)
- **User Property**: `req.user` (Evidence: middleware/auth.ts:15, routes/protected.ts:8)
- **Token Storage**: httpOnly cookies (Evidence: auth/jwt.ts:23, middleware/auth.ts:41)
- **Error Response**: `{error: string}` with 401 status (Evidence: middleware/auth.ts:67)

### API Responses (Confidence: 90%)
- **Success Format**: `{data: any}` wrapper (Evidence: utils/response.ts:12, routes/*.ts)
- **Error Format**: `{error: string, code?: number}` (Evidence: middleware/error.ts:19)

### Critical Guardrails
- ❌ NEVER use localStorage for tokens (project uses httpOnly cookies)
- ✅ ALWAYS wrap API responses in `{data: any}` format
- ✅ ALWAYS use `req.user` for authenticated user data
<!-- cortex:auto:end:implementation-patterns -->
```

## Success Metrics

### Prevention Effectiveness
- **Wrong Suggestion Reduction**: Target 30% fewer implementation-specific errors
- **Pattern Consistency**: 95%+ adherence to detected patterns
- **First Interaction Accuracy**: Correct guidance from first Claude Code query

### Context Freshness
- **Boot-time Readiness**: CLAUDE.md populated before first interaction
- **Update Speed**: Pattern changes reflected within 10 seconds
- **Evidence Quality**: All assertions backed by file citations

### stdio Performance
- **Message Latency**: < 1ms for MCP communication
- **Process Stability**: Zero orphaned processes
- **Memory Efficiency**: Shared context engine across sessions