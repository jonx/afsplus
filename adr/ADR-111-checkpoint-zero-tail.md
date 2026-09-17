# ADR-111: Nothing follows the payload of a checkpoint

Status: Accepted
Amended by: ADR-112
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

The zero tail alone leaves one case: a checkpoint in the short form with a
genuinely zero tail on a snapshot volume, or in the long form on a volume
without the feature. Neither can be written by this filesystem. Identification
is immutable ([ADR-061](ADR-061-shared-extent-references.md)), so the feature
is decided when the volume is formatted, and the formatter writes the first
checkpoint in the volume's form; every later checkpoint is written by a mount
that negotiated the same feature. There is no "checkpoint from before the
feature was enabled". The core already refused such a volume when that slot
was the selected one, and deliberately never stepped past it to the older
slot: a valid newest checkpoint in the wrong form is not a torn write. The
portable C reader stepped past it.

## Decision

1. A checkpoint block with a nonzero byte after its payload is corrupt, in
   every reader.
2. The payload form belongs to the persistent-snapshots feature: 184 bytes on
   a volume with it, 168 without. When the checkpoint that selection chooses
   is in the other form, the volume is refused, by every reader, and the older
   slot is not used instead. A slot in the other form that selection does not
   choose changes nothing. The portable C reader implements no snapshots and
   refuses the feature at identification, so for it the rule reads: a selected
   checkpoint with snapshot roots refuses the volume.

With the rules already in force (zero common-header flags and owner,
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

Decision 2 is held by `selected_snapshot_extension_mismatch_never_falls_back`
in the checker's `mount_modes` test (the core, both directions), by
`the_portable_c_reader_reports_the_same_label_and_the_same_fallback` in
`volume_label` (the core and the C reader on the same images: the wrong form
in the newest slot refuses the volume, in the older slot it changes nothing)
and by `explain_more` for the explain walk. With the C reader stepping past
the slot as it did, the `volume_label` test fails.

## API contract consequences

- The portable C reader exposes `afspr_decode_checkpoint_block`, a standalone
  decoder of both payload lengths against a volume UUID. Its volume paths keep
  refusing a checkpoint that carries snapshot roots: the reader does not
  implement persistent snapshots, and an image with that feature does not
  probe.

## Consequences

- Q15 keeps one item: the snapshot record codecs.
