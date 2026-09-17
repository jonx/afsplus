# Proposed ADR: exact admission of object records

> **ADRs:** [ADR-065](../adr/ADR-065-persistent-data-update-policy.md), [ADR-029](../adr/ADR-029-dual-reference-implementations.md) · **Spec:** [disk layout](../spec/disk-layout.md) ·
> **Tests:** [fuzzing](../testing/fuzzing.md) · **Milestones:** M14

Target on acceptance: a numbered ADR in `adr/`, closing Q13 in
[open questions](../implementation/open-questions.md); the object-record
section of [docs/04](../docs/04-object-model.md) and the
[disk layout](../spec/disk-layout.md) gain the admission rule.

Decisions requested: M1 (the rule), M2 (the extension path), M3 (portable C
parity).

## Context

An object record is one checksummed block: the 32-byte common header, a
96-byte fixed payload, an inline target for a symlink, and a zero tail. Three
fields of that block had no admission rule in the generic file and directory
readers:

- the 16-bit `flags` word of the common header;
- a `payload_len` larger than the fixed record;
- the bytes between the end of the payload and the end of the block.

The experiment that Q13 names ran against the readers as they stood. After a
CRC reseal, the generic reader, the generation-returning reader and the
metadata reader each admitted all 16 header flag bits on files and
directories, admitted payload lengths of 97, 104, 112 and the full block, with
zero and nonzero extension bytes, and admitted a nonzero byte at the first,
second, middle and last tail position. The inline symlink reader refused all
three. The portable C reader refused header flags and admitted the longer
payload and the nonzero tail, so the two reference implementations disagreed
on the same block.

Every mutation path decodes the record into its fields and encodes those
fields into a zeroed block with `flags = 0` and `payload_len = 96`. A byte
admitted without a field is therefore dropped by the next protection edit,
rename, write or link-count change of that object. Acceptance without a
field ends in loss at the next rewrite, with no report.

Of the three options of Q13, "accept and preserve" requires a carrier for
unknown bytes in every rewrite path of every implementation, and "ignore"
requires a loss contract that tells a future writer its bytes may vanish.
Neither has a consumer. Object flags already follow the third option
([ADR-065](../adr/ADR-065-persistent-data-update-policy.md)): a validated
namespace, where a reader either understands a flag or refuses the record.

## Decision

M1. An object record is admitted only in its canonical image:

1. the common header `flags` word is zero;
2. `payload_len` equals the length that the object type and the object flags
   define: 96 bytes for a file or a directory, 96 bytes plus the target length
   for an inline symlink;
3. every byte after the payload is zero;
4. the reserved payload byte at offset 9 is zero and the object flags contain
   only assigned bits, as before.

The rule is shared: one check serves the generic, generation-returning,
metadata and symlink readers, so the readers cannot drift apart again. A
violation is reported as corruption of that object, never repaired silently.

M2. The record grows only through negotiation. An extension is an assigned
object flag, bound to a volume feature identity
([feature framework](../docs/09-feature-framework.md)), that defines the exact
new payload length and the meaning of every added byte. A reader that does not
know the flag refuses the record, which is the fail-closed behaviour the
validated flag namespace already gives. The common header `flags` word stays
zero for object blocks in epoch 1; assigning it a meaning needs its own ADR.

M3. The portable C reader adopts rules 2 and 3 for files and directories; it
already applies rule 1. The shared conformance corpus gains the resealed
negative images of the experiment, so both implementations prove the same
verdict per image ([ADR-029](../adr/ADR-029-dual-reference-implementations.md)).

## Compatibility classification

No feature identity is allocated. Every writer in the repository, in Rust and
in C, produces the canonical image, so every existing image, fixture and
corpus entry stays valid. Only blocks that no writer can produce change
verdict, from admitted to corrupt.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | The admission rule lands in docs/04 and the disk layout on acceptance |
| ADR | This proposal |
| Compatibility classification | None needed: canonical images are unchanged |
| Conformance image | Resealed negative images join the corpus with M3 |
| Parser tests | `crates/afsplus-format/tests/object_admission.rs`: 32 header-flag images, 16 extended-payload images, 8 tail images, each refused by three readers with an exact reason; a canonical control that decodes to the literal record and re-encodes to the identical block; a symlink header-flag image |
| Repair-tool behavior | The checker reports the object as corrupt through the shared decoder; it never rewrites such a block into canonical form, because the extra bytes have no known owner |
| Resource impact | One pass over the block tail per object decode, inside a block the CRC pass already reads in full; no allocation, no I/O |

## Consequences

- A rewrite can no longer destroy bytes that a reader accepted, because no
  reader accepts bytes without a field.
- The Rust and C readers agree on header flags today and on payload length
  and tail after M3.
- A future extension costs an object flag and a feature identity. The
  security reference of the B5 security preservation container is the first
  user of that path.
- Forward compatibility by silent tolerance is given up deliberately. A
  volume written by a later implementation is read by an earlier one only
  where its feature class permits, which the
  [mount decision algorithm](../spec/compatibility-rules.md) already states.

## Open after this decision

- The same question for the other block types (tree nodes, object-map nodes,
  checkpoint `flags`, which is Q10) stays with the M14 format review. The C
  reader already requires zero header flags on tree nodes and checkpoints.
