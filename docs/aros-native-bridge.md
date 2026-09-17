# Native AROS bridge

> **ADRs:** [ADR-044](../adr/ADR-044-aros-trackdisk-viewport.md),
> [ADR-045](../adr/ADR-045-native-aros-handler-shell.md),
> [ADR-046](../adr/ADR-046-hosted-aros-same-image.md),
> [ADR-047](../adr/ADR-047-hosted-aros-crash-replay.md),
> [ADR-048](../adr/ADR-048-hosted-aros-system-pivot.md),
> [ADR-049](../adr/ADR-049-hosted-aros-desktop-pivot.md),
> [ADR-050](../adr/ADR-050-external-aros-handler-lifecycle.md),
> [ADR-051](../adr/ADR-051-explicit-aros-aarch64-platform-profiles.md),
> [ADR-052](../adr/ADR-052-versioned-directory-comparison-keys.md),
> [ADR-053](../adr/ADR-053-native-macaros-retained-image-transport.md),
> [ADR-054](../adr/ADR-054-native-macaros-crash-replay-extraction.md),
> [ADR-055](../adr/ADR-055-aros-m68k-emulator-gate.md),
> [ADR-056](../adr/ADR-056-native-aros-m68k-alpha0-and-replay.md),
> [ADR-057](../adr/ADR-057-plain-m68000-emulator-gate.md),
> [ADR-058](../adr/ADR-058-m68000-memory-and-restart-lifecycle.md),
> [ADR-059](../adr/ADR-059-guest-failure-diagnostics-are-gate-verdicts.md),
> [ADR-060](../adr/ADR-060-mountable-alpha0-completion-gate.md) · **Spec:** none ·
> **Tests:** [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) · **Milestones:** M06

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

<!-- toc -->

