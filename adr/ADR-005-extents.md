# ADR-005: Extent-based file allocation

Status: Accepted

## Decision
Files map logical ranges to physical ranges using extents, with inline extents and overflow trees.

## Rationale
This scales better than per-block pointer chains and is simpler for large sequential files.
