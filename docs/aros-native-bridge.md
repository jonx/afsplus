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
   exported-symbol checks;
5. compilation of the public headers, packet translator, trackdisk adapter and
   native handler shell under AArch64 Clang, both packet DOS alias modes, plus
   m68k GCC, including the generated handler entry; and
6. an intermediate AArch64 link which rejects unresolved AFS+ symbols, followed
   by `genmodule`, compilation of the MacAROS Rust platform glues and a complete
   AROS handler-module link which must have no undefined symbols; and
7. a machine-code audit requiring AROS AArch64 `ET_REL` identity and rejecting
   every `x18` or architectural TLS-register instruction.

Defaults assume sibling `Macaros`, `~/aros-build`, `~/aros-crosstools` and
`~/aros-m68k-build` trees. Override `MACAROS_ROOT`, `AROS_BUILD`,
`AROS_CROSSTOOLS`, or `AROS_M68K_BUILD` when needed. A host without the m68k SDK
may set `AFSPLUS_AROS_SKIP_M68K_ABI=1`; release qualification must not skip it.

The Rust build uses `nightly-2026-06-27` by default because that is the version
paired with the current MacAROS Rust target and `rust-aros` standard library.
Override `AFSPLUS_AROS_RUST_TOOLCHAIN` only with a correspondingly rebased
target and standard library.

## AArch64 platform profiles

The default profile remains the qualified Hosted MacAROS build. Its target JSON
reserves `x18`, and every C object uses `-ffixed-x18`, because Darwin may alter
that platform register across host signal delivery. Those are Hosted runtime
requirements, not properties of the AFS+ format or handler ABI.

`tools/check-aros-ffi.sh` and `tools/package-aros-alpha0.sh` accept the same
profile variables:

| Variable | Hosted default | Contract |
|---|---|---|
| `AFSPLUS_AROS_SDK_ROOT` | `$AROS_BUILD/bin/darwin-aarch64` | Target SDK root containing `gen` and `AROS/Developer` |
| `AFSPLUS_AROS_BUILD_TOOLS_ROOT` | `$AFSPLUS_AROS_SDK_ROOT/tools` | Host-executable `collect-aros` and `genmodule` directory |
| `AFSPLUS_AROS_EXPECTED_PLATFORM` | unset | Required `AROS_TARGET_PLATFORM` value for a release profile |
| `AFSPLUS_AROS_PROFILE_ID` | SDK platform | Human-readable package profile identity |
| `AFSPLUS_AROS_OBJDUMP` | `$AROS_CROSSTOOLS/bin/llvm-objdump` | Disassembler used by the final machine-code ABI gate |
| `AFSPLUS_AROS_RUST_TARGET_JSON` | MacAROS `aarch64-unknown-aros.json` | Rust target used with `-Zbuild-std` |
| `AFSPLUS_AROS_RUST_ARCHIVE` | derived from the JSON filename | Optional explicit `libafsplus_aros_ffi.a` output |
| `AFSPLUS_AROS_PLATFORM_GLUE_DIR` | MacAROS `hosted/rust` | Seven AROS `std` C glue sources |
| `AFSPLUS_AROS_TARGET` | `aarch64-unknown-aros` | Clang driver/link target |
| `AFSPLUS_AROS_CODEGEN_TARGET` | `aarch64-unknown-none-elf` | Clang target for generated entry/glue objects |
| `AFSPLUS_AROS_ARCH_FLAGS` | `-mcmodel=large -ffixed-x18` | Whitespace-separated target ABI/codegen flags |
| `AFSPLUS_AROS_CROSS_LIB` | `$AROS_CROSSTOOLS/lib/generic` | Compiler runtime library directory |

A bare-metal MacAROS SDK plus its host build tools must provide these as one
coherent profile. In particular it must not inherit `+reserve-x18` or
`-ffixed-x18` unless that platform ABI independently reserves the register. The
build still requires the same public AROS headers/libraries and seven `std`
glue symbols; no filesystem source fork is permitted. Every package records
the profile values, target-JSON hash and per-glue hashes in
`build-profile.txt`. ADR-051 records this boundary.

The first native-linked pre-hardware package is reproduced with:

```sh
AFSPLUS_AROS_PACKAGE_OUTPUT="$PWD/build/aros-alpha0-apple-aarch64" \
AFSPLUS_AROS_SDK_ROOT="$HOME/Build/aros-apple-core/apple/bin/apple-aarch64" \
AFSPLUS_AROS_BUILD_TOOLS_ROOT="$HOME/Build/aros-apple-core/apple/bin/darwin-aarch64/tools" \
AFSPLUS_AROS_EXPECTED_PLATFORM=apple-aarch64 \
AFSPLUS_AROS_PROFILE_ID=macaros-native-apple-aarch64-prehardware \
tools/package-aros-alpha0.sh
```

It reuses the currently qualified MacAROS AROS-AArch64 Rust target and seven
glues, but links against the `apple-aarch64` SDK. Their hashes, the SDK target
configuration, host tools and ABI auditor are recorded in profile format v2.
The resulting machine-code report is a build gate, not a native runtime claim.

