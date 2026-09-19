# AFS+

AFS+ is a native filesystem for AROS and the wider Amiga-family ecosystem,
with a portable, OS-neutral on-disk format. This repository holds the
specification, the Rust reference implementation, a second implementation of
the reader in portable C, and the plans and decision records behind both.

> **AFS+ is not being created to rebuild ext4, NTFS, or another conventional filesystem with Amiga branding. Its strongest contract is to expose directly to modern applications what a filesystem already knows.**

<!-- toc -->

- [Why it exists](#why-it-exists)
- [How it works](#how-it-works)
  - [What is on disk](#what-is-on-disk)
  - [Two readers, and one canonical image](#two-readers-and-one-canonical-image)
- [Founding developer contract](#founding-developer-contract)
- [How the design stays cheap to maintain](#how-the-design-stays-cheap-to-maintain)
  - [Allocation regions](#allocation-regions)
  - [Compatibility profiles](#compatibility-profiles)
  - [Forensic no-write mount](#forensic-no-write-mount)
  - [Shared repair and invariant definitions](#shared-repair-and-invariant-definitions)
  - [Optional accelerators have explicit failure semantics](#optional-accelerators-have-explicit-failure-semantics)
  - [Tiny-file optimization is measured, not assumed](#tiny-file-optimization-is-measured-not-assumed)
- [Build, test and check](#build-test-and-check)
- [Repository map](#repository-map)
- [Documentation map](#documentation-map)
- [Where it stands](#where-it-stands)
- [License](#license)

<!-- /toc -->

## Why it exists

Classic AFS/FFS is woven into the Amiga model, and its on-disk format carries
the limits of the 32-bit era: block numbers and sizes that stop short, a
namespace that cannot hold Unicode, a directory structure that degrades with
size, and no way to survive a power cut except by scanning the volume. exFAT
is a good interchange format and a poor place to grow AROS-native semantics,
crash recovery, object identity, richer metadata and fast indexing.

Extending either one reaches the same wall. The things that matter most here,
stable object identity, cheap copies, atomic batches, incremental change
discovery, are not features you bolt onto a format that has no room for them;
they are consequences of how the format is laid out and how transactions
commit. So AFS+ starts from the layout.

Four goals follow, and they constrain each other:

1. **Keep what AROS got right.** Volumes, assigns, protection bits, comments,
   filesystem handlers as ordinary programs, and a system model a person can
   hold in their head.
2. **Provide a modern substrate.** 64-bit addressing, extents, Unicode names,
   directories that scale, crash consistency, metadata integrity, behaviour
   that suits both spinning disks and flash.
3. **Answer the application directly.** A filesystem already knows what
   changed, what is a copy of what, and what an object is. Making applications
   rediscover that by scanning is the waste this design exists to remove.
4. **Stay implementable elsewhere.** A portable core, a second implementation
   path in C, a small reader profile, public format documentation, and one
   identification block whose shape never moves.

AFS+ is not a binary-compatible extension of AFS/FFS, not an extension of
exFAT, and not a filesystem every application must understand. It ships for
AROS as an external `L:` handler plus a DOSDriver, so acceptance into the
upstream AROS tree is not a prerequisite
([ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md)).

## How it works

An existing AROS application talks to `dos.library` and needs no
recompilation. A new one opts into the modern surface through Filesystem
API v2 ([13](docs/13-filesystem-api-v2.md)), which is filesystem-neutral: the
capabilities it exposes are meant to be implementable by other handlers.

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

The implementation is layered the same way on every host
([02](docs/02-architecture.md), [27](docs/27-rust-implementation-strategy.md)),
and the AROS handler is a thin C shell over the same core the host tools use:

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

### What is on disk

One immutable identification block names the volume and its feature words.
Everything mutable hangs off an alternating pair of checksummed checkpoints:
publishing a transaction means writing new copies of the metadata it touched
and then committing one checkpoint, so an interrupted write leaves the older
checkpoint intact and a mount has two legal states to choose between, never a
half-written one. Namespace changes and updates to existing files also pass
through an intent log, which bounds how much work a commit costs and how much
a replay has to redo
([ADR-063](adr/ADR-063-intent-log-epoch1.md),
[ADR-064](adr/ADR-064-intent-log-data-update-compatibility.md)).

Free space lives in allocation regions, each with bounded local state, a
fixed allocation-root pool and a segmented reclaim queue, so the cost of an
allocation does not grow with the size of the volume
([ADR-067](adr/ADR-067-epoch1-allocation-state.md)). Files are extents, and an
extent can be shared, which is what makes `CloneFile()` and `CloneRange()`
cheap; a hole is simply an absent extent. Directories, the object map, the
extent map, the shared-extent table and the snapshot registry are all B+
trees with binary key ordering, never host-locale collation.

An object has an identity independent of any path, and a content generation
that changes when its data does. Names are valid UTF-8 kept byte for byte as
the user typed them, with lookup through a versioned normalized comparison
key, so a volume can be case-sensitive or case-insensitive without rewriting
names. The object record carries what is small and always wanted, including
the protection bits, the timestamps and a stored comment, and it references
two optional owned chains for what is neither: a security descriptor
container and an extended-attribute set.

### Two readers, and one canonical image

Every block is admitted only in its canonical image: reserved header fields
are zero, the payload is exactly the length its version defines, and the bytes
after it are zero
([ADR-110](adr/ADR-110-exact-reclaim-admission.md) to
[ADR-114](adr/ADR-114-reserved-header-fields.md)). A version or a block kind
that is retired is refused, never ignored, and its identity is never reused
([ADR-115](adr/ADR-115-retire-unwritten-surface.md)). This is not tidiness:
slack in what a reader accepts is exactly where two implementations drift
apart, and this repository has two.

The second implementation is the portable C reader under
[portable/](portable/README.md). It is written from the specification, and
cross-read tests require it to reach the same verdict as the Rust codec on
every image, including the malformed ones. Several defects of the Rust side
were found that way, which is the reason it exists
([ADR-029](adr/ADR-029-dual-reference-implementations.md)).

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

## How the design stays cheap to maintain

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

## Build, test and check

The build is the Cargo workspace under [crates/](crates/README.md):

```sh
cargo build --workspace
```

Tests are run by name, never as a suite. A test written beside the code it
covers confirms its author's intent; proof comes from checks that do not
share the author's assumptions, and
[implementation/development-method.md](implementation/development-method.md)
is binding on what that means here:

```sh
cargo test -p afsplus-check --test extended_attributes
make check-docs            # link, index, ADR and milestone consistency
make portable-c-gate       # the C reader against Rust-written images
make portable-c-fuzz-gate  # sanitizer mutations of the C reader paths
make rust-codec-fuzz-gate  # the codec fuzz targets
```

The host tools format and inspect an image without a mount. All of them open
the image read-only except `mkafsplus`, and their deterministic JSON, exit
statuses and diagnostic identifiers are defined in
[tools/tools-spec.md](tools/tools-spec.md):

```sh
cargo run -p afsplus-tools --bin mkafsplus -- --profile workstation demo.img
cargo run -p afsplus-tools --bin afsplus-info -- --json demo.img
cargo run -p afsplus-tools --bin afsplus-dump -- --json demo.img
cargo run -p afsplus-tools --bin afsplus-explain -- demo.img object 1
cargo run -p afsplus-tools --bin afsplus-image-diff -- before.img after.img
cargo run -p afsplus-tools --bin afsplus-extract -- demo.img recovered/
```

`afsplus-explain` answers what the committed state believes about one block,
object or path, `afsplus-extract` copies a volume out without writing to it, and `afsplus-image-diff` reports what two images differ by in
filesystem terms instead of block terms. Both walk the image with the format
codecs alone, so they are a second opinion on the checker and not a view of
it.

The host mount CLI is built with:

```sh
cargo build -p afsplus-fuse --features fuser-adapter --bin afsplus-mount
```

Linux uses fuser's native mount path. On macOS, install macFUSE 5.4.0 or later
and build with `--features macfuse-mount`; earlier releases deliver writes of
up to fourteen bytes to the driver as zeros
([testing/mounted-volume-testing.md](testing/mounted-volume-testing.md)). The
CLI selects macFUSE's user-space FSKit backend and the mountpoint must be an
existing directory or a new direct child of `/Volumes`. The library is loaded at runtime, so ordinary workspace builds do
not require a system FUSE installation. If macOS's File System Extensions
switches are inert, use the diagnostic and reversible workaround in
[docs/macos-fskit-activation.md](docs/macos-fskit-activation.md). FUSE-T's NFS
transport is not a substitute for macFUSE's message channel
([ADR-040](adr/ADR-040-fuse-protocol-boundary.md)).

The AROS gates build and run the handler on a hosted AROS. They need the
toolchains described in
[docs/aros-native-bridge.md](docs/aros-native-bridge.md), and
`tools/prepare-hosted-aros.sh` reapplies what a rebuilt AROS tree loses. Each
gate is listed with its purpose in [tools/README.md](tools/README.md).

## Repository map

| Directory | Content | Index |
|---|---|---|
| `docs/` | Design series `00`–`33`, platform integration documents, documentation rules | [docs/README.md](docs/README.md) |
| `adr/` | Architecture decision records, `ADR-001` onward | [adr/README.md](adr/README.md) |
| `spec/` | Normative constants, disk layout, invariants, feature registry | [docs/README.md](docs/README.md#spec) |
| `api/` | Draft public and developer C headers | [docs/README.md](docs/README.md#api) |
| `profiles/` | Compatibility profile definitions | [docs/README.md](docs/README.md#profiles) |
| `examples/` | Example usage flows | [docs/README.md](docs/README.md#examples) |
| `proposals/` | Drafts awaiting team review | [proposals/README.md](proposals/README.md) |
| `references/` | External design references and provenance | [references/REFERENCES.md](references/REFERENCES.md) |
| `implementation/` | Milestones, plans, measurement reports, the working method | [implementation/README.md](implementation/README.md) |
| `testing/` | Test plans: what each gate checks and what runs it | [testing/README.md](testing/README.md) |
| `tools/` | Official CLI tool specification and qualification gates | [tools/README.md](tools/README.md) |
| `crates/` | Rust workspace: format, block, core, check, measure, tools, vfs, backup, aros, aros-ffi, fuse, macfuse-sys | [crates/README.md](crates/README.md) |
| `fuzz/` | Codec fuzz targets and their oracles, a separate workspace | [testing/fuzzing.md](testing/fuzzing.md) |
| `portable/` | Independent non-Rust implementations and embedding guides | [portable/README.md](portable/README.md) |
| `native/` | Native AROS C handler shell, translator, and patches offered upstream | [docs/aros-native-bridge.md](docs/aros-native-bridge.md) |
| `build/` | Retained qualification evidence, ignored by git | [testing/README.md](testing/README.md) |

## Documentation map

Every fact has one home; everything else links to it.

| Kind | Answers | Home |
|---|---|---|
| State | where are we? | [ROADMAP.md](ROADMAP.md) (stages), [implementation/milestones.md](implementation/milestones.md) (gates and evidence) |
| Design | how is it built and why? | [docs/](docs/README.md), [spec/](docs/README.md#spec), [api/](docs/README.md#api), [adr/](adr/README.md) |
| Procedure | what do I run, what does it prove? | [testing/](testing/README.md), [tools/README.md](tools/README.md), [tools/tools-spec.md](tools/tools-spec.md), [CONTRIBUTING.md](CONTRIBUTING.md), [implementation/development-method.md](implementation/development-method.md) |
| History | what was decided, tried, delivered? | [adr/](adr/README.md) (decisions), [NOTES.md](NOTES.md) (journal) |
| Agent rules | how do agents work here? | [AGENTS.md](AGENTS.md) (with [CLAUDE.md](CLAUDE.md) pointing to it) |

## Where it stands

Stages are the finite acceptance units; their gates and the evidence behind
each one are in [ROADMAP.md](ROADMAP.md) and
[implementation/milestones.md](implementation/milestones.md), which are the
authority over this summary.

| Area | State |
|---|---|
| Executable core, devices, transactions, accounting, replay, diagnostics, cache matrix, fuzzing | [~~Stage A~~](ROADMAP.md#stage-a-make-the-core-executable) complete, eight of eight finite gates |
| Allocation state, update policy, durability, core structures, security container, C parity, explain, image diff | [~~Stage B~~](ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) complete, eight of eight finite gates |
| AROS handler ([\[M06\]](implementation/milestones.md)) | Runs on real AROS: a hosted `darwin-aarch64` build boots from an AFS+ partition (S2), survives 24 cut-and-reboot rounds with nothing torn or lost (S3), passes the DOS semantics and device write-ordering gates, holds one instance per medium and runs the seeded benchmark at 3.1 s against 1.4 s for FFS, at C boundary interface revision 17; S2/S3 on QEMU and Native, m68k and Apple hardware open |
| Host mount and portable API ([\[M07\]](implementation/milestones.md), [~~M08~~](implementation/milestones.md)) | Mountable Alpha-0 complete ([ADR-060](adr/ADR-060-mountable-alpha0-completion-gate.md)); full consumer qualification open |
| Catalog, change stream, resize, classic reader ([M09](implementation/milestones.md)–[\[M12\]](implementation/milestones.md)) | M09 to M11 not started; [\[M12\]](implementation/milestones.md) partial |
| Application qualification and epoch 1 ([\[M13\]](implementation/milestones.md), [\[M14\]](implementation/milestones.md)) | Fsync workload harness measured; the freeze gates are in [ROADMAP.md](ROADMAP.md#epoch-1-freeze-gates) |

Two limits a reader should not have to discover later. The rule that one
handler instance owns one medium is proven on a uniprocessor kernel, because
the claim walks the port list under `Forbid()`. The extension packet number
the AROS transport uses is provisional until AROS allocates a real one.

The on-disk format is not frozen. Fields marked `TBD`, Proposed or
experimental are not commitments, and an incompatible change must move the
format epoch or negotiate a feature, as
[spec/compatibility-rules.md](spec/compatibility-rules.md) defines.

## License

Licensing is proposed, not final: see [LICENSE.md](LICENSE.md). The intended
split is an open specification license for the documentation, MIT for the
portable reference code, headers, tools and examples, and the destination
component's license for code contributed directly to AROS.
