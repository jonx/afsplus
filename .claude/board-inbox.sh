#!/bin/sh
# SessionStart, UserPromptSubmit and PostToolUse. Bounded check, no polling loop.
# Installed by board init; Node handles JSON, cursor locking and atomic writes.
# The installed directory is baked in: a hook has $CLAUDE_PROJECT_DIR, an agent
# running a command in a shell may not, so the waiter is named absolutely.
exec node "/Users/aros/agent-board/configs/claude-code/board-inbox.js" "$1" "/Users/aros/aros-next/research/src/afsplus"
