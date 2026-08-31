# 06. Files and Extents

> **ADRs:** [ADR-027](../adr/ADR-027-reflink-clones.md), [ADR-061](../adr/ADR-061-shared-extent-references.md) · **Spec:** none ·
> **Tests:** [crash-testing](../testing/crash-testing.md) · **Milestones:** M03

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

### Executable Core Scale-1 representation

The current prototype deliberately starts with the smallest useful inline
case: one contiguous direct extent in the object record. A file sets the
experimental `OBJECT_FLAG_EXTENT_TREE` bit when `data_root` instead references
an `AFST` extent-map root. This is a prototype encoding, not an epoch-1 freeze.

Extent-tree leaves use the logical start block as an eight-byte big-endian
search key. The typed value contains a little-endian physical start block,
64-bit block count, 32-bit flags, and zeroed reserved field. Trees can be
bulk-built across multiple nodes on their first publication; later mutations
use the shared COW split/merge engine.

## 3. Sparse files

A missing logical range represents a hole.

Reads from holes return zero.

Writing into a hole allocates storage transactionally.

`Volume::write_file_at` implements this rule with full data COW: every touched
logical block is reconstructed in fresh storage, user data is made durable
before the new extent root, and the old mapping is quarantined. A write beyond
EOF therefore creates a real missing logical range rather than materializing
zero-filled blocks.

## 4. Preallocation

The API may request space reservation without immediately increasing visible logical file size.

This is useful for databases, large downloads, and reducing fragmentation.

The prototype marks reserved mappings `EXTENT_UNWRITTEN`. Such mappings count
as allocated storage but read as zeros and do not change logical size. A later
write replaces only the touched unwritten blocks with ordinary written COW
extents. Because preallocation does not change visible bytes, it advances the
metadata-change timestamp but preserves modification time and content
generation; content scanners therefore do not rescan an unchanged file.

## 5. Truncation

Shrinking a file must:

1. update logical size transactionally
2. retire/release fully unused extents according to checkpoint/reference rules
3. zero or define the newly exposed tail behavior when the file is re-extended
4. send discard only after blocks are no longer reachable by any state that may legally reference them

The prototype's `Volume::truncate_file` supports sparse growth and shrinking.
A partial written tail block is copied and zeroed past the new EOF before the
new size is published, preventing stale tail bytes from reappearing after a
later extension. Removed tree/data blocks enter the same checkpoint quarantine
as unlink; physical discard is still deferred.

## 6. Shared extents/reflinks

The epoch-1 extent architecture must be able to represent shared physical data ranges for reflinks.

A shared range is never modified in place while another live object still references the same bytes. A write first creates private replacement storage for the modified logical range.

The reference mechanism that carries this is a volume-wide typed reference tree keyed by physical run, with the extent's shared flag acting as a hint and the tree as the authority ([ADR-061](../adr/ADR-061-shared-extent-references.md)); the requirement it satisfies is [ADR-027](../adr/ADR-027-reflink-clones.md).

The general policy for writes to **unshared** committed data remains an explicit transaction-prototype question. See [`docs/08-transactions-and-journal.md`](08-transactions-and-journal.md).

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
