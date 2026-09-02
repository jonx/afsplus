# AFS+ Benchmark Contract

> **ADRs:** none · **Spec:** none ·
> **Tests:** [aros-system-volume-qualification](aros-system-volume-qualification.md) · **Milestones:** M13, M14

AFS+ must measure performance and resource use continuously. A new filesystem has no credibility if its design claims are not tied to repeatable workloads and published metrics.

<!-- toc -->

- [1. Benchmark dimensions](#1-benchmark-dimensions)
- [2. Implementation comparison](#2-implementation-comparison)
- [3. Mandatory workload classes](#3-mandatory-workload-classes)
  - [Small-file development tree](#small-file-development-tree)
  - [Large sequential files](#large-sequential-files)
  - [Directory scale](#directory-scale)
  - [Metadata-heavy package/build workload](#metadata-heavy-packagebuild-workload)
  - [Clone/reflink](#clonereflink)
  - [Change stream/catalog](#change-streamcatalog)
  - [Crash/recovery](#crashrecovery)
  - [Low-memory mode](#low-memory-mode)
- [4. CPU measurement](#4-cpu-measurement)
- [5. Memory measurement](#5-memory-measurement)
- [6. I/O and write amplification](#6-io-and-write-amplification)
- [7. Benchmark reproducibility](#7-benchmark-reproducibility)
- [8. Regression gates](#8-regression-gates)
- [9. Benchmark philosophy](#9-benchmark-philosophy)

<!-- /toc -->

## 1. Benchmark dimensions

Every important benchmark should collect as many of these as applicable:

- wall-clock latency
- operations per second
- CPU user time
- CPU system time
- total CPU cycles where available
- peak resident memory
- steady-state resident memory
- allocator peak bytes
- metadata cache peak bytes
- block reads
- block writes
- bytes read
- bytes written
- flush/barrier count
- discard/TRIM count
- metadata write amplification
- total write amplification
- image size growth for sparse/overlay tests
- fragmentation/extents per file
- transaction/checkpoint count
- reclaim backlog
- bitmap pages written
- blocks allocated
- blocks retired/quarantined
- blocks reclaimed
- reclaim latency
- metadata bytes written per allocation/free

The benchmark harness must separate filesystem-cache effects from cold-storage behavior where practical.

## 2. Implementation comparison

Run equivalent workloads against:

- Rust AFS+ reference core
- portable C AFS+ implementation
- host/FUSE AFS+ where relevant
- native AROS AFS+ when available

Where platform permits, compare against representative filesystems such as:

- APFS
- ext4
- XFS
- Btrfs
- exFAT
- PFS3/FFS where meaningful

Cross-filesystem results must state platform, mount options, cache state, storage medium, OS version, and whether the filesystem is native or FUSE/user-space.

Native AROS system-volume qualification follows three target platforms in four
ordered stages: Hosted MacAROS, Amiga 500/m68k emulation, the physical Amiga
500, then native MacAROS on Apple Silicon. The native MacAROS gate applies
after its bare-metal target exists. Constrained resource profiles are
measurements within a stage, not an additional platform. The detailed
separation rules are defined in
[`testing/aros-system-volume-qualification.md`](aros-system-volume-qualification.md).

## 3. Mandatory workload classes

### Small-file development tree

Test at least:

- 100k files
- 1M files
- 4M files

Mix file sizes around:

- 0 bytes
- 100 B
- 1 KiB
- 4 KiB
- 16 KiB
- 64 KiB

Measure create, stat, enumerate, random read, rename, delete, and rebuild/index workloads.

### Large sequential files

Measure 1 GiB, 16 GiB, and sparse multi-TB logical files where host storage permits sparse allocation.

Test sequential read/write, append, truncate, sparse seek/write, clone, and partial COW after clone.

### Directory scale

Measure directories containing:

- 1k entries
- 100k entries
- 1M entries

Test lookup hit/miss, create, rename, enumeration, and deletion.

### Metadata-heavy package/build workload

Use reproducible traces inspired by:

- Cargo source/cache trees
- Git checkout/status/index-like access
- compiler/build output trees
- editor workspace metadata
- Ferail-style full-volume enumeration

### Clone/reflink

Measure:

- CloneFile latency for 1 MiB through 1 TiB logical files
- CloneRange latency
- bytes physically written during clone
- first-write COW cost
- highly fragmented source clone
- many-clone fanout and reclamation

### Change stream/catalog

Measure:

- full object enumeration
- catch-up of 100, 10k, and 1M changes
- stale accelerator rebuild
- memory use during enumeration

### Crash/recovery

Measure mount/recovery latency after injected interruption at every transaction stage.

Normal crash recovery must remain bounded and must not silently degrade into full-volume scan.

### Low-memory mode

Run the same correctness workloads under explicit cache limits, for example:

- 64 KiB metadata cache
- 256 KiB
- 1 MiB
- 16 MiB
- effectively unlimited host cache

Classic profiles can define even smaller targets.

## 4. CPU measurement

CPU consumption matters independently from elapsed time.

For each benchmark record at least:

```text
elapsed_ms
cpu_user_ms
cpu_system_ms
cpu_percent_normalized
```

Where available also capture instructions/cycles/cache misses, but do not make hardware performance counters mandatory for portable CI.

## 5. Memory measurement

Record separately where possible:

- process RSS
- Rust/C heap allocation peak
- metadata cache budget/usage
- transaction temporary memory
- checker temporary memory
- catalog/change-stream buffers

A fast benchmark that silently allocates gigabytes does not satisfy AFS+'s goals.

## 6. I/O and write amplification

The block backend should count exact logical filesystem I/O before it reaches the host filesystem.

For a workload with N bytes of user payload changed, report:

```text
metadata_bytes_written
user_data_bytes_written
physical_block_bytes_requested
flush_count
write_amplification = total_bytes_written / logical_payload_changed
```

This is essential when comparing checkpoint-COW against redo journaling and when evaluating tiny-file packing.

## 7. Benchmark reproducibility

Every result bundle includes:

- git commit
- format epoch/features
- Rust/C compiler and flags
- OS/architecture
- device/backend type
- workload seed
- cache settings
- image hash/base image ID
- benchmark configuration
- raw measurements

Results should be machine-readable JSON in addition to human summaries.

## 8. Regression gates

CI should keep historical baselines.

A change that improves throughput but substantially worsens peak RAM or write amplification must not be labeled simply as a performance improvement.

Initial suggested alert thresholds, to be refined empirically:

- >10% CPU regression
- >10% latency regression
- >10% write amplification regression
- >15% peak-memory regression

Threshold crossing requires explanation or an explicitly approved tradeoff.

## 9. Benchmark philosophy

The target is not to win synthetic charts at any cost.

AFS+ should aim for a strong Pareto position:

- fast enough for modern workstation workloads
- dramatically better than classic filesystems on scale
- low and predictable RAM
- low CPU overhead
- low metadata/write amplification
- bounded recovery
- strong developer-facing capabilities

If a feature makes the filesystem impressive in a matrix but consistently damages these fundamentals, the feature should be redesigned or removed.
