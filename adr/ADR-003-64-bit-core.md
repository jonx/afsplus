# ADR-003: 64-bit quantities in the core format

Status: Accepted

## Decision
Block numbers, object IDs, sizes, offsets, generations, and transaction/change sequence numbers are 64-bit.

## Rationale
The format must not need another redesign for ordinary future storage sizes.
