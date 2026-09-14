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
- [Per-command host accounting](#per-command-host-accounting)
- [Phased requested-heap workload](#phased-requested-heap-workload)
- [Tree-cache batch measurements](#tree-cache-batch-measurements)
- [Phase-boundary resident memory and repeated reads](#phase-boundary-resident-memory-and-repeated-reads)
- [Allocation origins and instrumentation cost](#allocation-origins-and-instrumentation-cost)
- [Borrowed payloads in atomic batches](#borrowed-payloads-in-atomic-batches)
- [Stage A accounting acceptance](#stage-a-accounting-acceptance)

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

## Per-command host accounting

Use [measure-command.py](../tools/measure-command.py) on macOS or Linux:

```sh
python3 tools/measure-command.py --output /tmp/afsplus-command-run.json -- command argument
```

The report path must be new and its parent must exist. The wrapper reserves it
before launching the command, passes arguments without shell interpretation,
disables interactive standard input and inherits standard output/error. It stores
argv, working directory, platform identity, monotonic wall duration, per-child
user/system CPU time, raw peak RSS and normalized bytes in schema version 1 JSON.
It does not collect environment values. Command arguments belong to the private
benchmark artifact and must follow the same confidentiality rules as test logs.

macOS reports raw RSS in bytes; Linux reports KiB. Other platforms refuse before
launch until their units and accounting semantics are qualified. CPU and RSS are
the OS wait4 child-accounting values; they do not establish simultaneous peak RAM
across a process tree, allocator/cache ownership, steady-state memory or physical
block traffic. Combine the record with component-owned I/O counters and workload
operation/byte denominators. Run the built workload executable to separate build
cost from runtime cost when reporting filesystem performance.

A nonzero exit or signal retains its status and makes the wrapper fail. Launch or
measurement errors produce an explicit error outcome with no invented metrics.
An absent, incomplete or malformed report cannot certify measurement completion.
Run `python3 tools/test-measure-command.py` for private temporary-fixture tests.

## Phased requested-heap workload

Build and run the host-only [afsplus-measure](../crates/afsplus-measure/src/main.rs)
executable for the deterministic `small-files-v1` workload:

```sh
cargo build -p afsplus-measure --offline
target/debug/afsplus-measure
python3 tools/test-measure-workload.py
```

The default workload accepts no arguments. The executable accepts no device
paths. Its fixed 16 MiB memory
image is allocated before sampling, with 4096-byte blocks, 256-block regions,
eight intent-log slots, shared extents enabled, data policy disabled and sensitive
names. A fixed UUID and operation timestamps make the image repeatable. The core
uses its default runtime settings. This workload has no configurable cache cap.

The operation ladder creates sixteen 6000-byte files, writes across a block
boundary in eight files, truncates eight files, renames one and deletes one.
Separate phases measure format, initial mount, creation, edits, sync, unmount,
raw checker, remount, exact namespace/content verification, final unmount and
recovered checker. Both complete checker reports must be clean and the exact
fifteen-file result must match before a success report is emitted. An assertion
failure exits unsuccessfully; partial output cannot establish qualification.

Schema version 1 JSON reports each phase's:

- requested Rust heap at entry and exit, peak live requested bytes, peak above
  entry, acquired bytes and released bytes;
- elapsed monotonic time, successful logical block reads/writes, bytes and
  flush calls;
- explicit application payload read/write denominators, with zero indicating
  that a corresponding amplification ratio is undefined.

Acquired/released bytes include successful realloc growth/shrink deltas. Failed
allocation requests leave the counters unchanged. At quiescent phase boundaries,
`end - start = acquired - released`. Exit bytes describe memory retained at that
boundary. A peak above entry describes additional live requested bytes during
the phase; it cannot identify ownership when allocations replace older objects.
The last edit transaction's bitmap payload peak is reported separately from the
whole-process heap measurements.

The image and preallocated result-row storage stay live across all samples. The
block provider and I/O counters allocate no memory while servicing requests;
there is no growing trace log. Samples include filesystem allocations and small
workload allocations such as formatted names. JSON formatting occurs after all
samples. The image CRC32C supports deterministic-output regression checks; source
and artifact identity require the benchmark bundle's cryptographic hashes.

The [allocator forwarding boundary](../crates/afsplus-measure/src/heap.rs) is
confined to this host executable's crate. It forwards valid requests to Rust's
`System` allocator and updates atomic integers without allocation, formatting,
locking or unwinding in callbacks. Other filesystem crates keep their unsafe-code
prohibition. Phase reset and sampling require a single-threaded quiescent
workload. Internal allocator fragmentation, moving-realloc copy peaks, direct C
allocation, stack, mappings and OS cache memory are outside these counters.

Pair the executable with [per-command accounting](#per-command-host-accounting)
for CPU and process peak RSS. Those process totals include fixture setup, final
image checksum and JSON output; they cannot attribute CPU time to individual
phases. Steady RSS, individual cache ownership, other
workload families, constrained cache profiles and native resource qualification
require their own measurements. A 16 MiB fixture allocation is host test overhead
and does not establish a classic-system memory requirement.


## Tree-cache batch measurements

Run the same wide-name workload with each staged-tree profile:

```sh
cargo build -p afsplus-measure --offline
target/debug/afsplus-measure --cache-profile 2
target/debug/afsplus-measure --cache-profile 4
target/debug/afsplus-measure --cache-profile 8
target/debug/afsplus-measure --cache-profile unlimited
python3 tools/test-measure-workload.py
```

The [tree-cache-batch-v1 workload](../crates/afsplus-measure/src/cache_workload.rs)
creates 192 files with 185-byte names and seven-byte contents in one transaction,
deletes odd-numbered files in another, then verifies all 96 surviving files and
both full checker views across remount. Formatting uses the fixed-memory geometry
and feature settings of the small-file workload. The profile is applied before
both mounts. Workload input strings and batch vectors are included in each
phase's requested heap and released before its exit sample.

The version-1 JSON report adds `cache_pages` and `tree_phases` for creation and
deletion. These retain spill writes, reloads, pre/post-eviction staged-entry
peaks, decoded-node peaks and total metadata write requests. Each constrained
creation must cause real spills. Every commit must reconcile its byte/flush
accounting with the provider counters. Zero payload modification gives an
undefined amplification ratio for deletion; report its absolute I/O cost.

Compare the same phase across profiles using requested heap, total I/O and
payload denominators. Pair separate process runs with the CPU/RSS collector.
A smaller staged cache can add repeated I/O, and total heap can be dominated by
batch overlays and pending publication buffers. Report those costs together.
The executable accepts only the four named measurement profiles; the underlying
Rust mount option accepts any nonzero count. These are host memory-image results;
physical storage latency and classic-machine resource qualification need their
own measurements.

## Phase-boundary resident memory and repeated reads

The host-only workload accepts `--resident-rounds N`, with N from 3 through 32,
optionally combined with `--cache-profile 2|4|8|unlimited`. Default commands retain
their version-1 reports and perform no RSS probes. Opting in produces version-2
reports with whole-process RSS at the beginning and end of every phase and a
post-warmup series of repeated reads against unchanged, previously verified files.
For example:

```sh
target/debug/afsplus-measure --cache-profile 2 --resident-rounds 16
```

The `ps-rss-kib-v1` provider runs the fixed `/bin/ps -o rss= -p SELF` command
without a shell or inherited environment, selecting only the still-running
workload process. It normalizes units using the [Apple ps manual](https://raw.githubusercontent.com/apple-oss-distributions/adv_cmds/main/ps/ps.1)
and [Linux procps manual](https://man7.org/linux/man-pages/man1/ps.1.html):
RSS is reported in 1024-byte units. These provider contracts were reviewed on
2026-09-14. The implementation uses safe Rust process APIs; it introduces no new
unsafe ABI boundary. Other target OSes refuse this option before workload I/O.
Empty, ambiguous, zero, overflowing and failed provider results refuse a successful
report. Only macOS execution is currently qualified; Linux units are documented,
but Linux runtime qualification remains open.

RSS probes occur outside each phase's wall-time and requested-heap interval.
Their durations are separately reported as `resident_start_probe_wall_ns` and
`resident_end_probe_wall_ns`; sampling still affects process scheduling and
allocator history. Whole-command CPU/time accounting includes observer overhead.
Use an uninstrumented companion run for performance conclusions. Resident values
include process runtime, mappings, shared pages, the fixture and verification
state; they are not private heap, filesystem-owned cache memory, a process-tree
sum or a measurement of all physical memory consumed on behalf of the process.
Do not subtract the nominal 16 MiB image allocation from RSS: allocated virtual
bytes need not be resident.

After initial exact read verification, `steady-prepare` captures the expected
names, IDs and bytes in a separately measured phase. Each `steady-read` repeats
the same enumeration and exact byte comparison; checkpoint generation must remain
unchanged and writes/flushes must be zero. `steady-release` measures disposal of
the retained oracle. All phases retain I/O and requested-heap accounting, so the
oracle's setup, retained storage and release are visible. Report storage is
reserved before measurement and formatting occurs after sampling.

The `steady_read` summary reports round count and first/last/minimum/maximum end
RSS. `plateau_verified` remains false: a bounded stationary-work series measures
resident behavior, not an automatically proven plateau, leak absence or long-term
resource bound. Keep the raw series, per-command report, executable/provider
hashes and measured-source hashes in private artifacts. Mixed mutations, aged and
near-full workloads, cache ownership, sustained duration and native/constrained
qualification remain separate resource gates.

`tools/test-measure-workload.py` compares enabled/disabled image CRCs and original
phase I/O across the small-file workload and all four cache profiles, checks
read-only repeated phases, oracle heap balance, RSS series and argument refusals.
The resident parser unit test covers missing/ambiguous values and overflow.
A macOS 16-round observation measured end-RSS ranges of 5,177,344–5,210,112 bytes
for small files, and respectively 9,142,272–9,191,424; 9,191,424–9,224,192;
9,158,656–9,191,424; and 9,306,112–9,338,880 bytes for cache profiles 2, 4, 8 and
unlimited. These are one host observation with instrumentation and an in-memory
fixture, not portable memory requirements or evidence that one cache policy wins.


## Allocation origins and instrumentation cost

Build the optional host meter with `cargo build --offline -p afsplus-measure
--features allocation-domains`. Keep a separate copy of that executable before
building the ordinary meter without features. Run
`python3 tools/test-allocation-origins.py --tagged /path/to/tagged-measure
--baseline /path/to/ordinary-measure` to compare both variants across the small
file workload and the 2/4/8/unlimited cache profiles.

The feature emits schema version 3 with allocation profile
`requested-origins-v1`. Each phase includes `allocation_origins`,
`tracking_overhead` and `underlying_requests`; each contains entry, exit, peak,
acquired and released requested bytes. The nine origins are:

| Origin | Allocation context |
|---|---|
| other | Unclassified requests, including setup outside explicit scopes |
| fixture | Fixed memory image storage |
| reporting | Preallocated phase report storage |
| allocator | Transaction allocator begin, allocation, retirement and finish |
| tree | Staged tree mutation, including opaque container storage |
| batch | Batch work outside a more specific nested scope |
| snapshot | Snapshot-accounting initialization |
| verifier | Reachable-state loading and explicit workload verification |
| oracle | Retained expected contents for repeated-read verification |

An allocation keeps its original context through successful reallocations and
freeing, even on another thread or after a container moves. This is **allocation
origin, not current ownership**. Nested scopes temporarily override outer scopes;
uninstrumented work inherits the enclosing context or `other`. A backend that
allocates inside a core call can inherit that call's context. The fixed image
backend performs no allocation in its block read/write methods. Snapshot context
coverage does not imply that all snapshot-related work is independently measured.

The optional allocator stores an aligned private header before every payload.
`tracking_overhead` counts its header and padding; `underlying_requests` counts
the extended requests sent to the underlying allocator. At quiescent entry/exit
boundaries, origin bytes sum to requested payload bytes, and payload plus tracking
overhead equals underlying requested bytes. Independent origin peaks need not
occur together and must not be summed as a simultaneous process peak. Failed
requests preserve existing bytes and counters. Allocation callbacks use fixed
atomic counters and non-dropping thread-local context without allocating a
separate tracking table. Alignment and size-overflow handling, failed growth,
zeroing, cross-thread free and scope unwinding have allocator regression tests.

The instrumentation changes heap layouts, timing and potentially resident memory.
Compare ordinary and tagged executables using identical workloads, image hashes
and I/O denominators; retain their separate executable hashes. Header/padding
accounting does not measure the underlying allocator's private metadata, internal
reallocation-copy transients, C allocations, mappings or stack. RSS sampling is
an independent observation and can be combined with the tagged mode. Sampling
and peak resets require quiescent single-threaded phase boundaries; thread-safe
counters do not establish an atomic parallel-workload snapshot. Default builds
retain the ordinary allocator and have no allocation-context TLS observation.

The integration test checks unchanged image CRCs and I/O, per-origin balance,
fixture separation, release of mutation/verifier state and oracle lifetime across
repeated reads. Broader ownership accounting, sustained mixed workloads and native
resource qualification remain separate acceptance requirements.


## Borrowed payloads in atomic batches

`Volume::run_batch` validates its operations and retains references to the
surviving creates' caller-owned bytes until the synchronous commit returns.
Object identity selects surviving creates after cancellation or replacement,
including reuse of a cancelled allocation or name. The operation limit bounds
the descriptor count; it is not a limit on all transaction memory. Plain batches
do not retain a second heap block image for each payload block. Full blocks pass
directly to the device; one reusable zero-padded block handles partial tails.
The existing common commit tail issues the same data writes and data barrier
before metadata publication. Intent-log windows retain their separate operation-time
write-through behavior and retention rules.

The caller must keep input bytes alive for the synchronous call, as expressed by
the existing borrowed API. Caller storage is part of process memory and is not
eliminated by avoiding internal copies. Pending namespace state, encoded object
records, tree operations, allocator state and other mutation families have their
own resource costs. This path is not a whole-transaction memory cap or a native
low-memory qualification.

Run `cargo test --offline -p afsplus-check --test batch_payloads` for full-block
borrowing, mixed aligned/partial/empty files, tail zeroing, cancelled-name reuse,
write/data-barrier refusal and same-mount retry. The four cache profiles must
retain exact namespace and bytes after remount. A complete full-write-subset and
representative-tear matrix checks every recorded boundary for a small mixed-payload
batch, requiring both exact old and exact new outcomes. Invalid and fully cancelled
small batches must issue no payload writes or flushes.

Compare the tagged meter's fixed 192-file batch with a retained pre-change
executable. Require unchanged image CRC, per-phase I/O and payload denominators.
Record allocation-origin and total requested-heap peaks separately. Retain
per-child CPU and RSS observations for both variants, with repetition count,
variant order and concurrent host load stated. The
[origin integration test](../tools/test-allocation-origins.py) bounds batch-origin
peak below 1 MiB for this particular fixture across 2/4/8/unlimited profiles;
this regression budget includes its pending metadata and does not apply to
arbitrary batches. Broader metadata scaling and constrained-platform budgets
remain requirements of the complete resource-accounting gate.


## Stage A accounting acceptance

The finite executable-core accounting gate requires a working measurement
harness and qualified image workloads. It does not require every later application
workload, platform adapter or production memory limit to be qualified. Preserve
those requirements under the full benchmark contract and M12/M13/M14.

| Required dimension | Measurement and acceptance evidence |
|---|---|
| CPU | [Per-command accounting](#per-command-host-accounting) records child user/system CPU and elapsed time, preserving exit/failure information; its temporary-fixture gate checks failures and units. |
| Process RAM | Per-command peak RSS plus [phase-boundary RSS and repeated reads](#phase-boundary-resident-memory-and-repeated-reads), with explicit observer timing and no unsupported plateau claim. |
| Requested heap | [Phased workload](#phased-requested-heap-workload) and [origin accounting](#allocation-origins-and-instrumentation-cost) report entry/exit/peak/acquire/release balance, checker costs, fixed fixture storage and instrumentation overhead; allocator-internal overhead, stack and direct C allocation are not mislabeled as Rust payload. |
| Cache/resource policy | [Batch profiles](#tree-cache-batch-measurements) bind 2/4/8/unlimited settings, spills/reloads and resident staged-node limits; an image-page cap is distinguished from a whole-heap cap. |
| I/O and flushes | Phase counters and commit accounting agree on logical bytes, operations, spill writes and barriers. |
| Amplification | Explicit application read/write payload denominators accompany physical block-byte counts; zero denominators remain undefined rather than becoming invented ratios. |
| Correctness and repeatability | Exact workload namespace/content verification and clean raw/recovered checkers precede success; independent repetitions retain matching image CRCs and I/O. Tagged/ordinary comparisons and a [memory-budget negative control](#borrowed-payloads-in-atomic-batches) detect the relevant regression. |

The [ordinary workload tests](../tools/test-measure-workload.py),
[origin tests](../tools/test-allocation-origins.py) and
[command-accounting tests](../tools/test-measure-command.py) own these checks.
Resource optimization, current ownership after object transfer, arbitrary mixed
workload duration and native hardware budgets remain separate requirements.
Completion of this harness item cannot close their milestone gates.
