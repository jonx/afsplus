# ADR-080: Bind PAX completion to a streamed integrity envelope

Status: Superseded
Amends: ADR-076
Superseded by: ADR-081

## Context

A valid tar prefix can contain useful files while omitting the rest of a backup.
ADR-076 requires explicit integrity and completion evidence. Header checksums
cover individual headers; they cannot establish complete payload integrity.
A fixed terminal record can bind the exact preceding stream without retaining
its contents or a full manifest in RAM.

[NIST FIPS 180-4](https://nvlpubs.nist.gov/nistpubs/FIPS/NIST.FIPS.180-4.pdf)
limits SHA-256 messages to fewer than 2^64 bits and the SHA-512 family to fewer
than 2^128 bits. SHA-512/256 retains a 256-bit digest while covering streams
containing full 64-bit byte-sized files and archive overhead. SHA-256 would
require segmentation or a smaller stream limit. A custom hash composition adds
unnecessary protocol complexity here.

## Decision

Use completion envelope version 1 from
[the envelope specification](../spec/backup-envelope.md). Wrap the body in
ordinary PAX global headers with vendor keys: a beginning record identifies
version and hash algorithm; a terminal record binds the exact preceding byte
count, body-header count and SHA-512/256 digest. Count bytes in 128 bits and
refuse lengths outside the standard's message domain. Body-header counts are
64-bit; framing admission includes both control headers.

Hash the beginning control header, its payload/padding and every body header,
payload and padding byte. Exclude the terminal control header and everything
after it from the recorded digest. Require the terminal record, matching counts
and digest, valid tar end markers and actual EOF before issuing an integrity
receipt. Unknown versions/algorithms/fields, duplicate control keys, conflicting
counts and data after completion are errors. No valid prefix alone receives a
receipt.

Use RustCrypto's standard hash implementation, including its portable software
backend. Hardware acceleration is an implementation choice. Constrained builds
can select compact software rounds and smaller I/O buffers; they retain the
same wire digest and verification contract. Measure resource use and actual
target execution independently.

An integrity receipt verifies stream integrity and termination. The preservation
profile and its consumer independently verify metadata requirements, source
enumeration, loss policy, namespace confinement and restore authorization.
An unkeyed digest provides no sender authentication: signatures or a trusted
channel are separate host policy. Restore jobs must not accept archive metadata
as authority merely because the envelope verifies.

## Compatibility and qualification

This defines archive control records and changes no AFS+ disk records, feature
bits, C ABI or native advertisement. Ordinary tar readers can recover body files
while ignoring vendor PAX control metadata; qualify that behavior with independent
readers. The full preservation-profile encoding is a separate decision.

Require exact digest agreement with an independent implementation, body/header
mutation rejection, truncated-prefix rejection, absent/duplicate/altered terminal
records, count mismatch, malformed controls, unsupported versions/algorithms,
trailing data, limits, interrupted I/O and unfinished-output refusal. Verify
identical receipts with default and compact software hash backends. Preserve
explicit failures without declaring a completed backup or restore.
