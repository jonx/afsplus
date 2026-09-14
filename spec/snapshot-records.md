# Snapshot record encoding

The experimental [ADR-072](../adr/ADR-072-snapshot-record-codecs.md) records
implement the accounting direction in
[ADR-071](../adr/ADR-071-snapshot-lifetime-prototype.md). Qualification is in the
[book-review gate](../testing/book-review-qualification.md#snapshot-record-codecs).
The [milestone table](../implementation/milestones.md) owns implementation state.

<!-- toc -->

- [Feature and tree identities](#feature-and-tree-identities)
- [Checkpoint binding](#checkpoint-binding)
- [Registry control and entries](#registry-control-and-entries)
- [Lifetime control and entries](#lifetime-control-and-entries)
- [Transactional lifetime edits](#transactional-lifetime-edits)
- [Volume lifecycle and read views](#volume-lifecycle-and-read-views)
- [Exhaustive ownership checking](#exhaustive-ownership-checking)
- [Failure and resource contract](#failure-and-resource-contract)

<!-- /toc -->

## Feature and tree identities

`org.aros.afsplus:persistent-snapshots` uses INCOMPAT bit 2. Implementations need
the complete ownership protocol to support this feature; standalone codecs do
not authorize mounts. AFST kinds 6 and 7 denote the snapshot registry and
lifetime ledger. Both require owner zero and the common checked block header.
Keys are eight-byte big-endian integers; each leaf value is 32 bytes.

## Checkpoint binding

[ADR-073](../adr/ADR-073-snapshot-checkpoint-roots.md) extends the checkpoint
payload from 96 to 112 bytes on snapshot-enabled volumes. Offsets 96 and 104
hold the registry and lifetime roots as little-endian u64 values. Both are
nonzero, distinct and within allocatable geometry. Feature-absent checkpoints
use exactly 96 bytes. Reject other payload lengths; keep flags at offset 80
zero. The checksum covers the complete block.

After structural selection, reject disagreement between the selected payload
and immutable snapshot bit. This error cannot cause fallback to an older
namespace. Root/control reads remain bounded; full ownership validation belongs
to the checker. Publishing both roots uses the common COW checkpoint boundary.

## Registry control and entries

Key zero stores the next snapshot ID at value offset 0 as a little-endian u64.
Bytes 8 through 31 must be zero. Next ID begins at one; UINT64_MAX denotes
exhaustion and cannot be allocated. Reject zero, reuse and wraparound.

Each nonzero key below next ID names one view:

| Value offset | Size | Field |
|---|---|---|
| 0 | 8 | Captured checkpoint generation |
| 8 | 8 | Captured committed transaction ID |
| 16 | 8 | Captured object-map root LBA |
| 24 | 8 | Reserved zero |

Captured generation is nonzero and no greater than the enclosing checkpoint.
Committed transaction ID is nonzero and no greater than captured generation.
The object-map root is nonzero, within the volume and a valid namespace tree
root outside permanently reserved allocator storage.

## Lifetime control and entries

Key zero stores next scan position at offset 0 and retained physical block
count at offset 8, both little-endian u64. Bytes 16 through 31 must be zero.
Scan position is less than total blocks; zero starts or wraps traversal.
Retained count is at most total blocks and equals the contextual retired-run
sum. It excludes ordinary quarantine after ownership transfer.

Each nonzero physical-start key stores:

| Value offset | Size | Field |
|---|---|---|
| 0 | 8 | Run length in blocks |
| 8 | 8 | Allocation birth generation |
| 16 | 8 | Last-live-reference retirement, or zero while live |
| 24 | 8 | Reserved zero |

Length and birth are nonzero. Checked start plus length fits within the volume.
Birth does not exceed the enclosing checkpoint generation. Nonzero retirement
is strictly greater than birth and at most the enclosing generation. An
allocation discarded within its birth transaction is released without becoming
a committed retired lifetime. Splits preserve birth; merge adjacent records
only when their lifetime fields match. Reject overlaps and runs intersecting
reserved geometry or the allocation-root pool.

A snapshot generation S intersects `[birth, retirement)`; a live run has no
retirement upper bound. Global ownership validation checks live namespace,
retained snapshots, housekeeping and quarantine separately. Codec validation
checks fixed fields; it does not replace the contextual ownership proof.

## Transactional lifetime edits

[ADR-071](../adr/ADR-071-snapshot-lifetime-prototype.md) governs ownership edits.
New namespace allocations receive the publication generation as birth; reflinks
preserve the existing lifetime. Removing the last live reference splits affected
runs while preserving birth and sets retirement to the new generation. Discard
allocations removed within their birth transaction through uncommitted release.
Merge adjacent equal lifetimes after all edits, with checked retained-count
adjustments from the original and final affected retired records.

Before a retired record can leave the ledger, verify its exact committed fields
and every registered view's generation. No view may intersect its lifetime.
Exceeding a configured preparation or registry-scan budget is an error, never
permission to skip records. A deleted view can conservatively protect storage
until a subsequent reclamation transaction observes the updated registry.

Publish the ledger mutation, retained total, scan cursor and ordinary-quarantine
transfer in the same checkpoint. Queue transfers at that publication generation;
keep their allocation bits set through the existing selectable-checkpoint delay.
Ledger and registry COW nodes remain housekeeping allocations. Close namespace
capture before maintaining those trees. Require a successful lifetime seal before
allocator finalization; a failed seal invalidates the transaction, and callers
cannot mutate allocation ownership after sealing. Internal allocator finalization
can still allocate its housekeeping blocks. Return lifetime writes and replacement
roots with the bitmap/quarantine result for one checkpoint publication.
Preparing an edit alone cannot authorize physical reuse, publish a snapshot or
enable the feature.
The [transaction preparation gate](../testing/book-review-qualification.md#lifetime-transaction-preparation)
checks this stage independently of the enclosing Volume integration.

## Volume lifecycle and read views

Under ADR-071, snapshot creation closes the intent window before capturing the
committed namespace and publishes the registry entry through the common commit
engine. Admission checks include explicit work budgets, view capacity and ID
exhaustion; reject exhausted IDs before closing a pending window. Creation
preserves emergency metadata headroom. Deletion can consume that headroom to
release retention, without promising success under every resource shortage.
Uncertain checkpoint publication requires remount before further mutation.

A runtime handle pins its registered view. Clones share that lease; deletion
returns busy until the last handle closes. Handles belong to one mount and are
stale after remount. A directory cursor identifies volume, snapshot, generation,
directory and ordinal; a matching persistent view can resume it with a newly
opened handle. A cursor alone does not retain a snapshot.

Reads resolve objects, directories and extents through the captured object-map
root and generation, independently of live caches or an open intent window.
Registered views force COW even for files with persistent in-place opt-in.
[ADR-074](../adr/ADR-074-protect-previous-checkpoint.md) extends protection to
views registered in the previous structurally valid checkpoint until its slot
is replaced, together with the corresponding quarantine boundary.
Historical reflinks do not use the live shared-reference count to decide whether
their bytes exist. No rollback or historical-to-live clone is implied.

Reclaim examines bounded ledger pages and streams the committed registry within
an explicit view budget. A creation capturing generation G cannot protect a
previously retired run whose retirement is at most G. Consulting the original
registry during creation is therefore safe; consulting it during deletion can
conservatively delay release by a subsequent transaction. Transfer validation
rechecks exact lifetimes and all views before sealing ownership changes.
Report scanned records, scan wrap, transfers, observed checkpoint blocking and
physical promotions separately;
zero net ordinary-queue reduction does not mean the retained scan is finished.

The core experiment requires explicit edit, view and reclaim budgets. Selecting
shipping limits requires measured admission and resource qualification under
[Q4](../implementation/open-questions.md). Host management authorization and
historical-read access/revocation follow
[ADR-075](../adr/ADR-075-revocable-backup-capability.md) for trusted backup.
The [Q5](../implementation/open-questions.md) security gate requires testing
that host contract before exposure through filesystem-neutral capabilities;
ordinary-user historical browsing remains a separate decision. Core read handles do not establish that
policy. Full checker ownership, supported writable-mount negotiation and a real backup
consumer are separate integration gates. `mkfs_with_options` accepts an explicit
`MkfsOptions::persistent_snapshots` choice for new images; baseline `mkfs`
keeps the feature absent. This is not an in-place conversion.

`mount_with_snapshot_limits(device, options, limits)` explicitly opts the Rust
core into snapshot feature negotiation. All three budgets must be positive,
reclaim work must fit the edit budget, and the registered view count must fit
the supplied view budget. Validate these conditions before replay or orphan
recovery can write. The ordinary `mount` and `mount_with_options` entry points
retain their baseline supported-feature mask and reject bit 2. No shipping
budget, adapter capability or filesystem API v2 ABI is implied by this core API.

ReadOnly and NoChanges inspect the pending log without writing and expose the
selected committed namespace. Recovery replays acknowledged log operations,
then denies user mutations; ReadWrite permits both replay and later mutations.
Every mode validates selected bounded snapshot root/control state without
falling back on descendant corruption. Exhaustive per-view namespace checking
belongs to maintenance. Caller limits bound mutation work; a read-only mount
still requires explicit opt-in and valid view admission.

## Exhaustive ownership checking

The read-only checker validates all registry/ledger nodes, ID bounds, canonical
lifetimes and the exact retained sum. Each captured namespace receives its own
object, directory, link, topology and extent checks, bounded by its captured
generation. Historical shared mappings require negotiated sharing markers;
the live reference-count tree cannot validate their historical multiplicities.

Every live namespace block has a live lifetime. Every captured block intersects
its lifetime; every retired ledger block is absent from live ownership.
Housekeeping, quarantine and namespace ownership cannot alias. Reachable
metadata cannot overlap data across views. Compare immutable metadata header
generations with allocation births; raw file data has no embedded birth witness.
Bitmap accounting includes retired ledger runs even when no view intersects
them, because a bounded reclaim scan can legitimately leave eligible work.
Count physical metadata/data sharing across views once in checker summaries.

Exhaustive checking can materialize physical-block sets and one historical
namespace at a time. Its memory and work scale with ledger coverage, distinct
reachable blocks and the sum of traversed view sizes; this is not normal mount
behavior or a constrained-profile resource guarantee. Corrupt snapshot state
is reported, never discarded as repair. Ordinary mount support is negotiated
independently of read-only checker support.

## Failure and resource contract

Reject incorrect key/value lengths before slicing, nonzero reserved bytes,
overflow and invalid generation relationships. A failed decoder returns no
record. Repair tools identify the affected tree and key and require explicit
salvage policy before discarding registered views or lifetime evidence.

One record uses 48 bytes including key and generic item overhead. A 4 KiB leaf
holds 84 records. Fixed-record codecs use stack arrays with no heap allocation.
Record count, tree packing, cursor updates and snapshot count determine the
integrated metadata/RAM cost and require measured qualification. Root binding follows ADR-073; transactional lifetime maintenance and persistence
require the integrated ownership and crash gates.
