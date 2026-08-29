# ADR-020: Copy-on-write metadata with alternating checkpoints

Status: Proposed

## Context

PFS3 demonstrates that atomic metadata state can be achieved by writing changed metadata to new locations and changing the root reference only after the new tree is safely written.

AFS+ needs the same integrity property but should not depend on a large fixed root/root-cluster write being atomic. Modern storage can tear writes and different devices provide different durability guarantees.

## Proposed decision

Use copy-on-write for authoritative metadata and maintain at least two small checkpoint slots.

Each checkpoint contains, at minimum:

- filesystem UUID/epoch binding
- monotonically increasing generation
- root object/tree reference
- allocation metadata root
- reclamation/quarantine root
- feature-state root or digest
- newest change-stream sequence where applicable
- checksum

Commit sequence:

1. write user data that the new metadata will reference
2. flush/barrier according to block-provider durability contract
3. write all changed COW metadata bottom-up
4. flush/barrier
5. write the alternate checkpoint with generation + 1 and checksum
6. flush/barrier before reporting durable commit

Mount selects the highest-generation valid checkpoint whose referenced root structures validate.

A torn or incomplete new checkpoint is ignored, leaving the prior checkpoint usable.

## Retired storage

Storage that becomes unreachable is retired, not immediately free.

A block may be reused only after no retained valid checkpoint can reference it.

The exact generation quarantine/reclaim algorithm is specified separately and must preserve at least one known-good committed recovery state at all times.

## Why this is only Proposed

Before acceptance the design must pass:

- exhaustive fail-after-write crash injection
- torn checkpoint tests
- failed flush tests
- interrupted deferred-reclamation tests
- low-free-space tests
- write-amplification comparison against redo journaling
- real-device tests with documented flush semantics

If the approach proves more complex or writes more than a small redo journal for AFS+ workloads, this ADR must be revisited.
