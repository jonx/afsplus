# ADR-006: Region-based allocation

Status: Accepted

## Decision
Free-space bitmaps are partitioned into fixed-size allocation regions with local summaries.

## Rationale
This bounds memory, limits repair scope, improves locality, and keeps large volumes practical on constrained systems.
