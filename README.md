# AFS+ Specification and Engineering Plan

AFS+ is a modern native filesystem for AROS and the wider Amiga-family
ecosystem, with a portable, OS-neutral on-disk format.

> **AFS+ is not being created to rebuild ext4, NTFS, or another conventional filesystem with Amiga branding. Its strongest contract is to expose directly to modern software the filesystem primitives that applications are otherwise forced to reconstruct, approximate, or continuously rediscover above the filesystem.**

This repository holds the design specification, the implementation plan, the
compatibility and conformance contracts, and the Rust reference
implementation. It is written so that a developer who has not participated in
the design discussions can implement individual components without
undocumented context.

<!-- toc -->

- [What AFS+ is and is not](#what-afs-is-and-is-not)
- [Founding developer contract](#founding-developer-contract)
- [Architecture](#architecture)
- [Core design summary](#core-design-summary)
- [Design choices that reduce maintenance cost](#design-choices-that-reduce-maintenance-cost)
  - [Allocation regions](#allocation-regions)
  - [Compatibility profiles](#compatibility-profiles)
  - [Forensic no-write mount](#forensic-no-write-mount)
  - [Shared repair and invariant definitions](#shared-repair-and-invariant-definitions)
  - [Optional accelerators have explicit failure semantics](#optional-accelerators-have-explicit-failure-semantics)
  - [Tiny-file optimization is measured, not assumed](#tiny-file-optimization-is-measured-not-assumed)
- [Build, test and mount](#build-test-and-mount)
- [Repository map](#repository-map)
- [Documentation map](#documentation-map)
- [Status](#status)
- [License](#license)

<!-- /toc -->

## What AFS+ is and is not

Classic AFS/FFS is deeply integrated into the Amiga model, but its on-disk
format carries 32-bit era limits and assumptions. exFAT is excellent for
interchange, but it is not a good place to grow AROS-native semantics, crash
recovery, object identity, richer metadata, and high-performance indexing.

AFS+ therefore has four goals:

1. Preserve the good AROS ideas: volumes, Assigns, protection bits, comments, easy filesystem handlers, and a lightweight system model.
2. Provide a genuinely modern storage substrate: 64-bit addressing, extents, Unicode names, scalable directories, crash consistency, metadata integrity, SSD-friendly behavior, and precise durability semantics.
3. Provide a first-class developer contract for modern software instead of forcing applications to rediscover filesystem state through expensive scans and ad-hoc conventions.
4. Remain easy to implement elsewhere: a portable core, a portable C implementation path, a tiny reader profile, public format documentation, fixed identification records, feature negotiation, conformance images, FUSE support, and no AROS path syntax in the on-disk format.

AFS+ is not a binary-compatible extension of classic AFS/FFS, not an
extension of exFAT, and not a monolithic filesystem that every application
must understand. It is distributed for AROS as an external `L:` handler plus a
DOSDriver; acceptance into the upstream AROS source tree is not required
([ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md)).

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

## Architecture

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

Existing AROS applications use the existing DOS API and normally require no
recompilation. New applications opt into modern capabilities through
Filesystem API v2 ([13](docs/13-filesystem-api-v2.md)).

The implementation is layered the same way on every host
([02](docs/02-architecture.md), [27](docs/27-rust-implementation-strategy.md)):

```text
        host tools / afsplus-check / afsplus-mount (FUSE)
                    |                    |
   AROS handler (native C shell + DosPacket translator)
                    |
             portable VFS API (afsplus-vfs)
                    |
           AFS+ core (afsplus-core: mkfs, mount, transactions,
           allocator, reclaim queue, intent log)
                    |
        wire codecs (afsplus-format, no_std)   block backends (afsplus-block)
```

The specification is authoritative over both the Rust reference
implementation and the portable C implementation path
([ADR-028](adr/ADR-028-rust-reference-core.md),
[ADR-029](adr/ADR-029-dual-reference-implementations.md)).

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

AFS+ explicitly studies PFS3 and the proposed PFS4 design before freezing
epoch 1. PFS3 demonstrated unusually strong crash resilience, speed,
fragmentation behavior, and low-resource operation on classic Amiga hardware;
PFS4's published ideas overlap with several AFS+ goals and must be evaluated
rather than rediscovered. See
[22](docs/22-pfs3-and-pfs4-lessons.md), [23](docs/23-pfs3-stage0-review.md)
and [ADR-019](adr/ADR-019-pfs3-design-reference.md).

## Design choices that reduce maintenance cost

### Allocation regions

A single enormous global free-space structure scales badly and makes
low-memory implementations unattractive. AFS+ divides the volume into
allocation regions so allocation/repair work stays bounded. The executable
candidate uses independently checksummed bitmap pages selected by
triple-buffered region descriptors; it supports the proposed 1 GiB region
while loading pages on demand. The encoding is experimental until the Stage
B1 measurements and crash semantics are accepted
([07](docs/07-allocation.md), [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md)).

### Compatibility profiles

Feature flags alone answer whether a volume is mountable. Profiles answer a
more practical question: what set of features should be enabled if a volume
must remain usable by a bootloader, classic machine, recovery tool, or
portable implementation? The profiles are `reader-minimal`, `classic-rw`,
`boot-safe`, `workstation` and `full` ([profiles/](docs/README.md#profiles)).

### Forensic no-write mount

A normal read-only mount can still modify media in some filesystem designs,
for example during recovery or housekeeping. AFS+ defines a strict
`NO_CHANGES` mount mode in which the implementation must not write a single
block ([ADR-014](adr/ADR-014-no-changes.md)).

### Shared repair and invariant definitions

Filesystem repair logic must not become an unrelated second interpretation of
the format. The Rust and portable C implementations, checker, AROS handler,
host tools, and FUSE adapter are checked against the same normative
specification, conformance images, and invariant corpus
([ADR-015](adr/ADR-015-shared-repair-core.md)).

### Optional accelerators have explicit failure semantics

Optional structures are classified by what may safely happen to them: the
catalog is non-authoritative and rebuildable; the change stream is
non-authoritative and discardable, but lost history is not reconstructible and
forces `RESCAN_REQUIRED`; future reverse maps and directory statistics should
be rebuildable whenever practical. The feature registry records these
properties separately rather than hiding them behind one `derived` flag
([09](docs/09-feature-framework.md)).

### Tiny-file optimization is measured, not assumed

Tiny files dominate source trees, Cargo metadata, package caches, editor
state, and configuration directories. AFS+ reserves an extension path for
tiny-file optimization and compares inline data, packed small-file storage,
and ordinary extents with strong locality before freezing an approach
([ADR-018](adr/ADR-018-inline-data-optional.md)).

## Build, test and mount

The build is the Cargo workspace under [crates/](crates/README.md). Three
commands build, test and check the repository:

```sh
cargo build --workspace
cargo test --workspace --all-features
make check            # fmt, clippy, tests and the documentation checker
```

The host mount CLI is built with:

```sh
cargo build -p afsplus-fuse --features fuser-adapter --bin afsplus-mount
```

Linux uses fuser's native mount path. On macOS, install macFUSE and build with
`--features macfuse-mount`; the CLI selects macFUSE's user-space FSKit backend
and the mountpoint must be an existing directory or a new direct child of
`/Volumes`. The library is loaded at runtime, so ordinary workspace builds do
not require a system FUSE installation. If macOS's File System Extensions
switches are inert, use the diagnostic and reversible workaround in
[docs/macos-fskit-activation.md](docs/macos-fskit-activation.md). FUSE-T's NFS
transport is not a substitute for macFUSE's message channel
([ADR-040](adr/ADR-040-fuse-protocol-boundary.md)).

The AROS qualification gates and build helpers under `tools/` need the
MacAROS, native Apple-AArch64 and m68k toolchains described in
[docs/aros-native-bridge.md](docs/aros-native-bridge.md); each is listed with
its purpose in [tools/README.md](tools/README.md).

## Repository map

| Directory | Content | Index |
|---|---|---|
| `docs/` | Design series `00`–`32`, platform integration documents, documentation rules | [docs/README.md](docs/README.md) |
| `adr/` | Architecture decision records `ADR-001`–`ADR-060` | [adr/README.md](adr/README.md) |
| `spec/` | Normative constants, disk layout, invariants, feature registry | [docs/README.md](docs/README.md#spec) |
| `api/` | Draft public and developer C headers | [docs/README.md](docs/README.md#api) |
| `profiles/` | Compatibility profile definitions | [docs/README.md](docs/README.md#profiles) |
| `examples/` | Example usage flows | [docs/README.md](docs/README.md#examples) |
| `proposals/` | Drafts awaiting team review | [proposals/README.md](proposals/README.md) |
| `references/` | External design references and provenance | [references/REFERENCES.md](references/REFERENCES.md) |
| `implementation/` | Milestones, plans, measurement reports | [implementation/README.md](implementation/README.md) |
| `testing/` | Test plans: what each gate checks and what runs it | [testing/README.md](testing/README.md) |
| `tools/` | Official CLI tool specification and qualification gates | [tools/README.md](tools/README.md) |
| `crates/` | Rust workspace: format, block, core, check, vfs, fuse, aros | [crates/README.md](crates/README.md) |
| `native/` | Native AROS C handler shell and translator | [docs/aros-native-bridge.md](docs/aros-native-bridge.md) |

## Documentation map

Every fact has one home; everything else links to it.

| Kind | Answers | Home |
|---|---|---|
| State | where are we? | [implementation/milestones.md](implementation/milestones.md) |
| Design | how is it built and why? | [docs/](docs/README.md), [spec/](docs/README.md#spec), [api/](docs/README.md#api), [adr/](adr/README.md) |
| Procedure | what do I run, what does it prove? | [testing/](testing/README.md), [tools/README.md](tools/README.md), [tools/tools-spec.md](tools/tools-spec.md), [CONTRIBUTING.md](CONTRIBUTING.md) |
| History | what was decided, tried, delivered? | [adr/](adr/README.md) (decisions), [NOTES.md](NOTES.md) (journal) |
| Agent rules | how do agents work here? | [AGENTS.md](AGENTS.md) (with [CLAUDE.md](CLAUDE.md) pointing to it) |

## Status

Detail, exit criteria and per-milestone links are in
[implementation/milestones.md](implementation/milestones.md).

| Area | State |
|---|---|
| Core format, transactions and checker ([M00](implementation/milestones.md)–[M05](implementation/milestones.md)) | Executable prototype; wire formats experimental, epoch 1 unfrozen |
| Host mount and portable API ([M07](implementation/milestones.md), [M08](implementation/milestones.md)) | Mountable Alpha-0 complete ([ADR-060](adr/ADR-060-mountable-alpha0-completion-gate.md)) |
| AROS handler ([M06](implementation/milestones.md)) | Hosted, native-QEMU and m68k-emulator platforms qualified; hardware open |
| Catalog, change stream, resize, classic reader ([M09](implementation/milestones.md)–[M12](implementation/milestones.md)) | Not started |
| Application qualification and epoch 1 ([M13](implementation/milestones.md), [M14](implementation/milestones.md)) | Fsync workload harness measured; freeze gates in [ROADMAP.md](ROADMAP.md#epoch-1-freeze-gates) |

Fields marked `TBD`, Proposed, experimental, or otherwise unfrozen are not
format commitments. Incompatible format changes must update the format epoch
or use feature negotiation as defined in
[spec/compatibility-rules.md](spec/compatibility-rules.md).

## License

Licensing is proposed, not final: see [LICENSE.md](LICENSE.md). The intended
split is an open specification license for the documentation, MIT for the
portable reference code, headers, tools and examples, and the destination
component's license for code contributed directly to AROS.
