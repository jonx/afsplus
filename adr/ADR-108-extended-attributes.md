# ADR-108: Extended attributes are one owned chain per object

Status: Accepted
Amends: ADR-102

## Context

[docs/12](../docs/12-metadata-and-xattrs.md) promised typed attributes in
explicit namespaces and gave them no storage. Hosts need them: AROS for icon
and type hints, POSIX hosts for `user.*` and `security.*`, Windows hosts for
named metadata beside the descriptor. [ADR-102](ADR-102-clone-metadata-inheritance.md)
left their clone behaviour open until the format existed.

Three storages were possible. Attributes inline in the object record fit a
few dozen bytes and compete with the symlink target. A keyed tree per object
costs a root block for one small attribute and a tree walk for a listing. One
blob per object, in blocks the object owns, costs one block for the common
small set, reads the whole set with one short chain walk, and already exists:
the security descriptor container of
[ADR-101](ADR-101-security-preservation-container.md) is exactly such a chain.
Attribute sets are small and read far more often than written, so the blob is
the fit.

## Decision

1. The chain of ADR-101 becomes the *owned chain*: an immutable run of linked
   blocks owned by one object, written whole by one commit, all segments in
   one generation ([ADR-105](ADR-105-security-reference-admission.md)),
   replaced whole. Its segment layout is unchanged. A kind of owned chain is a
   block magic, a bound on the content and a reference in the object record.
   `"AFSX"` is the security kind, byte for byte as before.
2. `"AFSA"` is the attribute kind. Its content is the whole attribute set of
   the object, at most 65,536 bytes; its segments carry format identity 1,
   version 0.
3. The set is: entry count (2 bytes, nonzero), reserved (2, zero), then the
   entries in strictly ascending order of name bytes. An entry is: name length
   (1, nonzero), reserved (1, zero), value length (2), name, value. Admission
   is exact: the entries end where the content ends. One set has one
   encoding, so two implementations that hold the same attributes write the
   same bytes.
4. A name is 1 to 255 bytes of UTF-8 without NUL and starts with one of
   `user.`, `system.`, `security.` or `aros.`, followed by at least one byte.
   A value is 0 to 65,535 opaque bytes. The filesystem gives no namespace a
   meaning: access policy for `system.` and `security.` belongs to the host
   adapter. The security descriptor stays in its own container; it is not an
   attribute.
5. Object flag bit 4, `OBJECT_FLAG_ATTRIBUTES`, marks a record that carries
   the attribute reference: first segment block (8, nonzero), set length (4),
   segment count (2, consistent with the length), reserved (2, zero). It
   follows the security reference and precedes the comment. An object without
   attributes has no reference and no chain; an empty set is never stored.
   Every object type carries attributes. No volume feature gates the flag:
   an implementation that does not read attributes still knows the field and
   preserves it.
6. Every change to the attributes of an object is one commit: the new chain,
   the retirement of the old one and the rewritten record are published
   together. A batch of changes to one object is one such commit; a batch
   with one refused change publishes nothing. A batch that leaves the set as
   it was publishes nothing. The change time advances; the modification time
   stays.
7. `CloneFile` copies the set into a fresh chain owned by the destination.
   Deleting an object retires its chain. A damaged chain never blocks the
   deletion: the segments proven consistent are retired and the rest leak,
   as for the descriptor chain.
8. Until the snapshot lifetime ledger owns owned chains, a volume with
   persistent snapshots ([ADR-070](ADR-070-persistent-snapshot-priority.md))
   refuses to set attributes, so no retained record can name a chain the live
   side retires. Planned work, owner claude-b, directly after the portable C
   reader learns the field: ledger ownership of owned chains, which removes
   this refusal and the mount refusal of ADR-101 for descriptors together,
   and adds the snapshot readers.

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released. Records without attributes keep their exact image, and the
`"AFSX"` segment image is unchanged, so every existing fixture, conformance
image and corpus entry stays valid.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | The object record table of [docs/04](../docs/04-object-model.md), [docs/12](../docs/12-metadata-and-xattrs.md), rule 12 of [the disk layout](../spec/disk-layout.md). The constants of the [format header](../spec/afsplus_format.h) arrive with the portable C reader lot |
| ADR | This ADR |
| Compatibility classification | Whole-format change, reasoned above |
| Conformance image | Arrives with the portable C reader lot: cross-read of records with the reference and of `"AFSA"` segments, with resealed negatives. Until then the C reader refuses flag bit 4, which is the safe side |
| Parser tests | `crates/afsplus-format/tests/owned_chain.rs` (the generic segment codec equals the security codec byte for byte; one kind refuses another's blocks) and `crates/afsplus-format/tests/object_attributes.rs` (literal bytes of the reference and of the set, field order, every malformed reference refused both ways, every proper prefix and extension of a set refused, order, duplicates, namespaces, bounds). The independent fuzz oracle models the reference |
| Repair-tool behavior | The checker walks the chain, decodes the set and claims the blocks; a damaged chain or an invalid set is a corrupt object. It never edits one. `afsplus_check::explain` names every segment and lists names and value lengths from its own walk |
| Resource impact | None for an object without attributes. With them: 16 bytes in the record, one block per 4,040 bytes of set at 4 KiB blocks, one chain read per attribute read, and a whole-set rewrite per change, bounded by 17 blocks |

Executable proof, in `crates/afsplus-check/tests/extended_attributes.rs`: a
three-segment set on a file and sets on a directory, a symlink and the root
survive a descriptor change, a comment change, a data write into an extent
tree, truncation, the data-policy flag, hard link, rename, a batch unlink, a
window write and commit, child creation and a symlink rename, across remount;
a clone carries the set and the two are independent afterwards; removing the
last attribute and deleting the object leave no allocated block without an
owner; the three write modes, the bounds, the whole-or-nothing batch and the
no-ops publish what they should and nothing else; a snapshot volume refuses;
a segment with a broken checksum is reported by the checker and by explain,
fails reads and writes as corrupt, and leaves the object deletable; and every
modeled power cut of replacing, emptying and first writing a set (1,988
states) mounts to the old set or the new one with a clean checker verdict.
With the delete retirement, the record-flag mask of the data path and the
clone copy removed, four of the six tests fail.

## API contract consequences

- The core exposes `attribute`, `attribute_names` and `set_attributes` with
  `AttributeWriteMode` (`Upsert`, `Create`, `Replace`). Writing reports
  `AlreadyExists` and `NotFound` for the mode, `NotFound` for removing an
  absent attribute, `InvalidMetadata` for a name, value or set out of range,
  and `FeatureDisabled` on a volume with persistent snapshots.
- Filesystem API v2 gains the three operations and states the limits as
  capabilities: 255-byte names, 65,535-byte values, 65,536 bytes per object.
  The API documents and the AROS adapter take this up with the Stage C work.
- ADR-102 is amended: `CloneFile` carries the attribute set.

## Consequences

- A small set costs one block and one read; a listing of names reads the
  same chain.
- Changing one attribute rewrites the set. That is the price of the blob, and
  the 64 KiB bound keeps it at 17 blocks.
- The owned chain is now a format concept with two kinds, and a third costs
  a magic and a reference.
