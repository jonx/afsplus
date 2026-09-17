# ADR-104: The volume label is committed checkpoint state

Status: Accepted

## Context

The volume label lived in one place, the identification block at LBA 0. That
block exists in one copy and is written once by the formatter, which is what
keeps it safe from torn rewrites: mutable committed state lives in the
checkpoints. A relabel therefore had no legal home. Rewriting the
identification block in place leaves, after a power cut, a block that may be
neither the old nor the new one, on the only copy of the structure every tool
reads first. `ACTION_RENAME_DISK` and every other relabel were unimplementable
without a format decision.

Three homes were possible for a mutable label:

1. a well-known attribute of the root object;
2. a small record referenced from the checkpoint;
3. a field of the checkpoint itself.

The first needs the attribute subsystem, which does not exist yet, and makes
mount depend on it to learn the volume's name. The second adds a block, its
ownership, its reclamation and its checker rules for 64 bytes. The third adds
nothing: the checkpoint is already the structure that a commit publishes in an
alternate slot, so it has two legal states under a power cut by construction,
and it has room.

## Decision

1. The checkpoint payload carries the current volume label at a fixed place,
   directly after its 96 fixed bytes:

   ```text
   offset size field
   96     1    label length in bytes, 0 to 64
   97     7    reserved, zero
   104    64   label, UTF-8 without NUL, zero padded
   168    8    snapshot registry root   (payload of 184 bytes only, ADR-073)
   176    8    lifetime ledger root     (payload of 184 bytes only, ADR-073)
   ```

   The payload is 168 bytes, or 184 with snapshot roots. No other length is a
   checkpoint. The label field is canonical: a length above 64, a nonzero
   reserved byte, a nonzero padding byte, a NUL inside the label or invalid
   UTF-8 makes the slot invalid, and checkpoint selection treats it like any
   other invalid slot.
2. One label rule serves the formatter, the relabel operation and both
   readers: at most 64 bytes of UTF-8 and no NUL. The empty label is legal.
   Host naming syntax (a DOS colon, a path separator) is host policy and
   absent from the format.
3. The formatter writes the same label into the identification block and into
   the first checkpoint. The identification block keeps that format-time
   label for ever and is never rewritten. Every reader reports the
   checkpoint's label as the volume's label.
4. A relabel is one ordinary commit that changes nothing but the label.
   Every later commit carries the label forward. An unchanged label publishes
   nothing.

## Compatibility classification

The checkpoint layout changes for every volume, without a feature identity:
no image of this filesystem has been released, so there is no earlier layout
to negotiate with. The 96-byte and 112-byte checkpoint images of the earlier
prototype are retired and every reader refuses them; they stay in the fixture
set as negative images.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | [Disk layout](../spec/disk-layout.md) rule and `AFSP_LABEL_MAX_UTF8_BYTES`, `AFSP_CHECKPOINT_LABEL_OFFSET` and both payload sizes in the [format header](../spec/afsplus_format.h); the module documentation of the checkpoint codec carries the offsets |
| ADR | This ADR |
| Compatibility classification | Whole-format change, reasoned above |
| Conformance image | `checkpoint-label-v2.bin` and `checkpoint-snapshot-v2.bin`, assembled byte by byte by [the independent Python generator](../crates/afsplus-format/tests/fixtures/generate-checkpoint-fixtures.py) and equal to the Rust encoder's output; the two v1 images stay as refused negatives |
| Parser tests | `crates/afsplus-format/tests/checkpoint_label.rs`: literal bytes and positions with and without snapshot roots, both bounds, seven resealed non-canonical fields, six retired or off-by-one payload lengths. The independent fuzz oracle models the field; the identification decoder applies the label rule it encodes with, a disagreement the fuzz mutations found |
| Repair-tool behavior | The checker reports the committed label and never edits a checkpoint. A slot with a non-canonical label is an invalid slot; the other slot is selected, exactly as for any other checkpoint damage |
| Resource impact | 72 bytes more in a block that is written whole at every commit: no extra I/O, no allocation, no extra block |

Executable proof, in `crates/afsplus-check/tests/volume_label.rs`: a relabel
survives remount and later commits, with the identification block unchanged
and the checker reporting the new label; the empty and the 64-byte label are
accepted; 65 bytes, 33 two-byte characters and an embedded NUL are refused
and publish nothing; a read-only mount refuses; every modeled power cut of a
relabel mounts to the old label or the new one, with file contents intact, a
further relabel possible and a clean checker verdict; and the portable C
reader reports the same generation and label as the Rust core, including the
same fallback to the older checkpoint for four resealed non-canonical label
fields.

## API contract consequences

- The core exposes `volume_label` and `set_volume_label`. The AROS adapter
  implements `ACTION_RENAME_DISK` over them and reports the committed label
  as the volume name; adapter, FFI and packet work belong to the handler lot.
- A host that restricts label syntax validates before calling; the core
  enforces only the format rule.
- Tools print the committed label. A tool that reads LBA 0 alone sees the
  format-time label, which is documented as such.

## Consequences

- Relabel has two legal states under a power cut without any new structure.
- The label costs nothing at mount: it arrives with the checkpoint that
  mount already selects.
- The identification block stays immutable, so its single copy stays safe.
- A volume can show two names to a tool that reads only LBA 0; every
  implementation in this repository reads the checkpoint.
