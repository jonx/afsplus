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
   reference on a volume without the feature is corruption, and the first
   segment is bounds checked where the record is admitted
10. the format descriptor records both the comparison-key algorithm and the Unicode normalization/casefold table version; prototype identification v3 uses Unicode 16.0.0
