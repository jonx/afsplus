# Core Invariants

A conforming implementation must enforce these invariants.

## Allocation

- every allocated physical block is owned by exactly one live allocation, unless an active shared-block feature explicitly changes this rule
- no free bitmap bit may mark a reachable authoritative metadata block as free
- no extent may exceed volume bounds
- extent logical ranges for a file do not overlap

## Objects

- object ID zero is invalid
- root object exists and is a directory
- live hard-link count matches reachable directory references, subject to orphan semantics
- object IDs do not change on rename

## Directories

- every directory key is correctly normalized for that directory's case policy
- directory tree keys are ordered
- no directory entry points to an invalid object
- `.` and `..` are namespace conveniences, not required disk entries

## Journal

- transaction sequence is monotonic
- only committed transactions affect recovered authoritative state
- replay is idempotent or otherwise safely detectable

## Catalog

- catalog is never authoritative
- catalog generation mismatch disables the fast path
- catalog records may not cause allocation or object lifetime decisions

## Checksums

- corrupted metadata is not silently accepted
