#!/bin/sh
# Wake an idle agent session when the board has a new notification for it.
# usage: board-wait.sh PROJECT AGENT_NAME
# Run it as a BACKGROUND task: it prints the new notifications and exits, and the
# end of a background task is what re-invokes an idle session. It only peeks
# (board_notifications); the agent acknowledges with board_receive after reading.
P=$1; A=$2; BOARD=/Users/aros/agent-board/bin/board.js
export PATH=/opt/homebrew/bin:$PATH
ids() { node $BOARD as "$P" "$A" board_notifications 2>/dev/null | python3 -c "
import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for n in d.get('notifications',[]): print(n['id'], n.get('thread_id'), (n.get('body') or '')[:300].replace('\n',' '))"; }
seen=$(ids | cut -d' ' -f1 | sort)
while :; do
  sleep 15
  now=$(ids)
  new=$(printf '%s\n' "$now" | while read -r id rest; do [ -n "$id" ] && ! printf '%s\n' "$seen" | grep -qx "$id" && echo "$id $rest"; done)
  [ -n "$new" ] && { echo "[board] new for $A:"; echo "$new"; echo "Read with board_inbox, act, acknowledge with board_receive, then start this waiter again."; exit 0; }
done
