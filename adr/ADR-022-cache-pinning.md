# ADR-022: Explicit cache-page pinning and stale-reference detection

Status: Accepted

## Context

Recent pfs3aio fixes exposed catastrophic bugs caused by retaining pointers to cached metadata while nested operations could trigger cache misses and evict/reuse the underlying block. Effects included allocator bitmap corruption and freeing live metadata.

AFS+ explicitly targets bounded caches and low-memory systems, so cache eviction will be common and must not rely on informal coding discipline.

## Decision

The portable core cache API must provide explicit page handles and pinning.

Rules:

- raw metadata views are valid only while their page handle is pinned or while the call contract guarantees no cache allocation/eviction
- APIs that can trigger I/O/cache misses are marked/documented accordingly
- debug builds track page generation tokens
- reuse/eviction changes the generation so stale handles are detectable
- nested operations may not silently invalidate a caller's pinned page
- tests run with deliberately tiny cache budgets to force worst-case eviction

## Consequences

The implementation may be slightly more verbose than passing raw block pointers everywhere, but an entire class of high-impact use-after-eviction filesystem bugs becomes testable and structurally harder to create.
