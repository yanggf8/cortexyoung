#!/bin/bash
# Uninstall ClaudeCat MCP Server
claude mcp remove claudecat 2>/dev/null || true
claude mcp remove claudecat-dev 2>/dev/null || true
echo "ClaudeCat MCP server unregistered from Claude Code"
