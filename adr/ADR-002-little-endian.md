# ADR-002: Little-endian explicit disk encoding

Status: Accepted

## Decision
All multi-byte fields are little-endian and decoded explicitly.

## Rationale
Modern AROS targets are commonly little-endian. Classic big-endian systems can still implement conversion cheaply. Explicit encoding avoids native-struct and compiler-packing bugs.
