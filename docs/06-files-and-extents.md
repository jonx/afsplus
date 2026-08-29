# 06. Files and Extents

## 1. Extent model

Regular-file data is represented as mappings:

```text
logical file block -> physical volume block, length, flags
```

Contiguous data requires one extent rather than one pointer per block.

The base extent record includes a versioned flag namespace. Unknown semantic flags are governed by their owning feature's compatibility class.

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
2. retire/release fully unused extents according to checkpoint/reference rules
3. zero or define the newly exposed tail behavior when the file is re-extended
4. send discard only after blocks are no longer reachable by any state that may legally reference them

## 6. Shared extents/reflinks

The epoch-1 extent architecture must be able to represent shared physical data ranges for reflinks.

A shared range is never modified in place while another live object still references the same bytes. A write first creates private replacement storage for the modified logical range.

The general policy for writes to **unshared** committed data remains an explicit transaction-prototype question. See `docs/08-transactions-and-journal.md`.

## 7. Reserved optional user-data checksum association

Full user-data checksumming is not required by the first production profile.

However, the base extent flag namespace reserves a `DATA_CHECKSUM_PRESENT` association bit and the feature registry reserves `org.aros.afsplus:data-checksums` now.

This reservation does **not** freeze:

- checksum algorithm
- checksum block/range granularity
- checksum tree/layout
- compatibility class
- whether checksums are stored inline or separately

It only guarantees that a later checksum feature can associate checksum metadata with extents/ranges without redefining the base extent record incompatibly.

## 8. Optional tiny-file storage

A future optional feature may store tiny file contents inside object metadata or another compact small-file representation.

Motivation:

- source trees contain huge numbers of sub-block files
- Cargo and package metadata often consists of tiny files
- avoiding separate data-block allocation may reduce metadata traffic and I/O

The exact mechanism is not frozen. Inline data, packed small-file storage, and ordinary extents with strong locality must be benchmarked first.

A volume that activates an incompatible tiny-file representation must advertise the corresponding feature.

## 9. Maximum file size

The format uses 64-bit byte sizes and 64-bit block addressing.

Implementations may publish smaller operational limits, but those limits are implementation limits, not format-era 2 GiB or 4 GiB barriers.
