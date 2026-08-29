# 20. Performance Architecture

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
