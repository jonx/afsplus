# ADR-044: Bounded AROS trackdisk viewport

Status: Accepted for Mountable Alpha-0

## Context

The native C boundary from ADR-042 consumes logical AFS+ blocks, while an AROS
filesystem handler opens a trackdisk-compatible device described by a
`FileSysStartupMsg` and `DosEnvec`. The partition may begin at a non-zero byte
offset, and offsets beyond 4 GiB are safe only after the device has positively
advertised TD64 or NSD commands. A wrapped 32-bit offset would read or overwrite
the wrong part of the disk.

The reusable boundary must also remain independent of Exec's message and
device lifetime. Those objects belong to the handler process and must not leak
into Rust or into host tests.

## Decision

`native/aros/afsplus_trackdisk.c` converts a checked partition byte viewport
into the one-block callbacks required by `AfsplusArosDevice`. The handler
supplies two synchronous functions: one transfer function which receives the
already checked absolute byte offset and selected command, and one durability
barrier function. The reusable adapter itself performs no Exec or DOS calls.

`afsplus_aros_trackdisk_geometry` derives the viewport from the inclusive
cylinder interval using checked 64-bit products:

```text
physical block bytes = de_SizeBlock * 4
blocks per cylinder  = de_Surfaces * de_BlocksPerTrack
partition start      = de_LowCyl * blocks per cylinder * physical block bytes
partition length     = (de_HighCyl - de_LowCyl + 1)
                       * blocks per cylinder * physical block bytes
```

The handler rejects negative `DosEnvec` values before converting them to the
unsigned geometry API. Alpha-0 supports exactly 4096-byte logical AFS+ blocks.
The physical device block size must divide 4096. The viewport start need only
align to that physical unit, so an old partition beginning on a 512-byte sector
is not rejected merely because its absolute offset is not 4-KiB aligned. The
viewport length must contain a whole number of AFS+ logical blocks.

Every block callback checks multiplication, addition, logical bounds and the
complete end offset before calling the device. It never clips a request. A
viewport ending above 4 GiB is rejected at initialization unless the handler
has positively probed a 64-bit command pair. An exactly ending-at-4-GiB block
remains valid with the legacy commands.

The handler's transfer implementation places the low offset word in
`io_Offset`, the high word in `io_Actual`, performs `DoIO`, and rejects a short
transfer. Its sync implementation issues `CMD_UPDATE`; a successful filesystem
flush therefore includes the physical device barrier rather than only draining
an AFS+ cache. Read-only mounts omit write and flush callbacks entirely.

## Consequences

The geometry and viewport logic has a runnable host matrix and the exact same C
source compiles with warnings as errors for AROS AArch64 and AROS m68k. Tests
cover overflow, alignment, the last legacy-addressable block, rejection beyond
4 GiB without TD64/NSD, callback-error propagation and read-only setup.

This does not yet prove a native mount. The remaining shell must own
`OpenDevice`, capability probing, short-I/O checks, media-change lifecycle,
startup/reply plumbing and the message loop. It can now do so without owning
partition arithmetic or a second block-bounds policy. No MacAROS source tree is
modified by this decision.
