# ADR-045: Native AROS handler shell assembly

Status: Accepted

## Context

ADR-042 through ADR-044 leave three independently qualified pieces: the Rust
static library, the native `DosPacket` translator and the bounded trackdisk
viewport. A real filesystem process must still acquire their operating-system
resources in the only safe order, publish a DOS volume only after mount has
succeeded, and unwind every partial initialization path.

This assembly is target-specific, but it must not duplicate filesystem,
packet, partition or transaction semantics. It also cannot assume that an AROS
device accepts 64-bit offsets, arbitrary buffers, writes, or short transfers.

A handler is not an AROS command: it has a generated Resident entry and is
entered as `handler(SysBase)`, so the normal command `startup.o`/`main` link
recipe is not valid. The MacAROS Rust `std` port also expects its platform glue
objects and the `aros_argc`/`aros_argv` process globals even when the filesystem
core itself does not use networking or child processes.

## Decision

[`native/aros/afsplus_handler.c`](../native/aros/afsplus_handler.c) is the native shell source. It receives the
startup `DosPacket`, validates its `FileSysStartupMsg` and bounded `DosEnvec`,
opens the requested device and refuses a definitely absent medium. It probes
write protection before selecting read-only or read-write mount mode.

The shell first validates a format-0 `NSCMD_DEVICEQUERY`, including the
returned sizes and both members of a command pair. It falls back to zero-length
TD64 probes and finally to legacy commands. A partition beyond 4 GiB is still
rejected by ADR-044 unless that process positively selected a usable 64-bit
path.

Each actual transfer writes both offset halves, calls `DoIO`, and requires the
reported byte count to equal the complete logical block. `CMD_UPDATE` implements
the physical durability barrier. `de_MaxTransfer` is honored when present. If
a `de_Mask` is present, buffers outside it use a handler-owned 4-KiB bounce
buffer allocated with the advertised `de_BufMemType`; the mount fails if no
compliant buffer can be obtained.

Only after the Rust mount succeeds does the shell create and register its DOS
volume node and packet context. Packet-wrapper allocation uses public memory.
The clock callback converts the local 1978 `DateStamp` to Unix UTC using the
optional locale GMT offset; timestamps still work with a zero offset when
locale.library is unavailable.

The handler then processes one message at a time. `ACTION_DIE` is accepted only
after the packet layer has checked open resources and completed a filesystem
flush. Shutdown destroys packet-owned wrappers, removes the volume entry,
unmounts, closes the device and libraries, and frees resources in reverse order.
The startup packet is replied even when the first handler allocation fails.

`native/aros/afsplus.conf` is the `genmodule` input. It assigns the Alpha-0 DOS
type `0x4146532b` (`AFS+`) and generates the actual Resident/SegList entry around
the target-neutral `handler` function. Because no command-line entry exists,
the shell exports an intentionally empty `aros_argc`/`aros_argv` pair for the
life of the handler process.

## Consequences

The AArch64 shell compiles against genuine MacAROS headers. Qualification first
links it relocatably with the packet translator, trackdisk adapter and Rust
static library and rejects unresolved AFS+ symbols. It then runs `genmodule`,
compiles MacAROS's Rust platform glues, and produces a complete AROS `ET_REL`
handler module with no undefined symbols. The generated entry and shell also
compile with the m68k SDK, while the m68k Rust library remains outside the
supported Alpha-0 path described by ADR-042.

ADR-056 subsequently supplies that m68k Rust library and qualifies the complete
shell under native AROS for the M68020-or-newer emulator profile. ADR-057 adds
the corresponding A500-configured plain-M68000 emulator result; neither result
is a physical-hardware claim.

[`tools/package-aros-alpha0.sh`](../tools/package-aros-alpha0.sh) turns the qualified off-tree link into a
self-contained pre-install artifact: handler, DOSDriver, target operation
probe, full-length sparse image, clean host-checker report and hashes. This is
still not a runtime-mount claim. ADR-046 through ADR-050 subsequently qualify
Hosted execution, recovery, the system pivot and unload/reload lifecycle. An
in-tree AROS build is now optional rather than a release gate; removable-media
notification handling remains required before hot-swappable devices are
supported. No MacAROS source tree is modified by this decision.
