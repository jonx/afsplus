# ADR-043: Native AROS DosPacket translator

Status: Accepted

## Context

ADR-041 defines packet-neutral DOS semantics and ADR-042 exposes them through a
versioned C ABI. A native handler still needs to translate AROS `DosPacket`,
`BPTR`, `BSTR`, `FileLock`, `FileHandle`, `FileInfoBlock`, `InfoData` and
`DosPacket64` layouts without putting target headers or unsafe pointer rules
inside Rust.

The translation cannot treat a numeric Rust lock or file ID as an `IPTR`.
That would truncate identifiers on m68k. It also cannot assume that a 64-bit
AROS target selects the 64-bit `FileInfoBlock`: `__WORDSIZE` controls packet
field width, while `__DOS64` independently selects the default DOS structure
aliases. Explicit `ACTION_EXAMINE_*64` and `ACTION_INFO64` packets always carry
the wide structures.

## Decision

[`native/aros/afsplus_packet.c`](../native/aros/afsplus_packet.c) is the shared native packet translator. It is
compiled as ordinary C against genuine AROS headers and calls only
[`api/afsplus_aros.h`](../api/afsplus_aros.h). It performs no Exec or DOS library calls. A small handler
shell remains responsible for startup, block-device callbacks, receiving and
replying to messages, and public-memory allocation.

The translator owns linked native wrappers. A `FileLock` is the first member
of a wrapper that also contains the complete 64-bit adapter lock ID. A file
wrapper containing the complete 64-bit file ID is stored behind `fh_Arg1`.
Incoming wrapper pointers are validated by identity against the context-owned
lists before they are dereferenced. No 64-bit identifier is stored in
`fl_Key`, `fh_Arg1` or another 32-bit scalar.

DOS paths are resolved component by component through the C boundary. The
last colon resets resolution to the volume root, slash separates components,
and an empty component means parent. Namespace operations resolve their parent
and pass only the final component to the safe adapter. Temporary locks are
released on both success and failure.

Native wrappers for create/truncate opens and directory creation are reserved
before the mutating Rust call. An allocation failure therefore cannot mutate
the namespace and then report failure. Packet output pointers are checked
before stateful operations such as `ExamineNext`, so a rejected output cannot
advance enumeration.

`ACTION_END` calls per-file `fsync` for writable handles before consuming the
handle. `ACTION_FLUSH` is the filesystem-wide durability barrier. The
translator supports the Alpha-0 lock, open, read/write, seek/truncate,
namespace, examine, disk-info, flush and lifecycle actions. Unsupported
actions return `ERROR_ACTION_NOT_KNOWN`.

On 32-bit targets, the four AmigaOS 4 compatible position/size actions require
`DosPacket64.dp_Res0 == DP64_INIT` and preserve that overlay while writing the
wide result. `ACTION_SEEK64` and `ACTION_SET_FILE_SIZE64` are rejected there
because their MorphOS-style arguments do not fit the standard packet. On all
targets, explicit 64-bit examine/info actions use the wide structures. Legacy
32-bit `FileInfoBlock` and `InfoData` counters are saturated rather than
silently wrapped.

The handler supplies an allocation callback returning suitably aligned public
memory, its matching free callback, and a UTC Unix time callback. Stored AFS+
timestamps are converted to the 1978 AROS `DateStamp` epoch at presentation
time. At most 107 filename bytes may be configured for the packet mount so the
BCPL length byte still fits in `fib_FileName`.

## Consequences

The packet logic has a runnable host matrix using the real AROS AArch64
layouts and stubbed C-boundary operations. It covers path reset/parent
semantics, BPTR wrappers, failed-allocation non-mutation, invalid-output
non-advancement, 64-bit seek/size, 32/64-bit FIB and disk-info presentation,
durable close and `ACTION_DIE`. The same source compiles with `-Werror` as:

- AROS AArch64 using the normal DOS structure aliases;
- AROS AArch64 with `__DOS64=1`; and
- AROS m68k using `DosPacket64` compatibility layouts.

This still does not prove a native mount. The remaining MacAROS work is the
small handler shell and block-device adapter, link integration, then the
same-image operation and crash-replay qualification. No MacAROS source tree is
modified by this decision.
