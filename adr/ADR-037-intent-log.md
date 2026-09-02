# ADR-037: Intent log for forced durability between checkpoints

Status: Mechanism accepted by ADR-063; record wire remains experimental
Amended by: ADR-063

## Context

The measured blocker-2 baseline
([`implementation/fsync-intent-log-baseline.md`](../implementation/fsync-intent-log-baseline.md)) established that group
commit (ADR-026 batches) fully solves burst throughput, and isolated the one
remaining problem: a *forced* durable operation still costs a full
checkpoint publication — 10 block writes and 3 barriers for a Git-style ref
update — versus ~1 write and ~1 barrier for a log-based design. The bake-off
gate for this prototype is explicit: beat 3 flushes / 10 writes per durable
ref update under the same crash matrices, or be rejected.

## Decision

The intent log is a **persistent journal of the open operation window**. It
adds no second transaction engine: a window is exactly an open ADR-026
batch, and the log records what would be lost if power failed before the
window's checkpoint.

```text
window_op(op)      apply to the batch overlay; write the op's data blocks
                   to freshly allocated (still uncommitted) locations
window_fsync()     append ONE log record covering every not-yet-logged
                   window op, then ONE barrier -> those ops are durable
window_commit()    materialize the batch, publish a checkpoint; every log
                   record becomes stale by generation binding
```

Recovery at mount: after checkpoint selection, scan the log area; replay
the valid record prefix by re-running its logical operations through the
batch engine — claiming the exact data extents recorded — and publish one
checkpoint. Volumes with an empty or stale log mount without writing.

## Placement and the absence of recursion

The log area is a fixed run of `log_slots` blocks (an mkfs parameter
recorded in the identification block; 0 disables the feature), located at
deterministic LBAs directly after the allocation-root pool and permanently
marked allocated by mkfs. Like the bitmap slots and the pool, log blocks
are never owned by the allocator they help protect: appending a record
allocates nothing, and no committed structure ever references a log block,
so the reclaim queue and the COW engine are untouched.

## Record format and binding

One block per record (`AFSJ`), one record per fsync, containing:

- volume UUID and **base checkpoint generation** — a record is meaningful
  only against the exact checkpoint generation it extended; after the next
  checkpoint commits, every record is stale by binding and slots are
  reused without erasure;
- a sequence number that must equal its slot position;
- the fsync group: every window operation not covered by an earlier
  record (create with name/size/extents/content CRC32C plus the expected
  object ID, delete, rename with replace), including each operation's
  timestamp, bounded by one block —
  a group too large for one record is a reported prototype limit.

Because one fsync is one record, an fsync group is all-or-nothing under
crash: either the whole group replays or none of it. Replay stops at the
first invalid record (bad checksum, wrong binding, broken sequence, or
content whose CRC no longer matches the recorded extents); with barriers
ordered as written, a durable record can never follow a lost one.

## Why claiming recorded extents is sound

Data blocks referenced by a record were allocated by the open window's
transaction, so the live volume cannot give them to anyone else, and from
the base checkpoint's view they are FREE — unreachable from any selectable
state, safe to overwrite before the fsync barrier and safe to claim during
replay. Replay claims exactly the recorded runs (an error if any block is
not free), reconstructs the file records against them, and never rewrites
the data itself. The content CRC covers the torn-data-with-durable-record
crash state; such a record invalidates the tail, which is legal because
that fsync never completed.

## Window failure semantics

Validation errors (name exists, not found, invalid name) leave the window
usable. A non-validation error poisons the window: further window
operations are refused and a remount recovers exactly the fsynced prefix
from the log. Regular immediate-commit operations and reclaim steps are
refused while a window is open.

## Checker obligations

The checker validates the log area against the chosen checkpoint: record
checksums, binding, sequence continuity, extent bounds, extents free in
the committed bitmaps, and content CRCs; it reports the pending record
count. An invalid tail is a normal crash artifact (warning); a valid
record referencing allocated blocks is corruption (error).

## Consequences and status

ADR-038 makes replay policy explicit. `ReadWrite` replays automatically;
`ReadOnly` and `NO_CHANGES` expose the pre-replay checkpoint and the pending
record count without writes; `Recovery` replays and then remains read-only.
The feature is advertised by INCOMPAT bit 0 under the permanent identity
`org.aros.afsplus:intent-log`. Its record format remains experimental until
the broader workload suite and external interoperability qualification pass.
