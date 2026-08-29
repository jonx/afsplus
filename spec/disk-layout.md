# Disk Layout Draft

This document gives the logical layout. Exact block offsets marked TBD must be frozen before epoch 1.

```text
+------------------------------+
| identification / boot-safe   |
+------------------------------+
| primary superblock           |
+------------------------------+
| journal area                 |
+------------------------------+
| allocation region table      |
+------------------------------+
| allocation region 0          |
|   bitmap                     |
|   metadata/data              |
+------------------------------+
| allocation region 1          |
|   bitmap                     |
|   metadata/data              |
+------------------------------+
| ...                          |
+------------------------------+
| backup superblock(s)         |
+------------------------------+
```

Rules:

1. superblocks and metadata are logical-block aligned
2. all physical ranges are bounds checked against total blocks
3. internal objects have reserved object IDs
4. placement hints are not semantic invariants
5. backup superblock locations are deterministic from immutable format parameters
