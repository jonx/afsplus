# 07. Allocation

## 1. Allocation regions

The volume is divided into fixed-size allocation regions.

Each region owns:

- allocation bitmap
- free-block count
- largest-known-free-run hint
- metadata/data allocation hints
- generation
- checksum

The region structure is intended to bound memory use and repair scope.

## 2. Proposed region size

Initial default: 1 GiB of address space per region.

At 4 KiB blocks:

```text
1 GiB / 4 KiB = 262,144 blocks
bitmap = 262,144 bits = 32 KiB
```

A low-memory implementation can load a single 32 KiB bitmap rather than a bitmap for the entire disk.

Region size is stored in the superblock and must be a power-of-two multiple of the logical block size.

## 3. Allocation strategy

Preferred order:

1. extend adjacent file extent if possible
2. allocate within the object's current locality region
3. use a nearby region with sufficient free run
4. fall back to general free regions

Directories and their small child objects should have locality hints, not hard placement requirements.

## 4. Free-space summaries

Global free-space summaries are accelerators.

The local region bitmap is authoritative.

If a summary disagrees with a bitmap:

- ignore/rebuild the summary
- do not mark allocated blocks free based only on the summary

## 5. Metadata reservation

A small emergency metadata reserve prevents the filesystem from becoming impossible to update cleanly when nearly full.

User-visible free-space reporting must distinguish normally allocatable space from emergency reserved metadata space.

## 6. Discard

Discard/TRIM is issued through the block provider after deallocation becomes durable.

Discard failure does not make the filesystem inconsistent.

## 7. Allocation integrity

No committed allocation state may allow two live objects to own the same physical block unless a future explicitly enabled shared-block feature defines such behavior.
