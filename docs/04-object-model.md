# 04. Object Model

> **ADRs:** [ADR-066](../adr/ADR-066-bounded-orphan-directory.md) · **Spec:** none ·
> **Tests:** [crash-testing](../testing/crash-testing.md) · **Milestones:** M03

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
is open; the ordinary object record retains `link_count == 1`.

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
atomically. A final unlink with no handle immediately retires the file. With
live handles it atomically moves the only link into object 2; reads, writes,
truncate and fsync retain stable identity until last close. Open atomic-replace
targets follow the same rule in the replacement checkpoint. Directory hard
links remain intentionally unsupported.
