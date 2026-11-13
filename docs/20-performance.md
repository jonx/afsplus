# 20. Performance Architecture

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

<!-- toc -->

- [1. Performance principles](#1-performance-principles)
- [2. Metadata locality](#2-metadata-locality)
- [3. Small files](#3-small-files)
- [4. Bulk enumeration](#4-bulk-enumeration)
- [5. Incremental indexing](#5-incremental-indexing)
- [6. Write amplification](#6-write-amplification)
- [7. Cache independence](#7-cache-independence)
- [8. Executable Core Scale-1 measurements](#8-executable-core-scale-1-measurements)
- [Cache integration qualification](#cache-integration-qualification)

<!-- /toc -->

## 1. Performance principles

Optimize measured workloads, not hypothetical ones.

Never trade away crash correctness for an unmeasured microbenchmark.

## 2. Metadata locality

Prefer allocating:

- directory metadata
- small child objects
- extent overflow nodes

near related objects when practical.

Locality is a hint, not an invariant.

## 3. Small files

Source trees contain many small files.

Performance techniques:

- inline extents
- optional inline data
- region locality
- compact directory entries
- batch metadata I/O
- avoid unnecessary synchronous journal commits

## 4. Bulk enumeration

When the catalog feature is active, volume-wide enumeration should be dominated by sequential reads and record parsing.

The API remains streaming.

## 5. Incremental indexing

The change stream avoids full rescans after normal application restarts.

## 6. Write amplification

Benchmarks must measure device bytes written for:

- create file
- append
- overwrite
- rename
- unlink
- Git checkout
- Cargo build

Future compression or CoW features must justify their write-amplification behavior.

## 7. Cache independence

Correctness must never depend on an unbounded metadata cache.

A 16 MB cache and a 4 GB cache may have different performance but must produce identical on-disk results.

## 8. Executable Core Scale-1 measurements

The explicit typed-directory qualification inserts 100,000 permuted entries
under an eight-page staged-image budget and then validates them with the
streaming typed visitor. A release run on the development host measured:

| result | measured value |
|---|---:|
| wall time | 7.44 s |
| tree height / nodes | 3 / 2,011 |
| device reads | 172,409 |
| provisional spill writes | 174,411 |
| provisional spill reloads | 172,408 |
| maximum staged full pages | 8 |
| maximum decoded/derived full nodes | 2 |
| process peak RSS | 36.5 MB |

The RSS is deliberately reported rather than inferred from page counters. It
includes the caller-owned 100,000-operation batch, memory backend, compact
per-node overlay index, allocator state, and test harness. The 8+2 counters
measure only full tree-page residency. Constrained mode exchanges I/O for
memory; modern mode keeps dirty images resident and is the performance
default.

The separate 1 TiB sparse-image qualification records format/mount/checker
wall time and per-commit bitmap pages, allocation records, metadata nodes, and
flushes. [`crates/afsplus-check/tests/measurements.rs`](../crates/afsplus-check/tests/measurements.rs) remains the executable source
of those numbers so regressions cannot be papered over by documentation.

## Cache integration qualification

A host cache must preserve the transaction layer's durability barriers and
provide coherent data across buffered and bypass operations. Dirty metadata
versions remain pinned or copied until safe publication; eviction cannot
write a newer uncommitted version into an older durable transaction.
Bound pinned state and provide backpressure. Avoid global cache locks across
blocking I/O, and test progress under contention and failed writeback.

The block read cache
([`CachedDevice`](../crates/afsplus-block/src/cache.rs)) is the first such
cache: a bounded LRU of device blocks that writes through, drops a block
whose write the device refused, and takes its memory when its size is set.
[`read_cache`](../crates/afsplus-vfs/tests/read_cache.rs) runs one workload
at no cache, 16 blocks and an unbounded one and requires the same writes and
barriers in the same order and the same image from all three; the reads fall
from 8,731 to 1,736 to 11.

The cache has two pools with separate capacities. Blocks read one at a
time are metadata and live in the first; runs of blocks
(`BlockDevice::read_blocks`, `write_blocks`) are file data and live in the
second, so reading files cannot evict the trees and records every operation
needs. A write through either call updates its own pool and drops the block
from the other. The AROS boundary gives each pool half of the one cache
setting. A run is one device request for each stretch the data pool does not
hold, straight into the caller's buffer and bounded by the device's maximum
transfer; a run that continues the previous one is read 64 KiB further than
asked, into the pool. The format carries no checksum of file data
([06-files-and-extents.md](06-files-and-extents.md)), so a run is checked
exactly as its blocks were one at a time: not at all.

A cached metadata block may also keep its decoded tree node or object record
beside its bytes; any write of the block, a refused one included, and its
eviction drop it. Tree descents and record reads ask for the kept form first
and read the block's bytes only when there is none: it was checked when it
was decoded, and checking the bytes again would cover only the copy beside
it, not the decoded form that is used. The identity and key range of a node
are still checked on every visit, against the parent it was reached through.
The `verify-cached-metadata` feature of `afsplus-core` reads and checks the
bytes on every reuse, at the cost of a copy and a CRC32C per read, and
turns off the three memories below; it is compiled in or out, so the default
build carries no test of it. A kept form damaged in memory could steer a
lookup that a change then follows. Having every change find its objects
through checked bytes is a further step, not taken.

Three more things are remembered, each built so that it cannot describe
anything but the bytes it was read from:

| What | Where it lives | Why it cannot be stale | Memory |
|---|---|---|---|
| Which entries of a directory leaf were checked in full | with the kept leaf node, in the cache slot | dropped with the node by any write or eviction of the block | one bit an entry |
| Object ID to record block, or "no such object" | a two-way table in the volume | named by the committed checkpoint generation and object-map root; a committed tree never changes, and under any other pair the table is empty | 16 bytes for each metadata cache block, at most 1 MiB |
| That the orphan directory is empty | one field in the volume | named by the same pair | 24 bytes |

The record itself is never copied into the table: a hit takes it from the
cache slot of its block. A debug build makes the full descent on every hit
of the last two and requires the same answer, so every test that reads an
object checks them;
[`cached_reads`](../crates/afsplus-vfs/tests/cached_reads.rs) compares all
the mount answers with a fresh mount of a copy of its device after every
kind of change. The hot-path figures are those of
[`zz_hotpaths`](../crates/afsplus-aros-ffi/tests/zz_hotpaths.rs) and
[`zz_dataio`](../crates/afsplus-aros-ffi/tests/zz_dataio.rs).

On a mount that delays its commits the AROS handler asks the library what
is due only after a batch that held a packet which may change the volume,
while the library reports work pending, and once in 64 batches otherwise
([`afsplus_commit_tick.h`](../native/aros/afsplus_commit_tick.h)); a batch
of reads reads no clock and makes no call.

Native VM integration must state which callbacks may allocate or page-fault
and test reclaim reentrancy under memory pressure. The
[book review](33-practical-filesystem-design-review.md) and
[qualification plan](../testing/book-review-qualification.md) own these
integration experiments. Choose cache size, read-ahead and bypass thresholds
from target workloads rather than importing the historical BFS constants.
