# ADR-113: The checkpoint's flags word is zero

Status: Accepted

## Context

The checkpoint payload has a 64-bit `flags` word at offset 80. No
implementation ever assigned it a meaning. Q10 asked for its epoch-1 rule:
require zero, open a negotiated namespace, or remove the field.

Both readers had already answered in code. `Checkpoint::decode` refuses a
nonzero word ("checkpoint payload reserved flags are nonzero"), the encoder
cannot produce one that decodes, and the portable C reader refuses it in
`afspr_decode_checkpoint_block`. What was missing is the record of the rule.

Removing the field would move every later field of a structure that both
readers, the fixtures and the fuzz seeds already agree on, for eight bytes in
a block of 4,096. A negotiated namespace has no customer: anything a volume
must declare to a reader before the reader trusts a checkpoint belongs in the
immutable identification block's feature words, which exist for that and are
read first.

## Decision

The checkpoint `flags` word is reserved and zero. A checkpoint whose word is
nonzero is corrupt, in every reader, and is not a selection candidate. A
future use of the word arrives with a feature identity in the identification
block; a reader that does not know that feature never reaches the checkpoint.

## Compatibility classification

No format change: the rule is the one both implementations enforce and every
written image satisfies.

## Format-change procedure

Not a format change. The rule is held by
[`checkpoint_c.rs`](../crates/afsplus-format/tests/checkpoint_c.rs), where the
image "nonzero flags word" is refused by the Rust decoder and by the portable
C decoder in a strict and a sanitized build, for the plain and the
snapshot-bearing form; by `checkpoint_rejects_every_reserved_field` in
`roundtrip.rs`; and by the independent fuzz oracle of the checkpoint.

## Consequences

- Q10 is closed.
- The general demand of Q10, that every retained reserved field has an
  explicit rule before the wire freeze, is met for the structures with exact
  admission: object records (ADR-100), owned-chain segments (ADR-101), reclaim
  blocks (ADR-110), the checkpoint (ADR-111 and this ADR), and the tail of
  every block (ADR-112). The M14 format review still owns the same question
  for the identification block, the tree nodes, the bitmap pages, the region
  descriptors and the intent-log records.
