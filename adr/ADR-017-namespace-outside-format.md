# ADR-017: AROS path namespace stays outside the disk format

Status: Accepted

## Decision
AFS+ stores names and hierarchy only. `SYS:`, Assigns, `PROGDIR:`, POSIX mount paths, and Windows drive syntax are host-layer concepts.

## Rationale
This makes the format portable and avoids encoding one operating system's namespace into disk metadata.
