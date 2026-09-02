# Core Invariants

A conforming implementation must enforce these invariants.

## Allocation

- every allocated physical block is owned by exactly one live allocation,
  except for data blocks accounted by the active shared-extents feature
- no free bitmap bit may mark a reachable authoritative metadata block as free
- no extent may exceed volume bounds
- extent logical ranges for a file do not overlap

## Shared extents

When `org.aros.afsplus:shared-extents` is active, the volume-wide shared-extent
tree is the authority for data blocks referenced by multiple live mappings in
the selected checkpoint ([ADR-061](../adr/ADR-061-shared-extent-references.md)).

- the canonical tree contains exactly one maximal record for every physical
  sub-run with two or more live mappings, and no record for a sub-run with
  fewer than two mappings
- each record's reference count equals the number of live mappings covering
  every block in that record; records are ordered, non-overlapping, within
  allocatable bounds and merge adjacent runs with equal counts
- an extent without `EXTENT_SHARED` may not overlap a shared-tree record; an
  extent with `EXTENT_SHARED` but no overlapping record is legal and private
- direct-layout files may not participate in shared runs, and a file whose
  extent map carries `EXTENT_SHARED` may not collapse to direct layout
- shared data blocks are allocated exactly once in the bitmap and may not
  overlap metadata, authoritative free space or the reclaim queue
- dropping one of exactly two references removes the shared-tree record but
  does not retire or free the run; only dropping its final private mapping may
  enqueue it for deferred reclamation
- the shared-tree update and every extent-map update it describes become
  visible through the same checkpoint
- reference counts describe the selected checkpoint only; older selectable
  checkpoints are protected independently by deferred reclamation
- a non-zero shared-tree root or an `EXTENT_SHARED` flag without the feature
  bit is corruption; the feature bit with a zero root is the legal
  enabled-but-unused state

## Objects

- object ID zero is invalid
- root object exists and is a directory
- live hard-link count matches reachable directory references, subject to orphan semantics
- object IDs do not change on rename

## Directories

- every directory key is correctly normalized for that directory's case policy
- directory tree keys are ordered
- no directory entry points to an invalid object
- `.` and `..` are namespace conveniences, not required disk entries

## Journal

- transaction sequence is monotonic
- only committed transactions affect recovered authoritative state
- replay is idempotent or otherwise safely detectable

## Catalog

- catalog is never authoritative
- catalog generation mismatch disables the fast path
- catalog records may not cause allocation or object lifetime decisions

## Checksums

- corrupted metadata is not silently accepted
