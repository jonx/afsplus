# ADR-098: Bind persistent block replay traces to their base image

Status: Accepted

## Decision

The host qualification harness serializes recorded writes and flushes in a
versioned, checksummed artifact. It is independent of the filesystem disk format.
Replay requires matching block geometry and a SHA-256 identity of the complete
logical base image, computed over a domain prefix, geometry and block bytes in
address order. The caller explicitly bounds base blocks before this exhaustive
read; ordinary mount never performs it.

Version 1 starts with eight bytes `AFSTRC00`, followed by little-endian version
u32 (1), block size u32, block count u64, operation count u64 and base SHA-256
(32 bytes). Each record is tag 1 plus a u64 LBA and exactly one block of payload,
or tag 2 for a flush. A final SHA-256 covers every preceding artifact byte.
Unknown versions/tags, invalid geometry or LBAs, trailing bytes, truncation,
checksum mismatch and caller-limit excess refuse decoding.

Caller limits bound wire bytes, operation count, block size and base blocks.
Encoding validates the entire operation set before producing the artifact;
decoding checks the checksum and admits allocation before returning operations.
No operation is applied during decoding. Checksums detect corruption and do not
authenticate an artifact's author. Filesystem semantic scenarios, expected state,
source revision, fault selection and completion records belong in the enclosing
replay bundle. A decoded trace alone never certifies filesystem correctness.

## Qualification

Require deterministic write/flush/repeated-LBA round trips, every truncated prefix,
resealed malformed headers and records, explicit allocation/geometry limits,
modified-base refusal and zero writes during base verification. Exercise persisted
trace replay through the overlay cut oracle and independently verify the digest.
Keep serialization, complete bundle replay and minimization as separate gates.
