# ADR-024: Rebuildable reverse physical-to-owner map

Status: Proposed

## Context

Forward metadata answers where an object's data lives. Repair and diagnostics often need the reverse question:

> Who owns physical block or extent X?

XFS reverse mappings are a major enabler for targeted online repair, cross-link detection, ownership validation, and safe reaping of old metadata.

AFS+ also wants `ExplainBlock`, targeted scrub, online relocation, and stronger corruption attribution.

## Proposed decision

Define an optional derived reverse-map feature:

```text
physical extent -> semantic owner
```

Owner kinds may include:

- regular file data + object ID + logical offset
- extent-tree metadata + owner object
- directory tree + owner object
- allocation metadata
- catalog
- change stream
- checkpoint/system metadata
- retired/reclamation state

## Rules

- authoritative forward metadata remains the source of truth
- reverse map is checksummed and generation-tagged
- stale reverse maps are never trusted
- it can be rebuilt by scanning authoritative metadata
- implementations that do not support it can still mount according to its compatibility classification
- repair code may use it only after validation or cross-checking

## Benefits

- fast `ExplainBlock`
- targeted scrub
- cross-link detection
- safer old-extent reclamation
- online defragmentation/relocation support
- media-error attribution to files
- repair without always scanning the entire filesystem

## Cost

Maintaining reverse mappings adds metadata writes and implementation complexity.

Before activation in the workstation profile, benchmarks must measure write amplification and space overhead.

If maintenance cost is too high, the feature may remain an offline/generated repair index rather than a continuously maintained production index.