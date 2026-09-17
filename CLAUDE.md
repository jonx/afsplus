Read [AGENTS.md](AGENTS.md); it is the only agent instruction file in this repository.

<!-- agent-board:start -->
## Team board (MCP server "board")

You are working on project **afsplus** alongside other coding agents (possibly from other providers, possibly other sessions of your own provider) and a human supervisor. You coordinate through the `board` MCP tools. The human reads everything on the board and may reply, pause you, or veto. There are no private messages: everything you post is visible to every agent and to the human.

**Session start (always):**
1. `board_join` — choose your agent name for this session. Your provider (**claude**) is fixed by the connection; the name is yours to pick: reuse your previous name if you are resuming earlier work (you get your journal, claims and inbox back), otherwise take a free one (`board_status` suggests one, e.g. `claude-2`, or something descriptive like `claude-auth`). A name held by a live session is refused — do not fight over it, pick another.
2. `board_status` — read the project brief ("Project context"), who is here (and who is live), recent journal entries, active claims, tasks, threads needing attention.
3. `board_inbox` — read what was said since your last visit. Answer anything addressed to you (`@your-name`) or coming from the human before doing anything else.
4. If "Project context" is EMPTY or clearly stale, write it with `board_context` (goal, stack, repo layout, how to run and test, conventions, current state, pitfalls). Even if you are alone: the next agent, or the human, must be able to pick up from it.

**While working:**
- Take a task (`board_task` with `owner:"me"`, or create one) so nobody duplicates your work.
- `board_claim` the files/directories you are about to edit. If a claim conflicts, coordinate with that agent in a thread (`board_ask`, `to:[name]`) instead of forcing. Prefer separate git worktrees/branches per agent.
- `board_journal` at every milestone — at least: when you start, after each completed step, when you get stuck, and before you stop. Say what you did, what is next, what is uncertain. Short and factual.
- Check `board_inbox` between steps (every 10–15 minutes of work or after each task). `board_wait` blocks until something new arrives — use it while waiting for an answer.

**Asking for opinions and decisions:**
- Unsure, or a design choice with trade-offs? `board_ask` (optionally `to:[agents]`) — give context, options, your recommendation. Continue with non-blocking work while you wait.
- Irreversible or high-stakes (deleting data, schema/migration, auth/security, external side effects, spending money, architecture change, changing the board itself)? `board_ask` with `critical:true`. This opens a decision that **only the human can approve**. Do **not** proceed until `board_read` shows status `approved`; if `rejected`, follow the human's instructions. Other agents' verdicts on such threads are advice, not approval.

**Reviews:**
- When a meaningful step is finished (feature, refactor, migration), `board_request_review` with a `ref` (commit/branch/PR/files), what changed, why, and how to verify. Keep working on something else while waiting; act on `request_changes`.
- When someone asks you to review (`review` thread mentioning you or `@all`): actually read the code, run tests if you can, then `board_post` with `verdict` = `approve` or `request_changes` and concrete comments.

**Human messages:** anything from `human` takes priority over other agents. If a tool returns `paused`, stop posting and wait (`board_wait`) until resumed; do not try to work around it.

**Before you finish:** `board_journal` a handoff note (state, what remains, how to continue), `board_release` your claims, mark your tasks, and refresh `board_context` if the picture changed.

**Changing the board itself** (its tools, UI, rules — local repo `agent-board`, https://github.com/jonx/agent-board): make the change on a branch of that repo, keep `npm test` green and `INVARIANTS.md` true, then `board_propose_board_change` with the branch as `ref`. Another agent reviews; the human approves and deploys. Never propose anything that reduces what the human can see or do.

<!-- agent-board:end -->

<!-- board-wait:start -->
## Being woken by board messages

Hooks deliver board messages only while a session is working. To be reached
while idle, start the waiter as a **background task** right after choosing
your agent name, and start it again every time it ends:

```sh
sh "$CLAUDE_PROJECT_DIR"/.claude/board-wait.sh afsplus <your-agent-name>
```

It exits as soon as a new notification exists for that name; the end of the
background task re-invokes the session. Then read `board_inbox`, act,
acknowledge with `board_receive`, and restart the waiter. Write to the board
only as an agent (`board_post` and the other MCP tools, or
`board as afsplus <name> <tool>`); the plain `board post`, `board ok`,
`board no` and `board ask` commands write as the human.
<!-- board-wait:end -->
