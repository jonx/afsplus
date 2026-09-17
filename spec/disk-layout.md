# Disk Layout Draft

This document gives the logical layout. Exact block offsets marked TBD must be frozen before epoch 1.

The layout uses a discoverable auxiliary intent-log area alongside checkpoint
COW. Its exact size and record encoding remain unfrozen until the operation
coverage and interoperability gates of
[ADR-063](../adr/ADR-063-intent-log-epoch1.md) pass. The authoritative
allocation architecture follows [ADR-067](../adr/ADR-067-epoch1-allocation-state.md);
exact byte layouts require the epoch-1 freeze gate.

```text
+----------------------------------+
| identification / boot-safe       |
+----------------------------------+
| checkpoint / format descriptors  |
+----------------------------------+
| optional auxiliary transaction   |
| state/log area descriptor         |
+----------------------------------+
| allocation region table          |
+----------------------------------+
| allocation region 0              |
|   bounded free-space state        |
|   metadata/data                   |
+----------------------------------+
| allocation region 1              |
|   bounded free-space state        |
|   metadata/data                   |
+----------------------------------+
| ...                              |
+----------------------------------+
| redundant recovery descriptors   |
+----------------------------------+
```

Rules:

1. checkpoint/superblock and metadata structures are logical-block aligned where their encoding requires it
2. all physical ranges are bounds checked against total blocks
3. object ID 0 is invalid, object ID 1 is the user root, object ID 2 is the
   feature-gated orphan directory, IDs 3 through 15 remain reserved, and
   dynamic allocation begins at 16
4. placement hints are not semantic invariants
5. redundant recovery/checkpoint descriptor locations are deterministic from immutable format parameters or discoverable through an independently validated format descriptor
6. a small durability intent log uses a reserved/discoverable auxiliary area;
   no record format or mandatory size is frozen until cross-implementation,
   real-device and independent format-review gates pass
7. allocation regions use the authoritative triple-version bitmap/descriptor
   slots, fixed allocation-root pool and segmented quarantine of ADR-067
8. an object record is admitted only in its canonical image
   ([ADR-100](../adr/ADR-100-exact-object-record-admission.md)): zero common
   header flags, exact payload length (96 bytes, plus each optional field its
   object flags announce, plus the inline target of a symlink) and a zero
   tail
9. with `org.aros.afsplus:security-descriptors` (INCOMPAT bit 3) an object
   record may carry the 16-byte security reference at payload offset 96 and
   own a chain of `"AFSX"` descriptor segments of 24 fixed bytes plus
   descriptor bytes, every segment but the last full
   ([ADR-101](../adr/ADR-101-security-preservation-container.md)); the
   reference on a volume without the feature is corruption; where a
   well-formed reference points is chain state, judged when the chain is
   walked ([ADR-105](../adr/ADR-105-security-reference-admission.md))
10. the permanently allocated areas are consecutive runs of allocatable
   blocks counted from the start of the volume, skipping every region's
   reserved head ([geometry](../crates/afsplus-format/src/geometry.rs)):
   first the four bootstrap metadata blocks the formatter writes (root object
   record, root directory, object-map root, reclaim-queue root), then the
   allocation-root pool of `3N` blocks
   ([ADR-035](../adr/ADR-035-allocation-root-reserved-pool.md),
   [ADR-036](../adr/ADR-036-reclaim-queue.md)), then the `log_slots`
   intent-log slots ([ADR-037](../adr/ADR-037-intent-log.md)). `N` is the node
   count of the bulk-packed allocation-root tree: one leaf per leaf capacity
   of regions (four-byte key, 16-byte value), then levels of internal nodes
   of full fanout (four-byte separator, 16-byte child reference, plus the
   leftmost child) up to a single root. At 4 KiB blocks and 512-block regions
   a four-region volume has its bootstrap metadata at blocks 9 to 12, its pool
   at 13 to 15 and eight log slots at 16 to 23
11. the current volume label is committed state in the checkpoint payload
   ([ADR-104](../adr/ADR-104-volume-label-in-checkpoint.md)): length at offset
   96, seven zero bytes, 64 bytes of NUL-free UTF-8 zero padded, payload of
   168 bytes, or 184 with the snapshot roots after it (registry root at 168,
   lifetime-ledger root at 176, both nonzero and distinct,
   [ADR-073](../adr/ADR-073-snapshot-checkpoint-roots.md)); nothing follows
   the payload, and the form belongs to the persistent-snapshots feature: a
   selected checkpoint in the other form refuses the volume
   ([ADR-111](../adr/ADR-111-checkpoint-zero-tail.md)); the
   identification block keeps the format-time label and is never rewritten
12. an object record may carry the 16-byte attribute reference (object flag
   bit 4) after the security reference and own a chain of `"AFSA"` segments,
   laid out as the `"AFSX"` segments are, whose content is the object's whole
   attribute set: a count, then entries in strictly ascending order of name
   bytes, at most 65,536 bytes
   ([ADR-108](../adr/ADR-108-extended-attributes.md)); no feature gates it
