# ADR-032: Advisory access-intent hints

Status: Proposed

## Context

Different workloads stress storage in opposite ways. Video streaming prefers aggressive sequential readahead and low cache retention. Git prefers metadata locality and tiny-file efficiency. LLM runtimes frequently use mmap or range reads against very large mostly immutable files.

Existing operating systems expose advisory interfaces such as sequential/random/will-need/don't-need/no-reuse hints. AFS+ should map cleanly to those concepts without baking application-specific behavior into the on-disk format.

## Proposed decision

Filesystem API v2 exposes advisory per-handle/per-range access intent.

Initial semantic set:

- NORMAL
- SEQUENTIAL
- RANDOM
- WILL_NEED
- DONT_NEED
- NO_REUSE
- LATENCY_SENSITIVE
- BULK_THROUGHPUT
- MMAP_EXPECTED
- DIRECT_IO_PREFERRED
- TEMPORARY
- IMMUTABLE_EXPECTED

Hints:

- never change correctness semantics
- never weaken durability/security
- may be ignored
- are not persistent disk-format metadata by default
- can affect readahead, cache retention, preallocation strategy, writeback grouping, and host I/O selection

## Consequences

The same API can benefit media players, backup tools, databases, compilers, Git, model runtimes, dataset pipelines, and filesystem benchmarks.

No hint may create a required epoch-1 on-disk feature merely because one host implements an optimization for it.
