# ADR-019: PFS3 and proposed PFS4 are mandatory design references

Status: Accepted

## Context

PFS3 is one of the most successful classic Amiga filesystem designs. It achieved strong performance and crash resilience on hardware far more constrained than AFS+'s primary targets.

Its author later described an unreleased PFS4 design with B+ tree directories, redesigned atomic commit, grouped small-file storage, and improved fragmentation handling.

Several of these ideas overlap with AFS+ design decisions reached independently.

## Decision

Before AFS+ format epoch 1 is frozen, the team must review the PFS3 implementation and available PFS4 design material.

For each relevant subsystem, record one of:

- adopt concept
- adapt concept
- reject, with reason

Mandatory comparison areas:

- transaction/atomic update model
- directory indexing
- tiny-file storage
- allocation/locality
- fragmentation handling
- recovery
- memory use
- deletion recovery behavior

## Consequences

AFS+ remains a new format and is not constrained by PFS3 compatibility.

PFS3 becomes a primary Amiga-native reference alongside modern filesystem designs rather than being treated merely as a legacy filesystem.
