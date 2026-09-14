# ADR-041: Packet-neutral AROS DOS adapter

Status: Accepted

## Context

The portable VFS from ADR-039 deliberately contains no AROS path syntax,
BPTR/BSTR handling or DOS packet result conventions. A native MacAROS handler
still needs stateful `FileLock` and `FileHandle` semantics, 64-bit positions,
`Examine` iteration and exact `IoErr()` values. Putting those rules directly in
a message-port loop would make them difficult to qualify on the host and would
duplicate filesystem behavior already exposed by `afsplus-vfs`.

The AROS headers and existing hosted and PFS3 handlers establish the relevant
contract: `ACTION_FINDINPUT`, `ACTION_FINDOUTPUT` and `ACTION_FINDUPDATE` map to
old-file, truncate/create and update/create modes; reads, writes and seeks use
file-handle state; `ACTION_EXAMINE_NEXT` advances state associated with a lock;
and `ACTION_FLUSH` is the filesystem-wide durability operation. MacAROS uses
64-bit `SIPTR` packet fields. A future 32-bit port must instead decode the
documented `DosPacket64` layouts where a standard packet cannot carry the
value directly.

## Decision

`afsplus-aros` is a safe, packet-neutral adapter over `afsplus-vfs`. It owns:

- bounded numeric lock and file-handle tables;
- shared/exclusive lock conflicts and lock duplication;
- lexical parent context for `Parent`, `ParentOfFH` and hard-linked files;
- independent 64-bit positions for every open file;
- old-file, new-file and read/write open behavior;
- read, write, seek, truncate, fsync and filesystem flush;
- create-directory, type-aware delete, rename and hard links;
- stateful, generation-checked directory enumeration;
- AROS `FileInfoBlock` and `InfoData` presentation values;
- configurable UTF-8 or ISO-8859-1 name conversion; and
- exact DOS secondary error numbers used by the implemented operations.

The null DOS lock maps to the AFS+ root object. A lock created while resolving
a component records its lexical parent; object identity remains the AFS+
object ID and is never derived from a path. Directory cookies remain 64-bit in
the adapter and fit MacAROS's `IPTR`-sized `fib_DiskKey`. The classic
`FileInfoBlock` name limit is enforced explicitly rather than truncating an
AFS+ name.

The native MacAROS layer will only:

1. receive and reply to `DosPacket` messages;
2. validate and translate BPTR, BSTR, buffers and AROS structures;
3. walk multi-component DOS names through component operations;
4. convert `DateStamp` values; and
5. select the native or 32-bit compatibility layout for 64-bit actions.

It must not reinterpret on-disk state or maintain a second namespace model.
No change to the MacAROS repository is part of this decision; native build
integration remains a separate, explicitly qualified step.

## Consequences

The AROS operation slice now runs on an ordinary host against the same VFS and
raw image semantics as FUSE. Tests cover create/read/write/sparse seek,
truncate, rename, hard-link, fsync, flush, enumeration, remount and the strict
checker, plus Latin-1 conversion, lock conflicts and read-only errors.

This does not by itself prove a native MacAROS mount. ADR-042 defines the
C/staticlib boundary and ADR-043 now supplies the cross-qualified packet
translator. Alpha-0 still requires them to be linked into a MacAROS handler and
the same-image workflow to pass there before that gate can be closed.
