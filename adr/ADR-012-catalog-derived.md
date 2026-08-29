# ADR-012: Global catalog is derived, not authoritative

Status: Accepted

## Decision
The optional catalog can be discarded and rebuilt from object/directory metadata.

## Rationale
Older writers can safely ignore it, catalog corruption cannot strand user data, and low-resource implementations remain viable.
