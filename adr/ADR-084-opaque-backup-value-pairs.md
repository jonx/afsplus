# ADR-084: Stream opaque backup values as bound descriptor and data pairs

Status: Accepted under the owner's delegated recommended-option authority; complete archive-profile integration required
Amended by: ADR-085, ADR-086
Amends: ADR-076, ADR-082, ADR-083

## Context

Bounded snapshot metadata reads and staged destination writes need an archive
transport for arbitrary binary values. Encoding all value bytes inside a PAX
string would require UTF-8 conversion or expansion and value-sized metadata
buffers. A generic tar member already provides streamed binary payload framing.

## Decision

Represent each opaque value by an ordinary descriptor member followed by an
ordinary binary data member in the auxiliary namespace. A canonical unsigned
64-bit ordinal binds the pair. The descriptor's versioned PAX records bind the
exact source path, metadata class, key, encoding and byte size. The data member's
effective size must agree. Use a local PAX size override for 64-bit data sizes;
never truncate to the narrower raw ustar field.

Export reads the captured value through its original backup grant, checks exact
length and EOF, and invalidates archive completion after any export error.
Import validates both members and the expected source binding before opening a
staged destination upload. It streams data through caller buffers and finishes
only after the exact value has arrived. Import errors invalidate stream completion
and drop private staging. Unknown encodings remain opaque.

A value's successful installation is not whole-archive restoration. Earlier
installed values may remain after later corruption or revocation; report partial
restoration and never issue an overall success outcome. The full consumer must
match every descriptor to its source object, enforce unique ordinals and keys,
check inventory completeness, validate the final envelope and synchronize the
destination before reporting its qualified completion outcome.

## Compatibility and qualification

The detailed wire contract is [opaque values](../spec/backup-opaque-values.md).
No filesystem record, feature bit, C ABI or ACL evaluation rule changes. Ordinary
tar tools recover auxiliary descriptor/data files without understanding them;
only the profile-aware consumer installs the metadata.

Require end-to-end captured-source to archive to staged-destination byte and
identity comparisons, small buffers, empty/binary values, mismatched binding,
short/long source data, revocation, corruption/truncation and no completion after
failure. Bound descriptor bytes separately from value length. Actual AFS+ opaque
storage, complete inventory orchestration and native resource/durability gates
remain mandatory before claiming full platform preservation.
