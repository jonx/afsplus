# 26. Debugging, Observability, and Fault Injection

Status: required development architecture. Most facilities are runtime-optional and must have near-zero cost when disabled.

Filesystem development is unusually painful because a bug may corrupt the state needed to diagnose the bug. A pointer lifetime mistake, ordering mistake, allocator error, or failed flush can produce damage minutes later and far from the original cause.

AFS+ should treat diagnosability as a design requirement rather than add logging after corruption starts happening.

## 1. Goals

The development system must make it easy to answer:

- what operation was running?
- which filesystem transaction was active?
- what objects and blocks were touched?
- why did the allocator choose a specific extent?
- which metadata version referenced a block?
- what was written before and after a flush?
- what was the last valid checkpoint?
- what changed between two checkpoints?
- which cache page was evicted/reused?
- what still references a retired block?
- where did corruption first become observable?

The system must also make failures reproducible.

## 2. Every mutating operation gets identities

Assign stable runtime identifiers:

```text
operation_id
transaction_id
checkpoint_generation
```

Trace records use these IDs consistently across:

- namespace layer
- object layer
- B+ trees
- allocator
- block cache
- block I/O
- checkpoint commit
- reclamation
- catalog/change stream

This makes one logical operation traceable across subsystem boundaries.

## 3. Binary flight recorder

AFS+ should include a fixed-size in-memory ring buffer for structured trace events.

Example conceptual event:

```c
struct AfspTraceEvent {
    uint64_t sequence;
    uint64_t timestamp;
    uint64_t transaction_id;
    uint64_t object_id;
    uint64_t block;
    uint32_t task_id;
    uint16_t category;
    uint16_t event;
    uint64_t arg0;
    uint64_t arg1;
};
```

The exact structure is not frozen.

Properties:

- fixed-size records where practical
- no dynamic allocation in trace emission
- overwriting ring by default
- dropped-event counter
- category mask
- optional high-detail modes
- cheap disabled path

Categories should include:

```text
TX
CHECKPOINT
IO
ALLOC
CACHE
BTREE
OBJECT
DIRECTORY
PATH
XATTR
CATALOG
CHANGE
RECLAIM
SCRUB
REPAIR
API
ERROR
```

## 4. Live attachment

The portable core exposes a trace-sink callback.

Possible front-ends:

- AROS message port/resource
- serial/debug channel
- host pipe in test builds
- TCP/UDP transport in controlled developer environments
- file sink
- in-memory consumer

The eventual AROS tooling should support something conceptually like:

```text
afsplus-trace Work: --follow --categories TX,ALLOC,IO
```

For Macaros Native development, the same sink can be forwarded to the M5 development host so a developer can watch filesystem activity on the M1 without interacting with the target UI.

## 5. Explain API

Tracing tells us what happened. Explain APIs tell us what the current filesystem believes.

Required debug/inspection operations should include:

```text
ExplainPath(path)
ExplainObject(object_id)
ExplainBlock(block)
ExplainExtent(object_id, offset)
ExplainDirectory(directory_id)
ExplainCheckpoint(generation)
ExplainReclaim(block_or_object)
ExplainSpace(region)
ExplainFeature(feature_id)
```

Example:

```text
$ afsplus explain block 0x123456

block: 0x123456
state: RETIRED
metadata-type: extent-tree-node
owner-object: 0x9981
written-generation: 182201
last-reachable-checkpoint: 182200
reclaim-after-generation: 182202
checksum: valid
```

The tool must never need private pointer addresses or an ad-hoc debugger script to answer basic ownership questions.

## 6. Optional reverse-map index

A rebuildable reverse map makes `ExplainBlock` and targeted repair dramatically stronger.

It maps physical ranges to semantic owners:

```text
physical extent -> object/tree/metadata owner
```

The reverse map must be derived, checksummed, generation-tagged, and safely discardable.

See ADR-024.

## 7. Metadata self-description

Every independently addressable metadata block should carry enough identity to make gross corruption diagnosable:

- type/magic
- filesystem UUID binding
- owner/object where meaningful
- physical/logical identity where meaningful
- generation
- checksum

XFS demonstrates how useful owner, UUID, physical-address, generation/log-sequence, and checksum fields become for online verification and repair.

AFS+ should design this in before disk-format freeze.

## 8. Deterministic test mode

Provide a mode intended only for testing where nondeterminism is minimized.

Possible controls:

- fixed allocator start region
- fixed pseudo-random seed
- synchronous reclamation or explicitly stepped reclamation
- deterministic timestamps supplied by test clock
- single-thread scheduling mode
- stable object-ID allocation
- deterministic cache eviction policy

Given the same starting image and operation trace, the resulting image should be byte-identical where the selected mode makes that practical.

