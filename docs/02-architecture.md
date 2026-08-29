# 02. Architecture

## 1. Layering

AFS+ is split into four logical layers.

### Layer A: namespace and compatibility

Owned by AROS DOS and Filesystem API v2.

Responsibilities:

- resolving `SYS:`, `Work:`, `PROGDIR:`, and Assigns
- parsing AROS path syntax
- mapping classic DOS calls onto 64-bit internal operations
- exposing capabilities
- compatibility with old binaries

### Layer B: filesystem front-end

`afsplus.handler` maps system operations onto the portable core.

It handles:

- DOS locks and handles
- AROS notifications
- removable-volume lifecycle
- system error translation
- handler task lifecycle

### Layer C: portable filesystem core

`libafsplus` owns:

- on-disk parsing
- object lookup
- directory indexing
- extents
- allocation regions
- transactions
- journal replay
- checksums
- feature negotiation
- catalog/change-stream logic
- validation and repair primitives

It must not depend directly on DOS packets.

### Layer D: block provider

The core sees a narrow device abstraction:

```c
read_blocks(ctx, lba, count, buffer)
write_blocks(ctx, lba, count, buffer)
flush(ctx)
discard(ctx, lba, count)
get_size(ctx)
get_sector_size(ctx)
```

A host file, USB disk, Apple NVMe device, or classic trackdisk implementation can all satisfy this interface.

## 2. Bounded-memory rule

The on-disk structures and core algorithms must not require:

- loading the full allocation bitmap
- loading the full catalog
- loading the complete journal
- caching every directory
- caching every object

Iterators and indexes operate page by page.

The Core Scale-1 prototype uses one structurally shared COW B+ tree node
format for object maps, directories, extent maps, and the allocation-region
root. Typed adapters keep their semantic records distinct. Lookup reuses a
single raw block buffer plus one decoded node and discards ancestors; full
cycle, separator, range, and subtree-count validation remains checker work.
Persistent leaf-neighbor pointers are intentionally absent because updating a
neighbor under COW would expand an otherwise local mutation. Iteration uses a
bounded path stack instead.

Multi-operation transactions keep one final staged image per modified tree
LBA. A committed node is copied and retired on first touch; subsequent touches
reuse the transaction-local image, and splits allocate only the additional
nodes. This bounds device reads independently of the number of operations that
hit the same path and avoids writing intermediate tree states.

The first executable mutation overlay retains all dirty nodes in memory. It is
therefore not the tiny-cache implementation yet. A constrained implementation
must be able to spill those still-unreachable staged images to their allocated
blocks, reload them on demand, and keep only the active path/split peer in its
2/4/8-page cache; the on-disk tree semantics do not change.

Deletion defers staging the changed node until its parent has rebalanced it
with an adjacent sibling. The pair is merged when its combined encoding fits,
or redistributed into two balanced nodes otherwise. This prevents transient
empty leaves or one-child internal nodes from becoming on-disk states. Only a
one-child root may collapse directly to its child.

The checkpoint object-map pointer was the first authoritative root migrated
to this engine. Typed leaves encode `object_id` as an eight-byte big-endian
key and the object-record LBA as an eight-byte little-endian value. Ordinary
mount looks up only the root object; the checker performs exhaustive traversal.

Newly formatted directories use the same engine with comparison-key leaves
whose typed values preserve the original UTF-8 name, child object ID, and type
hint. Ordinary mount validates the directory root only, lookup descends one
path, and enumeration/checking walks the tree. Root create/delete publishes
the directory mutation atomically with the object-map mutation. An executable
300-entry volume test crosses the removed legacy one-page limit, runs the
checker, remounts, and verifies enumeration and lookup.

The authoritative allocation-region root is also AFST-backed, using a
permanently allocated triple-version node pool to avoid allocator
self-reference (ADR-035). Its current prototype update still loads and
rewrites all region records; dirty-path-only mutation remains scale work.

Allocation regions are specifically intended to make free-space operations bounded.

The executable allocator prototype follows this rule: normal mount reads no
region descriptors or bitmap pages, and a transaction loads them on demand.
A clean page
that does not satisfy an allocation scan is evicted immediately; dirty pages
remain pinned through checkpoint commit. The exhaustive checker may choose to
load every page because its operation is explicitly whole-volume.

This rule defines the minimum implementation path needed for classic and
constrained profiles. It does not forbid a modern implementation from using
all available RAM productively. Macaros Native and workstation-class ports
may retain many regions, directories, and objects in cache, run prefetch in
parallel, and enable advanced accelerators. Cache size changes performance,
not disk semantics or correctness.

## 3. Authoritative versus derived structures

Authoritative:

- superblock/root metadata
- object records
- directory indexes
- extent mappings
- allocation records
- committed journal state

Derived and rebuildable:

- global catalog
- search accelerators
- free-space summaries above the authoritative region bitmap
- performance caches

If a derived structure disagrees with authoritative metadata, the derived structure loses.

## 4. Failure containment

Optional features should fail independently where possible.

Examples:

- damaged catalog: disable catalog, mount volume
- truncated change stream: report rescan required
- missing compression provider for compressed file: fail access or mount according to feature class
- bad secondary superblock: continue from valid primary and schedule repair
- metadata checksum failure: never silently interpret corrupted bytes

## 5. No hidden writes

All mount modes define write behavior.

`NO_CHANGES` must never write media, including:

- journal replay
- dirty-bit clearing
- mount timestamps
- catalog rebuild
- free-space summary repair

This mode is required for forensics and safe recovery.
