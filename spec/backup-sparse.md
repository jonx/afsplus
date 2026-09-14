# Sparse archive content transport

[ADR-087](../adr/ADR-087-sparse-archive-content.md) selects GNU sparse PAX 1.0;
[ADR-088](../adr/ADR-088-sparse-stored-size-field.md) defines raw stored-size
encoding and conflicting-override refusal. The
[GNU tar reference](https://www.gnu.org/software/tar/manual/tar.html)
is the interchange source. Sparse contents alone do not preserve reservations,
object metadata, hard-link identity, attributes or security values.

## Header admission

An ordinary file carries all four local PAX records:

| Key | Value |
|---|---|
| `GNU.sparse.major` | `1` |
| `GNU.sparse.minor` | `0` |
| `GNU.sparse.name` | Canonical source file path under `files/` |
| `GNU.sparse.realsize` | Canonical unsigned 64-bit logical file length |

The raw header size is the stored map-plus-data length. A sparse entry must not
carry local `size` or `path` overrides. Unknown sparse versions/keys, incomplete
field sets, non-file types and names escaping the source namespace are refused.
The existing ordinary fields, including exact `mtime`, retain their documented
admission rules. Emission uses a local header named
`_AROS_BACKUP/metadata/sparse-N.pax` and raw file name
`files/GNUSparseFile.N/payload`, mode 0600, zero raw uid/gid/mtime and empty link,
user and group strings. Local `mtime` carries the captured modification time.

Raw size uses octal through `2^33-1`, then positive GNU binary encoding: `0x80`,
three zero bytes and eight unsigned big-endian value bytes. Decoding also accepts
positive binary encodings of smaller sizes. Negative or overflowing binary sizes
are refused. Other raw numeric fields are octal. Checksum and payload admission
are unchanged. This GNU extension is not a strict POSIX-only archive promise.

Ordinary stream/spool constructors refuse sparse fields. Explicit sparse-aware
constructors expose both lengths: member `size` is stored length and
`sparse_size` is logical length. Payload reads return condensed bytes. Consumers
must parse the map before interpreting data. Spool integrity verification does
not certify the semantic validity of a map.

## Map and data

The payload begins with a canonical decimal entry count followed by each entry's
unsigned decimal offset and length. Every number ends in newline. The complete
map is zero-padded to a 512-byte boundary. Entries are sorted, nonoverlapping and
within logical length. A zero-length entry is permitted only as the final EOF
marker at logical length. A zero-entry map is valid. Source emission appends an
EOF marker, including for all-hole and empty files.

Stored data is the concatenation of the positive-length runs in map order.
Padded map length plus summed run lengths must equal the raw stored size exactly.
Arithmetic overflow, noncanonical numbers, malformed padding, overlapping or
out-of-file runs and mismatched lengths refuse the member. Map counts must fit
both the explicit entry limit and the minimum possible admitted map byte length
before vector allocation. Parsing/emission uses a fixed 512-byte map block;
retained ranges occupy 16 bytes per entry, excluding vector bookkeeping.

## Authorized content operations

The exporter obtains stat and paginated semantic allocations from the same
retained source view. Allocation cursors are entry ordinals, not byte offsets.
Pages must advance by their entry count, be ordered and nonoverlapping, and fit
caller limits. Byte range ends can equal `2^64`; written data is clipped to
logical EOF. Written zero ranges are transported as data; holes and unwritten
ranges are omitted. No zero scanning substitutes for unavailable allocation
knowledge. Source stat is checked again after output, including all-hole files,
so a changed captured descriptor or revoked grant prevents a successful result.

The content report carries logical, written and stored byte counts, plus omitted
unwritten range/byte counts. The enclosing content-recovery job must persist
these losses. Full preservation requires the separate allocation/reservation
metadata and qualified destination operations; it cannot reinterpret this
content report as preservation success.

Restore requires verified replay and a scoped fresh empty single-link regular
file with no allocation. It matches the source path, validates the complete map,
writes positive-length runs through the destination grant and establishes final
logical size. It never writes bytes for logical gaps. Errors poison the reader
and leave an explicit partial outcome. Metadata, reservations, archive-wide
binding/completeness, EOF and destination synchronization are enclosing-job gates.

## Resource and compatibility boundaries

Limits cover logical bytes, stored data bytes, map bytes, map entries and source
page entries (1 through 64). Caller buffers determine transfer chunks. Huge gaps
cost no gap-sized memory, archive payload or iteration. An admitted in-memory map
is a resource profile, not a format limit; a spooled map implementation is needed
for maps exceeding that profile. Provider write limits can refuse a requested
chunk; callers must use a qualified profile rather than silently alter semantics.

[Qualification](../testing/backup-archive-qualification.md#sparse-content-consumer)
covers independent generic extraction and the captured AFS+ content path. Native
file limits, full memory accounting, reservation equivalence and sustained large
payload transfers require their own evidence. Generic tools may have narrower
size and timestamp support than the profile representation.
