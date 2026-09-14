# ADR-031: Portable security metadata and canonical ACL candidate

Status: Proposed

## Context

AFS+ targets both classic/single-user Amiga-family systems and potentially multi-user operating systems. Native Unix UID/GID pairs and Windows SIDs do not round-trip cleanly as a universal on-disk identity model.

The earlier proposal went further and selected a canonical NFSv4/Windows-like ALLOW/DENY ACL evaluation model. Peer review correctly identified that freezing such semantics before any real multi-user AFS+ implementation enforces them would create a large, poorly tested format commitment.

## Decision now: freeze the preservation container before the policy semantics

Before epoch 1, AFS+ should provide a versioned security-metadata attachment/reference mechanism capable of preserving richer security descriptors without requiring a simple host to understand or rewrite them.

Required properties:

- an object can reference versioned security metadata/descriptor data
- descriptor identity/encoding is feature-versioned
- unknown richer security metadata can be preserved
- a host that cannot safely enforce active semantics must not silently weaken them in strict mode
- classic protection-bit projection must not erase unknown rich security state
- shared immutable descriptor storage may be prototyped as an optimization, not assumed as a mandatory format layout

## Candidate later semantic model

A canonical model close to NFSv4/Windows remains a strong interoperability candidate, potentially including:

- portable owner/group principal mapping
- ALLOW/DENY ACEs
- inheritance/control flags
- optional audit semantics
- stable principal realm/identity representation

But these evaluation semantics are not accepted for epoch 1 merely because they are documented.

They must first be exercised through real adapters/conformance tests, ideally at least:

- POSIX/Linux mapping
- Windows mapping
- classic/single-user preservation/projection

If translation proves too complex or lossy, the canonical model can be simplified or moved behind a later feature without changing the base object's ability to carry a versioned security descriptor reference.

## Administrative override

Root/administrator/superuser bypass remains host policy, not an on-disk magic principal.

## Security domains

Shared subtree security domains remain a separate Proposed extension and are not required by this ADR.

## Encryption

ACL enforcement and cryptographic confidentiality are separate layers. This ADR does not imply filesystem encryption.

## Consequences

Advantages:

- epoch 1 reserves a clean path for rich multi-OS security without prematurely freezing a complicated evaluation engine
- classic/simple hosts can preserve metadata they do not understand
- future ACL semantics can evolve through feature negotiation
- account mappings can remain host/security-service policy

Costs:

- early multi-user behavior is less fully specified
- richer ACL interoperability requires later prototypes and conformance work
- applications must query security capability/fidelity rather than assume one universal enforcement model
