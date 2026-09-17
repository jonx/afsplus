# ADR-112: A metadata block ends where its payload ends

Status: Accepted
Amends: ADR-100, ADR-110, ADR-111

## Context

Three decisions each required a zero tail for one block kind: the object
record ([ADR-100](ADR-100-exact-object-record-admission.md)), the reclaim
queue blocks ([ADR-110](ADR-110-exact-reclaim-admission.md)) and the checkpoint
([ADR-111](ADR-111-checkpoint-zero-tail.md)); the owned-chain segments had the
same rule from [ADR-101](ADR-101-security-preservation-container.md). Each was
found by looking at one kind. ADR-111 showed the cost of finding them one at a
time: a snapshot checkpoint resealed short read as a plain one, because the
bytes after the payload belonged to nobody. The identification block, the tree
nodes, the bitmap pages, the region descriptors and the intent-log records had
no such rule.

Every metadata block starts with the common header, and every reader verifies
it before anything else. The header states the payload length. That is the one
place where the rule can be said once.

A survey came first, because a rule in the header verification would refuse
any block an encoder of ours wrote with a nonzero tail. None does. Three built
volumes holding all twelve block kinds in use (667 blocks with a valid
checksum: `"AFSA"`, `"AFSB"`, `"AFSC"`, `"AFSG"`, `"AFSH"`, `"AFSI"`,
`"AFSJ"`, `"AFSL"`, `"AFSO"`, `"AFSS"`, `"AFST"`, `"AFSX"`), the four static
fixtures, and a reading of all fourteen block encoders of the format crate and
the three of the portable C writer: each starts from a zeroed block.

## Decision

A block whose common header verifies and that has a nonzero byte after its
payload is corrupt. The check is part of the header verification, in
`BlockHeader::verify` and in `afspr_verify_header`, so it holds for every
block kind, present and future, in every reader.

This supersedes, for the tail only, the per-kind statements of ADR-100,
ADR-110 and ADR-111 and the segment rule of ADR-101; their decoders no longer
scan the tail themselves. Everything else those decisions say stands.

Whether the stored checksum matches a block is a separate, diagnostic
question (`BlockHeader::checksum_matches`): explain reports a block with a
dirty tail as having a valid checksum, which it has.

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released, and the survey found no written block the rule refuses. No
written byte changes.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | [Structure headers](../docs/03-on-disk-format.md#7-structure-headers) |
| ADR | This ADR |
| Compatibility classification | Whole-format change that narrows admission only, reasoned above |
| Conformance image | None among written images; the tests build the refused ones |
| Parser tests | [`zero_tail.rs`](../crates/afsplus-check/tests/zero_tail.rs): the survey, kept as a test that fails if any encoder starts writing past its payload and that asserts all twelve kinds were seen; and, on a real volume, one resealed byte after the payload of the identification block, of both checkpoint slots, of the object-map node and of the object record, each refused by the core and by the portable C reader. The per-kind cross-read tests (`security_c`, `reclaim_c`, `checkpoint_c`, `attributes_c`) keep their tail images and now reach the common check |
| Repair-tool behavior | The checker reads through the shared verification and reports such a block as corrupt; it never edits one |
| Resource impact | One scan of the unused part of each metadata block at verification. A full tree node has none; an object record has about 3.9 KiB |

Four existing tests changed, none because an image changed: two named a
per-kind message that no longer exists (`object_admission`,
`security_container`); `checkpoint_label` resealed a checkpoint with shorter
lengths, which now leaves payload bytes behind as a tail, so the reason it
expects is computed from the block; and `roundtrip`'s
`checkpoint_rejects_every_reserved_field` sealed a payload length of 96, the
length of the layout before the label field, and had passed only because the
header fields were judged before the length. It now seals 168.

Negative controls: with the check removed from the C header verification the
lookup-path test and all four cross-read tests fail; with it removed from the
Rust one the lookup-path test fails.

## Consequences

- A new block kind cannot forget the rule.
- The independent fuzz oracles call the shared header verification, so they
  inherit this rule instead of restating it. Its second statement is the
  portable C reader.
