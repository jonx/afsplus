# Security and Antivirus Benchmark Plan

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** M10

Status: qualification requirement

<!-- toc -->

- [1. Purpose](#1-purpose)
- [2. Baseline scenarios](#2-baseline-scenarios)
- [3. Workloads](#3-workloads)
  - [Idle/read-heavy clean workstation](#idleread-heavy-clean-workstation)
  - [Save/build workload](#savebuild-workload)
  - [Massive tree update](#massive-tree-update)
  - [Clone workload](#clone-workload)
  - [Execution gate](#execution-gate)
  - [Event-overflow/backpressure](#event-overflowbackpressure)
- [4. Metrics](#4-metrics)
- [5. Key ratios](#5-key-ratios)
- [6. Scale matrix](#6-scale-matrix)
- [7. Correctness tests](#7-correctness-tests)
- [8. Acceptance direction](#8-acceptance-direction)

<!-- /toc -->

## 1. Purpose

AFS+ claims that first-class content generations, persistent change streams, object enumeration, and generation-stable scan handles can reduce the resource cost of antivirus and related whole-filesystem monitoring.

That claim must be demonstrated experimentally.

## 2. Baseline scenarios

Measure at least:

1. scanner disabled
2. scanner implemented as naïve recursive path walk
3. scanner consuming transient notifications only
4. scanner consuming AFS+ persistent content-inspection feed
5. scanner with execution authorization enabled

Where practical, compare equivalent host mechanisms such as fanotify/minifilter/Endpoint Security based scanners, while clearly separating OS overhead from filesystem overhead.

## 3. Workloads

### Idle/read-heavy clean workstation

- 1 million files already scanned
- repeated application launch
- repeated source-tree traversal
- Git status/build/editor indexing
- no content changes

Primary target: monitoring overhead should approach zero when content generations do not change.

### Save/build workload

- editor repeatedly rewrites source/config files
- compiler creates temporary files and outputs
- package manager extracts many small files

Measure event coalescing and number of actual rescans.

### Massive tree update

- 100k files changed
- scanner stopped during update
- restart scanner from persistent cursor

Compare catch-up cost with full tree rescan.

### Clone workload

- scan large file A
- CloneFile(A, B)
- verify whether scanner can reuse compatible content verdict without rereading data
- COW-modify B
- verify only B's content generation becomes untrusted

### Execution gate

- execute known-clean unchanged binary
- execute newly created binary
- execute modified binary

Measure p50/p95/p99 added launch latency.

### Event-overflow/backpressure

Artificially slow the scanner while generating events faster than it can consume them.

Verify persistent cursor catch-up avoids a full rescan while retained history remains available.

## 4. Metrics

Record:

- scanner CPU time
- filesystem CPU time attributable to monitoring
- peak and steady RAM
- events emitted
- events consumed
- events coalesced
- files opened for scanning
- total file-data bytes reread by scanner
- namespace operations issued by scanner
- block reads caused by scanning
- cache pressure caused by scanning
- application throughput impact
- p50/p95/p99 open latency
- p50/p95/p99 execution latency
- time to catch up after scanner downtime
- energy/power when measurable on supported hosts

## 5. Key ratios

Useful derived values:

```text
bytes_scanned / bytes_content_changed
files_rescanned / files_content_changed
security_events / committed_content_generations
monitoring_cpu / workload_cpu
added_launch_latency / baseline_launch_latency
```

For an unchanged, already-scanned workload, the ideal first two ratios approach zero.

## 6. Scale matrix

Run at least:

```text
files: 10k, 100k, 1M, 4M
cache: tiny, constrained, workstation
file sizes: 0B, 1KiB, 4KiB, 64KiB, 1MiB, large mixed
change rates: 0%, 0.01%, 1%, 10%, 100%
```

## 7. Correctness tests

Performance optimization may never cause missed content changes.

Tests must include:

- rename without content change
- metadata-only change
- truncate
- mmap-like/write-through paths exposed by AROS APIs
- reflink clone
- CloneRange
- hard links
- atomic replace
- crash between content write and checkpoint commit
- persistent change-stream retention expiry
- scanner crash/restart
- scanner signature-set/version change requiring verdict invalidation
- object deletion while scan handle is open

## 8. Acceptance direction

No fixed performance number is frozen yet.

The intended qualitative result is:

- unchanged content causes negligible scanner I/O
- monitoring overhead scales primarily with committed content changes, not total opens
- catch-up scales with changes since cursor, not total namespace size
- valid clone content can reuse scan work where policy permits
- execution authorization of known-clean content is a cheap cache lookup, not a rescan

Release claims must quote the benchmark workload and hardware rather than using an unqualified "faster antivirus" statement.
