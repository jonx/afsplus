# 17. Portability

> **ADRs:** [ADR-100](../adr/ADR-100-exact-object-record-admission.md), [ADR-111](../adr/ADR-111-checkpoint-zero-tail.md), [ADR-112](../adr/ADR-112-block-zero-tail.md), [ADR-114](../adr/ADR-114-reserved-header-fields.md), [ADR-115](../adr/ADR-115-retire-unwritten-surface.md) ·
> **Spec:** [format header](../spec/afsplus_format.h), [disk layout](../spec/disk-layout.md) ·
> **Tests:** [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md),
> [conformance](../testing/conformance.md), [fuzzing](../testing/fuzzing.md) · **Milestones:** M01, M08

## 1. Portable core requirement

`libafsplus` must build outside AROS.

The core may depend only on explicit portability shims for:

- block I/O
- memory allocation
- locking
- time
- logging
- random UUID generation where formatting

## 2. Host-file backend

The first backend should treat an ordinary host file as a block device.

This enables development on macOS/Linux:

```text
mkafsplus image.afsp 10G
afsplus-check image.afsp
afsplus-dump image.afsp
```

before the AROS handler exists.

## 3. FUSE

FUSE support is a first-class development objective.

Benefits:

- inspect AROS system disks from the M5 development Mac
- copy binaries without booting AROS
- run conformance tests using normal host tools
- make the format accessible to Linux/macOS users
- force clean separation between filesystem core and AROS handler

## 4. Windows

Native Windows support is not required for 1.0.

The specification, reader library, and conformance suite should make a future Windows implementation straightforward.

## 5. Other Amiga-family systems

Porting should require:

- block-provider glue
- filesystem handler glue
- encoding/path integration

It should not require reverse engineering the format from AROS source.

## 6. Independent portable C implementation

The `reader-minimal` C99 implementation lives under
[`portable/`](../portable/README.md). Its core has no POSIX dependency, owns no
memory, and receives logical-block I/O plus a scratch block from its caller.
The surface validates identification, compatibility summaries and geometry,
selects the newest structurally valid retained checkpoint, then performs
bounded lookups through typed object-map, directory and extent trees. It
decodes object records and reads direct, sparse and tree-mapped regular files
into caller-owned buffers. It does not share codecs with Rust;
interoperability is established by consuming Rust-produced images and matching
the corruption/fallback oracle in the
[conformance gate](../testing/conformance.md#portable-c-bootstrap-gate).

Every wire layout the Rust codec states has a decoder here, and each is held
to the same verdict image by image against the Rust one. Besides the
identification block, the object record and the trees the lookup path needs,
the reader decodes: one checkpoint slot in both payload forms
(`afspr_decode_checkpoint_block`); the security reference, an `"AFSX"`
descriptor segment and an object comment; the attribute reference, an `"AFSA"`
segment, a whole attribute set and its iterator; the reclaim root, sealed
segment and sealed table; and the snapshot registry and lifetime records with
their keys. It decodes those last two families without walking either
structure, because it implements neither reclamation nor snapshots. The
cross-read tests are named in [fuzzing](../testing/fuzzing.md) and in
[the portable reader's README](../portable/c/README.md).

Admission is exact, and the rules live where every kind passes through them.
`afspr_verify_header` refuses a nonzero byte after the payload for every block
kind ([ADR-112](../adr/ADR-112-block-zero-tail.md)); the common header's flags
and owner are zero on the kinds that assign them no meaning
([ADR-114](../adr/ADR-114-reserved-header-fields.md)); identification version 3
is the only version and the retired prototype layouts are refused, not read
([ADR-115](../adr/ADR-115-retire-unwritten-surface.md)). A reader written from
this chapter that admitted more would accept images the filesystem calls
corrupt.

The reader also scans the fsynced intent-log prefix and exposes a read-only
checkpoint-plus-log namespace and file view. Its bounded lookup and
caller-owned ordered cursor merge logged create, delete and rename operations
with the committed directory trees, including replacement and hard-link
identity. File reads apply logged create, write and truncate data to that final
identity. The reader content-verifies replacement blocks, validates replay
preconditions and reports the first excluded tail slot and LBA.

The C reader negotiates `org.aros.afsplus:orphan-directory` as `RO_COMPAT`
bit 1 and returns `NOT_FOUND` for ordinary object lookup or enumeration of
reserved object ID 2. It can therefore cross-read visible data on an active
orphan volume without confusing lifecycle state with user namespace. Forensic
tools, unlike the ordinary reader API, label that internal directory and its
pending-entry count explicitly.

Integrators get a versioned public ABI, symbolic and printable errors,
structured stage/LBA/per-checkpoint diagnostics, a source-vendoring contract,
a CMake target and a complete host-file example. Native targets replace only
the logical-block callback. Complete non-ASCII comparison-key validation and
an independent repair walk are separate C capabilities, never implicit
behavior of the bounded reader. Unicode-profile names outside ASCII fail
closed for logged-name overlays and direct lookup until the frozen Unicode 16
tables are present; committed ordinal enumeration has only structural key
validation. Legacy identity keys retain byte-exact UTF-8 behavior.

The companion C fuzz harness records successful reads as compact sparse-device
packets. Deterministic case numbers, artifact export and the standalone replay
tool make sanitizer failures reproducible without retaining or sharing a full
disk image. The same callback is directly consumable by libFuzzer-compatible
engines; the repository gate does not depend on such a runtime being installed.
See the [embedding and fuzzing guide](../portable/c/README.md#fuzzing-and-exact-reproduction).

The independent write path uses a separate ABI-1 writer surface. It validates
the checkpoint-plus-log view, then appends an empty-file create, data-free
truncate, regular-file delete or rename record (with optional replacement),
or one complete-block existing-file COW write. Create returns the monotone
object ID chosen after all prior logged creates. Truncate covers sparse growth
and block-aligned shrink; unaligned shrink uses a later tail-block consumer.
Rust replays and checks every variant. Metadata-only calls need no allocator
or checkpoint writer. The COW call independently validates allocation tree,
descriptor and bitmap bindings, excludes every durable log reservation,
preserves emergency headroom and performs data-flush before record-flush. All
paths return explicit uncertainty stages. Delete/replacement require
the orphan-directory feature, and final victims enter bounded cleanup during
replay rather than being retired in proportion to their fragmentation. The
same ABI keeps an 8 KiB minimum workspace and opportunistically uses extra
caller scratch as a bounded call-local read cache; the recommended 56 KiB
profile cuts seven-record namespace/truncate mutation preflight to 21 reads
(from 152/178); the allocation-validating COW path uses 25 cached or 223
minimum-memory reads.
