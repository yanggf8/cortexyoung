#!/bin/bash

# ClaudeCat Installation Script
# Installs CLI tool and optionally registers MCP server with Claude Code

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "Installing ClaudeCat..."
echo "Project root: $PROJECT_ROOT"

# Build the project
echo "Building..."
cd "$PROJECT_ROOT"
npm run build

# Check if build succeeded
if [ ! -f "$PROJECT_ROOT/dist/cli.js" ]; then
    echo "Build failed - dist/cli.js not found"
    exit 1
fi

# Make binaries executable
chmod +x "$PROJECT_ROOT/dist/cli.js"
[ -f "$PROJECT_ROOT/dist/stdio-mcp-server.js" ] && chmod +x "$PROJECT_ROOT/dist/stdio-mcp-server.js"

# Create symbolic links in ~/.local/bin
LOCAL_BIN="$HOME/.local/bin"
mkdir -p "$LOCAL_BIN"

# CLI tool (primary)
ln -sf "$PROJECT_ROOT/dist/cli.js" "$LOCAL_BIN/claudecat"
echo "Linked: ~/.local/bin/claudecat -> CLI tool"

# MCP server (legacy)
if [ -f "$PROJECT_ROOT/dist/stdio-mcp-server.js" ]; then
    ln -sf "$PROJECT_ROOT/dist/stdio-mcp-server.js" "$LOCAL_BIN/claudecat-mcp"
    echo "Linked: ~/.local/bin/claudecat-mcp -> MCP server"
fi

# Register MCP server with Claude Code (optional, uses claudecat-mcp name)
if command -v claude &> /dev/null; then
    echo "Registering MCP server with Claude Code..."

    # Remove old registrations
    claude mcp remove claudecat -s user 2>/dev/null || true
    claude mcp remove claudecat -s local 2>/dev/null || true
    claude mcp remove claudecat-mcp -s user 2>/dev/null || true

    NODE_PATH=$(which node)
    if [ -n "$NODE_PATH" ] && [ -f "$PROJECT_ROOT/dist/stdio-mcp-server.js" ]; then
        claude mcp add claudecat-mcp "$NODE_PATH" "$PROJECT_ROOT/dist/stdio-mcp-server.js"
        echo "MCP server registered as 'claudecat-mcp'"
    fi
else
    echo "Claude Code CLI not found - skipping MCP registration"
fi

echo ""
echo "Installation complete!"
echo ""
echo "CLI Usage:"
echo "  claudecat scan      # Analyze project patterns"
echo "  claudecat update    # Update CLAUDE.md with deep analysis"
echo "  claudecat status    # Show detected patterns"
echo "  claudecat --help    # Full usage info"
