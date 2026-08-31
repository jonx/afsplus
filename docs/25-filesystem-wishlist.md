# 25. What People Actually Want From a Filesystem

> **ADRs:** [ADR-027](../adr/ADR-027-reflink-clones.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Status: product/design exploration. Nothing in this document is automatically a 1.0 requirement unless promoted by an ADR.

A new filesystem is rare. That makes it worth asking a different question from "which features do existing filesystems have?":

> Which recurring annoyances could we remove because we are designing the filesystem, API, tools, and debugging model together from the beginning?

The rule is simple: a feature belongs in AFS+ only when it solves a real user, application, maintenance, or development problem at acceptable complexity.

<!-- toc -->

- [1. Instant answers about the namespace](#1-instant-answers-about-the-namespace)
  - [Problem](#problem)
  - [AFS+ direction](#afs-direction)
- [2. Instant directory size without crawling](#2-instant-directory-size-without-crawling)
  - [Problem](#problem-1)
  - [Proposal: derived directory aggregate index](#proposal-derived-directory-aggregate-index)
- [3. A real persistent change API](#3-a-real-persistent-change-api)
  - [Problem](#problem-2)
  - [AFS+ direction](#afs-direction-1)
- [4. Explainable storage](#4-explainable-storage)
  - [Problem](#problem-3)
  - [AFS+ direction](#afs-direction-2)
- [5. Structured tools instead of screen scraping](#5-structured-tools-instead-of-screen-scraping)
  - [Problem](#problem-4)
  - [AFS+ rule](#afs-rule)
- [6. Targeted online repair instead of "fsck the universe"](#6-targeted-online-repair-instead-of-fsck-the-universe)
  - [Problem](#problem-5)
  - [AFS+ direction](#afs-direction-3)
- [7. Safe rollback for humans, not only administrators](#7-safe-rollback-for-humans-not-only-administrators)
- [8. Atomic publication of more than one filename](#8-atomic-publication-of-more-than-one-filename)
  - [Problem](#problem-6)
  - [Proposal: bounded atomic namespace batches](#proposal-bounded-atomic-namespace-batches)
- [9. Cheap independent copies as a baseline capability](#9-cheap-independent-copies-as-a-baseline-capability)
  - [Problem](#problem-7)
  - [AFS+ direction](#afs-direction-4)
- [10. Integrity policy that can vary by workload](#10-integrity-policy-that-can-vary-by-workload)
- [11. Content identity without requiring applications to hash everything repeatedly](#11-content-identity-without-requiring-applications-to-hash-everything-repeatedly)
- [12. Per-directory policy instead of one volume-wide compromise](#12-per-directory-policy-instead-of-one-volume-wide-compromise)
- [13. First-class health information](#13-first-class-health-information)
- [14. Virtual disk images that are easy to branch and inspect](#14-virtual-disk-images-that-are-easy-to-branch-and-inspect)
- [15. Things we deliberately do not promise](#15-things-we-deliberately-do-not-promise)
- [16. Product differentiation target](#16-product-differentiation-target)

<!-- /toc -->

## 1. Instant answers about the namespace

### Problem

Many applications repeatedly crawl entire trees just to answer questions the filesystem already knows indirectly:

- what files exist?
- what changed since yesterday?
- how many files are under this directory?
- how much logical/physical space does this directory use?
- what owns physical block X?

This wastes I/O, CPU, battery, and developer time.

### AFS+ direction

Provide semantic indexes as optional rebuildable accelerators:

- global object catalog
- persistent change stream
- reverse physical-to-owner map
- recursive directory statistics

Applications use stable APIs and never parse AFS+ metadata directly.

Proposed operations:

```text
EnumerateObjects()
GetChangesSince(sequence)
GetRecursiveDirectoryStats(object_id)
ExplainBlock(block)
```

Fallback always exists when the accelerator is absent.

## 2. Instant directory size without crawling

### Problem

"How large is this folder?" is still surprisingly expensive on many filesystems because the answer requires visiting every descendant.

APFS explicitly advertises fast directory sizing, which demonstrates that this can be a filesystem capability rather than a GUI problem.

### Proposal: derived directory aggregate index

Optional feature:

```text
org.aros.afsplus:dir-stats
```

Per-directory derived values may include:

- descendant file count
- descendant directory count
- logical bytes
- allocated bytes

Updates can propagate along the ancestor chain, which is O(path depth), not O(number of descendants).

The index is derived and generation-tagged. If stale or unsupported, callers fall back to traversal.

## 3. A real persistent change API

### Problem

Transient file notifications only work while an application is listening.

Backup tools, indexers, editors, sync engines, antivirus software, and search tools often need to answer:

> What changed since sequence N, including while I was not running?

NTFS USN demonstrates how useful this is, and Linux filesystem discussions have repeatedly asked for a comparable persistent facility.

### AFS+ direction

The change stream is already part of the design.

This should become one of AFS+'s defining public capabilities rather than an internal implementation artifact.

## 4. Explainable storage

### Problem

When a filesystem behaves badly, users and developers often cannot answer basic questions without specialist tools:

- Why is this file fragmented?
- Why did this allocation go there?
- Which file owns this block?
- What checkpoint introduced this mapping?
- Why is 40 GB marked used but not visible?
- Why can this metadata block not be reclaimed yet?

### AFS+ direction

Provide supported introspection APIs and tools:

```text
afsplus explain path Work:src/foo.rs
afsplus explain object 0x1234
afsplus explain block 0x998877
afsplus explain space
afsplus explain checkpoint
afsplus explain reclaim
```

This is not a debug-only idea. Safe read-only explanation is also useful to administrators and repair tools.

## 5. Structured tools instead of screen scraping

### Problem

Filesystem management tools traditionally emit human-readable text with inconsistent syntax. Automation tools then parse text output, which is fragile.

This problem has been explicitly raised by Linux filesystem/tool developers for mkfs, fsck, resize, snapshots, and related operations.

### AFS+ rule

Every official tool has a stable structured mode from its first release.

Example:

```text
afsplus-info --json
afsplus-check --json
afsplus-resize --json-progress
afsplus-catalog --json
```

Long term, tools should be thin clients over a reusable management API rather than the API being their stdout format.

## 6. Targeted online repair instead of "fsck the universe"

### Problem

Traditional recovery often treats the filesystem as one giant object. XFS's modern online-repair work shows the value of sharding, self-describing metadata, reverse mappings, and rebuilding individual damaged structures while the rest of the filesystem stays available.

### AFS+ direction

Design metadata so that a checker can answer:

- what type of block is this?
- which filesystem UUID owns it?
- which object/structure owns it?
- where should it physically be?
- which generation wrote it?

Then permit targeted verification/rebuild of:

- one directory tree
- one extent tree
- one allocation region
- one catalog generation
- one reverse-map region

A full offline check remains available, but it should not be the only repair model.

## 7. Safe rollback for humans, not only administrators

Snapshots are not unique anymore, but they solve a very human problem: "I overwrote or deleted the wrong thing."

AFS+ COW checkpoints may make a lightweight recovery/history feature relatively natural later.

Possible future directions:

- named volume snapshots
- short automatic checkpoint retention
- Trash implemented as normal namespace policy
- read-only access to the immediately previous valid checkpoint in recovery mode

Do not make snapshot retention a 1.0 requirement unless the checkpoint architecture proves it cheap enough.

## 8. Atomic publication of more than one filename

### Problem

Applications often need to update a group of related files consistently:

```text
config
config.index
config.signature
```

Today they use temporary files, fsync, and a carefully ordered series of renames. This is error-prone and platform-specific.

### Proposal: bounded atomic namespace batches

A future Filesystem API v2 extension could provide a small transaction containing namespace/metadata operations:

```text
BeginAtomicBatch()
Replace(A.tmp, A)
Replace(B.tmp, B)
Rename(C, D)
CommitAtomicBatch()
```

Constraints:

- bounded operation count and metadata size
- same filesystem only
- not an arbitrary database transaction
- large file data must already be written/durable before publication

Potential users:

- package managers
- editors
- configuration systems
- build tools
- application databases that publish file sets

This must be benchmarked and kept optional at API level.

## 9. Cheap independent copies as a baseline capability

### Problem

Users and applications frequently want a new independent copy of a large file, not a symlink and not a hard link.

A traditional physical copy reads and rewrites every byte even when source and destination are on the same filesystem. That wastes time, flash endurance, bandwidth, energy, and cache capacity.

### AFS+ direction

Reflink/block cloning is promoted by ADR-027 into the epoch-1 extent-model requirements.

The disk format must permit multiple file objects to reference the same physical data extents safely.

Filesystem API v2 will expose semantic operations equivalent to:

```text
CloneFile(source, destination)
CloneRange(source, source_offset, destination, destination_offset, length)
```

The destination is a distinct object. Future writes are independent through copy-on-write.

Example:

```text
A -> X Y Z
B -> X Y Z

B modifies Y

A -> X Y  Z
B -> X Y' Z
```

This differs from:

- move/rename: same object, namespace metadata only
- hard link: same object through multiple names
- symlink: separate link object referring to another path/target
- physical copy: separate object and separate data from the start

AFS+ should make these distinctions explicit in the developer API rather than relying on applications to guess which cheap-copy primitive exists.

Potential users include:

- package/build caches
- VM and disk-image workflows
- editor temporary copies
- backup staging
- large media/project files
- test environments

`CloneTree()` remains a separate future design because directory/object identity and change-stream semantics make it significantly more complex than file/range cloning.

## 10. Integrity policy that can vary by workload

ReFS demonstrates a useful idea: data integrity checksums can be enabled selectively rather than forcing the same policy on every file.

AFS+ already requires metadata checksums.

Future optional data-integrity policy could be inherited from directories:

```text
source code       checksum data = yes
large cache       checksum data = no
important archive checksum data = yes
```

This is different from a content hash and should not be conflated with deduplication.

## 11. Content identity without requiring applications to hash everything repeatedly

Build systems, indexers, backup tools, and sync engines often recalculate hashes after checking mtime/size.

A future derived content-fingerprint cache could expose a filesystem-maintained hash associated with a specific object data-generation.

Requirements:

- optional
- derived/rebuildable
- clearly identifies hash algorithm
- never substitutes for data-integrity checksums unless explicitly designed to
- invalidated exactly when file contents change

This could materially benefit Ferail, build systems, and backup tools, but needs measurement before adoption.

## 12. Per-directory policy instead of one volume-wide compromise

Useful policies may include:

- case sensitivity
- compression provider
- data checksum policy
- tiny-file packing policy
- indexing/catalog inclusion

Inheritance from parent directories is often more useful than a volume-wide switch.

AFS+ should only permit policies whose semantics remain understandable to minimal readers and external implementations.

## 13. First-class health information

A mounted filesystem should expose structured health state rather than only printing an error once.

Examples:

```text
healthy
metadata_corruption_detected
catalog_stale
change_stream_near_retention_limit
pending_reclaim_large
checkpoint_fallback_used
device_flush_unreliable
allocation_region_degraded
```

Health should be queryable and observable through Filesystem API v2 and tools.

## 14. Virtual disk images that are easy to branch and inspect

Filesystem developers and OS developers repeatedly need disposable disks, snapshots of failing state, and huge-capacity test media without provisioning real hardware.

AFS+ development should treat a sparse raw image as a first-class real volume, then layer virtual block backends around it:

```text
FileBackend
SliceBackend
OverlayBackend
TraceBackend
FaultBackend
PowerCutBackend
```

A writable overlay should make a new test branch essentially instant while preserving the immutable base image.

Read-only checkpoint viewports should permit inspection of retained previous generations without pretending AFS+ already has a full user snapshot product.

See `docs/28-virtual-images-and-viewports.md`.

## 15. Things we deliberately do not promise

A new filesystem is not an excuse to embed every storage technology.

AFS+ should not add without a proven requirement:

- semantic/vector search in core filesystem metadata
- cloud synchronization protocol
- Git-like version control
- distributed consensus
- built-in RAID
- mandatory deduplication
- arbitrary database transactions

Those are better built above or below the filesystem unless a concrete AROS use case proves otherwise.

## 16. Product differentiation target

If AFS+ succeeds, its unusual strength should be this combination:

```text
Amiga simplicity
+
PFS3-style resource discipline
+
modern COW/integrity
+
cheap reflink clones
+
NTFS-like enumeration/change intelligence
+
XFS-like repairability
+
portable Rust reference implementation with C interoperability
+
developer-first observability and virtual-image tooling
```

That is a stronger reason to create AFS+ than simply saying "AROS needs files larger than 4 GB."