To materialize, without installing, everything needed for the first target
run, use:

```sh
tools/package-aros-alpha0.sh
```

It creates `build/aros-alpha0` atomically and refuses to replace an existing
package. The directory contains the complete handler, target operation probe,
DOSDriver, clean 64-MiB `fdsk.device` image, host checker report, instructions
and hashes. No MacAROS tree is modified by the packaging command.

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

An optional `afsp_io_activity_sink` in the trackdisk configuration exposes the
virtual drive LED contract. The adapter selects direct callbacks for every
masked-out operation, so leaving the sink empty has no per-operation activity
test. Select `AFSP_IO_ACTIVITY_MASK_WRITE | AFSP_IO_ACTIVITY_MASK_FLUSH` for an
Amiga-style write LED; timing and coalescing remain UI policy.

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

## Native handler shell

`native/aros/afsplus_handler.c` now performs the target-specific assembly: it
validates startup geometry, opens the device, probes NSD/TD64 and write
protection, handles optional DMA masks through a bounded bounce buffer,
registers the volume, provides UTC/public-memory callbacks, processes messages
serially and unwinds partial startup in reverse order. It is cross-compiled on
AArch64 and m68k. `native/aros/afsplus.conf` provides the AFS+ DOS type and
Resident definition. Qualification generates the AROS entry, supplies the
MacAROS `std` platform glues and fully links the AArch64 module with no undefined
symbols. See ADR-045.

That final off-tree link deliberately consumes the glue sources from
`$MACAROS_ROOT/hosted/rust` instead of copying them. This external build is the
release architecture, not a temporary dependency on AROS accepting AFS+ into
its source tree. The resulting module is installed in `L:` and selected by a
normal file in `DEVS:DOSDrivers`; it uses only public Exec, DOS and device APIs.
An optional upstream or distribution `mmakefile.src` would list the same seven
glues (`net`, `fs`, `process`, `proc`, `thread`, `sync`, `env`), the AFS+ static
library and the standard MacAROS `posixc/stdc/pthread` link set. A handler uses
its generated start/end objects, never the command-oriented `startup.o`.

## Runtime qualification stages

`tools/check-hosted-aros-alpha0.sh` now installs the off-tree package into a
dedicated Hosted test tree, executes create/read/write/truncate/rename/fsync on
AROS and macFUSE, returns to AROS for cross-created-file readback, and requires
a clean strict checker at every boundary. ADR-046 records the runtime startup
fixes and S0 evidence.

The deterministic Hosted replay gate now mounts six modeled power-cut images
through the native handler and validates their exact old/new state plus a clean
post-replay checker; ADR-047 records its model and limits. The cumulative S1
system pivot is also qualified: S1a moves the six core assigns, while S1b runs
Wanderer, IPrefs, Locale and Clock from a manifested AFS+ desktop image and
durably writes `ENVARC:`. ADR-048 defines the split and ADR-049 records the S1b
evidence; ADR-052 removes its former case-policy workaround with a versioned,
case-insensitive and spelling-preserving AROS namespace. ADR-050 records repeated standard
`Assign DISMOUNT` termination and reload, including the required ordering of
device-node removal and the deferred `ACTION_DIE` reply. No AROS kernel, DOS or
source-tree inclusion of the AFS+ handler is part of that contract. A genuine
bug in a generic AROS interface should still be fixed and proposed upstream
rather than hidden in the handler.

The native pre-hardware gate is reproduced with:

```sh
tools/check-macaros-native-alpha0-qemu.sh
```

It builds an external `afsram.device`, a strict version-2 retained-image
descriptor and the complete off-tree handler against the `apple-aarch64` SDK.
Under QEMU, the target mounts the 64-MiB AFS+ payload and executes create,
read, write, sparse write, truncate, two flushes, rename, case-folded lookup,
case-only rename and reopen/readback. It then inhibits and stops the handler,
waits for its task to disappear and unloads both modules before the boot gate
returns PASS. The report hashes every executable input and the command refuses
to mutate either the MacAROS or AROS worktree. ADR-053 defines the transport and
records the evidence.

`afsram.device` writes only the retained boot image. `CMD_UPDATE` therefore
tests the filesystem/device ordering path but cannot make data survive reset.
Native crash replay, extraction plus strict checking of the mutated payload,
and Apple-hardware execution remain unproven. The package contract moves next
to those native durability gates, then m68k emulation and the physical Amiga
500: three target platforms, four ordered validation stages.

The fixed-image Alpha-0 path does not yet install `TD_ADDCHANGEINT` handling.
Hot-swappable media remains disabled until removal can detach the mounted Rust
instance and DOS volume without racing outstanding locks.

The AArch64 library is cross-build qualified. The m68k header/layout is
qualified, but the experimental m68k Rust `std` toolchain is not yet a
production path; classic support will either repair that toolchain or move the
core dependency graph to a bounded `no_std + alloc` profile.
