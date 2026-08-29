# 08. Transactions, Checkpoints, and Recovery

## 1. Requirement, not mechanism

AFS+ requires atomic metadata transactions, bounded recovery, and explicit durability semantics.

A conventional metadata redo journal was the original proposal. After the PFS3/PFS4 Stage 0 review it is no longer a predetermined requirement.

The leading candidate is copy-on-write metadata with alternating checksummed checkpoint records. See ADR-009 and ADR-020.

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

1. write required new user data
2. durability barrier/flush
3. write COW metadata from leaves toward roots
4. durability barrier/flush
5. write alternate checkpoint with new generation/checksum/root references
6. durability barrier/flush
7. report durable commit

The previous checkpoint remains untouched until the new checkpoint is independently valid.

## 4. Recovery

Mount examines checkpoint candidates and chooses the newest valid generation.

Validation includes:

- checkpoint checksum
- filesystem UUID/epoch
- root references in bounds
- root metadata checksums
- feature compatibility

A partially written newer checkpoint is ignored.

No full-volume scan is required for ordinary crash recovery.

## 5. Retired blocks and quarantine

A block that becomes unreachable in the new state is not necessarily safe to reuse immediately because an older retained checkpoint may still reference it.

AFS+ therefore tracks retired storage until it is older than every recovery state that may still be selected.

On uncertainty the allocator must quarantine/leak space rather than reuse it early.

## 6. Deferred reclamation

Large deletes/truncates are split into:

- a small atomic logical transaction
- bounded resumable reclamation work

This avoids enormous journal records, huge temporary free lists, and long uninterruptible commits.

## 7. Alternative redo journal

A redo journal remains a prototype/reference implementation candidate.

Before epoch 1, compare checkpoint COW and redo journal using identical workloads:

- metadata bytes written
- user-data write amplification
- commit latency
- peak RAM
- recovery latency
- low-free-space behavior
- implementation complexity
- crash-state count and repair complexity

The simpler design that meets correctness and resource goals wins.

## 8. NO_CHANGES mode

`NO_CHANGES` never writes media.

It may construct an in-memory recovered view if necessary, but it must not:

- replay anything to disk
- clear dirty state
- advance checkpoints
- reclaim blocks
- rebuild catalog
- repair summaries
- update timestamps/counters

## 9. Durability contract

The block-provider API must define what `flush`/barrier means. AFS+ cannot promise durable commit on a device/backend that cannot make prior writes durable in the required order.

The filesystem API must separately document guarantees for:

- data write without fsync
- file fsync
- directory fsync
- atomic replace + fsync
- filesystem sync

## 10. Testing gate

No transaction mechanism is accepted for epoch 1 until deterministic fault injection demonstrates that after every modeled crash the mounted state is one of the explicitly allowed pre-commit or post-commit states and all allocation/object invariants hold.
