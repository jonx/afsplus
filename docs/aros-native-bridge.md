# Native AROS bridge

The native integration is deliberately split at a stable C ABI:

```text
DosPacket / BPTR / BSTR / DateStamp       AROS trackdisk or image device
                 |                                      |
     native/aros/afsplus_packet.c       native/aros/afsplus_trackdisk.c
                 |                                      |
                 +---------- api/afsplus_aros.h --------+
                                      |
                             afsplus-aros-ffi.a
                                      |
                         adapter -> VFS -> AFS+ core
```

The shared C translator owns AROS structure conversion and native wrapper
lifetime without calling Exec or DOS. The bounded trackdisk adapter owns
checked partition arithmetic and logical-block bounds, also without calling
Exec or DOS. A small target handler still owns the message port,
startup/shutdown and device requests. The static library owns filesystem,
numeric lock/file-handle and durability semantics.

## Reproduce the bridge qualification

On the MacAROS development machine:

```sh
tools/check-aros-ffi.sh
```

The script performs these independent gates:

1. the host operation matrix through the exported Rust functions;
2. runnable host `DosPacket` and bounded trackdisk viewport matrices compiled
   with genuine AROS AArch64 headers;
3. the standalone C11 header check;
4. a release staticlib build with MacAROS's `aarch64-unknown-aros.json`, plus
   exported-symbol checks; and
5. compilation of the public headers, packet translator and trackdisk adapter
   under AArch64 Clang, both packet DOS alias modes, plus m68k GCC.

Defaults assume sibling `Macaros`, `~/aros-build`, `~/aros-crosstools` and
`~/aros-m68k-build` trees. Override `MACAROS_ROOT`, `AROS_BUILD`,
`AROS_CROSSTOOLS`, or `AROS_M68K_BUILD` when needed. A host without the m68k SDK
may set `AFSPLUS_AROS_SKIP_M68K_ABI=1`; release qualification must not skip it.

The Rust build uses `nightly-2026-06-27` by default because that is the version
paired with the current MacAROS Rust target and `rust-aros` standard library.
Override `AFSPLUS_AROS_RUST_TOOLCHAIN` only with a correspondingly rebased
target and standard library.

## Trackdisk viewport

Use `afsplus_aros_trackdisk_geometry` to convert the non-negative `DosEnvec`
cylinder geometry into an exact byte start and length. Initialize
`AfsplusArosTrackdiskConfig` with that viewport, the physical block size
`de_SizeBlock << 2`, the Alpha-0 logical size of 4096, the command pair selected
by the handler's TD64/NSD probe, and whether the mount is read-only.

The supplied transfer callback receives an absolute device byte offset. For
TD64 and NSD commands it writes the low 32 bits to `io_Offset` and the high 32
bits to `io_Actual`, calls `DoIO`, and accepts success only when `io_Actual`
then equals the requested byte count. The sync callback issues `CMD_UPDATE`.
The adapter rejects an inaccessible viewport at initialization rather than
allowing a later 32-bit offset wrap. See ADR-044.

## Native lifecycle

The handler keeps its device state alive, fills `AfsplusArosDevice` and
`AfsplusArosMountConfig`, then calls `afsplus_aros_mount`. `struct_size` must be
`sizeof` the C struct compiled for that target. The block callbacks return zero
on success and a non-zero native I/O status on failure. They receive exactly
one logical AFS+ block per call.

Lock value zero represents the DOS null lock/root at the C boundary. A native
`FileLock` or file-handle wrapper stores the returned 64-bit ID; on m68k it must
not be squeezed into the 32-bit `fl_Key`/`fh_Arg1` scalar itself. Those fields
point to native wrappers instead.

Create the packet context from `native/aros/afsplus_packet.h` after mounting
the Rust bridge. Its allocation callback must return suitably aligned public
memory; the translator clears each allocation itself. Its clock callback
returns UTC Unix seconds and nanoseconds. Process one packet at a time with
`afsplus_aros_packet_process`, then reply to the original message from the
handler loop. Destroy the packet context before calling
`afsplus_aros_unmount`.

Names are raw byte spans with an explicit encoding selected at mount. Examine
functions write name bytes separately from `AfsplusArosFileInfo`; the packet
layer adds the BCPL length byte and terminator required by `FileInfoBlock`.
Use `max_file_info_name_bytes <= 107` for a packet mount.

The usual packet mapping is direct:

| DOS action | C boundary |
|---|---|
| locate/copy/parent/same/free lock | `locate`, `duplicate_lock`, `parent_lock`, `same_lock`, `free_lock` |
| find input/output/update, end | `open`, `close` |
| read/write | `read`, `write` |
| seek and 64-bit position actions | `seek`, `file_position` |
| set/get file size, including 64-bit actions | `set_file_size`, `file_size` |
| create/delete/rename/link | corresponding namespace function |
| examine object/FH/next | corresponding examine function |
| flush | `flush` |
| info/disk info | `disk_info` |

`ACTION_SEEK64`, size/position variants and `DosPacket64.dp_Res0 == DP64_INIT`
are decoded and encoded in the C packet layer. The Rust ABI always receives the
already reconstructed `int64_t`/`uint64_t` value.

`__WORDSIZE` and `__DOS64` are intentionally handled separately. Standard
examine/info actions use the structure selected by the native DOS headers;
explicit `ACTION_EXAMINE_*64` and `ACTION_INFO64` always use the wide form.
Legacy 32-bit counters saturate at `INT32_MAX` rather than wrapping.

## Remaining native gate

The next integration step is a small MacAROS handler target that links the
static library, packet translator and trackdisk viewport, implements the
actual `IOExtTD` transfer/barrier callbacks and owns device/media lifecycle plus
the message receive/reply loop. It must then mount the same
image used by the host, execute
create/read/write/truncate/rename/fsync, reboot at controlled durability
points, replay, unmount and pass the strict host checker.

The AArch64 library is cross-build qualified. The m68k header/layout is
qualified, but the experimental m68k Rust `std` toolchain is not yet a
production path; classic support will either repair that toolchain or move the
core dependency graph to a bounded `no_std + alloc` profile.
