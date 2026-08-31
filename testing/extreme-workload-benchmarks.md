# Extreme Workload Benchmarks

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** M13

Status: required qualification design

<!-- toc -->

- [1. General metrics](#1-general-metrics)
- [2. Streaming video / huge sequential file](#2-streaming-video--huge-sequential-file)
- [3. Git-scale working trees](#3-git-scale-working-trees)
- [4. LLM model loading](#4-llm-model-loading)
- [5. Model copy/variant experiments](#5-model-copyvariant-experiments)
- [6. Sealed content](#6-sealed-content)
- [7. Training/checkpoint publication](#7-trainingcheckpoint-publication)
- [8. Dataset workloads](#8-dataset-workloads)
- [9. Cache-pressure matrix](#9-cache-pressure-matrix)
- [10. Cross-filesystem comparison](#10-cross-filesystem-comparison)

<!-- /toc -->

## 1. General metrics

Every workload records at least:

- elapsed time
- CPU user/system time where available
- peak and steady-state RAM
- metadata-cache RAM
- block reads/writes
- bytes read/written
- flush/barrier count
- read/write amplification
- extent count and fragmentation
- page faults/mmap load latency where the host exposes them

## 2. Streaming video / huge sequential file

Datasets:

- 1 GiB
- 100 GiB sparse/real where practical
- 1 TiB sparse geometry test

Operations:

- sequential read
- sequential write
- append
- seek + resume
- read with SEQUENTIAL
- read with NO_REUSE
- read with DONT_NEED trailing window
- direct I/O where supported

Measure throughput, CPU/GB, metadata writes/GB, cache pollution, and extent growth.

Success target: throughput should approach the underlying block backend/device while metadata overhead remains negligible relative to payload size.

## 3. Git-scale working trees

Trees:

- 100k files
- 1M files
- 4M files

Size distributions:

- 0 bytes
- 100 bytes
- 1 KiB
- 4 KiB
- 16 KiB
- mixed realistic source-tree distribution

Operations:

- checkout/create tree
- Git status cold
- Git status warm
- status with persistent change feed integration
- modify 1, 10, 100, 10k files
- rename directories
- create/remove untracked files
- branch/worktree-style clone experiments
- package-cache extraction

Compare:

- normal traversal
- global catalog where enabled
- change stream
- batch stat/lookups
- tiny-file format candidates

Correctness gate: no optimization may make untracked/changed-file discovery stale.

## 4. LLM model loading

Model-file shapes:

- single 1 GiB model
- 8 GiB model
- 64 GiB sparse/real where practical
- multi-shard models
- model larger than configured memory/cache budget

Operations:

- mmap cold start
- mmap warm start
- random tensor-range access
- sequential layer-style access
- parallel page-fault/read workers
- pread/range-read backend
- Direct I/O where supported
- WILL_NEED/prefetch ranges
- NO_REUSE access

Measure:

- time to first usable page/tensor
- time to touch selected percentages of model
- page faults
- host cache footprint
- CPU/GB
- filesystem metadata reads
- extent count

The FUSE implementation must be benchmarked separately from native/block-core paths because mmap behavior may differ.

## 5. Model copy/variant experiments

Operations:

- normal copy 1/8/64 GiB
- CloneFile same sizes
- clone then modify 4 KiB
- clone then modify 1 MiB
- clone then rewrite entire file

Record physical bytes written and fragmentation.

The benchmark must clearly show where reflink helps and where a full rewrite removes the benefit.

## 6. Sealed content

Compare sealed vs ordinary immutable/read-mostly files for:

- repeated antivirus/security lookup
- content fingerprint reuse
- backup/indexer repeated runs
- reflink clone identity reuse
- ordinary read/mmap performance

Sealing must not add meaningful steady-state read overhead.

## 7. Training/checkpoint publication

Checkpoint sets:

- 2 x 1 GiB shards + manifest
- 8 x 8 GiB sparse/real as practical

Compare:

- naive independent rename publication
- AtomicBatch publication
- preallocated vs non-preallocated writes
- sealed-after-publication

Inject crashes at every publication boundary and verify only allowed complete checkpoint generations are visible.

## 8. Dataset workloads

Test both extremes:

- few huge shard/archive files
- millions of independent small samples

Measure random sample latency, sequential scan throughput, metadata cost, cache pollution, and catalog/change-stream value.

## 9. Cache-pressure matrix

Repeat representative streaming/Git/LLM workloads with deliberately constrained filesystem metadata caches:

```text
64 KiB
256 KiB
1 MiB
16 MiB
unbounded/reference
```

Also constrain host page cache where practical.

This validates that large model/video reads do not catastrophically destroy metadata performance and that the low-memory profile remains real.

## 10. Cross-filesystem comparison

When practical, run equivalent workloads on:

- APFS
- XFS
- ext4
- Btrfs
- NTFS/ReFS where a suitable Windows runner exists
- PFS3/classic Amiga for low-memory historical comparison where applicable

Do not claim a win unless the test methodology and platform differences are documented.

AFS+ should target a strong Pareto position across latency, throughput, CPU, RAM, and write amplification rather than cherry-picking one fastest number.
