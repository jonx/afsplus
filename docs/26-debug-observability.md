# 26. Debugging, Observability, and Fault Injection

> **ADRs:** [ADR-024](../adr/ADR-024-rebuildable-reverse-map.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Most facilities are runtime-optional and must have near-zero cost when disabled.

Filesystem development is unusually painful because a bug may corrupt the state needed to diagnose the bug. A pointer lifetime mistake, ordering mistake, allocator error, or failed flush can produce damage minutes later and far from the original cause.

AFS+ should treat diagnosability as a design requirement rather than add logging after corruption starts happening.

<!-- toc -->

- [1. Goals](#1-goals)
- [2. Every mutating operation gets identities](#2-every-mutating-operation-gets-identities)
- [3. Binary flight recorder](#3-binary-flight-recorder)
- [4. Live attachment](#4-live-attachment)
  - [4.1 Virtual drive activity LED](#41-virtual-drive-activity-led)
- [5. Explain API](#5-explain-api)
- [6. Optional reverse-map index](#6-optional-reverse-map-index)
- [7. Metadata self-description](#7-metadata-self-description)
- [8. Deterministic test mode](#8-deterministic-test-mode)
- [9. Fault-injection API](#9-fault-injection-api)
- [10. Power-cut simulator](#10-power-cut-simulator)
- [11. Operation record/replay](#11-operation-recordreplay)
- [12. Semantic block trace](#12-semantic-block-trace)
- [13. Invariant levels](#13-invariant-levels)
  - [ALWAYS](#always)
  - [DEBUG](#debug)
  - [PARANOID](#paranoid)
  - [FULL](#full)
- [14. Tiny-cache mode](#14-tiny-cache-mode)
- [15. Shadow verification](#15-shadow-verification)
- [16. Previous checkpoint access](#16-previous-checkpoint-access)
- [17. Structured health/event stream](#17-structured-healthevent-stream)
- [18. Debug features must not become disk-format dependencies](#18-debug-features-must-not-become-disk-format-dependencies)
- [19. Release gate](#19-release-gate)

<!-- /toc -->

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

The optional Rust [core API span contract](../testing/developer-harness.md#core-api-call-spans)
defines nested call identities, commit correlation, outcome semantics and bounded
storage. The [deferred-window contract](../testing/developer-harness.md#deferred-window-observation)
joins staged calls, intent groups and checkpoint attempts. Object and platform
scopes follow their own integration requirements.

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
The [runtime adapter contract](../testing/developer-harness.md#category-selection-and-live-diagnostics)
defines category admission and separate local-loss/live-delivery counters. A
trusted bounded callback hands events to a consumer queue; arbitrary consumer
processing runs outside the filesystem operation.

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

### 4.1 Virtual drive activity LED

AFS+ also exposes a much smaller, optional block-activity sink for front-ends
that want to reproduce the Amiga drive LED. It emits `BEGIN` and `END` around
`READ`, `WRITE`, and `FLUSH`, including the LBA and block count when those
fields apply. A failed operation still gets an `END` event marked unsuccessful.

This facility is deliberately not implemented by enabling the full trace
stream. In the Rust prototype, `ActivityBackend` is an opt-in generic wrapper
around `BlockDevice`. If it is not installed, the normal I/O path contains no
test, callback, timestamp read, allocation, or copied payload. If it is
installed, emission is a synchronous, fixed-size callback; an operation mask
filters unwanted classes before events are built. The receiver must stay fast
and must not re-enter the same block device.

The Macaros virtual write LED should treat both `WRITE` and `FLUSH` as write
activity. The UI may keep the light visible for a short minimum interval so
fast operations do not flicker invisibly, but that timing policy belongs to the
UI and must never introduce a sleep or delay in the filesystem. A queue or UI
adapter may timestamp and coalesce events after reception.

The contract is portable: an AROS handler, a host-image tool, another OS, or a
physical driver can expose the same event model. It changes no on-disk data and
is neither a persistent log nor part of the filesystem change stream.

The native AROS trackdisk adapter accepts the same C sink. It chooses direct or
instrumented block function pointers once at initialization, per operation, so
an absent sink and masked-out reads add no conditional to their I/O callbacks.
This lets a MacAROS front-end request only `WRITE | FLUSH` for its virtual LED.

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

`ExplainBlock` is executable as a host-side module,
[`afsplus_check::explain`](../crates/afsplus-check/src/explain.rs). It walks
the committed state with the block codecs only, shares no traversal or claim
set with the checker, and answers for one block: its allocation bit, every
role the committed state gives it (reserved slot and whether it is live,
allocation-root pool, intent-log slot, volume tree node, object record,
directory or extent node of an object, file data with its logical block and
its shared and unwritten marks, security descriptor segment with its index,
reclaim structure, or quarantined with its retire generation) and what the
block says about itself (magic, owner, generation, checksum validity). A block
that is allocated and has no role is owned by nothing the live state reaches:
a leak on an ordinary volume, a block kept by a retained view on a snapshot
volume. A branch the walk cannot decode is reported and the blocks behind it
stay unattributed, which is what the committed state proves.
[Its test](../crates/afsplus-check/tests/explain.rs) requires the explanation
of every block of populated images to agree with the checker's committed
state and leak findings, with the live bitmap and with the bytes the core
reads at the attributed offsets. The walk depends on `afsplus-format` alone:
the placement of the allocation-root pool and of the intent-log slots is
geometry ([disk layout](../spec/disk-layout.md)). `explain_object` states one object from the same walk: its record block
and fields, its comment, a symlink's target, the node count of its own tree, a
directory's entry count, the extents and the mapped, shared and unwritten
blocks of a file, the state of its security descriptor chain (length, expected
and consistent segments, format identity, divergence mark), the state of its
attribute chain with the name and value length of every attribute, and every
directory entry that names it. `explain_path` resolves a path from the root by the
spelling each directory stores and returns every component with the object it
names; the walk reads the format alone, so it applies no case folding.

`explain_extent` says where one byte offset of a file lives: past the end, in
a hole, or in an extent, with the physical block and the shared and unwritten
marks. `explain_checkpoint` states both slots (the generation each carries, or
why it cannot be selected, "never written" included) and every field of the
selected checkpoint. `explain_reclaim` summarises the queue and, for a block,
names the pending run that holds it with its retire generation and its
position. `explain_space` counts one region from the live bitmap and the roles:
reserved, allocated, free, quarantined, allocated with no live role, and the
longest free run. `explain_features` lists every feature bit this
implementation assigns and every bit the volume sets, so a bit nobody assigned
shows without an identity. `ExplainDirectory` is `explain_object` on a
directory. [Their test](../crates/afsplus-check/tests/explain_more.rs) holds
each against a witness outside the walk: the bytes the core reads at every
block boundary of three files, the core's checkpoint selection, the checker's
reclaim runs and bitmap, the identification block.

The command is `afsplus-explain [--json] <image> (block <number> | object <id>
| path <path> | extent <id> <offset> | checkpoint | reclaim [<block>] | space
<region> | feature [<id>])`, in [`afsplus-tools`](../crates/afsplus-tools/src/explain.rs).
It opens the image read-only. The JSON document carries `schema_version` 2
(ADR-025), the kind of question, the generation answered for, `has_snapshots`,
`partial` with the list of branches the walk could not decode, and the answer;
a block also carries a one-word verdict (`owned`, `free`, `leaked`,
`retained-or-leaked` on a snapshot volume, `owned-but-free`). Status 0 is an
answer from a complete walk, 1 an answer from a partial walk or a question
about something the image does not hold, 2 a usage or host I/O failure.
[Its test](../crates/afsplus-tools/tests/explain_cli.rs) reads the documents
back with an independent JSON parser.
`ExplainCheckpoint` answers for the two slots an image holds; a generation
that no slot carries any more has no answer in the image. `ExplainReclaim` by
object has none either: a reclaim entry records a run and its retire
generation, not the object that owned it.

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

The API exposes current health plus an event stream. The AROS handler raises
`CHECKPOINT_FALLBACK`, `RECLAIM_BACKLOG_HIGH` and `REGION_FREECOUNT_MISMATCH`
besides the events that a failed call already carries; they describe the
volume without failing anything, so they have no DOS error and set no
degraded-state flag ([the bridge](aros-native-bridge.md),
[C9](../implementation/stage-c-gap.md#c9-health-reporting)).

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
