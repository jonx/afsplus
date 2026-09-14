# ADR-082: Carry explicit object metadata and inventory knowledge in backups

Status: Accepted under the owner's delegated recommended-option authority; complete preservation consumer qualification required
Amended by: ADR-083, ADR-084, ADR-086, ADR-091
Amends: ADR-076

## Context

The authorized snapshot interface reports exact core timestamps and protection,
but an interface without attribute or security enumeration cannot prove that
those inventories are empty. Treating unavailable enumeration as absence would
permit an apparently successful backup to discard metadata. Tar mode bits also
cannot substitute for the filesystem-neutral API's preservation protection value.

## Decision

Carry versioned object metadata in an ordinary PAX-record payload under the
auxiliary archive namespace. Bind the payload to an exact canonical source path
and ordinary object kind. Preserve protection as an unsigned 64-bit semantic
value and creation, modification and change timestamps as exact signed seconds
with nanoseconds. Do not copy destination-local identifiers or physical layout.

Each attribute and security inventory explicitly reports `empty`, `present` or
`uninspected`. Empty requires provider evidence. Present requires the associated
lossless transport and validation before full preservation can succeed.
Uninspected means the provider did not establish completeness; it cannot certify
full preservation. Content recovery may proceed only with an explicit loss or
uncertainty report under ADR-078. Parser defaults never supply missing fields.
Unknown versions or fields require explicit refusal, not optimistic omission.

Use the bounded UTF-8 PAX record codec for this metadata payload. This preserves
inspectability and avoids a second binary scalar codec. Alternative implicit
absence was rejected because unsupported enumeration is not an empty inventory.
Packing custom fields directly into ordinary local headers was rejected because
an auxiliary ordinary member also accommodates independently streamed metadata
transports without teaching generic tar tools their semantics.

The detailed payload is [object metadata](../spec/backup-object-metadata.md).
Its decode success proves field admission only. Archive-wide binding, inventory
transport, hard-link consistency, consumer authority and completion checks are
separate mandatory integration gates.

## Compatibility and qualification

No AFS+ disk record, feature bit, native ABI or legacy DOS interface changes.
This is additive archive-profile metadata within the ADR-081 envelope. Preserve
ordinary recovery as a separate interoperability gate. Require exact scalar and
path round trips, malformed/duplicate/missing/unknown-field refusal, timestamp
and integer boundaries, explicit inventory states and configured byte budgets.

Bound payload bytes, record count and string lengths before allocation. Borrow
strings when decoding. Measure complete directory/hard-link/inventory bookkeeping
in the consumer; a bounded scalar codec does not prove bounded total restore RAM.
