# ADR-061: Shared-extent references in a typed reference tree

Status: Accepted for the prototype; wire format experimental
Amends: ADR-027

## Context

[ADR-027](ADR-027-reflink-clones.md) makes shared data extents an epoch-1
format requirement and an epoch-1 freeze gate depends on it, but deliberately
leaves the mechanism open: "The implementation may use a shared-extent/refcount
structure, extent-reference objects, or another format mechanism selected
during prototyping."

Nothing implements it. The extent record carries one flag
(`EXTENT_UNWRITTEN`), no reference state exists, and
[`spec/invariants.md`](../spec/invariants.md) still says every allocated block
is owned by exactly one live allocation "unless an active shared-block feature
explicitly changes this rule". ADR-027 states why this cannot wait: adding
sharing later changes the allocator, checker, reverse-map and reclamation
invariants at once.

The mechanism also decides where the write path forks. Reflink-shared ranges
always copy on write, so the code must distinguish shared from private ranges —
which is exactly where the unresolved user-data update policy
([open question Q1](../implementation/open-questions.md)) lives.

## Decision

### Reference authority

A volume-wide typed `AFST` tree, `TreeKind::SharedExtents`, owner 0, holds one
record per shared physical run, keyed by its first physical block:

```text
key    physical_start (u64)
value  block_count (u64), reference_count (u32), flags (u32, zero reserved)
```

Records are non-overlapping and ordered by `physical_start`. The tree uses the
bounded copy-on-write engine of [ADR-034](ADR-034-bounded-cow-tree.md)
unchanged: same node format, same split/merge, same exhaustive verifier, same
bounded lookup, and it is published in the same transaction as the extent maps
it describes.

Rejected alternatives: a reference count inside each extent record duplicates
the value in every sharer and leaves no single authority; physical-to-owner
back-reference objects are more general and would also serve
[ADR-024](ADR-024-rebuildable-reverse-map.md), but they are a second structure
with its own rebuild and staleness rules and are not required to satisfy
ADR-027.

### The extent flag is a hint, the tree is authoritative

`EXTENT_SHARED` marks an extent whose physical run may be shared. Set, it
obliges the reader to consult the reference tree before any in-place action.
Clear, the run is private with no lookup. A record is absent from the tree
exactly when its run has a single reference, so an extent may keep the flag
after its peers are gone: a stale flag costs one bounded lookup and never
costs correctness. The tree stays empty on volumes that never clone, and
`shared_extent_root_block` is zero there.

### Checkpoint binding

The checkpoint payload gains `shared_extent_root_block` (u64) at offset 96,
where the transitional inline region records used to begin; they have been
empty since [ADR-035](ADR-035-allocation-root-reserved-pool.md). Zero means no
shared-extent tree exists on this volume. The root is allocated by the first
clone, inside that clone's transaction.

### Operations

- **Clone.** `CloneFile`/`CloneRange` copy the source extent records into the
  destination map, set `EXTENT_SHARED` on both sides, and insert or increment
  the reference records for the covered runs. One transaction, one checkpoint.
- **Write into a shared range.** Replacement blocks are allocated for the
  modified logical range, the reference count of the overwritten sub-range is
  decremented, and the private extent replaces it in that object's map only.
  This is unconditional and independent of the Q1 outcome, which governs
  writes to *private* committed data.
- **Split and merge.** Cloning or overwriting part of a run splits its record
  into up to three records whose counts sum to the original accounting. Two
  adjacent records merge only when their reference counts are equal.
- **Free.** Unlink and truncate consult the tree for every `EXTENT_SHARED`
  extent: a count above one is decremented and the blocks are *not* retired; a
  count of one, or an absent record, retires the run into the reclaim queue
  ([ADR-036](ADR-036-reclaim-queue.md)) exactly as an unshared run is retired
  today. The decrement and the extent-map change are in the same transaction.
- **Uncertainty.** A reference state that cannot be read is never resolved in
  favour of freeing: the run is quarantined and reported, never returned to
  free space.

### Compatibility

The feature registry gains `org.aros.afsplus:shared-extents`, read-compatible
and write-incompatible: an implementation that does not honour reference counts
may mount read-only but must refuse read-write, because freeing a shared run
would destroy another object's data. Minimal readers need only accept that
several objects may map the same physical run.

## Validation

The checker rebuilds the expected reference table by scanning the extent map of
every live object — a forward scan, no reverse map — and requires:

- every record's `reference_count` equals the number of live references found;
- no record's run intersects authoritative free space, quarantine, or another
  record;
- records are ordered, non-overlapping, and within volume bounds;
- an extent flagged `EXTENT_SHARED` whose run is absent from the tree is
  accounted as private, which is allowed;
- an extent *not* flagged shared whose run appears in a record is a corruption.

Because the tree is published in the checkpoint that publishes the extent maps,
every modelled crash state selects either the complete pre-operation or the
complete post-operation state. The mandatory matrix injects a power cut after
every write and every flush of: clone creation, a write that splits a shared
run, unlink of one clone while another lives, and reclamation of the last
reference.

## Consequences

ADR-027's required invariants become executable and checkable, and the epoch-1
gate "shared extents cannot be freed while referenced by any live object or
retained recovery state" acquires a mechanism and a test.

`spec/invariants.md` gains the shared-block exception its own text anticipates.
The write path acquires the shared/private fork that open question Q1 needs in
order to be decided by measurement rather than by default.

The wire format is experimental. The record shape, the tree kind, the flag bit
and the checkpoint offset are not epoch-1 commitments until M14, and prototype
images are rebuilt rather than migrated.

`CloneTree`, the reverse map of ADR-024 and user-data checksums stay out of
scope, as ADR-027 and the open-questions register already record.
