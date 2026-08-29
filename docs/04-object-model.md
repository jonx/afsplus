# 04. Object Model

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

AFS+ must explicitly handle objects that have no directory links but remain open.

A transactionally maintained orphan structure records such objects until the final open reference is released.

This prevents leaked extents and makes crash recovery deterministic.

## 6. ID reuse

Object IDs should not be aggressively reused.

A monotonically increasing allocator is preferred until wraparound is no longer a realistic concern.

If reuse is ever implemented, generation information must prevent stale `(object ID)` references from being mistaken for a newly created object.

## 7. Executable prototype status

The current core creates files and directories under any directory object ID.
Same-directory rename and cross-directory move preserve the child object ID;
directory moves into self/descendants are rejected before a transaction
starts. Regular-file hard links update the object record and destination tree
atomically. Unlink decrements the authoritative count and retains content
until the final link, when record and data blocks enter checkpoint quarantine.
Directory hard links and open-but-unlinked orphans are intentionally not yet
implemented.
