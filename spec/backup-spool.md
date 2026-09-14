# Verified archive scratch capture and replay

[ADR-085](../adr/ADR-085-verified-archive-spooling.md) defines the constrained
replay mechanism. This scratch representation is ephemeral and private to the
host process. It changes neither the backup interchange format nor AFS+ records.

## Admission and ownership

The host supplies owned read/write/seek scratch storage, a positive archive-byte
limit, a positive scratch-byte limit, and a power-of-two chunk size from 512 bytes
through 1 MiB. Capture overwrites the admitted scratch prefix. Scratch quotas
include padding and all hash nodes. Overflow or exhaustion refuses capture;
partial scratch output confers no verified capability.

Construction copies the input, builds the private integrity root, and verifies
all captured envelope framing, effective ordinary fields, terminal digest and
EOF through checked replay before returning a verified owner. This verifies
archive integrity and admitted fields, not inventory completeness or authenticity
of the sender. No constructor accepts a caller-supplied root or reopens an
unverified scratch file as trusted. The owner exposes replay readers and resource
statistics, without exposing storage mutation or root replacement.

## Chunk proofs

For chunk size `C`, each leaf occupies `C + 32` scratch bytes: zero-padded data
followed by its SHA-256 hash. A leaf input is `0x00`, its little-endian unsigned
64-bit index, its little-endian unsigned 64-bit actual length, then its actual
data bytes. An internal node hashes `0x01`, the left hash and the right hash.
An unpaired final node is promoted unchanged. The empty-tree root is SHA-256 of
empty input. These separated domains and tree shape follow
[RFC 6962 section 2.1](https://www.rfc-editor.org/rfc/rfc6962.html#section-2.1),
with the chunk index/length/data tuple as each leaf's data entry.

Parent levels follow the leaf area as contiguous 32-byte hashes. Each level has
`ceil(previous_count / 2)` entries. The root, exact archive length, chunk count
and level offsets are retained privately in memory; stored hashes are proof
inputs, never replacement trust roots. Replaying a chunk recomputes its leaf,
validates unused padding and combines its sibling proof to the retained root
before returning any bytes. Bad proofs, truncated storage and I/O failures
permanently fail that replay cursor without returning altered bytes.

## Publication and resource behavior

A reader created by the verified owner carries prior integrity admission while
continuing to check replay bytes. A staged opaque value bound to that reader may
publish after its own complete replay, releasing the upload slot before the next
value. Unverified readers retain the final-EOF publication gate. Any reader error
withdraws admission for further publication. Whole-job success requires complete
inventory processing, replay EOF and qualified destination synchronization.

Capture and replay use one chunk buffer at a time plus bounded parser buffers
and logarithmic level metadata. The verification discard buffer is 512 bytes;
PAX metadata keeps its separately supplied limits. The chunk cursor caches at
most one chunk. Proof I/O is logarithmic in chunk count. The backing store holds
the archive and tree; using a memory store does not make total memory bounded,
whereas a file-backed store avoids archive-sized heap storage.

Scratch flush is not a power-loss durability claim. On corruption or loss,
discard scratch and recapture from the original archive; no repair or restart
protocol is implied. Actual providers require resource and lifecycle qualification.
This implementation uses unsigned 64-bit seek addresses and explicit host quotas.
Archives exceeding a host file/seek limit require qualified segmented backing
storage before that workload is supported; this is an owned large-archive gate,
not permission to omit source data or weaken integrity verification.


## Explicit sparse admission

Sparse-aware capture and replay preserve the distinction between stored payload
length and logical file length under [ADR-087](../adr/ADR-087-sparse-archive-content.md)
and [ADR-088](../adr/ADR-088-sparse-stored-size-field.md). Ordinary constructors
refuse these fields. The [content consumer](backup-sparse.md) must validate the
map before writes; verified scratch certifies input integrity and header
admission, not sparse-map semantics or complete restoration.
