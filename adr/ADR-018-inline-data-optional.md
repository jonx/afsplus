# ADR-018: Tiny-file storage optimization

Status: Reopened

## Context

Modern development trees contain huge numbers of small files. The initial AFS+ draft proposed an optional inline-data feature.

Michiel Pelt's PFS4 design notes independently identified automatic grouping of small files as an important performance and space improvement. That makes the problem more important, but does not prove that inline data is the best mechanism.

## Decision

AFS+ reserves an extension path for tiny-file optimization, but does not select its on-disk representation before benchmarking.

The prototype must compare:

1. inline payload in object metadata
2. packed small-file slabs/containers
3. ordinary extents with strong allocation locality

Measure at least:

- disk overhead
- metadata write amplification
- create/delete performance
- random tiny-file read performance
- peak RAM
- crash/recovery complexity
- reclaim/compaction cost

## Consequences

No `inline-data` feature may be marked stable or active in the epoch-1 format until the benchmark and recovery comparison is complete.

Minimal readers must not be forced to implement an unproven tiny-file format.
