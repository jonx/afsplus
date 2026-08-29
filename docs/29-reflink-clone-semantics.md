# 29. Reflink and Clone Semantics

Status: epoch-1 design requirement for shared data extents

## 1. Move, hard link, symlink, clone, and copy are different

AFS+ must expose these semantics clearly.

| Operation | New object | Initial data sharing | Later writes independent | Namespace target dependency |
|---|---:|---:|---:|---:|
| Move/rename | no | same object | n/a | no |
| Hard link | no | same object | no | no |
| Symlink | yes, link object | no | target semantics | yes, stores path/reference semantics |
| Reflink clone | yes | yes | yes through COW | no |
| Physical copy | yes | no | yes | no |

## 2. Same-volume moves

Moving an object inside one AFS+ volume must be a metadata-only namespace operation. File data is never copied merely because the parent directory changes.

Stable object ID is preserved across rename/move.

Cross-directory move participates in one metadata transaction so the object is never simultaneously lost from both names or committed into a half-moved state.

## 3. CloneFile

`CloneFile()` creates a new file object whose data mapping initially references the same physical extents as the source.

Example:

```text
source object 100 -> extent X, Y, Z
clone  object 101 -> extent X, Y, Z
```

Object metadata is independent unless explicitly inherited by API contract.

The clone gets a new stable object ID.

## 4. Copy-on-write after cloning

If object 101 modifies bytes inside extent Y:

```text
100 -> X, Y,  Z
101 -> X, Y', Z
```

Only the modified logical range requires newly allocated storage.

The implementation should avoid copying an entire large extent when a smaller safe split can represent the modified range.

## 5. CloneRange

`CloneRange()` shares only the requested aligned or representable source range with a destination file.

The API defines byte ranges. The filesystem may internally round/split extents as required while preserving exact visible byte semantics.

Useful workloads include:

- package caches
- VM images
- build trees
- editor temporary copies
- large database/image-file branching
- backup staging

## 6. Interaction with sparse files

Holes remain holes and should not allocate physical storage merely because a range is cloned.

A cloned allocated range remains shared until one side writes.

## 7. Interaction with checksums

If optional user-data checksums are enabled, shared extents share the same checksum state while their bytes are identical.

A COW write creates new checksum state for the newly allocated extent/range.

## 8. Interaction with deferred reclamation

Unlinking one clone does not make shared data reusable.

A physical run becomes reclaimable only after:

1. no live object references it
2. no retained checkpoint can reference an older mapping that requires it
3. all shared-reference metadata changes are durably committed

## 9. Validation

The checker must verify independently that:

- every shared extent's accounting matches live references
- no free-space bitmap bit overlaps a live shared extent
- extent split/merge operations preserve reference counts
- reverse-map data, when present, agrees or is marked stale/rebuildable

Fault tests must inject crashes at every step of clone creation, COW split, unlink, and last-reference reclamation.

## 10. CloneTree is separate

Recursive tree cloning is intentionally not implied by `CloneFile`.

A future tree-clone design must decide whether directory objects are duplicated, structurally shared, or regenerated. It must also define parent IDs, hard-link behavior, notifications, change-stream events, policies, and catalog semantics.

Until then, tools may recursively create directories and reflink individual files, which already avoids copying file data.
