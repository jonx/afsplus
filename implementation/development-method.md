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

## What a test proves

A test written by the author of the code, at the same time as the code, shows
that the code does what its author had in mind. It does not show that what
the author had in mind is correct, and a green run of such tests teaches
nothing: the complete Stage A run, four and a half hours over 1,505 tests,
found no failure. Every real defect of Stage A came from a check that did not
share the author's assumptions:

- a separate model of the expected state, as in the generated operation
  families, which found the intent-log replay that reused a logged data run
  and the refusals that poisoned an open window;
- a second implementation, the portable C reader, reading what the Rust
  code wrote;
- a negative control, which shows that a check is able to fail;
- a reviewer who reads the code paths without having written them;
- the real target: the handler mounted under AROS, real programs, real power
  cuts.

Spend test effort there. Count a piece of work as proven when one of these
independent checks covers it, not when its own tests are green.

## What is never run, proposed or discussed

No test suite, whatever its length: no workspace run, no crate-wide run, no
"quick" tier, no closing proof at the end of a stage or milestone. Nothing
has been released, so there is nothing to regress against. Do not propose
such a run, do not plan one in a handoff, do not spend time making one
faster or measuring it. Non-regression tests enter the repository when a
real problem appears and needs pinning, with the reproducer that found it.

A lot is integrated on three things: the named tests of what it built, run
alone; formatting, Clippy and the documentation checker, which are compile
and lint checks; and an independent check from the list above, done or
named with its owner. The status cell in [milestones](milestones.md) cites
those, the story goes to [NOTES.md](../NOTES.md), commit, push.

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
row by row, and runs the named tests of each lot on the combined sources,
nothing more. One heavy build occupies the machine at a time; three agents
plus a toolchain build overload a 16 GB host.

## Documentation

Status lives only in [milestones](milestones.md); the story only in
[NOTES.md](../NOTES.md), as facts about the system. Finished-state prose,
no em dashes, no definition by negation, no journey wording, no attribution
of decisions to people. `make check-docs` runs before every commit, with its
exit code read directly.
