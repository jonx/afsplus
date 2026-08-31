# 20. Performance Architecture

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

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
