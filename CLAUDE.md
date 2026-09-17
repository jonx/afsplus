Read [AGENTS.md](AGENTS.md); it is the only agent instruction file in this repository.

<!-- agent-board:start -->
## Team board (MCP server "board")

Project: **afsplus**. Provider: **claude**. Everything on the board is public to the project and human supervisor.

At session start, use `board_status`, `board_notifications` and `board_inbox`. Use a distinct, stable agent name; `board_join` is optional. Pin your name with `/mcp/afsplus/claude/<name>` when reconnects lose session identity. The CLI fallback is `board as afsplus <name> <tool> '<json>'`.

**Delegate and continue.** Use `board_delegate` with a specific owner, scope, acceptance criteria, optional commit reference, dependencies and deadline. It returns immediately. Work on independent steps. If nothing else is available, leave a handoff and end the execution. A configured `board run` worker can start a follow-up from the durable result notification; without a worker, it is delivered at your next checkpoint or session. Never poll another agent in a loop.

**Take responsibility explicitly.** Read `board_tasks`; accept an offered task with `board_task_update`, supplying its `expected_version`. Use the returned version for the next update. Dependencies must be done before doing or completing dependent work. Record blockers with a reason. Finish with a concrete result, verification and artifact/commit reference; a review requesting changes is a completed review. Decline work you cannot take. Transfer work through `board_task_transfer`, with a handoff reason. A receipt is not completion.

**At checkpoints**, inspect `attention` in tool replies and call `board_notifications` for the full queue. Human instructions come first. Confirm notification IDs with `board_receive` once you have taken note; doing so does not close the task. `board_inbox` remains the complete project conversation, with human and mentioned messages first. `board_ack` communicates seen/working/done/blocked/declined on a thread; it does not complete a delegated task.

**Coordinate edits.** Claim paths before editing and attach `task_id` for delegated work. Coordinate conflicting claims; forced claims require a reason. Prefer distinct git worktrees for concurrent code changes. `board_checkpoint` journals a milestone and can update a task and release your claims atomically. Record the actual checks run. Before stopping, leave what is done, what remains and how to continue. Refresh `board_context` when the shared brief changes materially.

**Use and improve skills.** `board_skills` lists short descriptions; `board_skill_read` loads only the relevant skill. Bundled skills cover review, refresh, restructure and de-ai-fy. Create or update a project skill with `board_skill_write`, the current `expected_version` (0 for the first project version), full Markdown, description and a concrete reason. Ordinary skill changes are autonomous and automatically notify the human: do not ask for a separate approval. Keep skills focused, preserve user scope, and record observed results with `board_skill_feedback`. Read historical versions to restore an earlier method as a new version. Do not claim improvement without evidence or turn a one-off failure into a universal rule. Skills never grant new permissions or override human decisions.

**Reviews and decisions.** Settle routine questions between agents. Use `board_request_review` with an exact reference for finished work, or delegate a review with explicit acceptance criteria. Give a verdict based on inspected code and verification. Human-gated decisions and changes to the board's implementation still use `board_ask critical=true` / `board_propose_board_change`; proceed only when approved. Make decisions concise: recommendation, consequences of yes/no, and why human input is needed. Respect existing user authorization.

If paused, stop the affected work and leave it for the next notification/checkpoint after resumption. Never bypass a pause. Archive a thread only with an honest account of the completed work and verification; outstanding delegated tasks and requested changes must be handled first.

**Stay reachable while idle.** Hooks deliver board messages only while you are working. Right after you have your agent name, start `sh /Users/aros/aros-next/research/src/afsplus/.claude/board-wait.sh <your-agent-name>` as a **background task** (never in the foreground), and start it again every time it ends. That one command is the whole cycle: it ends at once and hands you anything already unread, or it blocks until something new arrives and then hands you that. When it ends, read `board_inbox`, act, confirm with `board_receive`, and start it again. Without `board_receive` it ends at once again, which is deliberate. While it runs it is visible to the inbox hook, so if you forget, every checkpoint tells you that you are not reachable.

<!-- agent-board:end -->
