# MCP Server Setup for Claude Code

## Overview

Model Context Protocol (MCP) enables Claude Code to connect to external tools and data sources. This guide covers setup and configuration.

**✅ Cortex MCP Server Status**: Updated to use proper JSON-RPC protocol with HTTP transport, compatible with Claude Code.

## Quick Start

### Check Current Status
```bash
# View configured servers
claude mcp

# Get help
claude mcp --help
```

### Add MCP Servers

Basic syntax:
```bash
claude mcp add <server-name> <server-path> [options]
```

## Common Server Types

### 1. PostgreSQL Database
```bash
claude mcp add postgres-server /path/to/postgres-mcp-server \
  --connection-string "postgresql://user:pass@localhost:5432/mydb"
```

### 2. GitHub Integration
```bash
claude mcp add github-server /path/to/github-mcp-server \
  --token "your-github-token"
```

### 3. File System Tools
```bash
claude mcp add filesystem-server /path/to/filesystem-mcp-server \
  --root-path "/your/project/root"
```

### 4. HTTP/REST APIs
```bash
claude mcp add api-server /path/to/http-mcp-server \
  --base-url "https://api.example.com" \
  --auth-header "Bearer your-token"
```

## Server Scopes

- **Local**: Personal, project-specific configurations
- **Project**: Team-shared configurations  
- **User**: Cross-project accessibility

Specify scope with `--scope`:
```bash
claude mcp add myserver /path/to/server --scope project
```

## Using MCP Resources

### Reference Resources
Use `@` mentions to reference MCP resources:
```
@database/users - Reference users table
@github/issue/123 - Reference GitHub issue
@docs/api - Reference documentation
```

### Slash Commands
Some MCP servers provide slash commands:
```
/db query "SELECT * FROM users"
/github create-issue "Bug report"
```

## Configuration Files

MCP configurations are stored in:
- Local: `.claude/mcp.json`
- Project: `claude_desktop_config.json`
- User: `~/.claude/mcp.json`

## Security Considerations

⚠️ **Important**: Only use trusted MCP servers. They have access to your development environment.

- Verify server sources
- Review server permissions
- Use authentication when available
- Limit server scope appropriately

## Troubleshooting

### Server Not Found
```bash
# Check server path exists
ls -la /path/to/mcp-server

# Verify executable permissions
chmod +x /path/to/mcp-server
```

### Connection Issues
```bash
# Test server directly
/path/to/mcp-server --test

# Check logs
claude mcp logs <server-name>
```

### Remove Servers
```bash
# Remove specific server
claude mcp remove <server-name>

# List all servers
claude mcp list
```

## Popular MCP Servers

| Server | Purpose | Installation |
|--------|---------|-------------|
| `@anthropic/mcp-postgres` | PostgreSQL access | `npm install @anthropic/mcp-postgres` |
| `@anthropic/mcp-github` | GitHub integration | `npm install @anthropic/mcp-github` |
| `@anthropic/mcp-filesystem` | File operations | `npm install @anthropic/mcp-filesystem` |
| `@anthropic/mcp-brave-search` | Web search | `npm install @anthropic/mcp-brave-search` |

## Cortex MCP Server

### Local Setup
1. **Start the server**:
   ```bash
   npm run build
   npm start
   # Server starts on http://localhost:8765
   ```

2. **Test the server**:
   ```bash
   # Basic test script
   node test-mcp.js 8765
   
   # OR use official MCP Inspector (recommended)
   npx @modelcontextprotocol/inspector
   ```

3. **Add to Claude Code**:
   ```bash
   claude mcp add cortex-server http://localhost:8765/mcp
   ```

### Available Tools
- `semantic_search` - Semantic code search using vector embeddings with file system persistence
- `contextual_read` - Read files with semantic context awareness  
- `code_intelligence` - High-level semantic codebase analysis

