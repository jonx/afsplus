# Roadmap

<!-- toc -->

- [Stage 0: Amiga-native design review](#stage-0-amiga-native-design-review)
- [Stage A: make the core executable](#stage-a-make-the-core-executable)
- [Stage B: resolve the epoch-1 architecture blockers](#stage-b-resolve-the-epoch-1-architecture-blockers)
  - [B1. Allocation state](#b1-allocation-state)
  - [B2. User-data update policy](#b2-user-data-update-policy)
  - [B3. Checkpoint and fsync](#b3-checkpoint-and-fsync)
  - [B4. Core filesystem structures](#b4-core-filesystem-structures)
  - [B5. Security preservation container](#b5-security-preservation-container)
- [Stage C: integrate AROS and begin independent C portability](#stage-c-integrate-aros-and-begin-independent-c-portability)
- [Stage D: portability and host tooling](#stage-d-portability-and-host-tooling)
- [Stage E: developer-contract accelerators and optional features](#stage-e-developer-contract-accelerators-and-optional-features)
- [Stage F: production qualification](#stage-f-production-qualification)
- [Epoch 1 freeze gates](#epoch-1-freeze-gates)

<!-- /toc -->

## Stage 0: Amiga-native design review

Milestones: none — the outcome is [docs/23](docs/23-pfs3-stage0-review.md). Subsystem-by-subsystem source review continues only when implementation reaches that subsystem.

- review PFS3 source subsystem by subsystem
- document PFS3 atomic commit
- evaluate PFS4 B+ tree, tiny-file, and fragmentation ideas
- produce adopt/adapt/reject matrix
- revise AFS+ transaction and small-file ADRs before format freeze

**Specification expansion is secondary to implementation. A proposed feature is unfrozen until it has real consumers, code, measurements, and crash semantics.**

See [`implementation/peer-review-prototype-plan.md`](implementation/peer-review-prototype-plan.md).

## Stage A: make the core executable

Milestones: M02, M03, M04, M05 ([status](implementation/milestones.md)).

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

- establish Rust workspace
- `afsplus-format`
- `afsplus-block`
- `afsplus-core`
- `afsplus-check`
- keep core disk semantics independent from host namespaces
- sparse raw host-file backend
- memory block backend
- trace wrapper
- deterministic fault-injection wrapper
- power-cut simulation backend
- minimal format/checkpoint descriptor
- checkpoint slots A/B
- root object
- minimal metadata encoding
- first create-object transaction
- remount/invariant checker
- deterministic crash matrix after every write/flush
- benchmark harness with CPU/RAM/I/O/flush/write-amplification accounting

Then add:

- SliceBackend for partition/disk-image viewports
- OverlayBackend for cheap writable test branches
- structured flight recorder
- operation record/replay
- tiny-cache test matrix
- fuzzing/property tests

Do not block this stage on:

- full ACL engine
- catalog/change stream
- AtomicBatch
- content inspection
- FUSE
- native AROS handler
- tiny-file packing
- LLM-specific tuning

## Stage B: resolve the epoch-1 architecture blockers

Milestones: M03, M04 ([status](implementation/milestones.md)); the blockers are tracked as questions in [implementation/open-questions.md](implementation/open-questions.md).

### B1. Allocation state

- prototype allocation regions
- explicitly test the COW free-space self-reference problem
- compare PFS3-like metadata reserve, bitmap+delta, spacemap/log-like, or proven hybrid
- verify nearly-full-volume behavior

Do not freeze the authoritative region free-space encoding before this passes crash tests.

### B2. User-data update policy

Prototype/measure:

- full data COW
- in-place overwrite for unshared committed data
- hybrid policy only if measurements justify its extra semantics

Required workloads:

- random 4 KiB rewrites
- VM/database-style files
- reflink/shared-range writes
- crash before/after metadata commit
- historical content-generation handle cost

Reflink-shared ranges always COW.

### B3. Checkpoint and fsync

- implement checkpoint-COW transaction engine first
- benchmark repeated small write + `fsync`
- benchmark Git/package-manager rename/fsync patterns
- add a small durability/intent-log prototype **only if measurements show the checkpoint path needs it**

Do not build a second complete redo-journal engine solely for a bake-off.

The format keeps a discoverable extension point for future auxiliary durability-log state without freezing its record encoding yet.

### B4. Core filesystem structures

- B+ tree directories
- normalized/versioned Unicode comparison keys while preserving original UTF-8 names
- extent mapping
- sparse files
- preallocation
- shared-extent/reference prototype for reflinks
- CloneFile/CloneRange semantics
- deferred reclamation
- checker
- explain APIs
- semantic image diff

### B5. Security preservation container

- versioned security descriptor/blob reference path
- preserve unknown rich security metadata
- classic/simple-host projection must not destroy it

Do **not** require full canonical NFSv4/Windows ACL evaluation semantics in the base writable milestone.

## Stage C: integrate AROS and begin independent C portability

Milestones: M06, M07 ([status](implementation/milestones.md)).

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

## Stage D: portability and host tooling

Milestones: M01, M08, M12 ([status](implementation/milestones.md)).

- FUSE host mount
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

Milestones: M09, M10 ([status](implementation/milestones.md)).

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

## Stage F: production qualification

Milestones: M11, M13, M14 ([status](implementation/milestones.md)).

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
