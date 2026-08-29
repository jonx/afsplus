# Constrained Reader Example

A classic-system reader can:

1. allocate one logical-block buffer
2. read the superblock
3. validate supported features
4. walk the directory B+ tree one page at a time
5. load an object record
6. stream file extents

It does not need:

- catalog
- change stream
- full allocation bitmap
- full journal in memory
- compression
- snapshots
