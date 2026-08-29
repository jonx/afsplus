# ADR-034: Shared bounded copy-on-write tree engine

Status: Proposed, executable prototype in progress

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

Deletion may initially defer occupancy rebalancing only if it still removes
empty children, keeps search correct, caps depth, and documents the resulting
space cost. The Core Scale-1 exit gate nevertheless requires tested merge and
root-height reduction.

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
