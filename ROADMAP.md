# Roadmap

Progress notation: plain `M01` means not started, `[M01]` means started or
partial, and ~~M01~~ means complete. Stage labels follow the same convention.
Links retain the visible brackets or strikethrough. `make toc` refreshes these
labels from the milestone status cells; `make check-docs` detects stale labels.
Stage completion follows the scoped finite acceptance gates in
[milestones](implementation/milestones.md#scoped-stage-acceptance). A stage without
an acceptance inventory uses contributing milestones conservatively. Stage 0
tracks ongoing design review, excluded from finite completion. A completed
stage scope can coexist with a shared milestone awaiting later qualification.


<!-- toc -->

- [Stage and milestone map](#stage-and-milestone-map)
- [Stage 0: Amiga-native design review](#stage-0-amiga-native-design-review)
- [~~Stage A~~: make the core executable](#stage-a-make-the-core-executable)
- [\[Stage B\]: resolve the epoch-1 architecture blockers](#stage-b-resolve-the-epoch-1-architecture-blockers)
  - [B1. Allocation state](#b1-allocation-state)
  - [B2. User-data update policy](#b2-user-data-update-policy)
  - [B3. Checkpoint and fsync](#b3-checkpoint-and-fsync)
  - [B4. Core filesystem structures](#b4-core-filesystem-structures)
  - [B5. Security preservation container](#b5-security-preservation-container)
- [\[Stage C\]: integrate AROS and begin independent C portability](#stage-c-integrate-aros-and-begin-independent-c-portability)
- [\[Stage D\]: portability and host tooling](#stage-d-portability-and-host-tooling)
- [Stage E: developer-contract accelerators and optional features](#stage-e-developer-contract-accelerators-and-optional-features)
- [\[Stage F\]: production qualification](#stage-f-production-qualification)
- [Epoch 1 freeze gates](#epoch-1-freeze-gates)
- [Integrated recovery and storage qualification](#integrated-recovery-and-storage-qualification)

<!-- /toc -->

## Stage and milestone map

Stages group dependencies; [implementation phases](implementation/implementation-plan.md)
identify deliverables. A milestone may contribute to several stages. Progress
is recorded only in [milestones](implementation/milestones.md).

Stage letters name dependency groups, not the working order. Work proceeds
B, C, E, F, then D: the catalogue and change stream of Stage E and the
production qualification of Stage F build on the AROS integration of Stage C,
while the portable implementations and host tooling of Stage D follow a
format that the earlier stages have settled.

| Stage | Contributing milestones | Intended outcome |
|---|---|---|
| [Stage 0](#stage-0-amiga-native-design-review) | Ongoing design review, excluded from finite completion | Amiga filesystem design references |
| [~~Stage A~~](#stage-a-make-the-core-executable) | [\[M01\]](implementation/milestones.md), [~~M02~~](implementation/milestones.md), [\[M03\]](implementation/milestones.md), [\[M04\]](implementation/milestones.md), [\[M05\]](implementation/milestones.md) | Executable core and reader/format foundations |
| [\[Stage B\]](#stage-b-resolve-the-epoch-1-architecture-blockers) | [\[M03\]](implementation/milestones.md), [\[M04\]](implementation/milestones.md) | Allocation, update and durability architecture |
| [\[Stage C\]](#stage-c-integrate-aros-and-begin-independent-c-portability) | [\[M06\]](implementation/milestones.md), [\[M07\]](implementation/milestones.md), [\[M12\]](implementation/milestones.md) | AROS adapters and independent C integration |
| [\[Stage D\]](#stage-d-portability-and-host-tooling) | [\[M01\]](implementation/milestones.md), [~~M08~~](implementation/milestones.md), [\[M12\]](implementation/milestones.md) | Portable implementations and host tooling |
| [Stage E](#stage-e-developer-contract-accelerators-and-optional-features) | [M09](implementation/milestones.md), [M10](implementation/milestones.md) | Catalog and persistent change services |
| [\[Stage F\]](#stage-f-production-qualification) | [M00](implementation/milestones.md), [\[M05\]](implementation/milestones.md), [M11](implementation/milestones.md), [\[M13\]](implementation/milestones.md), [\[M14\]](implementation/milestones.md) | Maintenance, workload qualification and format freeze |

## Stage 0: Amiga-native design review

Milestones: none — the outcome is [docs/23](docs/23-pfs3-stage0-review.md). Subsystem-by-subsystem source review continues only when implementation reaches that subsystem.

- review PFS3 source subsystem by subsystem
- document PFS3 atomic commit
- evaluate PFS4 B+ tree, tiny-file, and fragmentation ideas
- produce adopt/adapt/reject matrix
- revise AFS+ transaction and small-file ADRs before format freeze

**Specification expansion is secondary to implementation. A proposed feature is unfrozen until it has real consumers, code, measurements, and crash semantics.**

See [`implementation/peer-review-prototype-plan.md`](implementation/peer-review-prototype-plan.md).

## ~~Stage A~~: make the core executable

<!-- stage-gates: Stage A = a-core,a-devices,a-checkpoint,a-accounting,a-replay,a-flight,a-cache,a-fuzz -->

Milestones: [\[M01\]](implementation/milestones.md), [~~M02~~](implementation/milestones.md), [\[M03\]](implementation/milestones.md), [\[M04\]](implementation/milestones.md), [\[M05\]](implementation/milestones.md).


The milestone links above cover work across several stages; their labels are
not a completion count for Stage A. Finite acceptance is grouped into the eight
[scoped gates and their evidence](implementation/milestones.md#scoped-stage-acceptance):

| Acceptance gate | Corresponding items below |
|---|---|
| `a-core` — executable core | Rust workspace and the four core crates |
| `a-devices` — test devices | File/memory backends, trace/fault/power-cut wrappers, SliceBackend and OverlayBackend |
| `a-checkpoint` — transactions and recovery | Format/checkpoint descriptors, slots, root/metadata, first transaction, remount checker and crash matrix |
| `a-accounting` — resource accounting | Benchmark harness with CPU/RAM/I/O/flush/write-amplification accounting |
| `a-replay` — reproducible operations | Operation record/replay, retained artifacts and reconstruction |
| `a-flight` — internal diagnostics | Structured flight recorder: API/window/subsystem correlation and bounded export/replay |
| `a-cache` — constrained caches | Tiny-cache test matrix across mutation/publication families, faults and recovery |
| `a-fuzz` — malformed and generated inputs | Fuzzing/property tests across executable codecs and operation families |

Strikethrough marks completed individual deliverables. It does not claim that
an entire crate, cross-stage milestone or ongoing architectural constraint is
finished. The linked gate table owns completion status.

Primary goal:

```text
format image
 -> mutate
 -> checkpoint
 -> kill power at every point
 -> remount
 -> verify exact allowed state
```

Build first:

- ~~establish Rust workspace~~ <!-- progress: roadmap-01 -->
- ~~`afsplus-format`~~ <!-- progress: roadmap-02 -->
- ~~`afsplus-block`~~ <!-- progress: roadmap-03 -->
- ~~`afsplus-core`~~ <!-- progress: roadmap-04 -->
- ~~`afsplus-check`~~ <!-- progress: roadmap-05 -->
- ~~sparse raw host-file backend~~ <!-- progress: roadmap-06 -->
- ~~memory block backend~~ <!-- progress: roadmap-07 -->
- ~~trace wrapper~~ <!-- progress: roadmap-08 -->
- ~~deterministic fault-injection wrapper~~ <!-- progress: roadmap-09 -->
- ~~power-cut simulation backend~~ <!-- progress: roadmap-10 -->
- ~~minimal format/checkpoint descriptor~~ <!-- progress: roadmap-11 -->
- ~~checkpoint slots A/B~~ <!-- progress: roadmap-12 -->
- ~~root object~~ <!-- progress: roadmap-13 -->
- ~~minimal metadata encoding~~ <!-- progress: roadmap-14 -->
- ~~first create-object transaction~~ <!-- progress: roadmap-15 -->
- ~~remount/invariant checker~~ <!-- progress: roadmap-16 -->
- ~~deterministic crash matrix after every write/flush~~ <!-- progress: roadmap-17 -->
- ~~benchmark harness with CPU/RAM/I/O/flush/write-amplification accounting~~ <!-- progress: roadmap-29 -->

Then add:

- ~~SliceBackend for partition/disk-image viewports~~ <!-- progress: roadmap-18 -->
- ~~OverlayBackend for cheap writable test branches~~ <!-- progress: roadmap-19 -->
- ~~structured flight recorder~~ <!-- progress: roadmap-30 -->
- ~~operation record/replay~~ <!-- progress: roadmap-20 -->
- ~~tiny-cache test matrix~~ <!-- progress: roadmap-31 -->
- ~~fuzzing/property tests~~ <!-- progress: roadmap-32 -->

Task-level status, origin and completion evidence:
[structured flight recorder](implementation/milestones.md#structured-flight-recorder-tasks),
[tiny-cache test matrix](implementation/milestones.md#tiny-cache-test-matrix-tasks),
[fuzzing/property tests](implementation/milestones.md#fuzzing-and-property-test-tasks).

Ongoing constraint (kept visible, excluded from finite completion):

- keep core disk semantics independent from host namespaces <!-- progress: roadmap-33 -->

Do not block this stage on:

- full ACL engine
- catalog/change stream
- AtomicBatch
- content inspection
- FUSE
- native AROS handler
- tiny-file packing
- LLM-specific tuning

## \[Stage B\]: resolve the epoch-1 architecture blockers

Milestones: [\[M03\]](implementation/milestones.md), [\[M04\]](implementation/milestones.md).

### B1. Allocation state

Architecture closed by [ADR-067](adr/ADR-067-epoch1-allocation-state.md):

- allocation regions are implemented and qualified;
- the COW free-space self-reference problem is handled by deterministic triple
  slots and a fixed `3N` allocation-root pool;
- bitmap authority was selected over an authoritative delta or spacemap after
  measurement; and
- nearly-full-volume behavior has bounded destructive progress and explicit
  emergency headroom.

Exact byte layout and the global wire epoch remain subject to M14 review.

### B2. User-data update policy

[ADR-062](adr/ADR-062-explicit-hybrid-data-updates.md) defines full COW by
default and explicit private in-place opt-in. [ADR-065](adr/ADR-065-persistent-data-update-policy.md)
defines the persistent policy. Shared, snapshot-protected or uncertain ranges
require COW. Qualify the policy across Rust, portable C and host adapters.

Required workloads include random 4 KiB rewrites, database/VM hot sets, append,
reflinks, retained snapshots and crashes at each publication boundary. Measure
CPU, RAM, I/O amplification, fragmentation and recovery against the
[data-policy evidence](implementation/data-policy-bakeoff.md).

### B3. Checkpoint and fsync

[ADR-063](adr/ADR-063-intent-log-epoch1.md) defines checkpoint COW, bounded
group commit and an intent log. [ADR-064](adr/ADR-064-intent-log-data-update-compatibility.md)
defines existing-file write/truncate log compatibility. Preserve one shared
mutation/recovery engine.

Qualify namespace and existing-file durability across adapters and portable C,
including replay interruption, torn writes and real-device barriers. Use the
[fsync baseline](implementation/fsync-intent-log-baseline.md) and
[write/truncate qualification](testing/intent-log-write-truncate-qualification.md).
Wire freeze and hardware acceptance belong to [\[M14\]](implementation/milestones.md).

### B4. Core filesystem structures

- ~~B+ tree directories~~ <!-- progress: roadmap-21 -->
- ~~normalized/versioned Unicode comparison keys while preserving original UTF-8 names~~ <!-- progress: roadmap-34 -->
- ~~extent mapping~~ <!-- progress: roadmap-22 -->
- ~~sparse files~~ <!-- progress: roadmap-23 -->
- ~~preallocation~~ <!-- progress: roadmap-35 -->
- ~~shared-extent/reference prototype for reflinks~~ <!-- progress: roadmap-24 -->
- ~~CloneFile/CloneRange semantics~~ <!-- progress: roadmap-25 -->
- ~~deferred reclamation~~ <!-- progress: roadmap-26 -->
- ~~checker~~ <!-- progress: roadmap-27 -->
- explain APIs <!-- progress: roadmap-36 -->
- ~~semantic image diff~~ <!-- progress: roadmap-37 -->

### B5. Security preservation container

Architecture closed by [ADR-101](adr/ADR-101-security-preservation-container.md),
on the admission rule of [ADR-100](adr/ADR-100-exact-object-record-admission.md):

- an object record references a versioned, opaque security descriptor in a
  chain of segments it owns alone;
- every rewrite of the record carries the reference, so metadata no host
  evaluates survives byte for byte, and `CloneFile` copies it
  ([ADR-102](adr/ADR-102-clone-metadata-inheritance.md));
- a classic protection edit is refused or applied with a durable divergence
  mark, and only the explicit clear discards descriptor bytes.

The base writable milestone defines no canonical NFSv4/Windows ACL evaluation
semantics. Descriptor formats, their registry and evaluation remain with
[Q5](implementation/open-questions.md); exact offsets remain subject to M14
review.

## \[Stage C\]: integrate AROS and begin independent C portability

Milestones: [\[M06\]](implementation/milestones.md), [\[M07\]](implementation/milestones.md), [\[M12\]](implementation/milestones.md).

- AROS handler
- DOS compatibility
- Filesystem API v2
- modern 64-bit API
- clone/reflink capability API
- access-intent/preallocation mapping
- mmap-friendly large-file path
- notifications
- health reporting
- trace streaming / developer attachment
- structured management APIs
- Rust/C integration boundary
- generic file-backed virtual block device for mounting images
- native AROS benchmark runner
- classic/single-user security preservation adapter

Portable C work begins from the stable executable spec/conformance corpus:

- language-neutral C ABI boundary
- tiny portable C reader
- cross-implementation read/validation tests
- grow toward `classic-rw` after the Rust writable format stops moving rapidly

## \[Stage D\]: portability and host tooling

Milestones: [\[M01\]](implementation/milestones.md), [~~M08~~](implementation/milestones.md), [\[M12\]](implementation/milestones.md).

- ~~FUSE host mount~~ <!-- progress: roadmap-28 -->
- third-party probe kit
- compatibility profiles
- portable C `classic-rw` qualification
- portable C `full-portable` qualification where feasible
- JSON/structured tooling schemas
- host-side inspect/check/repair workflow
- sparse-image create/mount/fork/replay workflow
- cross-OS interoperability test matrix
- FUSE mmap and parallel page-fault qualification

If rich multi-user ACL semantics remain a project goal, this is the earliest sensible point to build real POSIX and Windows mapping adapters and use them to validate or revise the canonical ACL proposal.

## Stage E: developer-contract accelerators and optional features

Milestones: [M09](implementation/milestones.md), [M10](implementation/milestones.md).

A proposed feature enters this stage only after the core is proven and at least one real consumer exists.

Candidates:

- global catalog
- persistent change stream
- Git/FSMonitor-style adapter
- bulk metadata APIs (`StatBatch`, `LookupBatch`, streamed tree enumeration)
- directory namespace generations
- bounded AtomicBatch
- sealed content
- content-inspection/security feed
- tiny-file storage alternatives
- rebuildable reverse map
- recursive directory statistics
- optional data checksums using the already reserved feature/extent association path
- CloneTree evaluation
- derived content fingerprints
- full portable ACL semantics if real multi-user adapters validate them
- subtree security domains
- optional encryption/key hierarchy review

Features that fail to earn real use may be deprecated/retired. Their IDs remain reserved and existing active volumes require retained support or explicit migration.

## \[Stage F\]: production qualification

Milestones: [M00](implementation/milestones.md), [\[M05\]](implementation/milestones.md), [M11](implementation/milestones.md), [\[M13\]](implementation/milestones.md), [\[M14\]](implementation/milestones.md).

- grow resize
- minimum-size query
- shrink/relocation after safe mover exists
- targeted scrub
- online repair where justified
- performance qualification
- low-memory qualification
- CPU-efficiency qualification
- Rust-vs-C resource qualification where both implementations cover the workload
- streaming/video/large-file qualification
- Git 100k/1M/4M file qualification
- LLM mmap/range-load/model-larger-than-cache qualification
- checkpoint-publication workload qualification
- page-cache pollution qualification
- real SSD qualification
- exhaustive crash-point qualification
- independent format review
- epoch 1 freeze

## Epoch 1 freeze gates

Milestone: M14 ([status](implementation/milestones.md)). Do not freeze the format until:

- normal metadata crash recovery never requires a full-volume scan
- deterministic crash injection covers every transaction boundary
- the user-data crash/durability contract is explicit and tested
- free-space metadata cannot recursively corrupt its own allocation state
- repeated small-file `fsync` has measured/acceptable cost, with a durability log added if required
- `NO_CHANGES` performs zero media writes
- shared extents cannot be freed while referenced by any live object or retained recovery state
- timestamps have one portable UTC Unix-epoch wire definition
- Unicode key generation uses a recorded table version and binary tree-key ordering
- catalog hard-link semantics are explicit
- change-stream loss/reset semantics are explicit (`RESCAN_REQUIRED`, not fake rebuild)
- sparse raw images and physical devices exercise the same disk format
- performance reports include CPU, peak RAM, block I/O, flush count, and write amplification
- machine-readable tools/errors are versioned
- classic/minimal reader profile is demonstrably implementable
- unknown security metadata can survive a simple-host round-trip without silent downgrade
- iterator/concurrency visibility rules are documented and tested
- Cargo/Git/Zed-style/Ferail workloads are qualified

## Integrated recovery and storage qualification

Before claiming safe daily user storage, qualify the
[normative failure and recovery boundaries](spec/invariants.md#failure-and-recovery-boundaries)
through the [coverage map](testing/book-review-qualification.md#normative-coverage-map).
Checkpoint publication errors require reconciliation or remount before another
mutation. Replacement tests enumerate each write/flush failure and verify
complete namespace, bytes and ownership.

[ADR-069](adr/ADR-069-consistent-snapshots-first.md) selects consistent filesystem
snapshots as the first backup/scanner view, persistent across reboot under
[ADR-070](adr/ADR-070-persistent-snapshot-priority.md). Prototype snapshot enumeration and
reads against a fixed oracle while the live tree mutates, including in-place
opt-in files. Q4 decides bounded retention, admission, persistent representation and reclamation
from ENOSPC, release and crash evidence before stabilizing the format/API.
Q11 decides supported salvage and restoration outcomes; M05 owns corruption
classification/extraction and M13 owns actual backup/restore consumers.
Q12 binds M07 host integration to M13 device/cache qualification. M14 requires
these contracts and their stated limitations before format release; it does
not substitute format freeze for demonstrated recovery.

Choose implementations through the deciding experiments in
[open questions](implementation/open-questions.md), record accepted decisions
in ADRs, and turn accepted experiments into milestone gates. Missing platform
support is implementation work with an owner, not a permanent scope limit.

The [audit implementation queue](implementation/audit-work-queue.md) carries
the complete follow-up order and next-session entry point. Continue with integrated snapshot/backup qualification while preserving the independent
recovery, adapter, discovery and application qualification work.
