# ADR-111: Nothing follows the payload of a checkpoint

Status: Accepted
Amends: ADR-073

## Context

[ADR-073](ADR-073-snapshot-checkpoint-roots.md) gave the checkpoint two payload
lengths: 168 bytes, or 184 with the snapshot registry root and the lifetime
ledger root after the label ([ADR-104](ADR-104-volume-label-in-checkpoint.md)).
Neither reader looked at the bytes after the payload.

Giving the snapshot-bearing payload a second reader found what that allows. A
checkpoint written with its two roots, resealed with the payload length 168
and a valid checksum, was admitted by the Rust decoder and by the portable C
decoder as a checkpoint without snapshot roots. The roots were still in the
block, as tail. A mount that selected it would run a snapshot volume without
its registry and its ledger. Reaching that state needs a crafted block, since
the checksum covers the length; the format should still not have two readings
of one block.

## Decision

A checkpoint block with a nonzero byte after its payload is corrupt, in every
reader. With the rules already in force (zero common-header flags and owner,
a payload of exactly 168 or 184 bytes, a canonical label field, a zero flags
word, nonzero and distinct snapshot roots) a checkpoint has one image per
value.

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released. Every encoder writes a zeroed block, so every image,
fixture, conformance image and corpus entry produced so far satisfies the
rule. No written byte changes.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | Rule 11 of [the disk layout](../spec/disk-layout.md) |
| ADR | This ADR |
| Compatibility classification | Whole-format change that narrows admission only, reasoned above |
| Conformance image | The cross-read test builds the refused images; none exists among written images |
| Parser tests | [`checkpoint_c.rs`](../crates/afsplus-format/tests/checkpoint_c.rs): 52 images, plain and snapshot-bearing, each through the Rust decoder and the portable C decoder `afspr_decode_checkpoint_block` in a strict and a sanitized build against a literal verdict; every field rule broken one at a time, eight wrong payload lengths, the three malformed root pairs, the snapshot checkpoint resealed short, a plain nonzero tail. `roundtrip.rs` and the fuzz oracle's unit test asserted the old reading (168 admitted on a resealed snapshot image) and now assert the new one |
| Repair-tool behavior | The checker selects checkpoints through the shared decoder; such a slot is not selectable and is reported with its reason |
| Resource impact | A scan of the unused part of the block per decoded checkpoint: two blocks per mount |

Negative controls, each failing the cross-read test: the C decoder without the
tail scan, the C decoder admitting equal roots.

## API contract consequences

- The portable C reader exposes `afspr_decode_checkpoint_block`, a standalone
  decoder of both payload lengths against a volume UUID. Its volume paths keep
  refusing a checkpoint that carries snapshot roots: the reader does not
  implement persistent snapshots, and an image with that feature does not
  probe.

## Consequences

- Q15 keeps one item: the snapshot record codecs.
