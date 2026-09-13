# ADR-021: Deferred and resumable reclamation

Status: Proposed

Amended by: ADR-069

## Context

PFS3 persists operation state for long frees so interrupted work can resume safely. Large deletes/truncates can otherwise require oversized transactions, large temporary free lists, or long blocking operations.

AFS+ must remain bounded in RAM and transaction size even when deleting multi-terabyte sparse files or very large directory trees.

## Proposed decision

Separate logical deletion from physical reclamation.

A namespace/object operation commits the user-visible state quickly, then places no-longer-needed extents/metadata onto a persisted reclamation structure.

Reclamation:

- runs in bounded batches
- is idempotent or progress-tracked
- resumes after reboot
- obeys checkpoint-generation quarantine
- may be paused under I/O pressure
- never makes uncertain blocks allocatable

Examples include:

- final unlink of a very large file
- huge truncate
- removal of overflow extent trees
- deletion of obsolete catalog generations
- cleanup of retired metadata trees

## Consequences

Space may be reported as pending reclaim for a short period after deletion.

Filesystem APIs should distinguish immediately available free space from reclaimable/pending capacity where useful.

Repair tools can safely complete or rebuild reclamation state.
