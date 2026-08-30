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
- generation-stable read handles where the requested committed data generation is still retained by the filesystem's versioning policy
- `SealContent()` for immutable finalized content
- access-intent and preallocation hints for streaming, mmap, large files, and temporary data
- first-class security/content-inspection feeds that avoid repeated rescans of unchanged files
- `ExplainObject()`, `ExplainBlock()`, health reporting, and structured management APIs

These primitives must remain filesystem-neutral at the public API layer. Applications should not need to understand AFS+ block layouts to benefit from them, and other filesystem handlers should be able to implement equivalent capabilities or report that they are unsupported.

This developer contract is a design filter. A feature does not belong in AFS+ merely because another filesystem has it. It belongs when it solves a real workload or developer problem with acceptable complexity, resource cost, portability, and recovery semantics.

Just as importantly, an experimental feature is not a lifetime commitment merely because it once appeared in the design. Feature identities are permanent and never reused, but implementations may deprecate or retire unused features. Existing active volumes remain readable through retained support or explicit migration; new volumes need not keep enabling a feature that proved unnecessary.

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
- allocation regions with bounded-size local free-space state
- object IDs independent from paths
- extent-based files
- an extent model capable of supporting shared extents/reflinks
- B+ tree directory indexes
- valid UTF-8 names whose original bytes are preserved, with lookup through a versioned normalized comparison key
- binary B+ tree key ordering, never host-locale collation
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
- an optional non-authoritative, rebuildable global catalog for extremely fast full-volume enumeration
- an optional non-authoritative, discardable but non-reconstructible persistent change stream for incremental indexing, backup, security, and developer tooling
- portable ACL/security semantics under active prototype review
- first-class observability, deterministic fault injection, replay, and explain APIs for development and repair
- feature flags, lifecycle metadata, and compatibility profiles for long-term evolution and safe feature retirement
- a Rust reference implementation path plus a portable C implementation path, with the specification remaining authoritative over either implementation

## Amiga-native design ancestry

AFS+ explicitly studies PFS3 and the proposed PFS4 design before freezing epoch 1. PFS3 demonstrated unusually strong crash resilience, speed, fragmentation behavior, and low-resource operation on classic Amiga hardware. PFS4's published design ideas, including B+ tree directories, redesigned atomic commit, small-file grouping, and improved fragmentation prevention, overlap with several AFS+ goals and must be evaluated rather than rediscovered.

See `docs/22-pfs3-and-pfs4-lessons.md`, `docs/23-pfs3-stage0-review.md`, and `adr/ADR-019-pfs3-design-reference.md`.

## Design choices intended to reduce future maintenance cost

### Allocation regions

A single enormous global free-space structure scales badly and makes low-memory implementations unattractive. AFS+ divides the volume into allocation regions so allocation/repair work can remain bounded. The current executable candidate uses independently checksummed bitmap pages selected by triple-buffered region descriptors; it supports the proposed 1 GiB region while loading pages on demand. This remains an experimental format decision until the Stage B1 measurements and crash semantics are accepted.

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

### Optional accelerators have explicit failure semantics

Optional structures are classified by what may safely happen to them:

- the catalog is non-authoritative and rebuildable
- the change stream is non-authoritative and discardable, but lost history is not reconstructible and forces `RESCAN_REQUIRED`
- future reverse maps/directory statistics should be rebuildable whenever practical

The feature registry records these properties separately rather than hiding them behind one `derived` flag.

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

This repository now includes an executable Rust prototype, checker, portable
VFS API, FUSE protocol adapter, packet-neutral AROS DOS adapter, a versioned
AROS C/staticlib boundary, a cross-qualified native `DosPacket` translator,
bounded trackdisk partition adapter and fully linked off-tree handler module.
The host adapter and mount CLI can be built with:

```sh
cargo build -p afsplus-fuse --features fuser-adapter --bin afsplus-mount
```

Linux uses fuser's native mount path. On macOS, install macFUSE and build with
`--features macfuse-mount`; the mount CLI selects macFUSE's user-space FSKit
backend and the mountpoint must be an existing directory or a new direct child
of `/Volumes`. The library is loaded at runtime, so ordinary workspace builds
do not require a system FUSE installation. If macOS's File System Extensions
switches are inert, use the diagnostic and reversible workaround in
[`docs/macos-fskit-activation.md`](docs/macos-fskit-activation.md). Fuse-T's NFS
transport is not a raw substitute for macFUSE's message channel; see ADR-040.
The native AROS bridge and its cross-build qualification are documented in
[`docs/aros-native-bridge.md`](docs/aros-native-bridge.md), ADR-042 through
ADR-046. `tools/check-hosted-aros-alpha0.sh` now qualifies a bidirectional
Hosted MacAROS → macFUSE → Hosted MacAROS round trip on one checked image.
In-tree build integration, Hosted crash replay, an AFS+ system volume and the
native/classic ports remain explicit later gates. Qualification is reported as
three target platforms over four ordered stages: Hosted MacAROS, native
MacAROS/Apple Silicon, Amiga 500/m68k emulation, then the physical A500. The
emulator is the pre-hardware validation stage of the A500 target.
Fields marked `TBD`, Proposed, experimental, or otherwise unfrozen are not
format commitments. Incompatible format changes must update the format epoch
or use feature negotiation as defined in the compatibility specification.
