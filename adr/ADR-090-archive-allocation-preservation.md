# ADR-090: Bind allocation records to sparse contents and verify restoration

Status: Accepted
Amended by: ADR-091
Amends: ADR-078, ADR-087, ADR-089

## Context

Sparse content transport omits unwritten reservations. Full preservation needs
both their recorded byte coverage and evidence that the destination represents
that coverage. Comparing extent counts would incorrectly treat different
physical fragmentation as a semantic difference.

## Decision

Precede each primary sparse file with a versioned allocation record containing
its canonical source path, logical size and ordered written/unwritten byte
ranges. Use one captured source plan for the record and sparse map. No physical
address or sharing flag belongs in the archive. The exact PAX record contract is
[allocation preservation](../spec/backup-allocation.md).

Before destination content writes, validate the allocation record, file/path/size
binding and equivalence between its clipped written ranges and the sparse map.
Require verified replay and a fresh empty single-link destination file. An
allocation-preserving restore first probes readback support, admits reservation
budgets, reserves unwritten ranges in bounded calls and then writes sparse
contents and final logical size. Read back committed allocation under the same
grant and compare exact coverage and written/unwritten state, allowing equivalent
adjacent segmentation. Also compare logical size. Unsupported semantics or a
mismatch cannot yield allocation-preservation success.

Explicit content recovery skips reservations and returns their discarded range
count and byte sum. It never relabels that outcome as allocation preservation.
Both outcomes are file components: object/security metadata, namespace/link
identity, complete archive binding, EOF and destination synchronization remain
whole-job requirements. Failure poisons further archive use and reports partial
restoration; earlier committed writes or reservations need not roll back.

The allocation record uses ordinal N and its sparse member uses N+1. Admit this
range before output; report exhaustion of the following ordinal explicitly.
Limits cover record bytes, entries, logical/data sizes, reservation bytes and
per-call reservation size, and readback entries. Range endpoints may equal 2^64;
logical size and individual offsets/lengths remain unsigned 64-bit values.

## Qualification

Require captured AFS+ export and allocation-preserving restore with remount,
written zeros, holes, reservations inside/beyond EOF, rounded written tails and
the final address block. Compare content-recovery loss reports against the same
archive. Reject conflicting maps, wrong paths/sizes/ordinals, malformed records,
unsupported readback/reservations, mismatched destination coverage, revoked
authority and resource exhaustion. Equivalent segmentation must compare equal;
changed holes or written/unwritten state must not. No filesystem disk record,
feature bit or C ABI changes follow. Spooling large maps and native provider
resource/durability evidence retain separate gates.
