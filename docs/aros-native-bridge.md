# Native AROS bridge

The native integration is deliberately split at a stable C ABI:

```text
DosPacket / BPTR / BSTR / DateStamp       AROS trackdisk or image device
                 |                                      |
          native packet.c                       C block callbacks
                 |                                      |
                 +---------- api/afsplus_aros.h --------+
                                      |
                             afsplus-aros-ffi.a
                                      |
                         adapter -> VFS -> AFS+ core
```

Only `packet.c` is target-specific. It owns AROS structures and message-port
lifetime; the static library owns filesystem, lock, file-handle and durability
semantics.

## Reproduce the bridge qualification

On the MacAROS development machine:

```sh
tools/check-aros-ffi.sh
```

The script performs four independent gates:

1. the host operation matrix through the exported Rust functions;
2. the standalone C11 header check;
3. a release staticlib build with MacAROS's `aarch64-unknown-aros.json`, plus
   exported-symbol checks; and
4. compilation of `api/afsplus_aros.h` together with the genuine AROS
   `dos/dos64.h` under AArch64 Clang and m68k GCC.

Defaults assume sibling `Macaros`, `~/aros-build`, `~/aros-crosstools` and
`~/aros-m68k-build` trees. Override `MACAROS_ROOT`, `AROS_BUILD`,
`AROS_CROSSTOOLS`, or `AROS_M68K_BUILD` when needed. A host without the m68k SDK
may set `AFSPLUS_AROS_SKIP_M68K_ABI=1`; release qualification must not skip it.

The Rust build uses `nightly-2026-06-27` by default because that is the version
paired with the current MacAROS Rust target and `rust-aros` standard library.
Override `AFSPLUS_AROS_RUST_TOOLCHAIN` only with a correspondingly rebased
target and standard library.

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

Names are raw byte spans with an explicit encoding selected at mount. Examine
functions write name bytes separately from `AfsplusArosFileInfo`; the packet
layer adds the BCPL length byte and terminator required by `FileInfoBlock`.

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

## Remaining native gate

The next integration step is a small MacAROS handler target that links the
static library, implements trackdisk callbacks and translates the packets
above. It must then mount the same image used by the host, execute
create/read/write/truncate/rename/fsync, reboot at controlled durability
points, replay, unmount and pass the strict host checker.

The AArch64 library is cross-build qualified. The m68k header/layout is
qualified, but the experimental m68k Rust `std` toolchain is not yet a
production path; classic support will either repair that toolchain or move the
core dependency graph to a bounded `no_std + alloc` profile.
