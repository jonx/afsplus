# 23. Stage 0 PFS3/PFS4 Design Review

> **ADRs:** [ADR-009](../adr/ADR-009-journal.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Status: architecture review v1, completed before the first implementation milestone. Source-level study should continue when the corresponding AFS+ subsystem is implemented.

<!-- toc -->

- [1. Why PFS3 is a primary reference](#1-why-pfs3-is-a-primary-reference)
- [2. Executive decision matrix](#2-executive-decision-matrix)
- [3. Atomic commit: the most important PFS3 lesson](#3-atomic-commit-the-most-important-pfs3-lesson)
  - [3.1 Why AFS+ should not copy the exact PFS3 mechanism](#31-why-afs-should-not-copy-the-exact-pfs3-mechanism)
- [4. Retired-block quarantine](#4-retired-block-quarantine)
- [5. Deferred reclamation replaces special postponed operations](#5-deferred-reclamation-replaces-special-postponed-operations)
- [6. Allocation and fragmentation](#6-allocation-and-fragmentation)
- [7. Extents: learn from anodes, do not inherit them](#7-extents-learn-from-anodes-do-not-inherit-them)
- [8. Directory structure](#8-directory-structure)
- [9. Cache design and the dangers of raw pointers](#9-cache-design-and-the-dangers-of-raw-pointers)
- [10. Safe failure direction](#10-safe-failure-direction)
- [11. Small files](#11-small-files)
  - [A. Inline data](#a-inline-data)
  - [B. Packed small-file slabs](#b-packed-small-file-slabs)
  - [C. Ordinary extents with locality](#c-ordinary-extents-with-locality)
- [12. Online optimization instead of format-level defragmentation dependency](#12-online-optimization-instead-of-format-level-defragmentation-dependency)
- [13. Deldir and rollover files](#13-deldir-and-rollover-files)
  - [Deldir](#deldir)
  - [Rollover files](#rollover-files)
- [14. Testing lessons from current pfs3aio](#14-testing-lessons-from-current-pfs3aio)
- [15. Portable implementation lesson](#15-portable-implementation-lesson)
- [16. Consequences for the current AFS+ specification](#16-consequences-for-the-current-afs-specification)

<!-- /toc -->

## 1. Why PFS3 is a primary reference

PFS3 is not merely a legacy compatibility target. It is one of the most useful Amiga-native filesystem designs to study because it achieved atomic metadata updates, good allocation behavior, useful recovery semantics, and bounded cache operation on very constrained machines.

The current pfs3aio source is also actively maintained. Recent 2026 fixes are especially valuable because they expose subtle failure modes in atomic update, allocator, cache, and directory code that AFS+ can design out from the start.

PFS4 never became an implementation, but Michiel Pelt publicly described its intended improvements: B+ tree directories, redesigned atomic commit, grouping of small files, and improved fragmentation prevention/automatic defragmentation. Those ideas overlap directly with AFS+ goals.

## 2. Executive decision matrix

| PFS3/PFS4 idea | AFS+ decision | Reason |
|---|---|---|
| Copy changed metadata to new blocks | ADAPT | Excellent atomic-update foundation, but AFS+ will use explicit generations, checksums, and checkpoint records. |
| Root written last as commit point | ADAPT | Keep the single logical commit-point idea, but do not depend on one fixed root-cluster write being atomic. |
| Deferred freeing of old metadata blocks | ADOPT | A block reachable from any retained committed checkpoint must never be reused. |
| Prefer leaking uncertain blocks to early reuse | ADOPT | Space loss is repairable; overwriting reachable metadata is not. |
| Dedicated metadata reserve | ADAPT | Keep guaranteed metadata headroom, but avoid a fixed-size metadata partition that becomes a scaling constraint. |
| Extent-like anodes | ADAPT | Keep extents; reject linked anode chains as the primary random-access index. Use inline extents plus an extent tree. |
| Allocation bitmap and roving pointer | ADAPT | Keep bitmap efficiency and locality hints, but use allocation regions and per-region summaries. |
| Extend a file contiguously when possible | ADOPT | Cheap fragmentation prevention and good sequential I/O. |
| Fragmentation-sensitive preallocation | ADAPT | Keep adaptive preallocation, but base policy on measurements and workload hints rather than fixed historical constants. |
| B+ tree directories proposed for PFS4 | ADOPT | Directly matches AFS+ large-directory requirements. |
| Packed variable-length directory entries | ADAPT | Good density; AFS+ B+ tree leaves should use compact variable-length records with explicit bounds. |
| PFS3 LRU with a small bounded buffer pool | ADOPT | AFS+ correctness must work with a small cache and configurable memory budget. |
| Cache block locking/pinning | ADOPT | Required invariant. No pointer into an evictable page may survive an operation that can cause a cache miss unless pinned. |
| Persisted postponed operations | ADAPT | Generalize into a filesystem-wide deferred-reclamation queue instead of operation-specific fields. |
| PFS3 `.deldir` | ADAPT as policy | Useful UX, but Trash is not a mandatory core filesystem semantic. Implement above the core using stable objects and atomic namespace operations. |
| PFS3 rollover files | REJECT as core | Specialized behavior belongs in applications or an optional higher layer, not the core format. |
| PFS3 fixed index/superindex scaling layers | REJECT | They accumulated as disks grew. AFS+ uses scalable trees and 64-bit addressing from epoch 1. |
| PFS3 incremental large-file format extensions | REJECT | AFS+ starts 64-bit clean and does not reserve compatibility baggage for obsolete limits. |
| No metadata checksums | REJECT | AFS+ checksums independently addressable metadata. |
| PFS4 automatic grouping of small files | ADAPT | Benchmark inline data, packed small-file slabs, and normal extents before selecting an optional format feature. |
| PFS4 automatic defragmentation | ADAPT | Keep relocatable extents and add an online optimizer later. Do not make defragmentation a required mount-time subsystem. |
| PFS3 current two-layer test strategy | ADOPT | Portable core tests plus black-box real-handler fault injection should both exist. |

## 3. Atomic commit: the most important PFS3 lesson

PFS3 marks a metadata block dirty by allocating a new reserved block rather than overwriting the committed block. Parent metadata is updated recursively to point to the new location. During `UpdateDisk`, the new metadata blocks are written first and the root is written last. The root write is the logical commit point.

Old locations are not immediately freed. They are queued and released only after the new root has committed. Current pfs3aio goes further: if the queue cannot grow safely, it intentionally leaks the old block rather than making it allocatable while a committed root may still reference it.

The principle is excellent:

> Never reuse storage that could still be reachable from a committed recovery state.

AFS+ should adopt this as a formal invariant.

### 3.1 Why AFS+ should not copy the exact PFS3 mechanism

PFS3 writes a fixed root/root cluster as the final commit step. AFS+ should avoid depending on a multi-sector root write being physically atomic.

The current AFS+ proposal is therefore changed from "metadata redo journal is required" to a proposed checkpoint architecture:

1. write new user data required by the transaction
2. durable flush/barrier
3. write changed metadata copy-on-write, bottom-up
4. durable flush/barrier
5. write a small alternate checkpoint record containing generation, root references, allocator/reclaim roots, feature state, and checksum
6. durable flush/barrier

At mount, choose the highest-generation valid checkpoint whose referenced roots validate.

A torn new checkpoint is ignored and the previous valid checkpoint remains usable.

This must be proven with fault injection before the format is frozen.

## 4. Retired-block quarantine

PFS3's reserved-to-be-freed mechanism exposes an important general rule that AFS+ should make explicit.

When metadata or data becomes unreachable in a new transaction, the old blocks become `retired`, not immediately `free`.

A retired block is reusable only when no retained valid checkpoint can refer to it.

AFS+ should track retirement generation and reclaim safely in bounded batches. This has several advantages:

- no early reuse corruption window
- no requirement for one giant cleanup transaction
- interrupted cleanup is harmless
- recovery can conservatively leak/quarantine rather than corrupt
- old checkpoint redundancy remains meaningful

The exact retention/reclamation algorithm is an epoch-1 design item and must be tested against alternate-checkpoint failure cases.

## 5. Deferred reclamation replaces special postponed operations

PFS3 persists descriptions of certain long operations so they can be resumed after interruption. That is a strong idea, but AFS+ should generalize it.

For a huge file deletion, directory-tree removal, or large truncate:

1. make the namespace/object state change atomically
2. mark the unreachable storage as pending reclamation
3. commit quickly
4. reclaim extents and metadata in bounded background/maintenance transactions
5. persist progress so reboot resumes safely

This keeps transaction size, latency, RAM use, and journal/checkpoint pressure bounded regardless of object size.

The mechanism should be generic rather than having one hard-coded operation descriptor for each filesystem operation.

## 6. Allocation and fragmentation

PFS3 first tries to allocate directly after the current end of a file. When that is not possible it scans the bitmap from a roving position and creates another anode/extent. It also biases future allocation based on fragmentation.

AFS+ should retain the principles:

- extend the current extent first
- preserve locality
- avoid rescanning from block zero
- use preallocation where measurements show a benefit

AFS+ allocation regions remain a better modern foundation because they bound bitmap memory and repair scope.

The allocator should maintain cheap per-region hints such as:

- free block count
- largest-known run
- next-fit/roving position
- metadata/data locality hints

Hints are rebuildable. Region bitmaps are authoritative.

## 7. Extents: learn from anodes, do not inherit them

A PFS3 anode stores `blocknr + clustersize + next`, which is effectively an extent linked to another extent.

This proves that extent-based allocation fits Amiga semantics well.

AFS+ should keep:

- compact contiguous-range descriptions
- cheap sequential traversal
- easy relocation

But it should not use a linked extent chain as the only index because highly fragmented large files make random logical-offset lookup proportional to the number of extents.

AFS+ keeps the existing plan:

- a few inline extents in the object record
- an extent B+ tree for overflow

## 8. Directory structure

PFS3 repeatedly optimized its chained directory implementation for large directories. PFS4's proposed answer was a B+ tree.

AFS+ should keep B+ tree directories as the core design.

PFS3 does provide one useful density lesson: directory entries are compact variable-length records. AFS+ B+ tree leaf pages should likewise avoid fixed 255-byte name slots.

Requirements:

- bounded parsing
- packed variable-length UTF-8 names
- explicit record lengths
- checksummed pages
- no native C-struct serialization
- prefix/key compression may be considered only after measurement

## 9. Cache design and the dangers of raw pointers

PFS3 runs with a small LRU metadata cache. This is a major positive reference for AFS+ low-resource goals.

Recent pfs3aio fixes also show a serious class of bug: code retained pointers to cache blocks while nested operations could trigger a cache miss and evict/retype those blocks. In allocator and update paths this could lead to double allocation or freeing live storage.

AFS+ must make this hard to express.

Proposed implementation rule:

- cached pages are referenced through explicit handles
- a handle must be pinned while a raw view into the page is used across any call that may perform I/O/cache allocation
- debug builds track pin counts and generation tokens
- eviction changes a page generation so stale handles fail assertions
- tests run the core with deliberately tiny caches to maximize eviction pressure

The filesystem must be correct with a minimal cache. Larger caches only improve performance.

## 10. Safe failure direction

Several current PFS3 fixes reinforce a design rule that should be explicit in AFS+:

When allocator metadata is unreadable or ownership is uncertain, treat storage as allocated/quarantined, not free.

Examples:

- unreadable free-space page: do not allocate from it
- deferred-free memory exhaustion: leak/quarantine instead of early free
- failed metadata cleanup: keep the volume dirty and retry

`afsplus-check` can later recover leaked/quarantined capacity after proving it is unreachable.

This direction trades space for integrity.

## 11. Small files

PFS4 proposed grouping small files automatically. Modern AFS+ workloads make this even more relevant because Git, Cargo, editors, package managers, and language tools create large numbers of tiny files.

Do not prematurely select one mechanism.

The prototype benchmark must compare:

### A. Inline data

Store tiny payload directly in the object record.

Pros: one metadata read, simple lookup, no separate allocation.

Cons: object-page churn, object growth, COW amplification.

### B. Packed small-file slabs

Pack payloads from multiple tiny files into dedicated blocks/slabs.

Pros: space density, fewer allocated blocks.

Cons: unrelated files share failure/update units, more complex reclaim/compaction, potentially higher write amplification.

### C. Ordinary extents with locality

Keep the core representation, allocate tiny files close together.

Pros: simplest recovery and implementation.

Cons: block-size overhead.

The optional feature is enabled only if measured benefits justify the complexity.

## 12. Online optimization instead of format-level defragmentation dependency

PFS4 proposed built-in automatic defragmentation.

AFS+ should make extents relocatable so an online optimizer can be added without a format redesign:

1. allocate replacement extent
2. copy data
3. verify data where appropriate
4. atomically switch extent mapping
5. retire old extent
6. reclaim it after checkpoint safety allows

This mechanism can also support future tiering or block relocation.

Normal allocator quality remains more important than needing frequent defragmentation.

## 13. Deldir and rollover files

### Deldir

The recovery UX is useful, but the filesystem should not force every volume to retain deleted data.

Implement Trash as policy using ordinary AFS+ objects/namespace transactions, possibly with an `aros.trash` attribute or designated hidden directory.

### Rollover files

PFS3 rollover files are clever but specialized. AFS+ does not include them in the core format. They can be implemented by an application/library if a real modern use case appears.

## 14. Testing lessons from current pfs3aio

Current pfs3aio has two complementary test layers:

1. host-compiled tests that exercise real source functions with mocks
2. black-box execution of the real m68k handler through AmiFUSE with fault/power-cut injection

AFS+ should go one step further because `libafsplus` is portable by design:

- unit/property/fuzz tests run directly against the same portable core used in production
- a separate black-box AROS handler suite verifies DOS integration
- the block backend supports deterministic fail-after-N-write, torn write, read error, flush failure, and crash injection

The following regression classes are mandatory from day one because PFS3 has demonstrated that they can be catastrophic:

- cache eviction while allocator/update code holds metadata references
- rename failure at every allocation step
- deferred-free queue/resource exhaustion
- unreadable bitmap/free-space metadata
- block-size/geometry arithmetic overflow
- failed cleanup after the primary commit
- torn/failed checkpoint write

## 15. Portable implementation lesson

The 2026 `libpfs3` Rust crate provides a pure Rust, OS-independent PFS3 reader/writer/formatter/checker. It reinforces the AFS+ decision to make the canonical filesystem core portable and independent from DOS packet handling.

AFS+ should preserve the stronger architecture already proposed:

```text
libafsplus
  + host-file backend
  + AROS block backend
  + FUSE frontend
  + checker/repair
  + formatter
  + fuzz harness
```

No tool should need a second independent filesystem parser.

## 16. Consequences for the current AFS+ specification

This review changes the following assumptions:

1. `ADR-009` is reopened. Metadata journaling is no longer a predetermined solution.
2. A COW checkpoint transaction engine becomes the leading design candidate.
3. Retired blocks need explicit generation/quarantine semantics.
4. Large deletion/truncation cleanup becomes deferred and resumable.
5. Cache pinning becomes an implementation invariant.
6. Small-file storage remains optional and mechanism-TBD until benchmarks exist.
7. Crash testing must cover the actual failure classes seen in PFS3, not only generic write interruption.

The design remains subject to comparison against other modern filesystems before epoch 1 is frozen.
