# ADR-034: Shared bounded copy-on-write tree engine

Status: Accepted for the prototype; four authoritative adapters published

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
- [Bounded-resource contract](#bounded-resource-contract)
- [COW mutation contract](#cow-mutation-contract)
- [Typed adapters](#typed-adapters)
- [Validation](#validation)
- [Status and compatibility](#status-and-compatibility)

<!-- /toc -->

## Context

The executable core began with four independent one-block limits:

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

Modern mutation retains that complete dirty overlay in memory. Constrained
mutation may instead spill an unreachable staged node to its allocated block
and reload it later through an LRU. The implementation qualifies budgets of
2, 4, and 8 staged pages. It drops decoded ancestors before recursive descent,
keeps compact coordinates, and re-reads parents on unwind; RAII counters cover
decoded and derived nodes and observe a maximum of two during insertion and
merge-heavy deletion. The compact per-LBA overlay index and caller-owned
operation batch remain proportional to transaction size.

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

The executable engine covers bounded lookup, exhaustive validation,
transactional multi-upsert/delete, leaf/internal split, sibling
merge/redistribution, and root height growth/reduction. A permuted 300-key
test grows a three-level tree, replaces a key, then deletes 299 keys in a
different permutation and returns to a one-item root leaf; the exhaustive
verifier runs at both ends. Staged-node spill/reload passes the explicit
2/4/8-page matrix for insertion and deletion, including merges and root
collapse. Split selection computes candidate encoded sizes without repeatedly
cloning whole nodes. A transaction-level power-cut matrix also discovers and
publishes the directory-root 1→2 split boundary, then qualifies the merge and
root collapse back to height 1 under every recorded write and flush.

The object map is the first authoritative consumer. `mkfs` writes an AFST
leaf, checkpoints reference its root, normal mount/stat use bounded typed
lookups, create/delete share mixed-operation overlays, and the checker
exhaustively visits typed leaves plus all internal blocks. The legacy
single-block object-map codec remains only as transitional format/test code;
newly formatted volumes do not reference it.

Directories are the second ordinary-allocator consumer. `mkfs` creates an
empty typed AFST leaf; bounded mount validates only the directory root;
lookup descends one path; enumeration and the checker traverse exhaustively.
Create/mkdir/unlink/rmdir/link and same/cross-directory rename publish every
affected directory and object-map mutation in the same checkpoint transaction,
and each engine retires only the committed COW paths it replaces. Rename keeps
stable object identity, rejects directory cycles, and passes a dedicated
power-cut matrix. A typed 1,000-entry unit test crosses leaf/internal boundaries;
an end-to-end 300-entry volume test exceeds the legacy one-block capacity,
then checks, remounts, enumerates, and looks up entries. Ordinary transaction,
fault-injection, and power-cut matrices all exercise this authoritative path.
The legacy `DirBlock` codec remains transitional test coverage only.

The allocation root is another authoritative consumer and uses the same
engine with the permanent triple-version node pool defined by ADR-035.
Checkpoints publish its root instead of inline region records. Transactions
load current and retained region records on demand and upsert only dirty
records. A 145-region crash matrix crosses the first leaf-capacity boundary,
and the sparse 1 TiB qualification exercises a 1,024-record multi-level root.
