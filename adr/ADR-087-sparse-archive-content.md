# ADR-087: Transport sparse file contents with explicit GNU PAX 1.0 admission

Status: Accepted
Amended by: ADR-088, ADR-090
Amends: ADR-076, ADR-078, ADR-081, ADR-085

## Context

Dense ordinary tar entries expand logical holes into payload, defeating bounded
backup of large sparse files. A private content container would undermine the
owner's ordinary-tool recovery objective. The
[GNU tar sparse format reference](https://www.gnu.org/software/tar/manual/tar.html)
defines PAX sparse version 1.0 with a newline-decimal extent map preceding stored
data. This provides an independently implemented content interchange path.

## Decision

Use GNU sparse PAX 1.0 for sparse content transport. Require all four version,
name and real-size fields together, ordinary file type, a canonical source name,
and a separately admitted stored payload length. Do not reinterpret logical
size as tar payload size. Ordinary-reader constructors retain explicit refusal;
sparse-aware constructors expose the distinction to consumers.

Validate a bounded map before destination writes: canonical unsigned decimals,
sorted nonoverlapping in-file runs, checked counts/sums, exact zero padding to a
512-byte boundary and equality between map-plus-data bytes and stored size.
Permit a final zero-length EOF marker, never internal zero-length runs. Memory
and map-byte admission are explicit; file length must not determine allocation.

Source enumeration preserves written zero runs as data and omits holes and
unwritten extents from content. Return explicit unwritten-range loss information
for content recovery; full reservation preservation requires its separate
allocation metadata and equivalent restore operations. Never claim that the
GNU sparse map alone preserves reservations, sharing, ACLs or full objects.

Restore file contents only into a fresh empty destination file through its
revocable grant, using verified replay. Write admitted runs and establish final
logical length, without allocating holes. An error poisons further reader use
and reports partial work. Whole-job success additionally requires metadata,
namespace/link/inventory and reservation validation, EOF and synchronization.

## Qualification and alternatives

Qualify exact sparse contents with Python tarfile and libarchive extraction,
including leading/interior/trailing holes and written zeros. Exercise huge gaps
without gap-sized work, source snapshot stability, revoked grants, malformed
maps, truncation, byte/entry limits, wrong bindings and unsupported versions.
Spooling verifies integrity and admitted headers; semantic map validation remains
the content consumer's responsibility before publication.

A bounded in-memory map is an admitted resource profile, not a format limit.
Large fragmented maps require a separately qualified spooled map path. Unknown
sparse versions are refused rather than decoded as dense files. No disk record,
feature bit or C ABI changes follow. Native resources and reservation equivalence
retain independent gates.
