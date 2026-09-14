# Backup archive qualification

> **ADRs:** [ADR-076](../adr/ADR-076-pax-backup-interchange.md), [ADR-078](../adr/ADR-078-backup-preservation-modes.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** M13, M14

## PAX record codec

Run `cargo test -p afsplus-backup` for the bounded extended-header codec in
[pax.rs](../crates/afsplus-backup/src/pax.rs). The
[Oracle pax manual](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html)
specifies decimal record lengths in bytes, including the trailing newline,
and UTF-8 encoding. Values can contain embedded newlines and equals signs;
record boundaries follow the declared lengths. The codec preserves empty values
for interpretation by the profile layer.

Require exact equality with the independent Python tarfile fixture containing
UTF-8, a newline/equal-bearing path and a negative fractional timestamp. Exercise
decimal-length width transitions, every nonempty truncated prefix of each
single-record fixture, invalid UTF-8, embedded NUL, malformed/overflowing lengths,
duplicate keys, trailing garbage and deterministic arbitrary bytes.

The codec contract uses canonical positive decimal lengths and unique keys in
a record block. Reject duplicate keys before any profile interpretation. General
PAX global/local override and alternate character-set handling require a separate
archive-layer decision. Keywords containing NUL, newline or equals sign are
invalid under this strict codec. Values require UTF-8 and exclude NUL.

Callers supply positive byte, record and key limits; a zero value-byte limit
allows only empty values. Admission precedes output allocation. Decoding borrows
keys and values from the input and allocates record-index bookkeeping within the
record count; encoding preflights the complete output byte size. Test every
limit independently in both directions, including exact-boundary admission.
Caller buffer limits, process peak RAM and target runtime need separate evidence.

## Integration acceptance

Record decoding establishes syntax only. The complete consumer must enforce the
versioned preservation profile, path confinement, required metadata, payload
integrity and a completion record before declaring a complete backup/restore.
A valid sequence of PAX records or a valid tar prefix cannot establish completion.

Qualify snapshot capture and destination-scoped restoration, independent ordinary
file recovery, sparse layouts, hard-link identity, timestamps, attributes,
security metadata, reservations, revocation, truncated output, conflicting
metadata, unknown requirements and resource pressure under ADR-076/078.
Every full-preservation omission requires refusal; explicit content recovery
reports the losses allowed by its profile. Archive framing, profile identity,
integrity and completion semantics follow the format decision process before
normative archive fixtures are generated.
