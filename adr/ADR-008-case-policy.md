# ADR-008: Case behavior is configurable

Status: Accepted; comparison-key implementation pending

## Decision
AFS+ supports case-sensitive and case-insensitive directories, with a volume
default and an optional per-directory override. Traditional AROS/Amiga
namespaces default to case-insensitive lookup while preserving the creator's
original spelling. Development namespaces may select case-sensitive lookup.

Both policies use a versioned, on-disk comparison key. Case-insensitive lookup
uses the volume-pinned Unicode casefold algorithm and never the host locale.

## Rationale
Traditional AROS usage benefits from insensitive matching while modern
development trees may require sensitive names.

## Implementation status

The executable prototype currently uses the original UTF-8 bytes as an identity
comparison key and is therefore case-sensitive. S1b uses exact source-tree
spelling and records that boundary in ADR-049. This temporary behavior must not
become the AROS profile or be frozen as epoch 1.
