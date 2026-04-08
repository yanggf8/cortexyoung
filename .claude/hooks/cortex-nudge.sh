#!/usr/bin/env bash
# PreToolUse hook: nudge Claude to prefer `cortex search` over Grep when a cortex index exists.
# This is a soft nudge — it prints a message but exits 0 (non-blocking).

CONFIG="$HOME/.cortex/config.json"

# Only nudge for Grep tool calls
TOOL_NAME="${CLAUDE_TOOL_NAME:-}"
if [ "$TOOL_NAME" != "Grep" ]; then
  exit 0
fi

# Check if cortex is configured with a default project
if [ -f "$CONFIG" ]; then
  PROJECT_ID=$(python3 -c "import json; print(json.load(open('$CONFIG')).get('default_project_id',''))" 2>/dev/null)
  if [ -n "$PROJECT_ID" ]; then
    echo "Hint: A cortex index exists for this project. Consider \`cortex search\` for semantic results before falling back to Grep."
  fi
fi

exit 0
