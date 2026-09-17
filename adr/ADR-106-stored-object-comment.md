# ADR-106: The object comment is a field of the object record

Status: Accepted
Amends: ADR-102

## Context

File comments are first-class AROS semantics: `ACTION_SET_COMMENT` writes
one, and every `FileInfoBlock` a directory listing returns carries one. The
format had no place for it. [docs/12](../docs/12-metadata-and-xattrs.md)
allowed the comment to live in the attribute subsystem, which does not exist
yet, and [ADR-102](ADR-102-clone-metadata-inheritance.md) left its clone
behaviour open for the same reason.

Two homes were possible: an attribute, stored wherever attributes will be
stored, or a field of the object record. A directory listing decides between
them. `ExNext` and `ExAll` return the comment of every entry, so a comment
behind a second structure costs one more lookup per listed object, on the
path constrained hosts use most. The object record is one block per object
with about a hundred bytes used, so the room is free.

## Decision

1. Object flag bit 3, `OBJECT_FLAG_COMMENT`, marks a record that carries a
   comment. The comment follows the fixed payload, after the security
   reference when the record has one and before the inline target of a
   symlink: one length byte, 1 to 255, then that many bytes of UTF-8 without
   NUL. The empty comment is the absent one: the flag is clear and the record
   keeps its shorter image. Admission is exact
   ([ADR-100](ADR-100-exact-object-record-admission.md)): the flag with a zero
   length, a length that leaves the payload, a payload longer than the
   comment, a NUL, invalid UTF-8, or comment bytes under a clear flag make
   the record corrupt. No volume feature gates the flag: every implementation
   of the format reads the comment field.
2. Files, directories, symlinks and the root directory carry a comment. A
   symlink's longest target shrinks by the comment's wire length.
3. Setting or removing a comment is one metadata commit that rewrites the
   object record. The change time advances; the modification time stays,
   because the content did not change. An unchanged comment publishes
   nothing. The value travels as an argument into the commit, never as
   volume state.
4. The comment is a field of the record, so every read-modify-write of the
   record carries it, a hard link shares it, a rename keeps it, and a
   snapshot captures it with the record.
5. `CloneFile` copies the comment to the destination, as it copies the
   protection word: the comment describes the content, and the AmigaDOS
   `Copy CLONE` precedent carries it. `CloneRange` leaves the destination's
   comment alone. This closes the comment item that
   [ADR-102](ADR-102-clone-metadata-inheritance.md) left open; extended
   attributes stay open there.
6. 255 bytes is the bound of the length byte. It holds the 79 Latin-1
   characters of a classic comment in every UTF-8 expansion, and a host with
   a smaller limit truncates or refuses in its adapter.

## Compatibility classification

Whole-format change without a feature identity: no image of this filesystem
has been released. Records without a comment keep their exact image, so every
existing fixture and corpus entry stays valid.

## Format-change procedure

| Step | State |
|---|---|
| Specification update | The object record table of [docs/04](../docs/04-object-model.md), [docs/12](../docs/12-metadata-and-xattrs.md) and `AFSP_OBJECT_FLAG_COMMENT` with `AFSP_COMMENT_MAX_UTF8_BYTES` in the [format header](../spec/afsplus_format.h) |
| ADR | This ADR |
| Compatibility classification | Whole-format change, reasoned above |
| Conformance image | The cross-read test of the portable C reader generates commented records of each object type, with and without a security reference, and their resealed negatives |
| Parser tests | `crates/afsplus-format/tests/object_comment.rs`: literal bytes at offset 96 and at 112 behind a security reference, the symlink target after the comment, the 255-byte bound, flag and field congruence both ways, six resealed corruptions. The independent fuzz oracle models the field |
| Repair-tool behavior | The checker reads the comment through the shared decoder and reports a non-canonical field as a corrupt object; it never edits one |
| Resource impact | None for an object without a comment. With one: its length plus one byte inside the record block the object already owns; no extra block, no extra read on lookup, stat or directory listing |

Executable proof, in `crates/afsplus-check/tests/object_comment.rs`: a comment
on a file, a directory, a symlink and the root survives a descriptor change,
a data write into an extent tree, truncation, the data-policy flag, hard link,
rename, a batch unlink, a window write and commit, child creation and a symlink
rename, across remount; a clone carries it and the two are independent
afterwards; the 255-byte bound is accepted, 256 bytes and a NUL are refused
and publish nothing; a snapshot keeps the comment it captured while the live
one changes; and every modeled power cut of setting and of removing a comment
mounts to the old comment or the new one with a clean checker verdict.

## API contract consequences

- The core exposes `object_comment`, `set_object_comment` and
  `snapshot_object_comment`. The AROS adapter implements `ACTION_SET_COMMENT`
  and fills the `FileInfoBlock` comment over them, converting and bounding the
  text for its host.
- Filesystem API v2 states the comment as a metadata field of at most 255
  UTF-8 bytes, carried by clone and captured by snapshots.

## Consequences

- A directory listing reads the comment with the record it already reads.
- The comment has two legal states under a power cut without a new structure.
- The attribute subsystem, when it arrives, has no comment to carry and no
  migration to perform.
