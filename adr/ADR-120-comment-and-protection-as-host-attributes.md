# ADR-120: The comment and the protection word are host attributes

Status: Accepted
Amends: ADR-108

## Context

A file on a volume mounted through FUSE has two Amiga properties that no POSIX
call reaches:

- the comment, a field of the object record since
  [ADR-106](ADR-106-stored-object-comment.md);
- the bits of the protection word that the mode does not project: Script,
  Pure, Archive, and Delete forbidden without Write forbidden
  ([ADR-118](ADR-118-posix-permission-projection.md)).

A host tool that copies files between two volumes therefore dropped both, and
nothing on the host could read or set them. Host copy tools already carry
extended attributes when asked to (`cp -p` on macOS, `rsync -X`, the Finder
between volumes that store them).

## Decision

1. The FUSE adapter shows the comment and the protection word as two
   attributes. They are fields of the object record, not stored attributes:
   reading one reads the record and writing one writes the record.

   | Host  | Comment                     | Protection word                |
   |-------|-----------------------------|--------------------------------|
   | macOS | `afsplus.aros.comment`      | `afsplus.aros.protection`      |
   | Linux | `user.afsplus.aros.comment` | `user.afsplus.aros.protection` |

   Linux uses `user.`, not `trusted.`, so that an unprivileged copy tool
   carries them.
2. The comment is its UTF-8 text. It is absent when empty, and removing it
   clears it. A value that is not UTF-8 or holds a NUL is refused.
3. The protection word is always present. It reads as `0x` followed by eight
   upper-case hex digits. A write accepts an optional `0x` followed by one to
   sixteen hex digits and nothing else. A value above 32 bits names bits the
   word does not have. A write that is malformed, above 32 bits or a removal
   is refused with `EINVAL` and changes nothing. Writing it changes the host
   mode, which is the word's projection.
4. The attribute rules apply to both fields: a create finds the word, and a
   set comment, already present; a replace needs a comment.
5. The stored attribute names `aros.comment`, `aros.protection`,
   `user.afsplus.aros.comment` and `user.afsplus.aros.protection` are those the
   host names above would otherwise reach. No interface may store them:
   the VFS refuses them with `Invalid`, whether the write comes from FUSE,
   AROS or a restore. So no stored attribute sits hidden behind a field.
6. The listing names the stored attributes first, then the protection word,
   then the comment when set.

## Consequences

- `xattr -l` on macOS and `getfattr -d` on Linux show both. A host copy that
  keeps extended attributes between two AFS+ volumes, or out to a file system
  that stores them and back, keeps the comment and every protection bit.
- A copy to a file system without extended attributes, such as FAT or exFAT,
  loses both. The Finder then writes them into an AppleDouble `._` file that
  other systems ignore.
- Every file lists one more attribute, the protection word.
- A stored attribute of a reserved name, written before this decision, is no
  longer reachable through FUSE; there is no legacy to keep.
