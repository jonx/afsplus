# ADR-042: Versioned C boundary for native AROS handlers

Status: Accepted for Mountable Alpha-0

## Context

ADR-041 moved DOS semantics into a packet-neutral Rust adapter, but a native
handler still had no stable way to instantiate it. Passing an AROS device or a
`DosPacket` directly into Rust would bind the safe core to target headers,
BPTR layout and message-port lifetime. It would also make the block-I/O and
64-bit contracts difficult to test on the host.

MacAROS AArch64 can cross-compile Rust `std` static libraries. The m68k DOS
surface cannot store every AFS+ identifier or position in an `IPTR`; it already
uses `DosPacket64` for wide position and size actions. Both architectures can
call an ordinary C ABI and keep native `FileLock`/file-handle wrappers around
64-bit numeric IDs.

## Decision

`afsplus-aros-ffi` is the only raw-pointer boundary between a native handler
and the safe AFS+ crates. It builds as an `rlib` and a `staticlib` and exports
the hand-written, versioned interface in `api/afsplus_aros.h`.

The native handler provides:

- fixed block size and block count;
- one-block read and write callbacks;
- a durability-barrier callback; and
- an opaque device context that remains valid for the mount lifetime.

The bridge provides opaque mount, lock and file-handle state and exposes the
complete Alpha-0 DOS operation slice. Every scalar offset, position, size,
object identifier, lock ID and file ID is 64-bit at this boundary. Names and
I/O buffers are caller-owned byte spans; the Rust library never retains them
or allocates memory for C to release.

Every ABI struct carries either an ABI version plus `struct_size`, or has
compile-time size assertions in both Rust and C. Function-pointer fields are
nullable so read-only/no-change mounts do not need write callbacks. Read-write
and recovery mounts reject a device without write and flush support.

The returned instance belongs to one packet-processing task. Concurrent calls
against one instance are outside the contract. `afsplus_aros_unmount` consumes
the instance exactly once and attempts a final filesystem flush before closing
all adapter handles. C callback errors become block I/O errors; filesystem
failures return their AROS `ERROR_*` number. Rust unwinding is caught on host
builds, while native AROS uses `panic=abort`; no unwind may cross the C ABI.

The native packet layer, implemented by ADR-043, remains responsible for:

- message-port receive/reply and handler startup/shutdown;
- BPTR, BSTR, buffer and `FileInfoBlock` validation;
- native lock/file wrapper allocation;
- DOS path splitting and `DateStamp` conversion; and
- `DosPacket64` validation and reply layout on 32-bit systems.

It does not own a second namespace, cache or transaction model.

## Consequences

The host FFI qualification performs create, sparse write, fsync, truncate,
rename, hard link, read, enumeration, flush, unmount, remount and strict
checker validation through the exported boundary. The static library builds
with the real `aarch64-unknown-aros` Rust target. The public header compiles
together with MacAROS `dos/dos64.h` under both the AROS AArch64 Clang and m68k
GCC; its 64-bit and 32-bit layouts are compile-time pinned.

This is not yet a native mount. M06 remains partial until the C translator and
static library link into a handler and run the same-image matrix on MacAROS.
The m68k C ABI and translator are qualified, but the Rust static library is not
yet a supported classic artifact: the current experimental m68k `std` port has
unrelated build/runtime defects that must be resolved or avoided with a future
`no_std + alloc` profile.

ADR-056 subsequently qualifies a patched experimental `std` static library and
the full native handler on an M68020-or-newer emulator profile. That later
evidence supersedes only the classic-artifact status above; the plain-68000 and
production-toolchain limitations remain.
