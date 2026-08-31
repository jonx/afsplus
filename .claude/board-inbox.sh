#!/bin/sh
# Claude Code hook (SessionStart / UserPromptSubmit). Tells the agent what was posted on the
# board for PROJECT since *this Claude session* last looked (cursor keyed by the hook's
# session_id, read from stdin JSON) — no agent name or environment variable needed.
# If the board server is down, starts it (better: `board service install`).
# Installed by `board init`; usage: board-inbox.sh PROJECT
PROJECT="$1"
BOARD_URL="${BOARD_URL:-http://127.0.0.1:7777}"
BOARD_ROOT="/Users/jkn/Source/agent-board"
input=$(cat 2>/dev/null)
sid=$(printf '%s' "$input" | sed -n 's/.*"session_id" *: *"\([^"]*\)".*/\1/p')
[ -z "$sid" ] && sid=default
cur_dir="${TMPDIR:-/tmp}/agent-board-cursors"; mkdir -p "$cur_dir"
cur_file="$cur_dir/$PROJECT-$sid"
since=$(cat "$cur_file" 2>/dev/null || echo 0)
tmp=$(mktemp); code=$(curl -s -o "$tmp" -w '%{http_code}' --max-time 2 "$BOARD_URL/api/projects/$PROJECT/messages?since=$since&limit=20"); out=$(cat "$tmp"); rm -f "$tmp"
if [ "$code" = "404" ]; then
  # server up, project not created yet (first agent has not connected): just the welcome line
  echo "[board] This project uses the team board (MCP server 'board', project '$PROJECT'). Before anything else: board_join (choose your agent name), board_status, board_inbox. See the board section of CLAUDE.md."
  exit 0
fi
if [ "$code" != "200" ]; then
  if [ -f "$BOARD_ROOT/bin/board.js" ]; then
    port=$(printf '%s' "$BOARD_URL" | sed -n 's#.*:\([0-9][0-9]*\)/*$#\1#p'); [ -z "$port" ] && port=7777
    nohup node "$BOARD_ROOT/bin/board.js" serve --port "$port" >/dev/null 2>&1 &
    echo "[board] the board server was not running; started it in the background. If the 'board' MCP tools are unavailable, run /mcp to reconnect. (Install it permanently with: board service install)"
  else
    echo "[board] board server not reachable at $BOARD_URL — start it with \`board serve\`."
  fi
  exit 0
fi
last=$(printf '%s' "$out" | sed -n 's/.*"last_id":\([0-9]*\).*/\1/p')
count=$(printf '%s' "$out" | grep -o '"id":' | wc -l | tr -d ' ')
[ -n "$last" ] && echo "$last" > "$cur_file"
if [ "$since" = "0" ]; then
  echo "[board] This project uses the team board (MCP server 'board', project '$PROJECT'). Before anything else: board_join (choose your agent name), board_status, board_inbox. See the board section of CLAUDE.md."
  exit 0
fi
[ "$count" = "0" ] && exit 0
echo "[board] $count new message(s) on project '$PROJECT' since you last looked — call board_inbox before continuing. Preview:"
printf '%s' "$out" | head -c 3000; echo
