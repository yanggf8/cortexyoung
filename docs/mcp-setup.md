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
   node test-mcp.js 8765
   ```

3. **Add to Claude Code**:
   ```bash
   claude mcp add cortex-server http://localhost:8765/mcp
   ```

### Available Tools
- `semantic_search` - Semantic code search using vector embeddings
- `contextual_read` - Read files with semantic context awareness  
- `code_intelligence` - High-level semantic codebase analysis

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

## Resources

- [Official MCP Documentation](https://docs.anthropic.com/en/docs/claude-code/mcp)
- [MCP Server Registry](https://github.com/anthropics/mcp-servers)
- [Community Servers](https://github.com/topics/mcp-server)
- [Cortex MCP Server](docs/mcp-setup.md) - This documentation