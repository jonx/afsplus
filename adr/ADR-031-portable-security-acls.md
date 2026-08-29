# ADR-031: Portable canonical security model

Status: Proposed

## Context

AFS+ targets both classic/single-user Amiga-family systems and multi-user operating systems. Native Unix UID/GID pairs and Windows SIDs are not suitable as the canonical on-disk identity model because they belong to different security namespaces and do not round-trip cleanly across hosts.

POSIX ACLs are too limited to represent several semantics already common in NFSv4 and Windows, including ordered ALLOW/DENY entries and richer inheritance behavior.

## Proposed decision

AFS+ stores a canonical security descriptor per object, directly or through a shared immutable descriptor reference.

A security descriptor contains:

- portable owner principal
- portable owning-group principal
- ordered discretionary ACL
- optional audit ACL
- inheritance/control flags

Principal identity is based on a security realm plus stable principal identifier rather than raw host UID/GID/SID.

The access-right model is intentionally close to NFSv4/Windows file ACL semantics to maximize translation fidelity.

## Compatibility

Classic Amiga/AROS protection bits and POSIX mode bits are projections/adapters over the canonical model.

Simpler hosts must preserve richer ACL metadata they cannot represent. They must not silently weaken it.

A strict security mode may refuse read-write operation when the host cannot enforce active ACL semantics.

## Administrative override

Root/administrator/superuser bypass is host policy, not an on-disk magic principal.

## Security domains

Shared subtree security domains are a separate Proposed extension and are not required by this ADR.

## Encryption

ACL enforcement and cryptographic confidentiality are separate layers. This ADR does not imply filesystem encryption.

## Consequences

Advantages:

- high-fidelity Windows/NFSv4-style interoperability
- richer semantics than POSIX ACLs
- classic compatibility remains possible
- account mappings can change without rewriting every file
- foreign/unmapped principals can be preserved

Costs:

- host adapters are required
- ACL evaluation is more complex than Amiga protection bits
- exact cross-platform behavior requires extensive conformance tests
- security descriptor storage/indexing becomes a new metadata subsystem