### File System Persistence Features
- **Incremental Indexing**: Only processes changed files using SHA-256 hashing
- **Fast Startup**: Loads existing embeddings from `.cortex/index.json`
- **Delta Detection**: Identifies added, modified, and deleted files automatically
- **Cache Management**: CLI tools for stats, clearing, validation, and backup

### Indexing Modes
```bash
# Start with incremental indexing (default - fastest)
npm start
# or explicitly
INDEX_MODE=incremental npm start

# Force full reindex (slower, but complete refresh)
INDEX_MODE=full npm start
```

### Cache Management
```bash
# View cache statistics
npm run cache:stats

# Clear embedding cache
npm run cache:clear

# Validate cache integrity
npm run cache:validate

# Backup cache
npm run cache:backup
```

### Usage in Claude Code
```
@cortex-server/semantic_search query="authentication logic"
@cortex-server/contextual_read file_path="src/auth.ts"
```

## Example Workflow

1. **Install external server**:
   ```bash
   npm install -g @anthropic/mcp-postgres
   ```

2. **Add to Claude Code**:
   ```bash
   claude mcp add postgres-db $(which mcp-postgres) \
     --connection-string "postgresql://localhost:5432/myapp"
   ```

3. **Use in conversation**:
   ```
   Show me the users table structure using @postgres-db/users
   ```

## Best Practices

- Start with one server and test thoroughly
- Use project scope for team-shared configurations
- Document server configurations in your project README
- Regular security reviews of configured servers
- Keep server software updated

## Testing with MCP Inspector

The official MCP Inspector provides comprehensive testing and debugging for MCP servers:

### Installation & Usage

For **HTTP-based servers** (like Cortex):
```bash
# Start your Cortex server first
npm start

# Then run inspector - it will auto-detect HTTP servers
npx @modelcontextprotocol/inspector
# Opens at http://localhost:6274 with authentication token
```

For **CLI mode** (programmatic testing):
```bash
npx @modelcontextprotocol/inspector --cli node dist/server.js
```

### Security & Authentication
- Inspector requires authentication with a session token
- Token is automatically generated and displayed in console
- Pre-filled URL provided for easy access

### Inspector Features
- **Interactive Web UI**: React-based interface at `http://localhost:6274`
- **Tools Testing**: Test `semantic_search`, `contextual_read`, `code_intelligence`
- **Resources & Prompts**: View available resources and prompt templates
- **Protocol Validation**: Ensure MCP specification compliance
- **Real-time Monitoring**: Server logs and message inspection
- **Multi-transport Support**: stdio, SSE, HTTP protocols

### Development Workflow
1. **Start server**: `npm start` (Cortex on port 8765)
2. **Launch inspector**: `npx @modelcontextprotocol/inspector`
3. **Open UI**: Use the provided URL with pre-filled token
4. **Connect**: Inspector auto-detects your HTTP server
5. **Test tools**: Interactive testing with custom inputs
6. **Debug**: Monitor logs and validate protocol messages

### Configuration Files
For complex setups, use config files:
```bash
npx @modelcontextprotocol/inspector --config cortex-config.json
```

## Troubleshooting

### Server Won't Start
```bash
# Check if port is available
netstat -tlnp | grep 8765

# Check for TypeScript errors
npm run build

# View logs
tail -f logs/cortex-server.log
```

### MCP Connection Issues
```bash
# Test server directly
curl -X POST http://localhost:8765/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"tools/list","params":{},"id":1}'

# Check Claude Code MCP configuration
claude mcp list

# Remove and re-add server
claude mcp remove cortex-server
claude mcp add cortex-server http://localhost:8765/mcp
```

### Tool Call Failures
- Ensure repository is properly indexed
- Check semantic search embeddings are loaded
- Verify file paths exist for `contextual_read`
- Review logs for specific error messages

## Next Steps & Development Roadmap

### Phase 1: Claude Code Integration 🎯
**Priority: High**

