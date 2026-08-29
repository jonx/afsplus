# Disk Layout Draft

This document gives the logical layout. Exact block offsets marked TBD must be frozen before epoch 1.

The layout deliberately avoids freezing a conventional journal area or a flat authoritative allocation bitmap before the transaction/allocation prototypes resolve those questions.

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
6. a future small durability/intent log may use a reserved/discoverable auxiliary area, but no journal record format or mandatory size is frozen yet
7. allocation regions are required for bounded resource use, but their authoritative free-space encoding remains an epoch-1 prototype decision (reserved allocator, bitmap+delta, spacemap-like log, or proven hybrid)
8. the format descriptor must record the Unicode normalization/casefold table version used for directory comparison-key generation
