# 06. Files and Extents

## 1. Extent model

Regular-file data is represented as mappings:

```text
logical file block -> physical volume block, length, flags
```

Contiguous data requires one extent rather than one pointer per block.

## 2. Inline extents

Small files should store a small fixed number of extents directly in the object record.

Larger or fragmented files spill into an extent B+ tree.

This keeps the common case cheap without imposing a fixed maximum extent count.

## 3. Sparse files

A missing logical range represents a hole.

Reads from holes return zero.

Writing into a hole allocates storage transactionally.

## 4. Preallocation

The API may request space reservation without immediately increasing visible logical file size.

This is useful for databases, large downloads, and reducing fragmentation.

## 5. Truncation

Shrinking a file must:

1. update logical size transactionally
2. release fully unused extents
3. zero or define the newly exposed tail behavior when the file is re-extended
4. send discard only after blocks are no longer reachable by committed metadata

## 6. Optional inline-data feature

A future optional feature may store tiny file contents inside object metadata.

Motivation:

- source trees contain huge numbers of sub-block files
- Cargo and package metadata often consists of tiny files
- avoiding separate data-block allocation reduces metadata traffic and I/O

This feature is not part of the mandatory reader profile.

A volume that actually stores inline data must advertise the corresponding incompatible feature unless a reader can always obtain an equivalent external representation.

## 7. Maximum file size

The format uses 64-bit byte sizes and 64-bit block addressing.

Implementations may publish smaller operational limits, but those limits are implementation limits, not format-era 2 GiB or 4 GiB barriers.
