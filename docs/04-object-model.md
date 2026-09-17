# 04. Object Model

> **ADRs:** [ADR-066](../adr/ADR-066-bounded-orphan-directory.md) ·
> [ADR-068](../adr/ADR-068-portable-symlink-targets.md), [ADR-100](../adr/ADR-100-exact-object-record-admission.md), [ADR-101](../adr/ADR-101-security-preservation-container.md) · **Spec:** none ·
> **Tests:** [crash-testing](../testing/crash-testing.md) · **Milestones:** M03

<!-- toc -->

- [1. Stable objects](#1-stable-objects)
- [2. Object types](#2-object-types)
- [3. Core object record](#3-core-object-record)
- [4. Object identity invariants](#4-object-identity-invariants)
- [5. Orphan handling](#5-orphan-handling)
- [6. ID reuse](#6-id-reuse)
- [7. Executable prototype status](#7-executable-prototype-status)
- [Metadata mutation and restoration](#metadata-mutation-and-restoration)

<!-- /toc -->

## 1. Stable objects

Files and directories are objects with stable numeric IDs.

A path is a route to an object, not the identity of the object.

Each object is identified within a filesystem by:

```text
filesystem UUID + 64-bit object ID
```

Object ID zero is invalid.

## 2. Object types

Core object types:

- regular file
- directory
- symbolic link
- system/internal object

Future types require feature negotiation.

## 3. Core object record

The core record contains only fields required by almost every implementation:

- object ID
- object type
- flags
- link count
- logical size
- allocated size
- creation timestamp
- modification timestamp
- metadata-change timestamp
- AROS protection flags
- extent-root or inline extent data
- attribute-root
- generation
- checksum

Large or uncommon metadata belongs in attributes, not in an ever-growing fixed inode.

An object record is admitted only in its canonical image
([ADR-100](../adr/ADR-100-exact-object-record-admission.md)): zero common
header flags, a payload of exactly the length its type and flags define, a
zero reserved byte, assigned object flags only and a zero tail. Every reader
applies the rule through one shared check, in Rust and in portable C, because
a rewrite re-encodes decoded fields into a zeroed block and would drop any
byte admitted without a field. The record grows only through an object flag
bound to a volume feature identity that defines the exact new length.

The first such extension is the security reference
([ADR-101](../adr/ADR-101-security-preservation-container.md)). Object flag
bit 2 extends the fixed payload from 96 to 112 bytes, before any inline
symlink target: first descriptor segment block, descriptor length, segment
count and a flags word whose bit 0 marks a diverged classic projection. The
descriptor is an opaque byte string of 1 to 65,536 bytes with a format
identity and a format version, stored in a chain of `"AFSX"` segments owned
by that object alone. The filesystem stores, returns, copies on `CloneFile`
and removes it, and never evaluates it. The projection rule for protection
edits is in [docs/30](30-portable-security-model.md#9-classic-amiga-compatibility-profile).

## 4. Object identity invariants

Rename does not change object ID.

Moving an object between directories does not change object ID.

Creating a hard link does not create a second object.

Deleting the final directory link while a file is open must not immediately recycle the object ID.

## 5. Orphan handling

Object ID 2 is reserved for the internal orphan directory when the
`org.aros.afsplus:orphan-directory` feature is enabled. The directory is
created lazily and is never linked from the user root. Its canonical
lowercase hexadecimal entries preserve final-link files while a VFS handle
is open; the filesystem-facing VFS also uses this bounded transition when no
handle remains so unlink never has work proportional to file fragmentation.
The ordinary object record retains `link_count == 1`.

Last close and post-crash maintenance remove logical extent records from the
tail under a runtime budget, publishing each shorter layout before the final
small transaction removes the entry and object. This gives cleanup a durable
restart point without an object-map scan. Public lookup, enumeration, stat,
hard-link and rename APIs cannot address object 2 or rediscover its children.

## 6. ID reuse

Object IDs should not be aggressively reused.

A monotonically increasing allocator is preferred until wraparound is no longer a realistic concern.

If reuse is ever implemented, generation information must prevent stale `(object ID)` references from being mistaken for a newly created object.

## 7. Executable prototype status

The current core creates files and directories under any directory object ID.
Same-directory rename and cross-directory move preserve the child object ID;
directory moves into self/descendants are rejected before a transaction
starts. Regular-file hard links update the object record and destination tree
atomically. The VFS atomically moves every final file link into object 2; with
live handles, reads, writes, truncate and fsync retain stable identity until
last close, while no-handle cases are immediately eligible for bounded idle
cleanup. Final-link atomic-replace targets follow the same rule in the
replacement checkpoint. The lower-level core still permits the legacy direct
delete primitive for controlled tests and feature-absent volumes. Directory
hard links remain intentionally unsupported.

The symlink representation keeps a NUL-free UTF-8 target inline
after the fixed object-record fields, under the same whole-block checksum.
The target has no allocation extents and is returned byte-for-byte; OS path
layers, rather than the object codec, interpret its namespace syntax. The explicit target codec validates the full variable payload before returning
metadata. Fixed-record encoding rejects symlinks, so mutation paths cannot
silently discard target bytes.

`create_symlink` atomically publishes a type-3 object and directory entry.
`read_link` and `snapshot_read_link` return the required target byte count;
when the buffer is shorter, it is unchanged. Targets are not NUL-terminated or
followed. Protection edits, exact metadata restoration and rename preserve the
inline bytes under the new checksum. `unlink_symlink` retires metadata without
data extents or regular-file orphan handling. Symlink hard links are refused.
Changing a target requires replacement with a new object; atomic replacement
and adapter exposure require their own qualified operation paths.

## Metadata mutation and restoration

The core `set_object_protection` operation changes the existing 32-bit protection
field and change timestamp. It preserves creation and modification timestamps,
object identity, content generation, link count, layout and file policy flags.
An unchanged protection value is a no-op. These bits are the prototype's existing
protection representation; this operation does not reinterpret them as a POSIX
mode or a canonical rich ACL.

`restore_object_metadata` restores the exact `PreservedMetadata` tuple:
protection plus creation, modification and change timestamps. Archived change
time may precede the destination transaction; metadata restoration deliberately
preserves it. Destination object identities, link counts, data layouts and
content generations are established by destination operations, not supplied
through this tuple. Set directory timestamps after restoring their children.

Both operations reject hidden/internal objects, invalid timestamp nanoseconds,
read-only modes, open mutation windows and uncertain-publication state. All
signed timestamp seconds are representable; nanoseconds must be below one
billion. Object and intent-log encoders enforce the same timestamp constraint
as their readers. Mutation entry points validate caller-supplied times before
staging work. Invalid input is rejected before allocation or writes. Identical
restoration is a no-op, but still requires a writable, unpoisoned volume and a
valid target. Protection changes and exact restoration are immediate durable
metadata-COW transactions using the common commit and snapshot-lifetime tail.
They write no file data and leave registered historical metadata unchanged.

[ADR-077](../adr/ADR-077-separate-restore-authority.md) requires separate host
restore authority before exposing restoration to consumers. The trusted core
mutation does not perform OS authentication. The existing fields and encodings
are unchanged; this tuple is not a complete attribute/security transport or a
full-volume restore format. See [ADR-076](../adr/ADR-076-pax-backup-interchange.md).
