# Filesystem API v2: ABI freeze-0

> **ADRs:** [ADR-039](../adr/ADR-039-portable-vfs-slice.md),
> [ADR-042](../adr/ADR-042-aros-c-boundary.md) · **Spec:** none ·
> **Tests:** [test strategy](../testing/test-strategy.md) · **Milestones:** M07

Target on acceptance: a numbered ADR fixes the architecture, then
[`api/filesystem_v2.h`](../api/filesystem_v2.h) and
[`docs/13-filesystem-api-v2.md`](../docs/13-filesystem-api-v2.md) become the
normative v1 contract. Nothing in this proposal is implemented or frozen.
Decisions requested from the team are marked **D1-D8**.

<!-- toc -->

- [Summary](#summary)
- [1. Compatibility boundary](#1-compatibility-boundary)
- [2. Two ABIs, one semantic contract](#2-two-abis-one-semantic-contract)
  - [2.1 Client ABI](#21-client-abi)
  - [2.2 Handler transport](#22-handler-transport)
- [3. Versioning and C layout](#3-versioning-and-c-layout)
- [4. Identity and lifetime](#4-identity-and-lifetime)
- [5. Strings and names](#5-strings-and-names)
- [6. Errors and atomicity](#6-errors-and-atomicity)
- [7. Capabilities and limits](#7-capabilities-and-limits)
- [8. Durability](#8-durability)
- [9. V1 operation slice](#9-v1-operation-slice)
- [10. Expected provider profiles](#10-expected-provider-profiles)
- [11. Conformance gates](#11-conformance-gates)
- [12. Delivery sequence](#12-delivery-sequence)
- [13. Decisions requested from the team](#13-decisions-requested-from-the-team)
- [14. Out of scope](#14-out-of-scope)

<!-- /toc -->

## Summary

Filesystem API v2 should be an optional, filesystem-neutral service above an
Amiga-family filesystem handler. It must not replace the classic DOS packet
surface and must not make a handler depend on the presence of the v2 client
library. AFS+ and exFAT can therefore expose the same modern API while keeping
different on-disk semantics and different capability sets.

The current prototype is not an ABI:

- `api/filesystem_v2.h` declares values and data shapes, but no discovery,
  provider, operation or lifetime contract;
- `afsplus-vfs` is an executable Rust API over an AFS+ `Volume`, not a C ABI;
- C and Rust assign different meanings to the same capability bits;
- the current object-ID-first model cannot represent exFAT honestly because
  exFAT has no persistent inode identity to promise across rename, media
  removal and remount.

Freeze-0 separates three things that must not be conflated:

```text
application
    |
    | filesystem2.library client ABI
    v
filesystem-neutral request transport
    |
    | optional ACTION_FSV2_* packets
    v
handler task -> filesystem adapter -> filesystem core
```

Classic applications continue to use existing `ACTION_*` packets. A handler
that has never heard of v2 returns `ERROR_ACTION_NOT_KNOWN`; a v2-aware client
turns that into `FSV2_ERR_NOT_SUPPORTED`. A v2-capable handler still mounts and
serves classic applications when `filesystem2.library` is absent.

## 1. Compatibility boundary

Adopting v2 is additive at both source and runtime level:

- no existing DOS action changes number, layout or meaning;
- handler startup never requires a v2 library;
- v2 registration/discovery is optional and may be compiled out;
- the filesystem core contains no AROS library, BPTR, BSTR or packet types;
- AmigaOS, MorphOS and older AROS builds keep their classic front-end and may
  add a native v2 transport later without changing the core;
- one source tree may build several front-ends, but one binary is not promised
  to run unchanged across the different operating systems.

The existing [`api/afsplus_aros.h`](../api/afsplus_aros.h) remains a private
embedding boundary between the AFS+ Rust implementation and its native C
handler. It is not the public filesystem API and exFAT must not implement it.

## 2. Two ABIs, one semantic contract

### 2.1 Client ABI

Applications call an ordinary versioned library interface, provisionally
named `filesystem2.library`. Its functions accept native pointers but use
fixed-width integers for every semantic scalar. The library owns discovery,
request/reply marshalling and fallback detection.

The library interface is versioned independently from individual request
structures. Opening version 1 guarantees only the v1 entry points; new entry
points append to a size-delimited function table.

### 2.2 Handler transport

AROS handlers are message-driven tasks. Returning a pointer to a provider
vtable and calling it directly from an application task would bypass handler
serialization, give callbacks the wrong task context and contradict the
single-packet-task lifetime qualified by ADR-042. The handler side therefore
remains packet based.

`ACTION_FSV2_QUERY` negotiates the protocol version and returns capabilities
and limits into caller-owned storage. The remaining `ACTION_FSV2_*` requests
carry a pointer to a size-delimited request/reply structure. The client library
sends them to the mounted volume's handler port and waits for the ordinary DOS
reply.

The exact action values are allocated only when the proposal becomes an ADR.
They must live in a generic AROS namespace, never an AFS+ range. A focused AROS
patch may publish those constants and the client library, but an external
header copy keeps AFS+ usable before or without upstream acceptance.

**D1 - Transport.** Accept the two-level design: public library interface plus
optional generic DOS-packet transport, rather than direct calls into a handler
vtable. Recommendation: accept; it preserves handler task ownership and makes
old-handler detection identical to existing packet extensions.

## 3. Versioning and C layout

Every public structure starts with:

```c
uint32_t abi_version;
uint32_t struct_size;
```

The rules are:

- callers zero the whole structure, set `abi_version` and `struct_size`, and
  leave reserved fields zero;
- callees read only the prefix covered by `struct_size` and write only within
  that prefix;
- a larger known-version structure is accepted and its unknown tail ignored;
- an unsupported version returns `FSV2_ERR_VERSION` without partial effects;
- fields are appended, never reordered or repurposed inside one ABI version;
- enums cross the boundary as `uint32_t`, not as C `enum` objects;
- offsets, sizes, object identities and cookies are explicitly 64 bit;
- buffer lengths are `uint64_t` semantically and checked against native
  `uintptr_t`/address-space limits before pointer arithmetic;
- no allocation is freed by a component other than the one that allocated it;
  v1 operations use caller-owned buffers only;
- the in-memory ABI is native-endian and is not an on-disk format.

Architecture-specific pointer width means 32-bit and 64-bit structure sizes
may differ. Explicit size assertions are required for both targets, as in the
existing AFS+ AROS boundary. `struct_size` permits extension; it does not claim
that a 32-bit binary and a 64-bit binary share a byte-identical pointer-bearing
layout.

**D2 - Layout discipline.** Make version/size prefixes, fixed-width semantic
scalars, caller-owned buffers and append-only extension mandatory for every
v1 structure. Recommendation: accept and remove `size_t` plus raw C enums from
the current experimental header before freezing it.

## 4. Identity and lifetime

V1 distinguishes an operational reference from a persistent identity:

```c
typedef uint64_t FSV2_NodeRef;   /* opaque, mount-instance scoped */
typedef uint64_t FSV2_Handle;    /* opaque, open-instance scoped */
typedef uint64_t FSV2_ObjectId;  /* persistent only when capability says so */
typedef uint64_t FSV2_DirCookie; /* opaque, directory-handle scoped */
```

`NodeRef` is sufficient for lookup, open and namespace operations. It remains
valid until explicitly released, media change, unmount or provider-instance
reset. Its numeric value has no application-visible meaning. This lets exFAT
maintain a bounded in-memory reference even when moving a directory entry
changes its physical location.

`ObjectId` is returned separately. Zero means unavailable. A non-zero value is
promised only with `FSV2_CAP_PERSISTENT_OBJECT_IDS`: it names the same object
across handles, rename and clean remount of the same volume. Applications that
need indexing or change-stream correlation query the capability first.

Handles and cookies are never paths. A directory cookie is valid only for the
directory handle and enumeration epoch that produced it. A provider returns
`FSV2_ERR_STALE` rather than resuming against a different namespace view.

All references become stale on media removal even if a new medium has the same
filesystem type. The provider-instance nonce is not exposed but must prevent a
numeric reference from accidentally becoming valid for replacement media.

**D3 - Identity split.** Add mount-scoped `NodeRef` and reserve `ObjectId` for
the stronger persistent promise. Recommendation: accept; otherwise exFAT must
either lie about stability or cannot implement the common API.

## 5. Strings and names

All v2 names are well-formed UTF-8 byte spans with an explicit length; they are
not NUL terminated. A provider without `FSV2_CAP_UTF8_NAMES` accepts only the
ASCII subset, which is still valid UTF-8, rather than exposing an unspecified
legacy encoding. Embedded NUL and the API path separator are rejected.
Individual operations take one component, not a host path. Path parsing and
AROS device/assign semantics remain in the client layer.

The filesystem preserves and returns the spelling it accepted. Lookup,
normalization and case sensitivity are volume policy reported by `statfs`;
clients never infer them from the filesystem name. An exFAT adapter performs
the required UTF-8/UTF-16 conversion and reports a case-insensitive namespace.

Directory entry names are written into a caller-provided byte arena. Each
fixed-size entry contains an offset and length into that arena rather than a
pointer, so a page has one unambiguous lifetime and no per-entry allocation.
If either entry capacity or name-arena capacity is exhausted, the provider
returns the completed prefix and a continuation cookie. An entry whose single
name cannot fit returns `FSV2_ERR_BUFFER_TOO_SMALL` plus the required size.

**D4 - Enumeration buffers.** Use a fixed entry array plus a byte arena and an
opaque continuation cookie. Recommendation: accept; it is bounded on classic
machines and efficient on modern systems.

## 6. Errors and atomicity

The current result values keep their numbers. V1 appends at least:

- `ALREADY_EXISTS`
- `NOT_DIRECTORY`
- `IS_DIRECTORY`
- `DIRECTORY_NOT_EMPTY`
- `PERMISSION`
- `BUSY`
- `BUFFER_TOO_SMALL`
- `RANGE`
- `VERSION`
- `MEDIA_CHANGED`
- `NO_MEMORY`

The result is filesystem-neutral. A separate optional `native_error` field may
carry the original DOS error for diagnostics, but applications must branch on
the v2 result.

Each mutating call states its publication boundary. On failure, it must either
have no namespace-visible effect or report an explicit partial-I/O byte count
for `write`; it must never return a generic I/O error after silently publishing
a namespace operation. `rename` and `atomic_replace` are distinct: a provider
advertises atomic replacement only when the old-or-new crash contract is real.

## 7. Capabilities and limits

The C header becomes the sole authority for capability values. Rust imports or
tests those values rather than maintaining a parallel numbering scheme. The
existing experimental C assignments are retained to minimize churn:

| Bit | Capability | Promise |
|---:|---|---|
| 0 | `64BIT_IO` | offsets and file sizes beyond the classic signed-32-bit surface are accepted up to reported limits |
| 1 | `UTF8_NAMES` | names beyond the ASCII subset are accepted as valid UTF-8 |
| 2 | `SYMLINKS` | create and resolve symbolic links |
| 3 | `HARDLINKS` | multiple directory links to one object |
| 4 | `XATTRS` | namespaced extended attributes |
| 5 | `ATOMIC_REPLACE` | replacement publishes old or new namespace state after crash |
| 6 | `PERSISTENT_OBJECT_IDS` | non-zero IDs survive rename and clean remount |
| 7 | `PAGED_DIRECTORIES` | bounded cookie-based enumeration |
| 8 | `CHANGE_STREAM` | persistent ordered change enumeration |
| 9 | `SPARSE` | holes read as zero without allocated backing storage |
| 10 | `FSYNC` | explicit durability operations obey section 8 |
| 11 | `CLONE_FILE` | whole-file shared-data clone |
| 12 | `CLONE_RANGE` | representable-range shared-data clone |
| 13 | `WATCH` | live notification interface |

Bit 7 is renamed from the ambiguous `FAST_ENUMERATION`: performance is not a
binary semantic promise, while bounded paging is. No retired capability number
is ever reused after v1 freezes.

Capabilities describe the mounted provider instance, not what the on-disk
format could theoretically support. Read-only policy may remove mutating
capabilities. Query results include a monotonically changing instance token so
clients can discard cached capabilities after media or mount-policy change.

Limits are values, not capability bits: maximum component bytes, maximum I/O
request bytes, maximum directory entries per page, maximum xattr bytes,
supported clone alignment and granularity, and whether the namespace is case
sensitive. Zero means the operation is unsupported only where the field says
so; it never means "unlimited".

**D5 - Capability authority.** Preserve the current C bit allocation, rename
bit 7 to the semantic `PAGED_DIRECTORIES`, append clone/watch bits and make the
C definition authoritative through generated bindings or equality tests.
Recommendation: accept.

## 8. Durability

V1 defines three operations:

- `flush_handle`: make dirty data and metadata associated with one handle
  eligible for writeback; no independent crash-durability promise;
- `fsync_handle`: after success, prior writes and required metadata for that
  handle survive a modelled power loss according to the provider's published
  storage contract;
- `sync_filesystem`: establish the same durability boundary for all completed
  operations on the mounted instance before the call.

Directory fsync uses a directory handle and makes prior namespace mutations in
that directory durable. A provider that cannot distinguish file and directory
fsync may conservatively sync the whole filesystem. It must not advertise
`FSYNC` if the implementation merely empties a software cache without issuing
the device durability barrier required by its storage contract.

Successful close does not implicitly acquire stronger semantics than
`flush_handle`. Classic DOS close behaviour remains whatever that classic API
already promises; v2 callers request durability explicitly.

**D6 - Durability.** Freeze the distinction between writeback, handle fsync and
filesystem sync. Recommendation: accept; it lets AFS+ expose checkpoint/log
semantics and lets exFAT advertise fsync only when `CMD_UPDATE` is a sufficient
device barrier.

## 9. V1 operation slice

Freeze only the common executable slice:

- discovery: query version, capabilities, limits and filesystem statistics;
- navigation: get root, lookup one component, retain/release node reference;
- handles: open/close file or directory;
- I/O: read, write, seek position, truncate, flush and fsync using 64-bit
  offsets and lengths;
- namespace: create file, create directory, unlink, remove directory, rename
  and optional atomic replace;
- metadata: stat, protection and timestamps when supported;
- enumeration: bounded directory pages;
- synchronization: handle fsync and filesystem sync.

Hard links, symlinks, xattrs, clone, full-volume enumeration, change stream and
watch have reserved capability identities but their function structures are
appended only with an executable provider and conformance tests. Reserving a
bit does not advertise or stabilize an unimplemented operation.

`stat` contains both `NodeRef` and optional persistent `ObjectId`, logical size,
allocated size, type, link count when meaningful, protection, timestamps and a
valid-fields mask. A provider does not manufacture zero-valued metadata and
pretend it is meaningful.

**D7 - Initial scope.** Freeze the common slice first and append advanced
function groups behind independent size/version negotiation. Recommendation:
accept; it makes AFS+ and exFAT useful without forcing either to emulate the
other.

## 10. Expected provider profiles

The table is a validation target, not a capability decision made in advance:

| Semantics | AFS+ prototype | exFAT handler |
|---|---|---|
| classic DOS packets | retained | retained |
| 64-bit I/O | yes | yes, already using `DosPacket64` where required |
| UTF-8 API names | yes | adapter conversion from/to on-disk UTF-16 |
| case-sensitive namespace | per-volume policy | no |
| paged directories | yes | implementable without changing disk format |
| persistent object IDs | yes | no unless evidence establishes a stable identity contract |
| hard links/symlinks/xattrs | capability dependent | not advertised |
| sparse files | yes | not advertised until hole semantics are qualified |
| fsync | checkpoint/log contract | advertise only after the device-barrier contract is tested |
| clone file/range | ADR-061 work | not advertised; clients copy as fallback |
| change stream | future | not advertised |

The fallback belongs to the client or application. A provider returning
`NOT_SUPPORTED` for clone must not silently perform a full byte copy under a
call whose name promises shared storage.

## 11. Conformance gates

Acceptance of the ABI requires tests independent of the AFS+ implementation:

1. C headers compile with the supported AROS AArch64 Clang and m68k GCC, with
   size/offset assertions for both pointer widths.
2. Rust capability numbers and result values are proven equal to the C
   authority; a change on only one side fails CI.
3. A mock handler proves version/size prefix handling, zero-reserved-field
   validation, unknown-tail tolerance and precise old-handler fallback.
4. Caller-buffer tests cover zero capacity, too-small name arena, short I/O,
   null/misaligned spans and offset/length overflow before dereference. Address
   validity is checked where the host provides a reliable primitive; a flat
   Amiga-family address space is not misrepresented as a memory-safety boundary.
5. Handle, node and cookie tests cover close, release, namespace mutation,
   unmount and media replacement; stale values never become valid again.
6. AFS+ runs its existing operation matrix through the client ABI and remains
   checker-clean after remount.
7. exFAT runs the equivalent supported subset and returns `NOT_SUPPORTED` for
   every unadvertised group.
8. An old handler binary mounts and serves classic applications unchanged on a
   system that has the new client library installed.
9. A v2-capable handler mounts and serves classic applications on a system
   without the client library.
10. Resource measurements report fixed per-request overhead and bounded
    directory-page memory on the classic profile.

Fuzzing targets every size-delimited request decoder. The packet receiver
validates action, ABI version, structure size, buffer span, arithmetic and
handle ownership before reaching filesystem code.

## 12. Delivery sequence

| Step | Change | Exit gate |
|---:|---|---|
| 1 | Team review of D1-D8 | accepted ADR, no code |
| 2 | Replace the experimental C header and align Rust constants | cross-language constant and layout tests |
| 3 | Implement mock client/handler transport | old/new compatibility and hostile-request tests |
| 4 | Adapt `afsplus-vfs` and the native AFS+ handler | existing Alpha-0 matrix through v2, classic matrix unchanged |
| 5 | Implement the optional exFAT adapter | shared conformance subset plus explicit unsupported matrix |
| 6 | Propose the generic AROS constants/library upstream | AFS+ and MacAROS remain buildable without upstream acceptance |

The virtual disk-activity LED is deliberately not part of the application
filesystem ABI. It observes physical block-device operations, so its generic
form belongs at the handler/device adapter boundary. AFS+ and exFAT may feed the
same optional sink without exposing it as a filesystem capability.

## 13. Decisions requested from the team

- **D1:** public library plus packet transport, not a direct handler vtable.
- **D2:** version/size-prefixed C structures and caller-owned buffers.
- **D3:** mount-scoped `NodeRef` distinct from persistent `ObjectId`.
- **D4:** entry array plus name arena for bounded enumeration.
- **D5:** C-authoritative capability numbering and semantic bit 7 rename.
- **D6:** distinct flush, handle fsync and filesystem sync contracts.
- **D7:** freeze only the common operation slice; append advanced groups later.
- **D8:** keep v2 entirely optional and retain classic DOS packets forever for
  the compatibility profiles described here.

Recommendation: accept all eight as one coherent boundary. Rejecting D1 or D3
requires a replacement that still preserves handler task ownership and permits
an honest exFAT implementation; the other decisions can be revised
independently before the numbered ADR is written.

## 14. Out of scope

- changing either filesystem's on-disk format;
- making exFAT implement AFS+-only semantics;
- promising one handler binary across AROS, AmigaOS and MorphOS;
- choosing an upstream governance or library-distribution mechanism;
- exposing physical disk activity to ordinary applications;
- freezing clone/change-stream/watch function layouts before their providers
  and consumers are executable.
