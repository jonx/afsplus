# ADR-027: Reflink file and range cloning

Status: Accepted as an epoch-1 format requirement; implementation may be staged

## Context

Copying large files by reading and rewriting every byte wastes time, bandwidth, flash endurance, and energy when source and destination are on the same filesystem.

Modern filesystems such as APFS, XFS, Btrfs, OpenZFS, and ReFS demonstrate the value of copy-on-write cloning/reflinks.

AFS+ already requires stable object identity, extent-based files, transactional metadata, and deferred reclamation. Those primitives make cloning a natural capability if shared-extent semantics are designed into the format from the beginning.

Adding shared extents later would be much harder because allocator, checker, reverse-map, and reclamation invariants would all change.

## Decision

The epoch-1 AFS+ extent model must permit multiple file objects to reference the same physical data extent safely.

Filesystem API v2 will expose capability-gated semantic operations equivalent to:

```text
CloneFile(source, destination)
CloneRange(source, source_offset, destination, destination_offset, length)
```

The destination is a distinct object with independent metadata and future write behavior.

A clone is not a symlink and not a hard link.

## Write semantics

After cloning:

```text
A -> X Y Z
B -> X Y Z
```

If B modifies data covered by Y, AFS+ allocates new storage for the modified range and updates B only:

```text
A -> X Y  Z
B -> X Y' Z
```

The operation must preserve atomic visibility of the modified extent map.

## Shared-extent accounting

The base format must not assume that every allocated data block has exactly one file owner.

The implementation may use a shared-extent/refcount structure, extent-reference objects, or another format mechanism selected during prototyping.

Required invariants:

- a physical extent is freed only when no live object and no retained recovery checkpoint can reference it
- refcount/reference updates participate in the same metadata transaction as extent-map changes
- range splitting/merging cannot lose or duplicate references
- deferred reclamation understands shared extents
- checker and reverse-map logic can validate sharing independently
- uncertain reference state leaks/quarantines storage instead of freeing early

## CloneTree

Recursive directory-tree cloning is intentionally not accepted yet.

A future `CloneTree()` may be evaluated after file/range cloning is proven. Directory cloning interacts with parent identity, hard links, catalog/change-stream semantics, policy inheritance, notifications, and recursive object identity, so it is a separate design problem.

## Compatibility

Filesystems without clone support report no clone capability and Filesystem API v2 callers fall back to ordinary copy.

Minimal AFS+ readers need only understand that multiple objects may legitimately reference shared data extents; they do not need to implement clone creation.
