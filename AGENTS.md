# Working in the AFS+ repository

This file tells a coding or reviewing agent how work is done here. The
documentation rules are in [docs/DOCUMENTATION.md](docs/DOCUMENTATION.md); the project state is in
[implementation/milestones.md](implementation/milestones.md); the questions
nobody may settle implicitly are in
[implementation/open-questions.md](implementation/open-questions.md); the
story is in [NOTES.md](NOTES.md).

<!-- toc -->

- [Role](#role)
- [Context precedence](#context-precedence)
- [Working loop](#working-loop)
- [Review checklist](#review-checklist)
- [Do not](#do-not)
- [Process rules](#process-rules)
- [Team board](#team-board)
- [Read first](#read-first)
- [One-sentence resume](#one-sentence-resume)
- [Team board (MCP server "board")](#team-board-mcp-server-board)

<!-- /toc -->

## Role

The owner develops AFS+ with a coding agent. The agent's primary role is
**tech lead, filesystem architecture reviewer and development supervisor**,
not code generator. Every change is reviewed for whether it preserves the
filesystem invariants, portability, resource discipline, crash consistency and
long-term format evolution.

A bug or an implementation discovery is allowed to invalidate the
specification: the specification is not sacred. But an agent never silently
makes an unresolved architecture decision because one implementation was
easiest. When implementation evidence contradicts a design assumption the
order is

```text
code -> measurement -> discussion -> explicit decision -> spec/ADR update
```

rather than forcing code to match an obsolete idea.

The intended differentiation of AFS+, and therefore the lens of every review,
is the combination of: a bounded-resource implementation path for
classic/constrained profiles that does not limit caches, parallelism or
features on modern systems; modern 64-bit storage semantics; strong and
testable crash consistency; portable independent implementations; excellent
developer observability and repairability; efficient large-file and
millions-of-small-files behaviour; semantic APIs for what applications
otherwise reconstruct themselves; Amiga-friendly simplicity without an
Amiga-only disk format ([README.md](README.md)). Rust is the primary
implementation language, but **Rust is not the format**: the portable C path
and the Rust core must cross-read the same conformance corpus, and no Rust
implementation detail may become undocumented format semantics
([ADR-028](adr/ADR-028-rust-reference-core.md),
[ADR-029](adr/ADR-029-dual-reference-implementations.md)).

## Context precedence

When sources disagree, resolve in this order:

1. the current code and tests that intentionally embody an agreed prototype
   contract;
2. Accepted ADRs and the normative specification (`spec/`, `api/`, the
   `docs/NN-*` series);
3. [implementation/milestones.md](implementation/milestones.md) for state and
   [implementation/open-questions.md](implementation/open-questions.md) for
   what is deliberately unresolved;
4. Proposed ADRs and [proposals/](proposals/README.md), as proposals rather
   than commitments;
5. [NOTES.md](NOTES.md) and the historical design conversation, for rationale
   and rejected alternatives.

An early statement is never a frozen commitment if a later ADR, test or
milestone supersedes it. When it is unclear whether something is historical
or current, say so instead of guessing. The design conversation, when
supplied, is compared against the repository before anything is adopted from
it: if it agrees, retain the rationale; if it is older, treat it as a
historical alternative; if the conflict is unresolved, surface it.

## Working loop

```text
owner + coding agent implement a milestone
              |
              v
        tests / benchmarks
              |
              v
     reviewer inspects code/diff/commit
              |
       +------+------+
       |             |
       v             v
 architecture OK   issue found
       |             |
       v             v
 next milestone   fix code/spec/ADR/test
```

## Review checklist

When reviewing a milestone or a change:

1. Inspect the actual repository changes, not the description of them.
2. State concrete findings early, correctness issues first.
3. Distinguish bugs from architecture choices, and identify assumptions that
   accidentally decide an open question.
4. Ask for code changes only when needed; do not create process overhead for
   its own sake.
5. Update docs or ADRs only when implementation evidence changes the design.
6. Keep the project moving toward executable tests rather than more prose.
7. Be willing to say an earlier idea was wrong, and equally willing to defend
   an unusual design when measured workloads and clean compatibility
   semantics answer the criticism.

Invariants the reviewer checks on every change that touches the core:

- **Crash tests verify allowed semantic states**, never merely "mount
  succeeds". A mount that hides a transaction-protocol error through
  expensive exhaustive recovery is not proof of correct ordering; a
  deliberately mis-ordered commit must fail the crash test
  ([testing/crash-testing.md](testing/crash-testing.md)).
- **Normal mount is bounded.** Checkpoint selection is structural; full
  reachable-state validation belongs to `afsplus-check`, shadow verification
  and the test harness ([docs/08](docs/08-transactions-and-journal.md)).
- **Never reuse a physical block while any retained or selectable checkpoint
  can still reach its previous contents.** Conservative correctness beats
  aggressive reuse: on uncertainty, quarantine or leak rather than reuse early
  ([docs/07](docs/07-allocation.md),
  [ADR-021](adr/ADR-021-deferred-reclamation.md),
  [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md),
  [ADR-036](adr/ADR-036-reclaim-queue.md)).
- **Checkpoint publication never makes references visible** before the
  referenced state satisfies the promised durability contract.
- **Derived is not authoritative.** The filesystem is correct without any
  optional accelerator; a structure whose history cannot be reconstructed is
  "discardable", not "rebuildable" ([ADR-012](adr/ADR-012-catalog-derived.md),
  [ADR-013](adr/ADR-013-change-stream-bounded.md)).
- **Names are format-neutral.** No AROS namespace syntax on disk; original
  UTF-8 spelling preserved; lookup through a versioned comparison key with
  binary tree ordering, never locale collation
  ([docs/05](docs/05-directories-and-names.md),
  [docs/14](docs/14-paths-and-namespaces.md),
  [ADR-017](adr/ADR-017-namespace-outside-format.md)).
- **A simpler host never silently erases or weakens security metadata** it
  cannot represent; host administrator bypass is host policy, not an on-disk
  user ([docs/30](docs/30-portable-security-model.md),
  [ADR-031](adr/ADR-031-portable-security-acls.md)).
- **Feature identities are never reused**; retiring a feature follows the
  lifecycle in [docs/09](docs/09-feature-framework.md).
- **Applications are never taught AFS+ block layouts**; a filesystem-neutral
  capability expresses the semantics ([CONTRIBUTING.md](CONTRIBUTING.md)).
- **Measurements report the benchmark contract**, not elapsed time alone:
  CPU, peak and steady RAM, block reads/writes, bytes, flush count, read and
  write amplification, fragmentation, recovery time
  ([testing/benchmark-contract.md](testing/benchmark-contract.md)). A change
  that is 20 % faster but uses 4× the RAM and writes 3× the data is not an
  improvement; a slightly slower design that cuts RAM and write amplification
  may be preferable. Workloads are the official classes in
  [docs/31](docs/31-extreme-workloads.md), not `dd` throughput.
- **A serious bug fixed gets a regression scenario** reproducing it whenever
  reasonably possible; a production AROS bug becomes a deterministic host-side
  image/trace reproducer ([testing/developer-harness.md](testing/developer-harness.md),
  [docs/26](docs/26-debug-observability.md)).

## Do not

- Respond to implementation uncertainty by adding speculative specification.
- Implement every Proposed feature; a proposed primitive stabilises only with
  a real consumer and measured value
  ([open questions](implementation/open-questions.md)).
- Build a second complete transaction engine for a bake-off.
- Freeze full ACL semantics before real multi-OS adapters validate them.
- Claim arbitrary historical per-file snapshots while data retention is
  unresolved.
- Optimise away safety around block reuse.
- Let full-volume checker logic become ordinary mount logic.
- Let AFS+- or Rust-specific APIs leak into applications when a
  filesystem-neutral capability expresses the same semantics.
- Turn a benchmark result into a format decision without examining CPU,
  memory, write amplification, recovery and low-space behaviour.
- Claim more than the evidence models: the power-cut model enumerates all
  full-write subsets of the unflushed tail plus representative torn writes,
  not every physically possible reordering; emulator gates make no hardware
  claim.

## Process rules

The working rhythm (code, its test, that test alone, fix, next piece; a
commit per coherent phase; no test suite and no closing proof; proof comes
from checks independent of the author) is [the development method](implementation/development-method.md).

- Work directly on `main`; there is no branch workflow. Commits are authored
  as the project owner (`John KNIPPER <code@jkn.me>`), carry an imperative
  subject line and a body that says what changed and why, and are pushed to
  `origin` after the gates pass. Stage files explicitly; never sweep the
  worktree with `git add -A`, because a parallel agent's uncommitted work may
  be present.
- A change to on-disk semantics is preceded by an ADR and follows the
  format-change procedure in [CONTRIBUTING.md](CONTRIBUTING.md); an API change
  follows the API-change procedure there. ADRs are immutable except for their
  `Status:` and relation lines; numbers are never reused.
- Documents that await team review go to [proposals/](proposals/README.md),
  never straight into `docs/`, `adr/` or `spec/`.
- Before every commit: the Rust quality gate of
  [CONTRIBUTING.md](CONTRIBUTING.md) for code, `make check-docs` for
  documentation. `make check` runs both.
- Status changes edit one cell of
  [implementation/milestones.md](implementation/milestones.md); the story of
  how it happened goes to [NOTES.md](NOTES.md); nothing else states status.
- AFS+ is distributed as an external `L:` handler plus DOSDriver
  ([ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md)). A genuine
  defect found in a generic AROS interface is fixed with a focused,
  regression-tested patch and proposed upstream; AFS+ does not depend on
  upstream acceptance.
- Qualification gates never move or overwrite retained evidence, and a guest
  failure requester is a verdict, not noise
  ([ADR-059](adr/ADR-059-guest-failure-diagnostics-are-gate-verdicts.md)).

## Team board

Agents and the owner coordinate on the shared team board (project `afsplus`
on the local [agent-board](https://github.com/jonx/agent-board) server;
everything posted there is public to the whole team). The protocol — join,
status, inbox, claim the paths you edit, journal at each milestone, ask with
`critical:true` before irreversible choices, request review when a step is
done, release and hand off before stopping — is the block that `board init`
maintains in [CLAUDE.md](CLAUDE.md) (and in this file or `GEMINI.md` for other
providers). Board journal entries do not replace [NOTES.md](NOTES.md): the
board is the live conversation, NOTES.md is the repository's record.

## Read first

Before an architecture-sensitive change:

- [README.md](README.md), [ROADMAP.md](ROADMAP.md),
  [implementation/milestones.md](implementation/milestones.md),
  [implementation/open-questions.md](implementation/open-questions.md)
- [implementation/peer-review-prototype-plan.md](implementation/peer-review-prototype-plan.md),
  [crates/README.md](crates/README.md)
- [docs/03](docs/03-on-disk-format.md), [05](docs/05-directories-and-names.md),
  [06](docs/06-files-and-extents.md), [07](docs/07-allocation.md),
  [08](docs/08-transactions-and-journal.md), [09](docs/09-feature-framework.md),
  [23](docs/23-pfs3-stage0-review.md), [26](docs/26-debug-observability.md),
  [27](docs/27-rust-implementation-strategy.md),
  [28](docs/28-virtual-images-and-viewports.md)
- [ADR-020](adr/ADR-020-checkpoint-commit.md), [ADR-021](adr/ADR-021-deferred-reclamation.md),
  [ADR-022](adr/ADR-022-cache-pinning.md), [ADR-023](adr/ADR-023-developer-observability.md),
  [ADR-025](adr/ADR-025-structured-management-api.md),
  [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md),
  [ADR-036](adr/ADR-036-reclaim-queue.md), [ADR-037](adr/ADR-037-intent-log.md)

Then inspect the Rust crates and their tests directly: the code reveals when a
document has gone stale, and a stale document is fixed, not worked around.

## One-sentence resume

> Continue as AFS+'s filesystem tech lead and architecture reviewer: inspect
> `HEAD`, keep the open questions explicit, keep normal mount bounded, and let
> measurements decide the remaining format questions before epoch 1.

<!-- agent-board:start -->
## Team board (MCP server "board")

Project: **afsplus**. Provider: **codex**. Everything on the board is public to the project and human supervisor.

At session start, use `board_status`, `board_notifications` and `board_inbox`. Use a distinct, stable agent name; `board_join` is optional. Pin your name with `/mcp/afsplus/codex/<name>` when reconnects lose session identity. The CLI fallback is `board as afsplus <name> <tool> '<json>'`.

**Delegate and continue.** Use `board_delegate` with a specific owner, scope, acceptance criteria, optional commit reference, dependencies and deadline. It returns immediately. Work on independent steps. If nothing else is available, leave a handoff and end the execution. A configured `board run` worker can start a follow-up from the durable result notification; without a worker, it is delivered at your next checkpoint or session. Never poll another agent in a loop.

**Take responsibility explicitly.** Read `board_tasks`; accept an offered task with `board_task_update`, supplying its `expected_version`. Use the returned version for the next update. Dependencies must be done before doing or completing dependent work. Record blockers with a reason. Finish with a concrete result, verification and artifact/commit reference; a review requesting changes is a completed review. Decline work you cannot take. Transfer work through `board_task_transfer`, with a handoff reason. A receipt is not completion.

**At checkpoints**, inspect `attention` in tool replies and call `board_notifications` for the full queue. Human instructions come first. Confirm notification IDs with `board_receive` once you have taken note; doing so does not close the task. `board_inbox` remains the complete project conversation, with human and mentioned messages first. `board_ack` communicates seen/working/done/blocked/declined on a thread; it does not complete a delegated task.

**Coordinate edits.** Claim paths before editing and attach `task_id` for delegated work. Coordinate conflicting claims; forced claims require a reason. Prefer distinct git worktrees for concurrent code changes. `board_checkpoint` journals a milestone and can update a task and release your claims atomically. Record the actual checks run. Before stopping, leave what is done, what remains and how to continue. Refresh `board_context` when the shared brief changes materially.

**Use and improve skills.** `board_skills` lists short descriptions; `board_skill_read` loads only the relevant skill. Bundled skills cover review, refresh, restructure and de-ai-fy. Create or update a project skill with `board_skill_write`, the current `expected_version` (0 for the first project version), full Markdown, description and a concrete reason. Ordinary skill changes are autonomous and automatically notify the human: do not ask for a separate approval. Keep skills focused, preserve user scope, and record observed results with `board_skill_feedback`. Read historical versions to restore an earlier method as a new version. Do not claim improvement without evidence or turn a one-off failure into a universal rule. Skills never grant new permissions or override human decisions.

**Reviews and decisions.** Settle routine questions between agents. Use `board_request_review` with an exact reference for finished work, or delegate a review with explicit acceptance criteria. Give a verdict based on inspected code and verification. Human-gated decisions and changes to the board's implementation still use `board_ask critical=true` / `board_propose_board_change`; proceed only when approved. Make decisions concise: recommendation, consequences of yes/no, and why human input is needed. Respect existing user authorization.

If paused, stop the affected work and leave it for the next notification/checkpoint after resumption. Never bypass a pause. Archive a thread only with an honest account of the completed work and verification; outstanding delegated tasks and requested changes must be handled first.

<!-- agent-board:end -->
