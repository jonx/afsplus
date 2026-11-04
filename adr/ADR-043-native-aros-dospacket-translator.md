# ADR-043: Native AROS DosPacket translator

Status: Accepted

## Context

ADR-041 defines packet-neutral DOS semantics and ADR-042 exposes them through a
versioned C ABI. A native handler still needs to translate AROS `DosPacket`,
`BPTR`, `BSTR`, `FileLock`, `FileHandle`, `FileInfoBlock`, `InfoData` and
`DosPacket64` layouts without putting target headers or unsafe pointer rules
inside Rust.

The translation cannot treat a numeric Rust lock or file ID as an `IPTR`.
That would truncate identifiers on m68k. Packet width and structure layout
are separate contracts: the MorphOS-compatible FIB has classic counters and
64-bit size/block fields in its reserved area, while InfoData64 appends full
block counts to the standard InfoData prefix.

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

The MorphOS-compatible seek and resize actions take a pointer to a signed
64-bit offset in Arg2, the origin in Arg3 and a pointer to the signed old
position or resulting size in Arg4. Record lock/free actions take pointers to
unsigned 64-bit offset and length in Arg2/Arg3; lock mode and timeout retain
Arg4/Arg5. These packets work at either pointer width. Success is DOSTRUE,
failure is DOSFALSE with Res2 set; seek/resize results go through Arg4 only on
success. NULL required pointers return ERROR_BAD_NUMBER before changing file
state. Byte copies accept the two-byte alignment used by 32-bit DOS structures
and load inputs before writing potentially aliased outputs.

On 32-bit targets, the four AmigaOS 4 compatible position/size actions require
DosPacket64.dp_Res0 == DP64_INIT and preserve that overlay while writing the
wide result. On 64-bit targets those four actions retain their scalar form.

Explicit 64-bit examine actions fill fib_Size64 and fib_NumBlocks64 in the
standard FileInfoBlock, and clear each classic counter to zero when its value
exceeds INT32_MAX. Classic examine counters saturate at INT32_MAX. INFO64
fills the appended id_NumBlocks64/id_NumBlocksUsed64; its classic prefix and
classic info packets saturate their counters. ExAll ED_OWNER stops immediately
before ed_Size64. ED_SIZE64 includes the full structure and writes zero to
ed_Size when the size exceeds INT32_MAX; lower detail levels retain unsigned
32-bit saturation.

INFO and INFO64 accept a context-owned lock or the null root lock. INFO64
rejects a live file-handle token with ERROR_ACTION_NOT_KNOWN before interpreting
its output pointer. This refuses the console-geometry packet proposed with the
same action number 2020, whose Arg1 is fh_Arg1 and whose Arg2 is a raw pointer
to a smaller structure. Other invalid locks return ERROR_INVALID_LOCK. This
validation does not allocate distinct action numbers for the two interfaces.

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
durable close and `ACTION_DIE`. The source-header development matrix exercises AArch64 layouts. Target
qualification compiles the translator against generated AArch64 and m68k SDK
headers, including DosPacket64 compatibility layouts on m68k. It requires an
SDK with the MorphOS-compatible structures and pointer packet convention.

This still does not prove a native mount. The remaining MacAROS work is the
small handler shell and block-device adapter, link integration, then the
same-image operation and crash-replay qualification. No MacAROS source tree is
modified by this decision.
