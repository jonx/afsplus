# 08. Transactions, Checkpoints, and Recovery

## 1. Requirement, not mechanism

AFS+ requires atomic metadata transactions, bounded recovery, and explicit durability semantics.

A conventional metadata redo journal was the original proposal. After the PFS3/PFS4 Stage 0 review it is no longer a predetermined requirement.

The leading candidate is copy-on-write metadata with alternating checksummed checkpoint records. See ADR-009 and ADR-020.

The implementation phase must now answer several questions that cannot be settled honestly by prose alone. They are explicit epoch-1 blockers below.

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

## 3. Proposed COW checkpoint commit

Changed authoritative metadata is written to new blocks rather than overwriting blocks reachable from the current checkpoint.

Proposed ordering:

1. write required user-data blocks according to the selected data-versioning policy
2. durability barrier/flush as required by that policy
3. write COW metadata from leaves toward roots
4. durability barrier/flush
5. write alternate checkpoint with new generation/checksum/root references
6. durability barrier/flush before reporting durable commit

The previous checkpoint's **metadata graph** remains untouched until the new checkpoint is independently valid.

This statement deliberately does not yet claim that every byte of user data referenced by the previous checkpoint remains unchanged. That stronger guarantee depends on the data-update policy described next.

## 4. Epoch-1 blocker A: data overwrite versus data COW

Rewriting a logical range that already has allocated storage creates a fundamental choice.

### Candidate A: in-place data overwrite

Metadata is COW/checkpointed, but unshared user-data blocks may be overwritten in place.

Advantages:

- low fragmentation for database/VM-like workloads
- lower allocation/write-amplification cost

Consequences:

- after a crash, an older valid metadata checkpoint may reference partially newer user data
- the recovery contract resembles metadata-journaling filesystems: structurally consistent metadata does not imply old file bytes are preserved
- an exact previous committed content-generation handle cannot be guaranteed once its data blocks are overwritten

### Candidate B: full data COW

Every modification to committed data allocates replacement blocks until commit.

Advantages:

- previous retained checkpoints/content generations remain byte-stable
- reflink and generation-stable reads share one model

Consequences:

- potential fragmentation and write amplification, especially for databases, VM images, and random rewrites
- more reclamation/reference tracking

### Candidate C: explicit hybrid policy

Some files/ranges use data COW while others permit in-place overwrite under a clearly weaker historical-generation contract.

This may provide useful tradeoffs but creates policy and interoperability complexity.

### Decision rule

Do not freeze this choice on paper.

The first writable prototype must measure at least:

- random 4 KiB rewrites of large files
- database/VM-image style workloads
- reflink COW writes
- crash states before/after metadata commit
- fragmentation
- bytes written
- CPU and RAM
- ability/cost to serve an exact committed generation

Until this experiment is complete, any API promising an old content generation must qualify that the requested data generation must still be retained and physically stable.

### Current prototype experiment

Core Scale-1 currently implements Candidate B for range writes and partial-tail
truncate: touched committed data blocks are never overwritten. Fresh data is
flushed before COW extent metadata, then the alternate checkpoint publishes
the result. The exhaustive sparse-write power-cut matrix accepts only the
complete pre-write or post-write byte sequence. This is executable evidence
for full data COW, but does not yet freeze the epoch-1 policy; write
amplification, fragmentation, database/VM workloads, and the explicit hybrid
alternative still require measurement.

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

Shared/reflink extents additionally remain allocated until no live object or retained recovery state references them.

## 7. Deferred reclamation

Large deletes/truncates are split into:

- a small atomic logical transaction
- bounded resumable reclamation work

This avoids enormous temporary free lists and long uninterruptible commits.

## 8. Epoch-1 blocker B: fsync and small durability commits

A global checkpoint is conceptually simple, but a small-file `fsync()` must not accidentally require an expensive whole-filesystem commit path that makes Git/package/database workloads unusable.

The first implementation should build the simplest checkpoint-COW path first and measure it.

Required benchmark:

```text
create/write small file
fsync
repeat
```

plus rename/replace-heavy Git/package workloads.

If global checkpoint latency/write amplification is unacceptable, the expected next design candidate is **checkpoint COW plus a small durability/intent log**, rather than replacing the entire checkpoint engine with a second full transaction architecture.

AFS+ therefore reserves a discoverable extension point for an auxiliary durability log before epoch 1, but does not freeze its record format, mandatory size, or activation semantics until measurement proves it is needed.

The project no longer requires building two complete transaction engines merely for a bake-off. A redo/durability log prototype is built when the checkpoint prototype or its benchmarks demonstrate a concrete need.

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