13. the format descriptor records both the comparison-key algorithm and the Unicode normalization/casefold table version; prototype identification v3 uses Unicode 16.0.0

<!-- toc -->

- [Reclaim queue blocks](#reclaim-queue-blocks)
- [Snapshot records](#snapshot-records)

<!-- /toc -->

## Reclaim queue blocks

The reclaim queue ([ADR-036](../adr/ADR-036-reclaim-queue.md)) has three block
kinds. Each starts with the common 32-byte header; offsets below are inside
the payload, integers are little-endian. The Rust codec
([reclaim.rs](../crates/afsplus-format/src/reclaim.rs)) and the portable C
decoders (`afspr_decode_reclaim_root`, `afspr_decode_reclaim_segment`,
`afspr_decode_reclaim_table`) are held to the same verdict image by image in
[the cross-read test](../crates/afsplus-format/tests/reclaim_c.rs).

An entry is 20 bytes: run start (8), block count (4, nonzero), retire
generation (8, nonzero); the run end fits 64 bits. A reference is 12 bytes:
block (8) and item count (4): 1 to 338 segment references for a table, 1 to
202 entries for a segment.

Root, `"AFSH"`:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | version, 1 |
| 4 | 4 | reserved, zero |
| 8 | 8 | pending blocks, equal to appended minus reclaimed |
| 16 | 8 | appended blocks, monotonic total |
| 24 | 8 | reclaimed blocks, monotonic total, at most appended |
| 32 | 4 | head segment offset: consumed references in the first table; below that table's count, zero without a table |
| 36 | 4 | head entry offset: consumed entries in the head segment; zero without a sealed block, below the first segment's count when no table exists |
| 40 | 4 | head block offset: consumed blocks of the head entry; zero without a sealed block |
| 44 | 2 | inline entry capacity, nonzero |
| 46 | 2 | segment reference capacity, nonzero |
| 48 | 2 | table reference capacity, nonzero |
| 50 | 2 | reserved, zero |
| 52 | 4 | table reference count, at most its capacity |
| 56 | 4 | segment reference count, at most its capacity |
| 60 | 4 | inline entry count, at most its capacity |
| 64 | 12 × table capacity | table references, oldest first |
| after the tables | 12 × segment capacity | segment references, oldest first, newer than every table |
| after the segments | 20 × inline capacity | inline entries, oldest first |

The three areas sit at their capacity offsets whatever their counts, and
64 + 12 × (table + segment capacity) + 20 × inline capacity fits the block
and is the payload length.

Sealed segment, `"AFSS"`, and sealed table, `"AFSL"`:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | item count, nonzero, at most 202 entries or 338 references |
| 4 | 4 | reserved, zero |
| 8 | count × 20 or count × 12 | entries, or segment references |

The payload length of a sealed block is exactly 8 plus its items.

Admission is exact
([ADR-110](../adr/ADR-110-exact-reclaim-admission.md)): zero flags and zero
owner in the common header, a root payload of exactly 64 + 12 × (table +
segment capacity) + 20 × inline capacity, zero bytes in the unused slots of
each root area, and zero bytes after the payload.


## Snapshot records

A volume with persistent snapshots keeps two trees, named by the checkpoint
([ADR-073](../adr/ADR-073-snapshot-checkpoint-roots.md)): the snapshot registry
(tree kind 6) and the lifetime ledger (tree kind 7). Both use 8-byte big-endian
keys and 32-byte little-endian values whose unused bytes are zero
([ADR-072](../adr/ADR-072-snapshot-record-codecs.md)). Key zero of each tree is
its control record. The Rust codec
([snapshot.rs](../crates/afsplus-format/src/snapshot.rs)) and the portable C
decoders (`afspr_decode_snapshot_*`) are held to the same verdict value by
value in [the cross-read test](../crates/afsplus-format/tests/snapshot_c.rs).

| Tree | Key | Offset | Size | Field |
|---|---|---:|---:|---|
| registry | 0 | 0 | 8 | next snapshot ID, nonzero; never wraps |
| registry | snapshot ID | 0 | 8 | captured generation, nonzero, at most the checkpoint's |
| | | 8 | 8 | committed transaction, nonzero, at most the captured generation |
| | | 16 | 8 | object-map root of the view, nonzero, inside the volume |
| ledger | 0 | 0 | 8 | reclaim scan position, below the volume's block count |
| | | 8 | 8 | retained blocks, at most the volume's block count |
| ledger | first block of a run, nonzero | 0 | 8 | blocks in the run, nonzero; the run ends inside the volume |
| | | 8 | 8 | birth generation, nonzero, at most the checkpoint's |
| | | 16 | 8 | retirement generation: zero while live, otherwise above the birth and at most the checkpoint's |
