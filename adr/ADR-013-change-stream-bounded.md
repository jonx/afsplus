# ADR-013: Change stream is bounded and may require rescan

Status: Accepted

## Decision
Old change records may be discarded. API consumers receive `RESCAN_REQUIRED` when history is no longer available.

## Rationale
A permanently growing journal is not sustainable.
