# AFS+ Specification and Engineering Plan

AFS+ is a proposed modern native filesystem for AROS and the wider Amiga-family ecosystem.

> **AFS+ is not being created to rebuild ext4, NTFS, or another conventional filesystem with Amiga branding. Its strongest contract is to expose directly to modern software the filesystem primitives that applications are otherwise forced to reconstruct, approximate, or continuously rediscover above the filesystem.**

This repository is a design specification, implementation plan, compatibility contract, and conformance plan. It is intentionally written so that a developer who has not participated in the design discussions can implement individual components without needing undocumented context.

## Founding developer contract

AFS+ treats the filesystem as an active platform service, not merely a byte store with paths.

Where a recurring application problem can be solved more correctly, cheaply, and generically by the filesystem, AFS+ should expose a stable semantic primitive rather than force every application to rebuild the same mechanism with directory scans, temporary files, private databases, fragile watchers, repeated hashing, or filesystem-specific ioctls.

Examples include:

- `AtomicBatch()` for publishing bounded sets of namespace changes atomically
- `CloneFile()` and `CloneRange()` for cheap reflink copies
- `EnumerateObjects()` for efficient whole-volume object discovery
- `GetChangesSince()` for persistent incremental change tracking
- `StatBatch()` and other bulk metadata operations
- stable object IDs and content generations
- race-free access to a specific committed content generation
- `SealContent()` for immutable finalized content
- access-intent and preallocation hints for streaming, mmap, large files, and temporary data
- first-class security/content-inspection feeds that avoid repeated rescans of unchanged files
- `ExplainObject()`, `ExplainBlock()`, health reporting, and structured management APIs

These primitives must remain filesystem-neutral at the public API layer. Applications should not need to understand AFS+ block layouts to benefit from them, and other filesystem handlers should be able to implement equivalent capabilities or report that they are unsupported.

This developer contract is a design filter. A feature does not belong in AFS+ merely because another filesystem has it. It belongs when it solves a real workload or developer problem with acceptable complexity, resource cost, portability, and recovery semantics.

## Why AFS+

Classic AFS/FFS is deeply integrated into the Amiga model, but its on-disk format carries 32-bit era limits and assumptions. exFAT is excellent for interchange, but it is not a good place to grow AROS-native semantics, crash recovery, object identity, richer metadata, and high-performance indexing.

AFS+ therefore has four goals:

1. Preserve the good AROS ideas: volumes, Assigns, protection bits, comments, easy filesystem handlers, and a lightweight system model.
2. Provide a genuinely modern storage substrate: 64-bit addressing, extents, Unicode names, scalable directories, crash consistency, metadata integrity, SSD-friendly behavior, and precise durability semantics.
3. Provide a first-class developer contract for modern software instead of forcing applications to rediscover filesystem state through expensive scans and ad-hoc conventions.
4. Remain easy to implement elsewhere: a portable core, a portable C implementation path, a tiny reader profile, public format documentation, fixed identification records, feature negotiation, conformance images, FUSE support, and no AROS path syntax in the on-disk format.

## Important architectural rule

AFS+ is not intended to become a giant monolithic filesystem that every application must understand.

The design is split into:

```text
Applications
    |
dos.library / modern filesystem API
    |
Filesystem API v2
    |
+---------+-----------+-----------+
|         |           |           |
AFS+     exFAT       FFS         future handlers
```

Existing AROS applications continue to use the existing DOS API and should normally require no recompilation. New applications can opt into modern capabilities through Filesystem API v2.

## Core design summary

AFS+ 1.0 is built around:

