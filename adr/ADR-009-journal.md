# ADR-009: Metadata journaling

Status: Accepted

## Decision
AFS+ uses metadata transactions and a redo journal. Full file-data journaling is not mandatory.

## Rationale
The filesystem must recover quickly and consistently from crashes without multiplying data writes unnecessarily.
