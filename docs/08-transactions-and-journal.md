# 08. Transactions, Checkpoints, and Recovery

> **ADRs:** [ADR-009](../adr/ADR-009-journal.md), [ADR-020](../adr/ADR-020-checkpoint-commit.md),
> [ADR-026](../adr/ADR-026-bounded-atomic-batches.md), [ADR-036](../adr/ADR-036-reclaim-queue.md),
> [ADR-037](../adr/ADR-037-intent-log.md), [ADR-062](../adr/ADR-062-explicit-hybrid-data-updates.md),
> [ADR-063](../adr/ADR-063-intent-log-epoch1.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [crash-testing](../testing/crash-testing.md), [data-policy qualification](../testing/data-policy-qualification.md) · **Milestones:** M04

<!-- toc -->

- [1. Requirements and selected mechanism](#1-requirements-and-selected-mechanism)
- [2. Transaction boundary](#2-transaction-boundary)
- [3. COW checkpoint commit](#3-cow-checkpoint-commit)
- [4. Epoch-1 user-data update policy](#4-epoch-1-user-data-update-policy)
  - [Default full data COW](#default-full-data-cow)
  - [Explicit private in-place opt-in](#explicit-private-in-place-opt-in)
  - [Crash and generation contract](#crash-and-generation-contract)
  - [Executable qualification](#executable-qualification)
- [5. Recovery](#5-recovery)
- [6. Retired blocks and quarantine](#6-retired-blocks-and-quarantine)
- [7. Deferred reclamation](#7-deferred-reclamation)
- [8. Epoch-1 fsync and small durability commits](#8-epoch-1-fsync-and-small-durability-commits)
- [9. NO_CHANGES mode](#9-nochanges-mode)
- [10. Durability contract](#10-durability-contract)
- [11. Concurrency and readers](#11-concurrency-and-readers)
- [12. Testing gate](#12-testing-gate)

<!-- /toc -->

## 1. Requirements and selected mechanism

AFS+ requires atomic metadata transactions, bounded recovery, and explicit durability semantics.

A conventional metadata redo journal was the original proposal. After the PFS3/PFS4 Stage 0 review it is no longer a predetermined requirement.

AFS+ uses copy-on-write metadata with alternating checksummed checkpoint
records, bounded group commit, and a small intent log for forced durability
between checkpoints. ADR-063 records why these are complementary layers of
one transaction engine rather than competing engines.

## 2. Transaction boundary

Operations that must expose an atomic logical result include:

- rename
- atomic replacement
- directory insertion/removal
- object creation
- link/unlink
- allocation plus extent-map update
- metadata updates spanning multiple structures

Large physical cleanup need not be part of the same transaction if the user-visible change can commit and cleanup can safely continue through deferred reclamation.

## 3. COW checkpoint commit

Changed authoritative metadata is written to new blocks rather than overwriting blocks reachable from the current checkpoint.

Commit ordering:

1. write required user-data blocks according to the selected data-versioning policy
2. durability barrier/flush as required by that policy
3. write COW metadata from leaves toward roots
4. durability barrier/flush
5. write alternate checkpoint with new generation/checksum/root references
6. durability barrier/flush before reporting durable commit

The previous checkpoint's **metadata graph** remains untouched until the new checkpoint is independently valid.

This ordering alone does not claim that every byte of user data referenced by
the previous checkpoint remains unchanged. That stronger guarantee depends on
the data-update policy described next.

## 4. Epoch-1 user-data update policy

[ADR-062](../adr/ADR-062-explicit-hybrid-data-updates.md) selects an explicit
per-file hybrid. The policy controls user-data versioning only; metadata uses
COW in every mode, and reflink-shared ranges always use data COW.

### Default full data COW

Every modification to committed data allocates replacement blocks until
commit. Fresh data is flushed before the COW extent metadata, then the
alternate checkpoint publishes the result. While an older checkpoint and its
blocks remain retained, its file bytes are stable and crash recovery selects
the complete old or complete new content.

Full COW is the creation default and the mandatory fallback whenever privacy
or representability is uncertain.

### Explicit private in-place opt-in

A filesystem-neutral policy API may opt a file into private in-place updates.
The choice is persistent per file; AFS+ does not infer it from a workload.

An operation may overwrite its mapped physical blocks only when the complete
write is non-extending and every touched block is materialized and proven
private. A hole, unwritten extent, shared marker, unresolved reference state,
or extension sends the complete operation through COW. Metadata is still
published by the ordinary checkpoint transaction.

### Crash and generation contract

After an interrupted in-place operation, the older metadata checkpoint remains
structurally valid but the requested byte range may contain old, new or torn
data. If data reached the device and a later metadata operation failed, a
returned error also does not promise byte rollback.

An API therefore never labels such bytes as an exact historical content
generation. It returns `GENERATION_NOT_AVAILABLE` when physical stability
cannot be proved. This is independent of Q4's future retention-duration
policy.

### Executable qualification

The runtime-only `DataUpdatePolicy` switch keeps full COW as the mount default
and implements the conservative eligibility rule for comparison. Random 4 KiB,
database hot-set, append and reflink workloads, plus power-cut and injected-I/O
matrices, are specified in
[`testing/data-policy-qualification.md`](../testing/data-policy-qualification.md).
The measurements are retained in the
[Q1 bake-off](../implementation/data-policy-bakeoff.md). The shipping per-file
encoding and API remain an M14 gate rather than an implicit property of this
runtime switch.

## 5. Recovery

Mount examines checkpoint candidates and chooses the newest valid generation.

Validation includes:

- checkpoint checksum
- filesystem UUID/epoch
- root references in bounds
- root metadata checksums
- feature compatibility

A partially written newer checkpoint is ignored.

No full-volume scan is required for ordinary metadata crash recovery.

Normal mount validates the identification/checkpoint records and a bounded
set of root pages. It does not prove the integrity of every descendant before
returning. A corrupt descendant is reported when accessed; `afsplus-check`,
shadow verification, and the crash harness perform exhaustive walks. This is
the necessary distinction between bounded recovery and whole-volume
verification.

The exact user-data semantics after a crash are determined by the selected data-update policy in section 4 and must be documented separately from metadata consistency.

## 6. Retired blocks and quarantine

A block that becomes unreachable in the new state is not necessarily safe to reuse immediately because an older retained checkpoint may still reference it.

AFS+ therefore tracks retired storage until it is older than every recovery state that may still be selected.

On uncertainty the allocator must quarantine/leak space rather than reuse it early.

Blocks allocated by an in-flight transaction and discarded before publication are the one exception: no committed state can reference them, so they return to free immediately instead of entering quarantine.

Shared/reflink extents additionally remain allocated until no live object or retained recovery state references them.

## 7. Deferred reclamation

Large deletes/truncates are split into:

- a small atomic logical transaction
- bounded resumable reclamation work

This avoids enormous temporary free lists and long uninterruptible commits.

The executable prototype implements this as a segmented reclaim queue (ADR-036): retired runs are appended to a FIFO of immutable sealed blocks, each transaction reclaims at most a bounded block budget from the head, and a persistent cursor makes the work resumable across crashes and reboots. Multiple retire generations coexist in the queue; normal mount reads only its root block.

## 8. Epoch-1 fsync and small durability commits

A global checkpoint is conceptually simple, but a small-file `fsync()` must not accidentally require an expensive whole-filesystem commit path that makes Git/package/database workloads unusable.

ADR-063 selects checkpoint COW, bounded group commit and a small intent log as
the epoch-1 durability architecture. Group commit amortizes bursts under one
checkpoint. The intent log records fsynced prefixes of the same open batch and
recovery materializes them through the same checkpoint engine.

Both mechanisms are implemented and measured for the namespace window
([`implementation/fsync-intent-log-baseline.md`](../implementation/fsync-intent-log-baseline.md)).
The log makes a forced ref update cost about one sequential record write plus
one barrier between checkpoints, with per-fsync-group all-or-nothing recovery.

Qualification workloads include:

```text
create/write small file
fsync
repeat
```

plus rename/replace-heavy Git/package workloads.

Record wire version 2 covers create/delete/rename and created-file content. It
does not cover write or truncate of an existing file. Such fsyncs use a full
checkpoint until the log encoding, replay, checker and crash matrices support
them. Consequently the intent-log mechanism is accepted while its record wire,
mandatory size and universal cheap-file-fsync capability remain unfrozen.

AFS+ does not build a second redo-journal transaction engine: the log is a
bounded durability layer over checkpoint COW.

## 9. NO_CHANGES mode

`NO_CHANGES` never writes media.

It may construct an in-memory recovered view if necessary, but it must not:

- replay anything to disk
- clear dirty state
- advance checkpoints
- reclaim blocks
- rebuild catalog
- repair summaries
- update timestamps/counters

## 10. Durability contract

The block-provider API must define what `flush`/barrier means. AFS+ cannot promise durable commit on a device/backend that cannot make prior writes durable in the required order.

The filesystem API must separately document guarantees for:

- data write without fsync
- file fsync
- directory fsync
- atomic replace + fsync
- filesystem sync

No API may silently claim stronger historical-data durability than the selected data-update policy can provide.

## 11. Concurrency and readers

Before epoch 1, the implementation/spec must define and test:

- what a reader sees while a writer has uncommitted changes
- when a newly committed checkpoint becomes visible to existing/new handles
- iterator/cookie behavior across directory mutation
- object/content-generation handle lifetime
- cache/page pinning and stale-handle behavior

The baseline target is a clearly documented read-committed model, not accidental behavior inherited from lock implementation details.

## 12. Testing gate

No transaction mechanism is accepted for epoch 1 until deterministic fault injection demonstrates that after every modeled crash the mounted state is one of the explicitly allowed states and all allocation/object invariants hold.

Tests must distinguish:

- metadata consistency
- namespace atomicity
- data durability
- historical generation stability

because these are related but not identical guarantees.
