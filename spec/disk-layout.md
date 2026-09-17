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
   header flags, exact payload length (96 bytes, or 112 with the security
   reference, plus the inline target of a symlink) and a zero tail
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
   168 bytes, or 184 with the snapshot roots after it; the identification
   block keeps the format-time label and is never rewritten
12. the format descriptor records both the comparison-key algorithm and the Unicode normalization/casefold table version; prototype identification v3 uses Unicode 16.0.0
