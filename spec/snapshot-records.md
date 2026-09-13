# Snapshot record encoding

The experimental [ADR-072](../adr/ADR-072-snapshot-record-codecs.md) records
implement the accounting direction in
[ADR-071](../adr/ADR-071-snapshot-lifetime-prototype.md). Qualification is in the
[book-review gate](../testing/book-review-qualification.md#snapshot-record-codecs).
The [milestone table](../implementation/milestones.md) owns implementation state.

## Feature and tree identities

`org.aros.afsplus:persistent-snapshots` uses INCOMPAT bit 2. Implementations need
the complete ownership protocol to support this feature; standalone codecs do
not authorize mounts. AFST kinds 6 and 7 denote the snapshot registry and
lifetime ledger. Both require owner zero and the common checked block header.
Keys are eight-byte big-endian integers; each leaf value is 32 bytes.

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

## Failure and resource contract

Reject incorrect key/value lengths before slicing, nonzero reserved bytes,
overflow and invalid generation relationships. A failed decoder returns no
record. Repair tools identify the affected tree and key and require explicit
salvage policy before discarding registered views or lifetime evidence.

One record uses 48 bytes including key and generic item overhead. A 4 KiB leaf
holds 84 records. Fixed-record codecs use stack arrays with no heap allocation.
Record count, tree packing, cursor updates and snapshot count determine the
integrated metadata/RAM cost and require measured qualification. Root binding
and the transactional persistence protocol need a follow-up format record.
