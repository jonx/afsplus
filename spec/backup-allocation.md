# Archive allocation preservation

[ADR-090](../adr/ADR-090-archive-allocation-preservation.md) binds semantic
allocation to [sparse contents](backup-sparse.md) and
[committed destination readback](../docs/13-filesystem-api-v2.md#committed-destination-allocation-readback).
This is a regular-file content/allocation component. A complete preservation job
also validates object metadata, namespace/link identity, opaque inventories,
archive completeness, destination synchronization and its final loss report.

## Allocation record

An ordinary file named `_AROS_BACKUP/metadata/allocation-N.pax` precedes the
primary sparse member with ordinal `N+1`. Its header has mode 0600, zero uid,
gid and mtime, and empty link, user and group strings. Both raw and effective
paths must match the expected ordinal. The payload contains exactly these five
unique UTF-8 PAX records; unknown or missing fields are refused:

| Key | Value |
|---|---|
| `AROS.allocation.version` | `1` |
| `AROS.allocation.path` | Canonical regular-file source path under `files/` |
| `AROS.allocation.size` | Logical length as canonical unsigned decimal u64 |
| `AROS.allocation.count` | Number of allocation ranges as canonical unsigned decimal u64 |
| `AROS.allocation.ranges` | Concatenated range lines, or empty for count zero |

Each range line is `offset,length,state` followed by newline. Offset and length
are canonical unsigned decimal u64; state is `w` for written or `u` for
unwritten. Length is positive. Ranges are ordered, nonoverlapping and may end at
`2^64`; endpoints are computed with wider arithmetic. Physical addresses,
sharing flags and allocator segmentation identities are absent. Allocation
beyond logical EOF is represented, including the rounded tail of a written
block. No contents beyond logical EOF are exposed by this record.

The sparse raw name is `files/GNUSparseFile.N+1/payload`, where `N+1` denotes
the decimal successor ordinal. The sparse member's effective source path and
logical size match the record. Its positive-length runs exactly cover the
record's written ranges clipped to logical EOF. Adjacent splitting or merging
is equivalent; changed holes or written/unwritten state is not. The final
zero-length sparse EOF marker carries no allocation. Check this equivalence
before destination reservations or content writes.

The exporter derives both representations from one retained source view and
one allocation plan, then rechecks source authority/stat after data output.
Admit both ordinals before output. The returned next ordinal is absent when
`N+1` is the maximum u64. Any error poisons the writer or reader; no error can
produce a successful component report.

## Restore modes

Require integrity-verified sparse replay and a scoped fresh empty single-link
regular file. The host serializes destination edits through verification;
readback does not create an implicit snapshot of concurrent edits.

`PreserveAllocation` admits total unwritten bytes and reservation chunk limits,
then probes an empty committed allocation page under the destination grant.
Unsupported readback fails before content writes. Reserve each unwritten range
in bounded calls, write only sparse data runs and establish logical size. Query
committed allocation in bounded entry-ordinal pages and compare exact byte
coverage and written/unwritten state against the record. Different adjacent
segmentation is accepted. Compare logical size too. A destination with a
coarser allocation granularity must refuse if it cannot represent the same
coverage; successful reservation calls or equal allocated-byte totals alone do
not prove preservation.

`RecoverContents` skips reservation and allocation-readback requirements. It
restores written contents and logical gaps and returns the discarded unwritten
range count and u128 byte sum, including beyond-EOF reservations. The enclosing
job persists that report. This mode is explicit, never an automatic fallback
from a preservation error. Other metadata losses need their own accounting.

Each operation checks the original revocable destination grant. Error stops
further archive use, but earlier committed reservations or writes can leave a
partial destination. A component report is neither atomic publication of a job
nor its durability receipt. The enclosing job consumes the complete archive,
validates all other components and synchronizes before issuing completion.

## Resource and portability contract

Caller limits bound record bytes, keys, values and fields; allocation and sparse
entry counts; logical and stored data bytes; source/readback page size (1 to 64);
reservation bytes, per-call reservation size and readback entry count. Counts
must fit the minimum possible admitted record length before allocation. Range
memory is proportional to admitted entries, not logical file size. Allocation
export retains the full range list and sparse map together; content-only export
retains just its sparse map. Restore retains the decoded range list and sparse
map. PAX encoding/decoding also needs bounded record storage. These costs are
additional to the caller buffer, verified scratch replay and provider memory.

Huge holes cause no gap-sized buffer or loop. Hosts choose smaller pages and
transfer/reservation chunks within their provider's alignment and edit limits.
An insufficient map/record budget produces explicit refusal; spooling large
maps needs a separate implementation. Recovery can operate without reservation
support, but cannot claim capacity preservation. Native address-space, storage,
C interface and full peak-memory qualification are independent of hosted Rust
round trips. No filesystem disk layout or feature identity changes here.

See [allocation qualification](../testing/backup-archive-qualification.md#allocation-preserving-consumer)
for the component oracle and failure cases.
