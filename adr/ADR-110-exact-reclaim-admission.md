# ADR-110: Reclaim queue blocks are admitted only in their canonical image

Status: Accepted
Amended by: ADR-112
Amends: ADR-036

## Context

Giving the reclaim root, segment and table of
[ADR-036](ADR-036-reclaim-queue.md) a second reader showed five things no
reader looked at: flags in the common header, an owner in the common header,
bytes in the unused slots of a root area, a root payload longer than its three
areas, and bytes after the payload. A sealed segment or table already had an
exact payload length.

The object record was in that state before
[ADR-100](ADR-100-exact-object-record-admission.md), and the argument is the
same. The root is rewritten by every transaction from its decoded fields into
a zeroed block, so a byte admitted without a field is dropped by the next
commit. A byte that is dropped silently is a byte two implementations, or an
implementation and a fuzzer, can disagree about.

## Decision

A reclaim root (`"AFSH"`), sealed segment (`"AFSS"`) or sealed table
(`"AFSL"`) is corrupt when any of these holds, in every reader:

1. the common header's flags are nonzero;
2. the common header's owner is nonzero: the queue belongs to the volume;
3. a root's payload length differs from 64 + 12 × (table capacity + segment
   capacity) + 20 × inline capacity;
4. a byte is nonzero in the unused slots of a root's table, segment or inline
   area, beyond that area's count;
5. a byte is nonzero after the payload.

The rules of ADR-036 for the fields themselves are unchanged.

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released. Every encoder writes a zeroed block with zero flags and
owner, so every image, fixture, conformance image and corpus entry produced
so far satisfies the rule. No written byte changes.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | [Reclaim queue blocks](../spec/disk-layout.md#reclaim-queue-blocks) in the disk layout |
| ADR | This ADR |
| Compatibility classification | Whole-format change that narrows admission only, reasoned above |
| Conformance image | The cross-read test builds the refused images; none exists among written images |
| Parser tests | [`reclaim_c.rs`](../crates/afsplus-format/tests/reclaim_c.rs): for each of the three kinds, header flags, an owner and a nonzero tail; for the root, a byte in an unused slot of each of the three areas and a payload one byte longer than its areas. Each is refused by the Rust decoder and by the portable C decoder in a strict and a sanitized build. The independent fuzz oracle (`fuzz/src/reclaim.rs`) applies the same five rules |
| Repair-tool behavior | The checker reads the queue through the shared decoder and reports such a block as a corrupt reclaim structure; it never edits one |
| Resource impact | A scan of the unused part of one block per decoded reclaim block |

Negative controls, each failing the cross-read test: the C decoder ignoring
the owner, the C decoder admitting a long root payload, the Rust decoder
ignoring the unused inline slots. Unchanged and passing: the reclaim and
corruption-corpus tests of the checker, the codec round trips, the fuzz unit
tests with their seed fingerprints.

## Consequences

- The three reclaim kinds have one image per value, as the object record has.
- A second implementer has no slack to fill differently.