- little-endian explicit on-disk encoding
- 64-bit block numbers, object IDs, offsets, and file sizes
- 4 KiB default logical blocks, with format support for other powers of two
- allocation regions with bounded-size local bitmaps
- object IDs independent from paths
- extent-based files
- an extent model capable of supporting shared extents/reflinks
- B+ tree directory indexes
- UTF-8 names normalized to NFC, pending interoperability validation before epoch freeze
- configurable case-sensitive or case-insensitive namespaces
- atomic metadata transactions with explicit durability semantics
- copy-on-write metadata plus alternating checksummed checkpoints as the leading transaction design candidate
- metadata checksums
- redundant recovery/checkpoint state
- atomic rename and replacement
- bounded atomic namespace batches as a proposed developer API
- sparse files
- TRIM/discard support through the storage layer
- stable object identity, content generations, and modern file notifications
- an optional derived global catalog for extremely fast full-volume enumeration
- an optional persistent change stream for incremental indexing, backup, security, and developer tooling
- portable ACL/security semantics that can survive movement between operating systems
- first-class observability, deterministic fault injection, replay, and explain APIs for development and repair
- feature flags and compatibility profiles for long-term evolution
- a Rust reference implementation path plus a portable C implementation path, with the specification remaining authoritative over either implementation

## Amiga-native design ancestry

AFS+ explicitly studies PFS3 and the proposed PFS4 design before freezing epoch 1. PFS3 demonstrated unusually strong crash resilience, speed, fragmentation behavior, and low-resource operation on classic Amiga hardware. PFS4's published design ideas, including B+ tree directories, redesigned atomic commit, small-file grouping, and improved fragmentation prevention, overlap with several AFS+ goals and must be evaluated rather than rediscovered.

See `docs/22-pfs3-and-pfs4-lessons.md`, `docs/23-pfs3-stage0-review.md`, and `adr/ADR-019-pfs3-design-reference.md`.

## Design choices intended to reduce future maintenance cost

### Allocation regions

A single enormous global free-space structure scales badly and makes low-memory implementations unattractive. AFS+ divides the volume into allocation regions. Each region owns a compact allocation bitmap and summary.

With a 1 GiB region and 4 KiB blocks, the region bitmap is only 32 KiB. A constrained implementation can operate on one region at a time.

### Compatibility profiles

Feature flags alone answer whether a volume is mountable. Profiles answer a more practical question: what set of features should be enabled if a volume must remain usable by a bootloader, classic machine, recovery tool, or portable implementation?

Examples:

- `reader-minimal`
- `classic-rw`
- `boot-safe`
- `workstation`
- `full`

### Forensic no-write mount

A normal read-only mount can still modify media in some filesystem designs, for example during recovery or housekeeping. AFS+ defines a strict `NO_CHANGES` mount mode in which the implementation must not write a single block. This is intended for recovery, debugging, forensic access, and compatibility testing.

### Shared repair and invariant definitions

Filesystem repair logic must not become an unrelated second interpretation of the format. The Rust and portable C implementations, checker, AROS handler, host tools, and FUSE adapter should be checked against the same normative specification, conformance images, and invariant corpus.

### Rebuildable accelerators

The global catalog, change stream, reverse mapping, directory statistics, and similar accelerators improve performance or repairability but should be derived and rebuildable whenever practical. A volume must remain correct when a non-authoritative accelerator is absent, stale, unsupported, or rebuilt.

### Tiny-file optimization is measured, not assumed

Tiny files dominate source trees, Cargo metadata, package caches, editor state, and configuration directories. AFS+ reserves an extension path for tiny-file optimization but will compare inline data, packed small-file storage, and ordinary extents with strong locality before freezing an approach.

## Repository map

- `docs/` - normative architecture, workload, security, developer-contract, and behavior specifications
- `adr/` - architecture decision records
- `spec/` - machine-oriented constants and format definitions
- `api/` - draft public and developer APIs
- `implementation/` - phased implementation plan
- `testing/` - conformance, fuzzing, crash testing, workload qualification, and benchmarks
- `tools/` - official tooling requirements
- `profiles/` - compatibility profile definitions
- `examples/` - example records and usage flows
- `references/` - external design references and provenance notes

## Status

This is a development specification. Fields marked `TBD`, Proposed, or otherwise unfrozen are not format commitments. Once implementation begins, incompatible format changes must update the format epoch or be represented through feature negotiation as defined in the compatibility specification.
