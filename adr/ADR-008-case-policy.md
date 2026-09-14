# ADR-008: Case behavior is configurable

Status: Accepted

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

Identification version 3 records the comparison-key algorithm and Unicode
table version. New case-sensitive volumes use Unicode 16 NFC; insensitive
volumes use canonical decomposition, Unicode 16 full default case folding and
NFC recomposition. Original UTF-8 spelling remains in the leaf value. New AROS
qualification images select the insensitive policy, while the general mkfs
default remains sensitive unless explicitly requested.

Version-1/2 prototype images retain their byte-identity behavior through an
explicit legacy algorithm. The optional per-directory override and the final
epoch-1 table choice remain pending.
