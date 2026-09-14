# ADR-096: Expose bounded block-device slices

Status: Accepted

## Decision

A SliceBackend owns a parent BlockDevice and exposes a fixed, nonempty interval
in parent block units. Construction requires start below parent capacity and
length no greater than capacity minus start. This subtraction-first admission
rejects overflowing intervals without performing parent I/O.

The slice preserves the parent block size. Each read and write validates the
logical block number and exact buffer length before translating to start plus
logical block. Geometry is fixed for the wrapper lifetime. No mutable parent
access is exposed; consuming the wrapper returns ownership of the parent.
Nested slices compose using the same rules and require no allocation or copying
beyond the caller's block buffer.

Flush forwards directly to the parent and propagates its error. The parent's
barrier may cover writes outside the slice; the wrapper promises no independent
partition durability. It introduces no filesystem disk layout, raw-device access,
implicit partition discovery or host authorization. Callers retain responsibility
for choosing an authorized parent provider.

## Qualification

Require first/last block translation, untouched neighboring sentinels, nested
slices, overflowing/out-of-capacity and empty interval refusal, exact-buffer
admission with zero parent calls on error, and unchanged read/write/flush error
propagation. Run an actual format/mutate/remount/check cycle inside a slice before
closing the Stage A partition-view requirement. Physical storage remains separately
qualified through its native provider.
