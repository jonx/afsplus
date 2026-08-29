# 08. Transactions and Journal

## 1. Purpose

The journal protects metadata consistency across crashes and sudden power loss.

AFS+ 1.0 does not require full user-data journaling.

## 2. Transaction boundary

Operations that must appear atomically include:

- rename
- atomic replacement
- directory insertion/removal
- allocation plus extent-map update
- object creation
- final unlink
- metadata updates that span multiple structures

## 3. Journal style

The initial implementation should prefer a simple metadata redo journal using checksummed records and explicit commit markers.

The exact record encoding must be specified before epoch 1.

A transaction is visible after its durable commit record and required ordering guarantees have been satisfied.

## 4. Mount recovery

On dirty mount:

1. identify the last valid committed transaction
2. validate journal checksums and sequence continuity
3. replay committed metadata updates as required
4. ignore incomplete uncommitted tail records
5. update filesystem state only if the mount mode permits writes

## 5. NO_CHANGES mode

If mounted with `NO_CHANGES`:

- do not replay journal to disk
- do not clear dirty state
- do not update mount counters
- do not rebuild catalog
- do not repair summaries
- do not change timestamps

The implementation may replay committed metadata into an in-memory overlay for read access if that can be done without changing the underlying media.

## 6. fsync contract

The specification must define what is guaranteed after successful:

- file data flush
- metadata flush
- directory fsync
- filesystem sync
- atomic rename followed by sync

The contract must be strong enough for databases, Git, Cargo, editors, and package managers to make correct durability decisions.

## 7. Journal sizing

The journal is bounded.

Large operations may be split into multiple transactions if atomic semantics are not required across the entire operation.

Operations that require atomicity must fail cleanly if they cannot fit within supported transaction limits.

## 8. Write amplification

The journal must not become an excuse to write full metadata trees repeatedly.

Implementation benchmarks must track bytes written per logical metadata operation.