This makes regressions and fuzz failures dramatically easier to reproduce.

Production mode is not required to be deterministic.

## 9. Fault-injection API

The block provider and allocator expose named fault points.

Examples:

```text
fail read number N
fail write number N
tear write number N after K sectors
fail flush number N
return ENOSPC at allocation number N
fail metadata page allocation
force cache eviction before event X
corrupt checksum after write
crash immediately before checkpoint
crash immediately after checkpoint
crash during reclaim batch N
```

Fault points must be stable enough that a test can say:

```text
crash-after-event=CHECKPOINT_WRITE_BEGIN
```

rather than depending only on "the 432nd write", which changes whenever unrelated code changes.

## 10. Power-cut simulator

For host-image tests, every block operation passes through a simulation backend capable of maintaining:

- durable device image
- volatile write-cache state
- pending reordered writes
- sector-granular torn writes

A simulated power loss discards volatile state according to the selected storage model.

Models should include at least:

- strongly ordered simple device
- volatile write cache honoring flush
- volatile write cache with configurable reordering
- intentionally broken flush backend for negative testing

This lets the checkpoint contract be proven rather than assumed.

## 11. Operation record/replay

Tests should be able to record semantic operations:

```text
CREATE dir/file
WRITE object offset length hash
RENAME A B
FSYNC object
DELETE path
```

A failing sequence can then be replayed against:

- old implementation
- new implementation
- different cache size
- different fault point
- different transaction engine prototype

This is more robust than trying to reproduce a bug from a raw disk image alone.

## 12. Semantic block trace

A second optional trace layer records block I/O plus semantic meaning:

```text
WRITE block 0x123 type=DIR_NODE owner=42 tx=77
FLUSH tx=77
WRITE checkpoint=B generation=901
```

This permits automated analysis of write amplification and commit ordering.

It is inspired by semantic block-level analysis techniques, but AFS+ has the advantage that it can emit the semantic labels directly rather than infer them afterward.

## 13. Invariant levels

Validation should have selectable cost levels.

### ALWAYS

Very cheap checks in production:

- block bounds
- obvious type/magic mismatch
- checksum verification when metadata is read
- checked arithmetic

### DEBUG

More expensive local checks:

- tree ordering on modified pages
- extent overlap checks
- cache handle generation checks
- allocation-state assertions

### PARANOID

Development/testing:

- walk changed subtree after each transaction
- validate affected allocation region
- cross-check reverse-map ownership
- confirm catalog deltas
- verify retired blocks are not allocatable

### FULL

Complete filesystem invariant scan.

The test harness can run PARANOID/FULL frequently without imposing that cost on normal users.

## 14. Tiny-cache mode

Many cache bugs only appear under memory pressure.

Tests must deliberately run with absurdly small metadata caches:

```text
2 pages
4 pages
8 pages
```

along with forced eviction on nested operations.

This is a direct lesson from recent PFS3 bugs where retained pointers could outlive the cache page backing them.

## 15. Shadow verification

In host tests, after a mutating transaction commits, an independent read-only volume instance can reopen the image and validate the committed state.

This catches bugs hidden by the writer's in-memory cache.

The checker should not trust in-memory objects produced by the code it is checking.

## 16. Previous checkpoint access

If the checkpoint architecture retains the previous committed generation, development/recovery tooling should be able to inspect it read-only.

Example:

```text
afsplus-checkpoint --list
afsplus-checkpoint --mount-generation 182200 readonly
```

This can answer "what changed in the commit that broke the filesystem?" without needing a separate disk snapshot system.

Normal production retention policy may be minimal.

## 17. Structured health/event stream

Errors and degraded states should be queryable, not just printed.

Example events:

```text
CHECKPOINT_FALLBACK
METADATA_CHECKSUM_ERROR
CATALOG_STALE
REVERSE_MAP_STALE
RECLAIM_BACKLOG_HIGH
DEVICE_FLUSH_FAILED
REGION_FREECOUNT_MISMATCH
CACHE_PIN_VIOLATION
```

The API exposes current health plus an event stream.

## 18. Debug features must not become disk-format dependencies

Most of this chapter is implementation tooling, not filesystem format.

Do not write debug logs into normal AFS+ metadata merely because they are convenient.

On-disk additions require a genuine recovery/maintenance need and normal feature negotiation.

The filesystem must remain mountable without any debug transport or trace consumer.

## 19. Release gate

Before epoch 1 freeze, the development environment should make this workflow normal:

```text
create image
run operation sequence
inject fault
simulate power cut
reopen NO_CHANGES
select checkpoint
validate invariants
explain unexpected blocks/objects
replay exact operation trace
```

If diagnosing corruption still requires manually opening a hex editor and guessing what a block means, the observability architecture is not finished.