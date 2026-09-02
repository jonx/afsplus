# Intent log as an epoch-1 durability mechanism

> **ADRs:** [ADR-037](../adr/ADR-037-intent-log.md) · **Spec:** none ·
> **Tests:** [crash testing](../testing/crash-testing.md) · **Milestones:** M04, M14

Target on acceptance: a numbered ADR accepting the mechanism and amending
ADR-037; its current record encoding remains experimental. Decisions
requested: Q2-D1 through Q2-D3 below.

## Context and evidence

Checkpoint COW is the one metadata transaction engine. Group commit reduces
non-durable burst cost, but a forced durable ref update still costs 10 writes
and three flushes. The intent-log window records the already-staged logical
operations between checkpoints and measures 2.2 writes and 1.03 flushes per
update. The optimized crash suite accepts only monotone fsynced prefixes and
the checker validates every recovered image. The current requalification and
scope boundary are recorded in the
[fsync report](../implementation/fsync-intent-log-baseline.md).

## Proposed decision

Q2-D1: adopt **checkpoint COW + bounded group commit + a small intent log** as
the epoch-1 architecture. These are complementary layers, not two transaction
engines: group commit handles bursts, the log makes forced durability between
checkpoints cheap, and recovery materializes the recorded logical operations
through the same batch/COW engine.

Q2-D2: do not freeze the current log record encoding yet. Version 2 can replay
create/delete/rename and the bytes of a newly created file, but cannot encode a
write or truncate of an existing file. Until those operations exist, only the
implemented namespace window may advertise the cheap-fsync capability;
ordinary file-write fsync remains a full checkpoint.

Q2-D3: freeze the wire only after write/truncate records meet the existing
rules: fresh COW data extents are durable before the log record; a content
checksum rejects a durable record over torn data; replay claims exact extents,
updates file layout/reference state through the shared transaction engine, and
is idempotent under a second crash. In-place data is never journaled as if the
old generation were byte-stable.

## Rejected alternatives

- Checkpoint per fsync: its three-barrier structural floor is measured, not a
  tuning issue.
- Group commit alone: excellent for throughput, but it cannot acknowledge a
  caller's forced durability without publishing the batch.
- A second general redo-journal engine: duplicates allocation, replay and
  transaction semantics merely for a bake-off.
- Freeze record version 2 now: would permanently encode a namespace-only
  subset before the primary existing-file fsync use case is executable.

## Required follow-up before wire freeze

- design write/truncate log operations and their size/fragmentation bounds;
- qualify database rewrites, appends and write-then-rename sequences through
  the real logged path, not an estimated envelope;
- run every-write/every-flush and replay-during-replay crash matrices;
- specify log-full fallback and maximum fsync group behavior at the VFS API;
- cross-read the frozen records with the portable C implementation.
