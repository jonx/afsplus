<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright (c) 2026 John Knipper -->

# Development method

How a change is built, proven and delivered in this repository. Every agent
and contributor follows this loop; [AGENTS.md](../AGENTS.md) owns the review
checklist and the invariants, this page owns the working rhythm.

## The loop

1. Write the code for one piece of work.
2. Write the test that proves that piece: exact expected states, literal
   expectations written independently of the inputs the test feeds in, a
   negative control that fails when the expectation is deliberately wrong.
3. Run that test, and only that test (`cargo test -p CRATE --test FILE NAME`).
4. Fix the code and the test until the test passes.
5. Move to the next piece. Commit at the end of a coherent phase (a
   family, a feature, a fixed defect with its reproducer), with a subject
   and a body that say what changed and why.

Add as many tests as the work needs. A test that proves nothing new is
noise; a test that lowers a criterion to pass is a defect.

## What is never run during development

No test suite, whatever its length, after a change: no workspace run, no
crate-wide run, no "quick" tier. Nothing has been released, so there is
nothing to regress against. Non-regression tests enter the repository when a
real problem appears and needs pinning, with the reproducer that found it.

## The milestone proof

A stage, gate or release closes on one complete run over the final sources:
formatting, the workspace tests, Clippy, the codec fuzz gate, the Python
suites, the documentation checker and the whitespace check, followed by the
generated-family campaigns. The run:

- starts detached from the editing session and at normal priority, with its
  output on disk under `build/<name>/`;
- records the source identity before and after, so a change during the run
  voids the verdict;
- never stops early: `--no-fail-fast`, every step executed to the end, the
  complete list of failing tests written to `failing-tests.txt`.

On a red run: fix every listed failure at once, rerun only those tests, then
run the complete proof once more. On a green run: the status cell in
[milestones](milestones.md) cites the retained `build/` directory, the story
goes to [NOTES.md](../NOTES.md), commit, push.

## Lots and agents

Parallel work happens in one git worktree and one branch per lot, from the
same base commit. A lot:

- commits at the end of each coherent phase, so an interruption costs at
  most the phase in progress;
- keeps every tool call short and polls long runs with brief checks; nobody
  wakes an agent that ends its turn on a process it started;
- writes documentation in small steps;
- runs only the tests of what it builds;
- ends with a report: commits, files, commands with counts and durations,
  modeled counts, negative-control outcomes, log paths, defects found, and
  the exact list of what stays open with the reason.

A missing tool, SDK, toolchain or emulator is work to do, not a limit to
report. Install or build it, under the home directory and with a script that
resumes after an interruption, then carry on with the target run. "Proven on
the host only" is acceptable while that build is running, never instead of it.

The integrator merges lots by cherry-pick, resolving documentation conflicts
row by row, and runs nothing beyond a compile check until the milestone
proof. One heavy run occupies the machine at a time; three agents plus a full
run overload a 16 GB host.

## Documentation

Status lives only in [milestones](milestones.md); the story only in
[NOTES.md](../NOTES.md), as facts about the system. Finished-state prose,
no em dashes, no definition by negation, no journey wording, no attribution
of decisions to people. `make check-docs` runs before every commit, with its
exit code read directly.
