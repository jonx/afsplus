# ADR-119: A backup carries the comment and the owner

Status: Accepted
Amends: ADR-082

## Context

[ADR-076](ADR-076-pax-backup-interchange.md) requires a restore to preserve
every piece of metadata it is given or to refuse. The object metadata payload
of [ADR-082](ADR-082-backup-object-metadata.md) had no field for two values a
volume stores:

- the comment, a field of the object record since
  [ADR-106](ADR-106-stored-object-comment.md);
- the owner UID and GID, stored fields since
  [ADR-118](ADR-118-posix-permission-projection.md).

A backup and restore therefore lost both without a word: the restored file had
no comment and belonged to root. The tar headers of the archive are no place
for the owner either. Their mode, UID and GID are fixed values by the envelope
contract, so that an unaware tar extraction cannot give an archived file a
real owner or real permissions, and the protection word already travels in the
payload for the same reason.

## Decision

1. The payload carries three more required fields: `AROS.object.comment`, the
   comment as UTF-8 without NUL and empty when the object has none, and
   `AROS.object.uid` and `AROS.object.gid`, canonical decimal numbers from 0 to
   4294967295.
2. The payload version becomes `2`. Version `1` is refused like any unknown
   version: there is no legacy, and an archive written before this decision
   cannot be restored with its metadata. Tar headers keep their fixed owner.
3. A backup source reads the comment of the captured object from its
   snapshot. A source that cannot answer refuses the export.
4. A restore sets the comment first and then the protection, owner and times,
   so that the change time the comment advances is restored exactly. It then
   reads back all of them, the comment included.
5. A destination that cannot keep a comment refuses the restore, naming the
   archived path. A destination without comments satisfies an empty comment.

## Consequences

- A backup and restore through AFS+ keeps the comment, the owner and every
  protection bit, the Amiga-only bits included.
- Archives written with payload version 1 are refused.
- A restore provider must implement the comment, or restore only objects
  without one.
