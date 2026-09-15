# Crash and Power-Failure Testing

> **ADRs:** [ADR-020](../adr/ADR-020-checkpoint-commit.md),
> [ADR-063](../adr/ADR-063-intent-log-epoch1.md) ·
> **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [intent-log qualification](intent-log-write-truncate-qualification.md) ·
> **Milestones:** M03, M04

<!-- toc -->

- [Goal](#goal)
- [Fault model](#fault-model)
- [Core transaction workloads](#core-transaction-workloads)
- [Checkpoint-specific failure matrix](#checkpoint-specific-failure-matrix)
- [Allocation safety regressions](#allocation-safety-regressions)
  - [Unknown allocator metadata](#unknown-allocator-metadata)
  - [Deferred-free resource exhaustion](#deferred-free-resource-exhaustion)
  - [Tiny cache eviction pressure](#tiny-cache-eviction-pressure)
  - [Rename ENOSPC/failure at every step](#rename-enospcfailure-at-every-step)
  - [Geometry and arithmetic](#geometry-and-arithmetic)
- [Deferred-reclamation tests](#deferred-reclamation-tests)
- [Validation after every crash](#validation-after-every-crash)
- [Block reuse across generations](#block-reuse-across-generations)
- [Explicit full-enumeration budgets](#explicit-full-enumeration-budgets)

<!-- /toc -->

## Goal

Prove that every acknowledged transaction recovers to a valid state and every unacknowledged transaction resolves to one of the explicitly allowed states.

The suite is informed by real failure modes found in PFS3/pfs3aio, not only synthetic generic crashes.

## Fault model

The block backend supports deterministic injection of:

- drop write N
- tear write N at sector/sub-block boundary
- short write
- read error for block N
- reorder writes where barriers do not forbid it
- fail flush/barrier N
- out-of-memory at allocation N
- crash immediately before/after each write or flush

## Core transaction workloads

- create
- mkdir
- append
- overwrite
- truncate
- rename same directory
- rename across directories
- atomic replace
- hard link
- unlink open file
- symlink
- xattr update
- allocation-region transition
- catalog update/rebuild
- change-stream append

## Checkpoint-specific failure matrix

Inject failure:

1. before any new metadata write
2. during leaf COW metadata write
3. during parent/root COW metadata write
4. during pre-checkpoint flush
5. during/torn alternate checkpoint write
6. during post-checkpoint flush
7. immediately after durable checkpoint
8. during later reclamation of retired blocks

Expected rule: mount chooses either the old valid checkpoint or the new valid checkpoint, never a hybrid authoritative tree.

For open-unlinked files the allowed states are more specific: before orphan
insertion the visible name and complete file remain; after it the name is
absent and the canonical object-2 entry owns the complete file. Each cleanup
cut may select the prior layout, a shorter tail-trimmed layout, or the final
absent object. Atomic replacement of an open target is either the complete old
two-name namespace or the new target plus a hidden old target, never a missing
source with an unpreserved target.

## Allocation safety regressions

Mandatory tests derived from pfs3aio failure classes:

### Unknown allocator metadata

Make one allocation bitmap/region page unreadable. The allocator must not hand out any block whose state is unknown.

### Deferred-free resource exhaustion

Force allocation failure while recording retired/deferred-free extents. The filesystem may leak/quarantine capacity but must not reuse potentially reachable storage.

### Tiny cache eviction pressure

Run allocator, rename, extent-tree, and cleanup workloads with the smallest supported metadata cache. Force cache misses at nested call boundaries. Pinned pages must not be evicted and stale handles must be detected in debug builds.

### Rename ENOSPC/failure at every step

For cross-directory rename, inject failure at every allocation/write step. Result must be either:

- source still present and destination absent
- committed destination with source removed

Never orphan the object and never expose two accidental independent objects.

### Geometry and arithmetic

Fuzz:

- block sizes
- sector sizes
- partition start/end
- volume sizes near powers of two
- maximum 64-bit offsets

All arithmetic is checked before conversion/multiplication.

## Deferred-reclamation tests

Delete/truncate very large synthetic objects, crash after every reclamation batch, and verify:

- namespace state remains committed
- no live extent becomes free
- progress can resume
- repeating a completed batch is safe or detected
- pending space eventually returns to free pool

## Validation after every crash

1. open in NO_CHANGES mode
2. enumerate/validate checkpoint candidates
3. select expected recovery state
4. run full invariant checker
5. verify reachable file contents for acknowledged durability points
6. verify no physical block has conflicting live ownership
7. verify quarantined blocks are never in allocatable free space

A result that requires ordinary fsck repair after every injected crash does not satisfy the normal transaction guarantee.

For an acknowledged intent-log update, repeat the same cut matrix during the
recovery transaction itself and remount the resulting image again. Recovery
must either finish the new checkpoint or leave the original checkpoint plus
log replayable; it may not consume the only durable copy of the intent before
publication. Existing-file write/truncate cases, monotone fsync prefixes and
shared-range splits are enumerated in the
[intent-log gate](intent-log-write-truncate-qualification.md).

## Block reuse across generations

Reuse makes stale-but-valid block contents dangerous, so the reuse workload is
mandatory once the allocator recycles storage. Construct:

```text
G1: create A -> physical block X
G2: delete A -> X becomes retired, not free
G3: publish maintenance while the G1 checkpoint still protects X
G4: the oldest valid slot is G2; B transaction can reuse X for data or metadata
```

Inject power loss after every relevant write and flush during G2, G3 and G4.

Allowed mounted states:

```text
G1: A exists and its bytes are correct
G2: A absent and X not unsafely reused
G3: A absent, X quarantined, previous-checkpoint storage intact
G4: B exists and its bytes are correct
```

Forbidden:

```text
A visible but X contains B's bytes
FREE block reachable from any selectable checkpoint
same non-shared physical block owned by two live objects
reuse before every checkpoint that can reach old contents is retired
```

The executable form is the G1/G2/G3/G4 quarantine matrix under
[ADR-074](../adr/ADR-074-protect-previous-checkpoint.md) in
[`crates/afsplus-check/tests/alloc_crash.rs`](../crates/afsplus-check/tests/alloc_crash.rs); the allocator design it exercises
is [docs/07](../docs/07-allocation.md).

## Explicit full-enumeration budgets

The memory-image streaming simulator defaults to twelve unflushed writes per
cut. `for_each_crash_state_with_budget` in
[powercut.rs](../crates/afsplus-block/src/powercut.rs) accepts an explicit limit
up to twenty writes. It enumerates every full-write subset plus the same
representative prefix tears; it never falls back to sampling. The callback
receives one image at a time, but total runtime grows exponentially. Exceeding
the requested limit rejects the cut. Overlay enumeration retains its default
limit. [Budget tests](../crates/afsplus-block/tests/powercut_budget.rs) verify
all 8,192 distinct subsets and 39 tears for thirteen writes, and default refusal
before callbacks at the same size.
