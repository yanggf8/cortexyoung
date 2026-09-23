#!/bin/bash
# Register development version with hot reload
claude mcp remove claudecat-dev 2>/dev/null || true
claude mcp add claudecat-dev "tsx /home/yanggf/a/claudecat/src/server.ts"
echo "Development server registered. Use 'npm run dev' to start with hot reload."