- [Reproduce the bridge qualification](#reproduce-the-bridge-qualification)
- [AArch64 platform profiles](#aarch64-platform-profiles)
- [Trackdisk viewport](#trackdisk-viewport)
- [Qualification diagnostics](#qualification-diagnostics)
- [Native lifecycle](#native-lifecycle)
- [Native handler shell](#native-handler-shell)
- [Runtime qualification stages](#runtime-qualification-stages)

<!-- /toc -->

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

[`tools/check-aros-ffi.sh`](../tools/check-aros-ffi.sh) and [`tools/package-aros-alpha0.sh`](../tools/package-aros-alpha0.sh) accept the same
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

## Qualification diagnostics

Every AROS runtime gate treats guest failure output as a verdict rather than
assuming that emulator or host-process status represents the filesystem task.
[`tools/check-aros-serial-log.sh`](../tools/check-aros-serial-log.sh) scans FS-UAE serial output, Hosted AROS window
logs, and both native-QEMU serial and semihost logs. A software-failure
requester, Guru Meditation, trap, AFS+ failure, alert or unrecoverable halt
fails the gate before its normal operation/checker verdict can be accepted.
Successful evidence records the absence of a requester. See ADR-059.

## Native lifecycle

The handler keeps its device state alive, fills `AfsplusArosDevice` and
`AfsplusArosMountConfig`, then calls `afsplus_aros_mount`. `struct_size` must be
`sizeof` the C struct compiled for that target. The block callbacks return zero
on success and a non-zero native I/O status on failure. They receive exactly
one logical AFS+ block per call.

The boundary grows additively inside ABI version 1.
`afsplus_aros_interface` needs no mount and returns the interface revision
plus a mask of entry-point groups; a packet layer built against a newer header
asks it before calling a function of a later group and answers
`ERROR_ACTION_NOT_KNOWN` for a missing one. `afsplus_aros_capabilities`
returns the mounted volume's published `FSV2_CAP_*` mask, mount mode, name
limit, case policy, Unicode version, pending intent records and block counts.
Both use one growth rule: the caller stores its structure size in
`struct_size`, the library fills at most that many bytes and stores the count
it filled, and a size below the first published layout is `ERROR_BAD_NUMBER`.

Lock value zero represents the DOS null lock/root at the C boundary. A native
`FileLock` or file-handle wrapper stores the returned 64-bit ID; on m68k it must
not be squeezed into the 32-bit `fl_Key`/`fh_Arg1` scalar itself. Those fields
point to native wrappers instead.

Create the packet context from [`native/aros/afsplus_packet.h`](../native/aros/afsplus_packet.h) after mounting
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
| set protect, set date | `set_protection`, `set_modified`; a path without a leaf addresses the resolved lock's object |
| make link (soft), read link | `make_soft_link`, `read_soft_link` |
| fh from lock, change mode | `open_from_lock`, `change_lock_mode`, `change_file_mode` |
| write protect | `set_write_protect` |
| lock record, free record | `lock_record`, `free_record` |
| examine all, examine all end | `examine_next` per entry, `rewind_directory`; the packet layer packs `ExAllData` |
| examine object/FH/next | corresponding examine function |
| flush | `flush` |
| info/disk info | `disk_info` |

A file handle holds its object like a lock: `MODE_NEWFILE` exclusively, the
other modes shared. A held object is not deletable, and `ACTION_COPY_DIR_FH`
on an exclusive handle is `ERROR_OBJECT_IN_USE`, the behavior `NameFromFH`
in dos.library is written around.

The handler is the classic single-user adapter of the
[security model](30-portable-security-model.md#9-classic-amiga-compatibility-profile):
the session acts as the owner and the DOS protection word is a projection.
`ACTION_SET_PROTECT` on an object that carries security metadata the
projection cannot express is refused with `ERROR_WRITE_PROTECTED` and the
stored state is untouched; the mount flag
`AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE` is the explicit request to let
the write through. The adapter asks one question through `RichSecurityProbe`
and never interprets the metadata.

An on-disk security descriptor is the case where preserving the metadata is
possible, so the handler preserves it: `ACTION_SET_PROTECT` applies the new
protection word, every descriptor byte stays and the projection-diverged mark
is set in the same transaction, which a host that evaluates the descriptor
reconciles. The mount flag
`AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION` selects the core's
strict policy instead, where such a write is refused and nothing changes. Rename and hard link keep the object and
therefore its security metadata.

`ACTION_EXAMINE_ALL` packs as many entries as fit the caller's buffer at the
requested detail level. An entry read from the filesystem that does not fit
stays with the lock and is returned first by the next call, so a small buffer
never loses an entry; a buffer too small for one entry is
`ERROR_BUFFER_OVERFLOW`, and an entry fits when its own bytes fit, whatever
its alignment padding. A lock has one directory cursor: a zero `eac_LastKey`
starts a sequence, which takes the cursor and receives its own key, and a
continuation whose key no longer owns the cursor is `ERROR_OBJECT_IN_USE`.
Examine, `ExNext`, the last entry and `ACTION_EXAMINE_ALL_END` end the
sequence. A read that fails after entries were packed returns those entries
and surfaces on the next call; `ed_Type` is the directory entry type that
`ExNext` reports. A request
with a match string or match hook is answered `ERROR_ACTION_NOT_KNOWN`,
because matching needs dos.library, and dos.library then emulates `ExAll`
through `ExNext`.

`ACTION_FH_FROM_LOCK` turns a lock on a file into a file handle: the lock
identifier dies, its shared or exclusive hold continues as the handle's, and
a refused conversion leaves the lock usable. `ACTION_CHANGE_MODE` accepts
`SHARED_LOCK`, `MODE_OLDFILE` and `MODE_READWRITE` as shared and
`EXCLUSIVE_LOCK` and `MODE_NEWFILE` as exclusive; shared to exclusive needs
the caller to be the object's only holder, and `fl_Access` follows a
successful change only.

`ACTION_WRITE_PROTECT` protects the volume for the lifetime of the mount:
it flushes, then every mutating call answers `ERROR_DISK_WRITE_PROTECTED`,
handles opened for writing included, until the same 32-bit key unprotects it;
a zero key is the keyless lock that any key opens, and no master key exists.
A wrong key on unprotect is `ERROR_INVALID_COMPONENT_NAME`, as the AROS RAM
handler answers it; protecting a protected volume with another key is
`ERROR_DISK_WRITE_PROTECTED`. A protected volume is not changed at all:
flush, close, `ACTION_INHIBIT` and `ACTION_DIE` publish writes accepted before
the protection and start no orphan cleanup, which waits, visible in the
health snapshot, until the volume is unprotected. `ACTION_DISK_INFO` reports
the protected state, and nothing is written to the volume for it.

Record locks are advisory byte ranges in a bounded in-memory table, owned
by a file handle and released when it closes. Two ranges collide when they
overlap, belong to different handles of one object and at least one is
exclusive. The filesystem never waits: an immediate mode answers
`ERROR_LOCK_COLLISION`, and a waiting mode whose range is taken answers
`ERROR_LOCK_TIMEOUT` at once, the `dp_Arg5` tick count being a handler-loop
matter. `ACTION_FREE_RECORD` needs the owning handle and the exact range.
`ACTION_LOCK_RECORD64` and `ACTION_FREE_RECORD64`, which dos64.library sends
where packet arguments are 64 bits wide, carry full-width ranges; the classic
packets stop at 4 GiB.

Soft-link targets are opaque paths in the mount encoding. Locate and open
answer `ERROR_IS_SOFT_LINK`; `ACTION_READ_LINK` walks the path to the first
link and returns the path dos.library retries with: the components before the
link, the target, then the remaining components, or the target alone followed
by the remainder when the target names a volume. A buffer that cannot hold the
result and its terminator yields -2 and never a truncated path. The packet
layer asks `afsplus_aros_interface` at creation and answers
`ERROR_ACTION_NOT_KNOWN` for an action whose entry-point group the linked
library lacks.

Three entry-point groups make a running handler observable without a
console. `NOTIFY` is a bounded watch table: a watch is a parent directory
plus the comparison key of a name, so it covers names that do not exist yet
and any spelling the name policy folds together; a directory watch also fires
for changes to its entries. Pending state is one flag per watch, so changes
between two drains are one event and memory does not follow the change rate.
File writes are reported when the handle closes. `ACTION_ADD_NOTIFY`
resolves `nr_FullName` to a parent lock plus leaf, registers the watch and
keeps the request-to-watch pairing in the packet context;
`ACTION_REMOVE_NOTIFY` and context destruction release it. After every packet
the layer drains the fired watches and calls the `notify` callback of the
packet configuration once per request, and at once for `NRF_NOTIFY_INITIAL` on
an existing object. The handler shell owns the delivery itself: the
`NotifyMessage` or `Signal`, `nr_MsgCount` and `NRF_WAIT_REPLY` suppression.
The shell sends messages from its own reply port, waits on that port next to
the packet port, and takes replied messages back before each batch of
packets. After `EndNotify` the `NotifyRequest` belongs to the application
again, so a returning message touches `nr_MsgCount` only while
`afsplus_aros_packet_notify_registered` still knows the request. A change that arrives while an `NRF_WAIT_REPLY` message is unreplied is
owed, marked with `NRF_MAGIC` as the AROS RAM handler does, and sent when the
reply comes back, so it is delayed and never lost. A message can
stay out for good: `EndNotify` takes back only messages still queued at the
application, and one already fetched by an application that crashed is never
replied. The shell dies anyway, so the volume stays dismountable, and in that
case leaves its reply port behind, set to `PA_IGNORE`, so that a late reply
queues into valid memory and signals no dead task; the port and the message
are a deliberate leak of a few dozen bytes.
`nr_Handler` names the handler port for `EndNotify`, so `ACTION_DIE` is
`ERROR_OBJECT_IN_USE` while a request is registered and destroying the packet
context clears `nr_Handler` of every request it still holds. A
shell that supplies no callback answers `ERROR_ACTION_NOT_KNOWN`.

`OBSERVE` carries health and tracing. Every failed call on a mounted instance
that describes the volume or its device (device error, failed validation,
disk full, internal fault) enters a health log with counters, degraded-state
flags and a bounded event ring whose sequence numbers expose loss.
`afsplus_aros_health` returns that state with the generation, pending intent
records, pending orphans and block counts. `afsplus_aros_set_trace_sink`
attaches the core flight recorder to a callback of
[`debug_observability.h`](../api/debug_observability.h) with a category mask;
the callback runs inside filesystem operations and only hands the event to a
preallocated queue. `afsplus_aros_trace_counters` reports delivered, missed,
filtered and dropped counts, so a slow consumer costs counted loss and never
blocks the filesystem.

`OBJECT_IDS` carries the object-ID operations of the v2 API with UTF-8
names whatever the mount's DOS encoding. `lookup_id` takes no lock and never
follows a link; `stat_id` names the object through renames and answers
`ERROR_OBJECT_NOT_FOUND` for a deleted object or a guessed identifier.
`dir_open`, `dir_read` and `dir_close` are a paged directory walk in a bounded
table, independent of the lock it started from and of that lock's `ExNext`
cursor. Its position is the last returned name instead of an ordinal: every
entry ordered after it is returned once whatever was created, deleted or
renamed between two pages, one page is one consistent view, and a deleted
directory fails with `ERROR_OBJECT_NOT_FOUND`. `dir_read` reads no more
entries than its buffer is certain to hold, so none is taken from the walk
and then dropped.

`EXTENT_MAP` answers, for a byte range of an open file, which parts are
written, which are reserved and which are holes, clipped to the range and
without physical addresses: what a pager needs to plan faults and
block-aligned transfers. It describes committed state, so with unpublished
writes pending it is `ERROR_OBJECT_IN_USE` and commits nothing.

`COUNTERS` reports completed and failed calls on the mounted instance and
the block callbacks it issued: reads, writes, barriers, bytes each way and
failures. A native benchmark runner reads them around a workload, as the
[benchmark contract](../testing/benchmark-contract.md) requires, instead of
inferring traffic from elapsed time.

`MANAGE` serves the structured management rule of
[ADR-025](../adr/ADR-025-structured-management-api.md) from the mounted
instance: `afsplus_aros_info_json` returns one JSON object with schema
`afsplus-handler-info` and its `schema_version`, then volume identity and
feature masks, mount state, capability names, health and handle usage. Field
order is fixed and an addition raises the version. A target tool is a thin
client that prints this document.

`ACTION_SEEK64`, size/position variants and `DosPacket64.dp_Res0 == DP64_INIT`
are decoded and encoded in the C packet layer. The Rust ABI always receives the
already reconstructed `int64_t`/`uint64_t` value.

`__WORDSIZE` and `__DOS64` are intentionally handled separately. Standard
examine/info actions use the structure selected by the native DOS headers;
explicit `ACTION_EXAMINE_*64` and `ACTION_INFO64` always use the wide form.
Legacy 32-bit counters saturate at `INT32_MAX` rather than wrapping.

## Native handler shell

[`native/aros/afsplus_handler.c`](../native/aros/afsplus_handler.c) now performs the target-specific assembly: it
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

The complete Mountable Alpha-0 contract is reproduced with:

```sh
tools/check-mountable-alpha0.sh
```

This composite gate binds the portable VFS/FUSE/intent-log tests, a real
macFUSE same-image round trip, Hosted crash replay, native MacAROS Alpha-0 and
native crash replay into one checksummed requirement matrix. ADR-060 defines
the completion boundary and its explicit lack of a hardware claim.

[`tools/check-hosted-aros-alpha0.sh`](../tools/check-hosted-aros-alpha0.sh) now installs the off-tree package into a
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
case-insensitive and spelling-preserving AROS namespace. ADR-050 records
repeated standard `Mount SHUTDOWN`/restart cycles followed by final `Assign
DISMOUNT`, including retention of the device node until handler cleanup and the
deferred `ACTION_DIE` reply.
No AROS kernel, DOS or source-tree inclusion of the AFS+ handler is part of that
contract. A genuine bug in a generic AROS interface should still be fixed and
proposed upstream rather than hidden in the handler.

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
returns PASS. QEMU uses file-backed RAM, after which the gate extracts the exact
payload and requires a clean schema-5 checker report with zero pending records.
The report hashes every consumed runtime/build input and requires those bytes
to remain stable during the gate. This permits unrelated parallel worktree
edits without weakening evidence integrity. ADR-053 defines the transport.

The six native replay cases are reproduced with:

```sh
tools/check-macaros-native-replay-qemu.sh
```

The command generates the same deterministic cuts used by Hosted MacAROS,
requires the exact manifest `old`/`new` state on target, cleanly unloads, then
extracts and checks each resulting payload. ADR-054 records the four old-state
generation-2 and two new-state generation-3 results; every final image has zero
pending records.

The next-platform boot prerequisite is reproduced with matching official AROS
m68k media:

```sh
AFSPLUS_AROS_M68K_BOOT_ADF=/path/to/bootdisk-amiga-m68k.adf \
AFSPLUS_AROS_M68K_SYSTEM_ISO=/path/to/aros-amiga-m68k.iso \
    tools/check-aros-m68k-boot-fsuae.sh
```

This first FS-UAE gate uses the ROM pair from the ISO, retains the official
bootstrap floppy in `DF0:`, exposes the extracted system as the exactly named
`AROS Live CD:` volume and captures the guest result through `HOST:`. It proves
only the native m68k boot harness.

The complete M68020-or-newer filesystem reference gate uses the same media:

```sh
AFSPLUS_AROS_M68K_BOOT_ADF=/path/to/bootdisk-amiga-m68k.adf \
AFSPLUS_AROS_M68K_SYSTEM_ISO=/path/to/aros-amiga-m68k.iso \
    tools/check-aros-m68k-alpha0-fsuae.sh
```

It builds the Rust/C handler from scratch with the patched `m68k-ccr-fixed`
toolchain, executes the Alpha-0 operation matrix and boots all six deterministic
intent-log cuts. Every case is checked on the host, and the evidence binds the
toolchain, handler, target, media and ROM hashes. ADR-056 records the accepted
generation-7 Alpha-0 result and the four generation-2/two generation-3 replay
outcomes. ADR-055 defines the remaining emulator/hardware progression and the
standalone-reproducer rule for any AROS patch discovered during qualification.

The A500-configured plain-M68000 profile uses the same gate with the target and
resource overrides below. The patched backend and temporary dedicated Cargo
are documented in `native/aros/toolchain/`:

```sh
AFSPLUS_AROS_M68K_LLVM_LIB=/path/to/llvm/install/lib \
AFSPLUS_AROS_M68K_CARGO=/path/to/dedicated-cargo \
AFSPLUS_AROS_M68K_RUST_TARGET_JSON=native/aros/m68k-unknown-aros-m68000.json \
AFSPLUS_AROS_M68K_MODEL=A500 \
AFSPLUS_AROS_M68K_CPU_SPEED=max \
AFSPLUS_AROS_M68K_FAST_MEMORY=8192 \
AFSPLUS_AROS_M68K_ZORRO_III_MEMORY=0 \
AFSPLUS_AROS_M68K_BOOT_ADF=/path/to/bootdisk-amiga-m68k.adf \
AFSPLUS_AROS_M68K_SYSTEM_ISO=/path/to/aros-amiga-m68k.iso \
    tools/check-aros-m68k-alpha0-fsuae.sh
```

The gate compiles every native object with `-m68000`, rejects M68020 long
multiply opcodes in the linked handler, and records the selected CPU and LLVM
dylib hash. ADR-057 records the accepted Alpha-0 and six-replay result. No
standard-library `Vec` workaround is required.

ADR-058 adds guest `Avail FLUSH` samples around two sequential handler
instances. On the accepted 8-MiB profile, the DOS-loaded external segment costs
about 1.95 MiB, an active instance adds about 1.08 MiB, and two shutdowns differ
by only 64 bytes. A 4-MiB boot-only control does not reach its first guest
verdict, so that lower result cannot be attributed to AFS+.

`afsram.device` writes only the retained boot image. `CMD_UPDATE` therefore
tests the filesystem/device ordering path but cannot make data survive reset.
The extracted replay proof is not reset durability: persistent native-device,
controlled in-guest power-cut and Apple-hardware execution are separate gates.
The package contract orders its four validation stages as Hosted MacAROS,
Amiga 500/m68k emulation, the physical Amiga 500, then native MacAROS on Apple
Silicon. The last gate applies after a bare-metal MacAROS target exists.

The fixed-image Alpha-0 path excludes `TD_ADDCHANGEINT` handling. Hot-swappable
media is disabled unless removal can detach the mounted Rust instance and DOS
volume without racing outstanding locks.

The experimental m68k Rust `std` toolchain builds and runs the complete
reference handler for both M68020+ and plain M68000. It is a functional and
recovery path rather than the production A500 profile: the compiler/PAL is
experimental, and classic memory and performance budgets plus the
physical-machine gate are separate qualifications. A bounded `no_std + alloc`
or portable-C profile is the fallback for machines where the Rust profile is
too costly.
