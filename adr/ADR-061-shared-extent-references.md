# ADR-061: Shared-extent references in a typed reference tree

Status: Accepted; wire format experimental until M14
Amends: ADR-027, ADR-035

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
  - [Reference authority](#reference-authority)
  - [The flag is a conservative marker, not a symmetric hint](#the-flag-is-a-conservative-marker-not-a-symmetric-hint)
  - [Canonical form](#canonical-form)
  - [Checkpoint binding](#checkpoint-binding)
  - [Reference counts are per checkpoint](#reference-counts-are-per-checkpoint)
  - [Operations](#operations)
  - [Intent log](#intent-log)
  - [Compatibility](#compatibility)
- [Validation](#validation)
- [Consequences](#consequences)

<!-- /toc -->

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

A volume-wide typed `AFST` tree, `TreeKind::SharedExtents` (wire value 5),
owner 0, holds one record per shared physical run, keyed by its first physical
block:

```text
key    physical_start (u64)
value  block_count (u64), reference_count (u32), flags (u32, zero reserved)
```

`EXTENT_SHARED` is bit 1 of the extent flag word (bit 0 being
`EXTENT_UNWRITTEN`), and the feature occupies bit 0 of the still-empty
`RO_COMPAT` namespace. These three identities — tree kind 5, extent flag bit 1,
feature bit 0 — are what a codec needs and are fixed here.

The tree uses the bounded copy-on-write engine of
[ADR-034](ADR-034-bounded-cow-tree.md) unchanged, and is published in the same
transaction as the extent maps it describes. Every record is validated on
load: `block_count > 0`, `physical_start + block_count` without overflow and
within volume bounds, `reference_count >= 2`, reserved flags zero, and node
kind, owner and generation checked like every other typed adapter. A clone that
would take a reference count past `u32::MAX` fails cleanly instead of wrapping.

Rejected alternatives: a reference count inside each extent record duplicates
the value in every sharer and leaves no single authority; physical-to-owner
back-reference objects are more general and would also serve
[ADR-024](ADR-024-rebuildable-reverse-map.md), but they are a second structure
with its own rebuild and staleness rules and are not required to satisfy
ADR-027.

### The flag is a conservative marker, not a symmetric hint

`EXTENT_SHARED` on an extent asserts nothing except *this run may overlap
shared records*; the tree is the authority. The two directions are not
symmetric:

- **flag set, no overlapping record** — the run is private. Legal, and the
  expected steady state after peers disappear. It costs one bounded lookup.
- **flag clear, an overlapping record exists** — corruption. A false negative
  would let a caller overwrite or free storage another object still
  references, so the checker rejects it.

An extent's flag therefore covers an interval that may be partly shared and
partly private, because peers split the underlying runs independently. After
`A` and `B` share `[0, 100)` and `B` rewrites `[40, 60)`:

```text
tree      [0,40) rc=2        [60,100) rc=2      ([40,60) has one reference: no record)
A extent  [0,100) EXTENT_SHARED  — one extent, two shared sub-runs and one private gap
```

Every operation on a flagged extent therefore performs an **overlap search**
— floor record plus successors until past the extent's physical end — and
partitions the extent at record boundaries, treating only the gaps as private.
An exact lookup on `physical_start` is wrong: here it finds `[0,40)` and misses
`[60,100)`.

### Canonical form

Records are non-overlapping, ordered by `physical_start`, and **maximal**: two
adjacent records with equal `reference_count` must be merged, so a given
sharing state has exactly one representation and the checker compares without
guessing. Fragmentation is bounded by merging on every decrement rather than
left to accumulate.

A split partitions the original interval exactly — the resulting `block_count`s
sum to the original `block_count`, covering it without gap or overlap. Each
segment keeps the original `reference_count`; only the segment whose reference
is being dropped changes, and it disappears entirely when its count reaches 1.

### Checkpoint binding

The transitional inline region records are removed rather than made to coexist
with a new field. They have been empty in every written checkpoint since
[ADR-035](ADR-035-allocation-root-reserved-pool.md), but the codec still
decodes them when `allocation_root_block == 0`, and that layout puts the first
record at offset 96 — the same bytes a naively appended root would occupy.

Therefore: `allocation_root_block == 0` becomes structurally invalid, the
record count, its reserved word and the trailing records leave the payload, and
the fixed payload is 96 bytes:

```text
80     8    flags (zero; reserved)
88     8    shared-extent reference-tree root LBA
```

Zero means no shared-extent tree exists on this volume, the normal state of a
volume that has never cloned; the root is allocated by the first clone, inside
that clone's transaction. Structural validation accepts zero or an allocatable
LBA and nothing else; kind, owner and generation are checked when the node is
loaded.

### Reference counts are per checkpoint

A checkpoint's tree counts the live mappings **of that checkpoint**. References
held only by the other selectable checkpoint are not added to the current
count: they are protected by retirement and quarantine
([ADR-036](ADR-036-reclaim-queue.md)), which is the same mechanism that already
protects unshared blocks a stale checkpoint can still reach. Mixing the two
would make the count ambiguous and unverifiable.

### Operations

- **Clone.** `CloneFile`/`CloneRange` copy the source extent records into the
  destination map, set `EXTENT_SHARED` on both sides, and insert or increment
  the reference records for the covered runs. Clone is a checkpoint
  transaction, never an intent-log operation; if a log window is open it is
  committed first, so the clone is never split across the two mechanisms.
- **Direct layout cannot be shared.** The direct representation stores a run in
  the object record itself and carries no extent flag word, so a file entering
  a clone as source or destination is promoted to an extent-tree map in the
  same transaction, before publication. A direct-layout file is by construction
  private, and a file whose map carries `EXTENT_SHARED` — even a stale one —
  never collapses back to direct.
- **Write into a shared range.** Replacement blocks are allocated for the
  modified logical range, the reference to the overwritten sub-range is
  dropped under the rule below, and the private extent replaces it in that
  object's map only. This is unconditional and independent of Q1, which governs
  writes to *private* committed data.
- **Dropping a reference never frees the run.** Unlink, truncate and
  write-into-shared partition every flagged extent as above, and each sub-run
  is resolved by what the tree says *before* the operation:

  | Before | Action | Blocks |
  |---|---|---|
  | record, count > 2 | decrement, record kept | not retired |
  | record, count = 2 | record removed, since only counts of two or more are stored | **not retired** — one live mapping remains |
  | no record | sole owner | retired into the reclaim queue as today |

  The middle row is the one that destroys data if it is got wrong: a count
  falling to one means a peer still maps those blocks. The run leaves the tree
  and becomes an ordinary private run of that surviving object, and it is
  retired only later, when that last owner drops it as a gap. The reference
  change and the extent-map change are in the same transaction.
- **Uncertainty is fail-closed.** If the root or a lookup cannot be read, the
  mutation aborts and returns the error; the storage stays marked allocated and
  nothing is freed, published or invented. Moving such storage to quarantine is
  a later repair decision made by the checker, not something the core does
  behind an unreadable tree.

### Intent log

Clone is not journalled, but that alone is not enough: an existing
`LogOp::Delete`, or a `Rename` with replacement, can remove a file holding
shared extents. Their logical replay must therefore maintain reference counts
in the same transaction that applies the operation
([ADR-037](ADR-037-intent-log.md)). No new log operation is required, and the
crash matrix must cover at least one shared unlink and one shared
rename-replacement that are fsynced and then replayed.

### Compatibility

The feature registry gains `org.aros.afsplus:shared-extents`, class
`ro_compat`, lifecycle experimental, `authoritative = true`,
`rebuildable = true` (by exhaustive scan of every extent map),
`discardable = false`.

Identification is immutable, so the first clone cannot activate a bit. The
feature is therefore activated by `mkfs` according to the requested
compatibility profile — `workstation` and `full` enable it, `reader-minimal`,
`classic-rw` and `boot-safe` do not — with the root left at zero until the
first clone. A volume without the feature rejects clone operations as
unsupported rather than silently copying.

Feature, root and flag must agree, which makes both the mount contract and the
stale flag unambiguous: a non-zero root, or an extent carrying `EXTENT_SHARED`,
on a volume where the feature is not enabled is corruption; the feature enabled
with a zero root is the legal enabled-but-unused state. Once the first clone
allocates the root it stays allocated and non-zero even after the last sharing
disappears, so the tree is simply empty rather than oscillating between
existing and not. Making sharing mandatory epoch
semantics was rejected: it would deny read-write access to exactly the
constrained implementations the classic profile exists to serve.

## Validation

The checker rebuilds the expected reference state by scanning the extent map of
every live object — a forward scan, no reverse map — and comparing it with the
tree. The expected state is defined per physical block, but that is its
semantics and not a prescribed representation: an implementation sweeps
interval endpoints rather than materialising an array proportional to the
volume. Equality is required in **both** directions, so that a missing record
is as detectable as a wrong one:

- every maximal sub-run whose expected count is at least two has exactly one
  record, with that count and those bounds;
- every record corresponds to such a sub-run: no orphan, no stale count, no
  record whose expected count is one;
- records are ordered, non-overlapping, maximal, and within volume bounds;
- no record's run intersects authoritative free space or quarantine;
- an extent flagged `EXTENT_SHARED` with no overlapping record is accounted
  private, which is legal; an extent not flagged whose run overlaps a record is
  reported as corruption;
- no run that left the tree in the transaction under test appears in the
  reclaim queue while a live mapping still covers it — the direct check for the
  premature-release bug;
- no object in direct layout is reachable from a map carrying `EXTENT_SHARED`,
  and no shared run is covered by a direct-layout object.

Because the tree is published in the checkpoint that publishes the extent maps,
every modelled crash state selects either the complete pre-operation or the
complete post-operation state. The mandatory matrix injects a power cut after
every write and every flush of: clone creation, a write that splits a shared
run, unlink of one clone while another lives — the count-two case, whose
recovered states must still show the survivor's bytes intact — reclamation of
the last reference once it is an ordinary private run, and the
fsynced-then-replayed shared unlink and rename-replacement above.

## Consequences

ADR-027's required invariants become executable and checkable, and the epoch-1
gate "shared extents cannot be freed while referenced by any live object or
retained recovery state" acquires a mechanism and a test.

[`spec/invariants.md`](../spec/invariants.md) gains the shared-block exception
its own text anticipates. The write path acquires the shared/private fork that
open question Q1 needs in order to be decided by measurement rather than by
default.

Removing the transitional inline region records completes ADR-035 and closes
the checkpoint layout to one interpretation, at the cost of rebuilding
prototype images rather than migrating them.

The wire format is experimental. The record shape, the tree kind, the flag bit
and the checkpoint offset are not epoch-1 commitments until M14.

`CloneTree`, the reverse map of ADR-024 and user-data checksums stay out of
scope, as ADR-027 and the open-questions register already record.
