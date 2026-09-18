# ADR-063: Checkpoint COW plus an intent log for epoch-1 durability

Status: Accepted
Amends: ADR-009, ADR-020, ADR-026, ADR-037
Amended by: ADR-064, ADR-121

## Context

Checkpoint COW is the shared metadata transaction engine. Bounded group
commit amortizes non-durable bursts, but it cannot acknowledge a caller's
forced durability without publishing a checkpoint. The measured Git-style
atomic ref update costs 10 block writes and three flushes as one group-commit
transaction, or 27 writes and seven flushes through the original three
transactions.

ADR-037 prototypes a small logical intent log over the open batch window. A
fsync group appends one record and one barrier; recovery replays the valid
prefix through the same batch/COW engine. The requalified workload measures
2.152 writes, 1.032 flushes and 0.336 reads per ref update. Every recorded
write/flush cut, torn created content, replay-during-replay, shared unlink and
rename-replacement case converges to an allowed state. The evidence is in the
[fsync report](../implementation/fsync-intent-log-baseline.md).

The experiment also exposes a scope gap. Record version 2 represents
create/delete/rename and can reference checksummed extents for a newly created
file. It cannot represent a write or truncate of an existing file, so the
measured checkpointed append is not yet a measured logged-append path.

## Decision

The epoch-1 durability architecture is:

1. **Checkpoint COW** is the sole authoritative metadata transaction engine.
2. **Bounded group commit** combines bursts under one checkpoint and supplies
   the atomic-batch primitive.
3. **A small intent log** makes forced durability between checkpoints cheap.
   It records logical operations of the open batch; recovery materializes
   them through the same checkpoint engine rather than implementing a second
   transaction system.

The mechanism is accepted, but intent-log wire version 2 is not frozen.
Existing-file write and truncate must be representable and qualified before
the log can advertise universal cheap file fsync or become an epoch-1 wire
commitment.

Write/truncate log records must obey these rules:

- replacement user data uses fresh COW extents and reaches the durability
  barrier before the record that references it;
- the record checksum and content association reject a durable intent over
  torn or missing data;
- replay claims the exact recorded extents, updates the file layout and any
  shared-reference state through the common transaction engine, and remains
  idempotent if recovery itself crashes;
- an in-place data update is never logged as though the older generation
  retained exact bytes;
- log-full and over-size groups fall back explicitly to checkpoint commit or
  a reported limit, never silent loss of a completed fsync.

Until that gate passes, implementations advertise the cheap intent-log path
only for the logical operations their record version can replay. An ordinary
write/truncate fsync uses a checkpoint.

## Compatibility classification

The intent log retains the existing `INCOMPAT` feature identity
`org.aros.afsplus:intent-log`. An implementation unable to validate and replay
the active record version cannot safely mount the volume read-write. Read-only
and `NO_CHANGES` policy may expose the base checkpoint without replay under
ADR-038's rules and report the pending record count.

Accepting the mechanism does not freeze record version 2, the mandatory log
size, or future operation encodings. Those values follow the ordinary format
change procedure and interoperability corpus before M14.

## Rejected alternatives

- Checkpoint publication per fsync is rejected as the only durability path;
  its three-barrier structural floor is measured rather than a tuning issue.
- Group commit alone is rejected for forced durability because delaying the
  checkpoint cannot satisfy an fsync completion contract.
- A second general redo-journal engine is rejected because it would duplicate
  allocation, recovery and mutation semantics solely for a bake-off.
- Freezing record version 2 is rejected because it omits the primary
  existing-file write/truncate case.

## Validation and remaining freeze gates

The accepted mechanism has optimized workload requalification, log record
codec tests, checker validation, valid-prefix recovery, monotonic successive
fsync groups, exact all-or-nothing ref-update matrices, and shared-extent
replay matrices.

Before M14 freezes the wire, the implementation must add existing-file
write/truncate records, logged database/append/write-rename workloads, every
write/flush and replay-during-replay matrices for those operations, explicit
VFS capability/fallback semantics, and portable-C cross-reading.

## Consequences

Q2 is closed at the architecture level: AFS+ keeps checkpoint COW, group
commit and the intent log as complementary layers. The distinction between an
accepted mechanism and an unfrozen record encoding prevents the prototype's
namespace-only boundary from becoming accidental epoch-1 semantics.
