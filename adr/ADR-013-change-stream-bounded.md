# ADR-013: Change stream is bounded and may require rescan

Status: Accepted
Amended by: ADR-103

## Decision
Old change records may be discarded. API consumers receive `RESCAN_REQUIRED` when history is no longer available.

## Rationale
A permanently growing journal is not sustainable.
