# ADR-105: A well-formed security reference is admitted wherever it points

Status: Accepted
Amends: ADR-101

## Context

[ADR-101](ADR-101-security-preservation-container.md) holds two rules that met
badly. Its lifetime rule says an object stays deletable whatever the bytes of
its descriptor chain look like: retirement frees the segments consistent with
the reference and leaves the rest to the checker. Its admission text says
every implementation bounds the first segment block where it admits the
record, so a reference outside the allocatable range refuses the object at
lookup, unlink included.

Together they draw an arbitrary line. A first block one past the end of the
volume pinned the object's name, record and data for ever, while a first block
pointing at in-range garbage left the object deletable. Both are the same
damage: a reference whose chain cannot be read.

An independent reading of the chain walk also found that its acceptance test
was weaker than its comments claimed. A segment was accepted when its content
matched the reference: owner, position, count, length, format identity and a
committed generation. A stale segment of an earlier descriptor of the same
object and the same size matches all of that. Reached through a forged,
resealed next pointer, it made the walk return a mixture of two descriptors
and made retirement free blocks the replacement had already retired.

## Decision

1. Record admission judges the reference's shape only: nonzero first block,
   length of 1 to 65,536 bytes, the segment count that length implies,
   assigned reference flags, and the volume feature. Where the first block
   points is chain state. Lookup, stat, data I/O, rename and unlink work on
   such an object; reading or copying its descriptor fails whole; retirement
   frees only consistent segments; the checker reports the reference. The
   Rust core and the portable C reader apply the same rule.
2. Every segment of a chain carries the generation of its first segment,
   because one commit writes them all. The first segment's generation is
   nonzero and at most the committed generation; every later segment equals
   it. A segment with another generation ends the walk.
3. The walk judges content, never allocator ownership. A data block that
   holds a valid segment image of the same chain generation, reached through
   a forged and resealed next pointer, still passes. That needs a crafted
   image, which is the trust level the extent trees have; it is recorded as a
   known limit in the [open questions](../implementation/open-questions.md).

## Compatibility classification

No disk field, flag or feature changes. Images that every writer produces are
read as before; only the verdict on a damaged or crafted reference changes.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | [docs/04](../docs/04-object-model.md) and rule 9 of the [disk layout](../spec/disk-layout.md) |
| ADR | This ADR |
| Compatibility classification | None needed: no wire change |
| Conformance image | The cross-read volume test builds the image with a reference one block past the volume and requires both readers to admit the object |
| Parser tests | `crates/afsplus-check/tests/security_container.rs`: the out-of-volume reference is admitted, stats and reads, fails its descriptor read, deletes, and leaves its one real segment as the checker's only leak; a stale same-object middle segment behind a forged resealed link ends the walk, the descriptor read is corrupt, deletion frees only the first live segment and leaks the two behind the link, and the already retired stale chain is not freed twice. Without the generation rule that test fails: the read returns a mixture of two descriptors. `crates/afsplus-check/tests/security_c.rs` holds the agreement of both readers |
| Repair-tool behavior | Unchanged: the checker reports a reference it cannot walk and the leaked segments, and edits nothing |
| Resource impact | One comparison per segment; one fewer check at lookup |

## Consequences

- A user can always delete what the checker reports.
- A descriptor is returned whole and from one commit, or not at all.
- Lookup of an object with a damaged reference succeeds, so a host sees the
  object and its classic protection word while its descriptor is unreadable;
  the strict projection policy still refuses protection edits on it.