1. **Connect to Claude Code**:
   ```bash
   claude mcp add cortex-server http://localhost:8765/mcp
   ```

2. **Test Integration**:
   ```
   @cortex-server/semantic_search query="authentication logic"
   @cortex-server/contextual_read file_path="src/server.ts"
   @cortex-server/code_intelligence task="analyze MCP implementation"
   ```

3. **Validate All Tools**:
   - ✅ `semantic_search` - Tested and working with file system persistence
   - ⏳ `contextual_read` - Test with actual file paths
   - ⏳ `code_intelligence` - Test with analysis tasks
   - ✅ `embedding_persistence` - Implemented with incremental indexing

### Phase 2: Production Readiness 🚀
**Priority: Medium**

1. **Error Handling & Validation**:
   - Input parameter validation with Zod schemas
   - Comprehensive error responses
   - Request timeout handling
   - Graceful degradation

2. **Security & Performance**:
   - Rate limiting per client
   - Authentication/authorization
   - Request logging and monitoring
   - Memory usage optimization

3. **Deployment Configuration**:
   - Docker containerization
   - systemd service setup
   - Environment variable management
   - Health check endpoints

### Phase 3: MCP Ecosystem Expansion 📡
**Priority: Medium**

1. **Database Integration**:
   ```bash
   # Add PostgreSQL MCP server
   npm install -g @anthropic/mcp-postgres
   claude mcp add postgres-db $(which mcp-postgres) \
     --connection-string "postgresql://localhost:5432/cortex"
   ```

2. **GitHub Integration**:
   ```bash
   # Add GitHub MCP server
   npm install -g @anthropic/mcp-github
   claude mcp add github-cortex $(which mcp-github) \
     --token "$GITHUB_TOKEN"
   ```

3. **File System Operations**:
   ```bash
   # Add filesystem MCP server
   npm install -g @anthropic/mcp-filesystem
   claude mcp add filesystem-cortex $(which mcp-filesystem) \
     --root-path "/home/yanggf/a/cortexyoung"
   ```

### Phase 4: Advanced Features ⚡
**Priority: Low**

1. **Enhanced Code Intelligence**:
   - Multi-repository analysis
   - Code dependency mapping
   - Automated refactoring suggestions
   - Security vulnerability detection

2. **Collaborative Features**:
   - Team shared contexts
   - Code review automation
   - Documentation generation
   - Change impact analysis

3. **Integration Ecosystem**:
   - VS Code extension
   - CI/CD pipeline integration
   - Slack/Discord notifications
   - Custom webhook support

### Implementation Checklist

**Phase 1 Tasks:**
- [ ] Connect to Claude Code (`claude mcp add`)
- [ ] Test `contextual_read` with real files
- [ ] Test `code_intelligence` with analysis tasks
- [ ] Validate error handling and edge cases
- [x] **Implement file system persistence for embeddings with incremental indexing**

**Phase 2 Tasks:**
- [ ] Add input validation with Zod
- [ ] Implement rate limiting
- [ ] Add authentication layer
- [ ] Create Docker configuration
- [ ] Setup monitoring and logging

**Phase 3 Tasks:**
- [ ] Install and configure PostgreSQL MCP server
- [ ] Setup GitHub MCP server with repository access
- [ ] Configure filesystem MCP server
- [ ] Test multi-server workflows

**Phase 4 Tasks:**
- [ ] Design advanced code intelligence features
- [ ] Build collaborative tools integration
- [ ] Create comprehensive API documentation
- [ ] Develop community contribution guidelines

## Resources

- [Official MCP Documentation](https://docs.anthropic.com/en/docs/claude-code/mcp)
- [MCP Server Registry](https://github.com/anthropics/mcp-servers)
- [Community Servers](https://github.com/topics/mcp-server)
- [Cortex MCP Server](docs/mcp-setup.md) - This documentation
- [MCP Inspector Config](mcp-inspector-config.json) - Local testing configuration