# ADR-015: Filesystem and repair tools share validation code

Status: Accepted

## Decision
`afsplus-check` uses the same portable format decoders and invariant checks as the main core.

## Rationale
Separate filesystem and fsck implementations tend to drift and disagree.
