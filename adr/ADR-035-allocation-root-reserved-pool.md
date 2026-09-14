# ADR-035: Allocation-root tree in a reserved triple-version node pool

Status: Accepted

## Context

The checkpoint still embeds one `RegionRecord` per allocation region. That is
bounded for the current small images but cannot represent large volumes: at a
1 GiB region size, a 1 TiB volume already needs 1,024 records.

Moving those records into the shared AFST tree introduces a self-reference.
If replacement allocation-root nodes are allocated from ordinary free space,
their allocation changes the free counts the new nodes are meant to publish.
Repeatedly recomputing the values is not a satisfactory format invariant and
can fail to converge near region boundaries or a full volume.

## Decision

The allocation root uses the common AFST node format, validation, lookup, and
mutation engine, with typed records:

- key: four-byte big-endian region number
- value: descriptor slot, three zero reserved bytes, free-block count, and
  descriptor generation in explicit little-endian fields

Its node blocks come from a permanently allocated pool, never from the space
described as free. The region-key set is fixed by volume geometry, so updating
free counts cannot split, merge, or otherwise change the bulk-built topology.

If the bulk-built tree has `N` logical nodes, mkfs reserves `3N` physical
blocks. Two selectable checkpoints may together occupy at most `2N` images;
there is therefore space for a complete third image even when every region
record changes in one transaction. The generic COW engine receives a
`TreeAllocator` backed by this pool. Retiring an old committed pool image does
not put it into the ordinary retired list, because the older checkpoint may
still reference it and every pool block remains permanently allocated.

The pool LBA list is deterministic: it is the first `3N` allocatable blocks
after the three bootstrap metadata blocks (root record, root directory, and
initial object-map root), skipping every region's reserved descriptor/bitmap
head. Thus portable implementations can derive it from immutable geometry and
the fixed allocation-root bulk-packing rules; no host allocator state is
needed to discover it.

## Crash and corruption contract

- nodes referenced by the current or older selectable checkpoint are excluded
  from allocation
- a transaction writes only an unreferenced pool image
- allocation-root nodes and region descriptors are durable before checkpoint
  publication
- a pool LBA outside the deterministic set is corruption
- an image allocated and discarded before publication may be reused within
  that same transaction
- unused pool blocks are intentionally allocated reserve, not checker leaks

The existing three-slot region descriptor and bitmap rules remain unchanged.
The allocation root selects descriptors; descriptors continue to select
bitmap-page generations.

## Current executable evidence

The COW engine is allocator-agnostic through `TreeAllocator`. The typed
allocation-root adapter validates key/value width, reserved bytes, geometry,
slot, generation, and exact region coverage. A reserved-pool test retains
generation-1 and generation-2 roots while generation 3 is written to the sole
remaining pool block.

Checkpoint encoding now carries the root LBA and an O(1) total-free count;
new checkpoints contain no inline region records. Mkfs bulk-builds the tree,
marks the complete `3N` pool permanently allocated, and commits its root.
Ordinary commits COW all fixed region records through an unused pool image;
the checker claims active nodes and accounts for unused reserve blocks. The
ordinary create/delete/reuse power-cut matrices cover the one-node root.

Allocator record loading is temporarily exhaustive at transaction begin and
all records are rewritten per commit. On-demand lookup/dirty-leaf mutation,
the full sparse 1 TiB mkfs/check/remount path, and crash matrices that exercise
a multi-node allocation root remain required performance/scale gates.
