# ADR-107: Timestamps are twelve bytes on the wire

Status: Accepted

## Context

The format header declared `struct afsp_timespec_wire` as 16 bytes: signed
64-bit seconds, 32-bit nanoseconds and four reserved bytes. Every codec, both
readers and every image wrote and read 12 bytes, three of them in 36 bytes of
the object record. Nothing referenced the structure, so the disagreement went
unnoticed until a test pinned the header against the codecs.

## Decision

A timestamp is 12 bytes: little-endian signed 64-bit seconds since
1970-01-01 00:00:00 UTC, then little-endian unsigned 32-bit nanoseconds below
one billion. There is no padding and no reserved field. The header structure
states those 12 bytes.

Signed 64-bit seconds cover every representable time and 32-bit nanoseconds
cover the full sub-second range, so reserved bytes would buy nothing. They
would cost 12 bytes in every object record and move every field after the
first timestamp, in a record whose layout both readers, the intent log and
every fixture already share.

## Consequences

- The wire is unchanged; the header is corrected.
- `crates/afsplus-format/tests/c_constants.rs` holds
  `sizeof(struct afsp_timespec_wire)` equal to the Rust `Timespec::WIRE_SIZE`.
- A future need for finer resolution or an additional time is a new field
  behind an object flag, under [ADR-100](ADR-100-exact-object-record-admission.md).
