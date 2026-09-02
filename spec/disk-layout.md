# Disk Layout Draft

This document gives the logical layout. Exact block offsets marked TBD must be frozen before epoch 1.

The layout uses a discoverable auxiliary intent-log area alongside checkpoint
COW. Its exact size and record encoding remain unfrozen until the operation
coverage and interoperability gates of
[ADR-063](../adr/ADR-063-intent-log-epoch1.md) pass. The authoritative
allocation encoding remains a separate epoch-1 freeze question.

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
3. internal objects have reserved object IDs
4. placement hints are not semantic invariants
5. redundant recovery/checkpoint descriptor locations are deterministic from immutable format parameters or discoverable through an independently validated format descriptor
6. a small durability intent log uses a reserved/discoverable auxiliary area;
   no record format or mandatory size is frozen until cross-implementation,
   real-device and independent format-review gates pass
7. allocation regions are required for bounded resource use, but their authoritative free-space encoding remains an epoch-1 prototype decision (reserved allocator, bitmap+delta, spacemap-like log, or proven hybrid)
8. the format descriptor records both the comparison-key algorithm and the Unicode normalization/casefold table version; prototype identification v3 uses Unicode 16.0.0
