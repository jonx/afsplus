# ADR-034: Shared bounded copy-on-write tree engine

Status: Accepted for the prototype; mutation engine in progress

## Context

The current executable core has four independent one-block limits:

- checkpoint object map
- each directory
- each file's direct extent mapping
- checkpoint allocation-region records

Implementing four unrelated trees would duplicate crash ordering, cache,
split/merge, validation, and checker logic. It would also make the portable C
implementation substantially harder to keep behaviorally identical.

## Decision

Prototype one page-based B+ tree engine with binary keys and values. Typed
adapters define the key/value encoding for object maps, directories, extents,
and the allocation-region root.

Every node carries:

- tree kind
- level (`0` is a leaf)
- owning object/domain
- transaction generation
- strictly ordered binary keys
- subtree item count
- an internal-node leftmost child plus one right child per separator key,
  each carrying its exact subtree item count
- a checksum through the common metadata header

Internal separator key `K` is the minimum key reachable through the child to
its right. Search therefore chooses the rightmost separator `<= lookup_key`,
or the leftmost child when none qualifies.

Nodes do not contain persistent leaf-sibling pointers in this prototype.
Iteration uses a bounded root-to-leaf path stack. Under COW, sibling links
would make a local split require rewriting an otherwise unrelated neighbor,
expanding both the transaction and crash-consistency surface.

## Bounded-resource contract

- tree depth is explicitly capped and validated
- decoding validates counts and lengths before allocating
- search needs at most one node per level, and the tiny-cache implementation
  may release ancestors after copying only path coordinates
- mutation pins only the changed path plus at most one split/merge peer per
  level
- full-tree validation and item counting belong to the checker, not mount
- cache size may change performance but never tree semantics

The initial implementation must run with cache budgets of 2, 4, and 8 pages.

## COW mutation contract

A mutation writes replacement leaves first, then replacement ancestors up to
a new root. None of those blocks may be reachable from either retained
checkpoint. The new root becomes visible only through the normal metadata
barrier and alternate checkpoint publication.

Several upserts in one transaction share an in-memory write overlay. The
first change copies and retires each committed node on its path; later changes
rewrite already-staged nodes in place and allocate only for new splits. The
overlay is emitted as one final image per LBA, so intermediate node images are
never sent to the block device. Every descent validates that the child level
is exactly one less than its parent and remains within the depth cap.

The first mutation prototype retains that complete dirty overlay in memory.
This proves COW topology and eliminates repeated device reads, but is not yet
the cache-2/4/8 proof: the tiny-cache tranche must be able to spill an
unreachable staged node to its allocated block and reload it later while
keeping only the active path and split peer resident.

Deletion rebalances an underfull child with an adjacent sibling before the
parent is staged. If their combined image fits, they merge; otherwise their
combined ordered contents are split again into two balanced nodes. A root
left with one child is retired and replaced by that child. Consequently no
empty non-root leaf or one-child non-root internal node is ever serialized.

## Typed adapters

The common node format does not make core layouts runtime-pluggable:

- object map: `u64 object_id -> u64 object-record LBA`
- directory: `comparison_key -> original name + child ID + type/flags`
- extent map: `u64 logical block -> physical start + length + flags`
- allocation root: `u32 region -> descriptor slot/generation/free count`

Integer keys use a fixed big-endian key encoding so unsigned binary comparison
matches numeric ordering. Values remain explicit little-endian AFS+ records.

## Validation

Readers and the checker validate:

- node kind/owner/level consistency
- strict key ordering and separator ranges
- child block bounds and unique ownership
- acyclic traversal and maximum depth
- exact subtree item counts
- leaf value encoding for the selected tree kind
- checksums and checked arithmetic

## Status and compatibility

This is an experimental wire format, not an epoch-1 freeze. Measurements and
crash matrices may revise it. Rust data structures and allocation behavior are
not normative; the byte encoding and invariants must remain implementable by
the portable C profile.

The executable engine currently covers bounded lookup, exhaustive validation,
transactional multi-upsert/delete, leaf/internal split, sibling
merge/redistribution, and root height growth/reduction. A permuted 300-key
test grows a three-level tree, replaces a key, then deletes 299 keys in a
different permutation and returns to a one-item root leaf; the exhaustive
verifier runs at both ends. Typed adapters, publication through the checkpoint
transaction, and the associated power-cut matrices remain required before
Core Scale-1 is complete. So does staged-node spill/reload under the explicit
2/4/8-page cache matrix.
