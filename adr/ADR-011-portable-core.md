# ADR-011: Portable filesystem core

Status: Accepted

## Decision
Disk-format logic lives in portable `libafsplus`, not directly in DOS-packet code.

## Consequences
The same parser/invariants can power AROS, FUSE, format/check tools, fuzzing, and third-party ports.
