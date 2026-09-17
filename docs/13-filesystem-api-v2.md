# 13. Filesystem API v2

> **ADRs:** [ADR-039](../adr/ADR-039-portable-vfs-slice.md), [ADR-075](../adr/ADR-075-revocable-backup-capability.md) · **Spec:** none ·
> **Tests:** [test-strategy](../testing/test-strategy.md) · **Milestones:** M07

<!-- toc -->

- [1. Purpose](#1-purpose)
- [2. Design principles](#2-design-principles)
- [2.1 Mount policy](#21-mount-policy)
- [3. Core operations](#3-core-operations)
  - [handles and I/O](#handles-and-io)
  - [namespace](#namespace)
  - [metadata](#metadata)
  - [enumeration](#enumeration)
  - [synchronization](#synchronization)
  - [cloning](#cloning)
  - [preallocation and access intent](#preallocation-and-access-intent)
  - [observation](#observation)
- [4. Compatibility adapters](#4-compatibility-adapters)
  - [The AROS transport](#the-aros-transport)
- [5. Large files](#5-large-files)
- [6. Rust](#6-rust)
- [7. Zed](#7-zed)
- [8. Trusted snapshot backup extension](#8-trusted-snapshot-backup-extension)
  - [Captured metadata inventory knowledge](#captured-metadata-inventory-knowledge)
  - [Opaque captured metadata transport](#opaque-captured-metadata-transport)
  - [Captured allocation enumeration](#captured-allocation-enumeration)
  - [Versioning and compatibility](#versioning-and-compatibility)
- [9. Destination-scoped restore extension](#9-destination-scoped-restore-extension)
  - [Committed destination allocation readback](#committed-destination-allocation-readback)
  - [Staged opaque metadata restoration](#staged-opaque-metadata-restoration)
  - [Scoped created-entry lookup](#scoped-created-entry-lookup)
  - [Scoped directory emptiness](#scoped-directory-emptiness)
  - [Scoped symlink target transport](#scoped-symlink-target-transport)

<!-- /toc -->

## 1. Purpose

Filesystem API v2 provides modern operations without breaking the classic AROS DOS ABI.

Classic applications continue to call existing DOS APIs.

Internally:

```text
classic DOS call -> compatibility translation -> 64-bit filesystem operation
modern API call  -> direct modern operation
```

## 2. Design principles

- 64-bit clean
- capability-driven
- filesystem-neutral
- iterator/stream oriented for large result sets
- no private AFS+ assumptions
- stable object identity
- explicit durability
- explicit encoding
- structured errors

## 2.1 Mount policy

The portable core distinguishes `ReadWrite`, `ReadOnly`, `NoChanges`, and
`Recovery`. `ReadOnly` and `NoChanges` expose the last checkpoint without
replaying a pending intent log; callers can query the pending-record count.
`NoChanges` has a strict zero-write contract. `Recovery` may write only while
performing mandatory replay and returns a read-only recovered view.

## 3. Core operations

The executable Rust subset lives in `afsplus-vfs` (ADR-039). It currently
covers handles, caller-buffer 64-bit I/O, truncate, paged directories,
stat/statfs, create/mkdir/unlink/rmdir/rename/atomic replace/hard links, and
explicit sync. On a volume carrying the shared-extents feature it also exposes
filesystem-neutral `CloneFile` and `CloneRange` operations and advertises each
capability separately. A volume carrying the intent-log data-update feature
advertises `LOGGED_DATA_FSYNC`; this means existing-file writes and truncates
can satisfy `fsync` through the bounded log instead of publishing a checkpoint
per call. Unsupported categories below are not advertised in the capability
mask.

A volume carrying the orphan-directory feature advertises `OPEN_UNLINKED`
(additive Rust capability bit 12). The VFS counts handles by stable object ID:
every final file unlink and replacement uses the bounded orphan transition,
while existing handles continue to read, write, truncate and fsync. A target
with no handle becomes cleanup-eligible immediately. New opens or stat by a
guessed ID cannot rediscover the orphan. `pending_orphans` is diagnostic state,
and `resume_one_orphan` gives adapters an explicitly bounded idle-maintenance
hook; read-write mount and filesystem sync each advance at most one orphan.

`statfs` reports whether the mounted namespace is case-sensitive plus the
three-part Unicode table version. `free_blocks` is the raw checkpoint count;
`available_blocks` subtracts the runtime emergency-metadata headroom and is
the value adapters advertise for normal growth. Adapters must expose these
volume policies rather than infer them from the host OS or filesystem name.

Required categories:

### handles and I/O

- open
- close
- read
- write
- seek64
- truncate64
- flush file

### namespace

- create
- mkdir
- unlink
- rmdir
- rename
- atomic replace
- link
- symlink
- readlink

ADR-068 proposes these as bounded `CreateSymlink(parent, name, target,
metadata)` and `ReadLink(object, caller_buffer)` operations behind a
`SYMLINKS` capability. Targets are exact NUL-free UTF-8 bytes; the object-ID
VFS neither rewrites nor resolves them, and a short caller buffer receives the
required size without hidden allocation. The proposal remains unimplemented
until accepted.

### metadata

- stat64
- statfs64
- get/set protection
- get/set comment
- get/set xattr
- object ID

### enumeration

- directory iterator
- full-volume object iterator
- change-stream iterator

Directory cookies are opaque to OS adapters. The AFS+ implementation
binds an ordinal to the checkpoint generation and returns `STALE` after any
commit, requiring enumeration to restart rather than mixing namespace views.

### synchronization

- fsync file
- fsync directory
- sync filesystem

The Rust VFS owns one filesystem-wide data-update window. Writes and
truncates are visible immediately through `read` and `stat`, including sparse
ranges and the visible allocated size, but do not advance the checkpoint.
`fsync` on any valid handle makes every operation held in that window
durable. This is deliberately stronger than per-file durability; adapters must
not promise isolation between concurrent writers until the concurrency model
is frozen.

If a group is too large for one log record or the bounded log is full, `fsync`
falls back to an ordinary checkpoint commit. Namespace and reflink mutations
also publish an open data window before starting their own atomic transaction,
and `sync filesystem` publishes it explicitly. Closing a handle alone is not a
durability request. Volumes without `LOGGED_DATA_FSYNC` keep the immediate
checkpoint mutation path, so the API remains usable with older or log-free
images.

`LOGGED_DATA_FSYNC` is additive bit 10 in the Rust capability mask; it changes
no existing operation signature or on-disk identity. FUSE and the AROS Rust
adapter inherit it through the shared VFS. The AROS C bridge keeps its
existing write/truncate/fsync entry points, which receive the same semantics
transparently, and reports the capability through
[`afsplus_aros_capabilities`](../api/afsplus_aros.h) under the published
`FSV2_CAP_LOGGED_DATA_FSYNC` identity. The Rust mask and the C identities of
[`filesystem_v2.h`](../api/filesystem_v2.h) are separate numberings; the bridge
translates, and a Rust bit without a C identity is not advertised to C callers.
An independent portable-C filesystem implementation must negotiate the on-disk
feature and cross-read the conformance corpus before claiming parity.

### cloning

- `CloneFile(source, parent, name)` creates a distinct object and initially
  shares its mapped storage with the source;
- `CloneRange(source_handle, source_offset, destination_handle,
  destination_offset, length)` replaces a destination byte range while
  keeping later writes independent.

The executable prototype accepts range offsets with matching intra-block
alignment. Complete interior blocks are shared, complete source holes remain
holes, and at most two partial boundary blocks are copied privately to retain
the destination bytes outside the requested range. Other alignments and
same-file range cloning return an explicit implementation-limit error.

These are additive, capability-gated operations: they do not change existing
structure layouts or the classic DOS ABI. A profile without shared extents
does not advertise either capability and returns `NOT_SUPPORTED`, allowing a
caller to fall back to an ordinary copy. Read-only mounts may report that the
format supports cloning but still reject mutation as `READ_ONLY`.

### preallocation and access intent

- `Preallocate(handle, offset, length)` reserves storage for a byte range
  without changing the logical size; the range reads as zeros until written.
  It is advertised as `PREALLOCATE` (additive Rust capability bit 14,
  `FSV2_CAP_PREALLOCATE`). One request is bounded by a caller-supplied block
  budget and 64 extent records; beyond that it fails with a limit error and
  the volume unchanged, and the caller splits the reservation.
- `Advise(handle, offset, length, hint)` admits every hint of
  [`performance_hints.h`](../api/performance_hints.h) and returns the effect
  it had. No hint alters durability, contents or allocation. An
  implementation without read-ahead to steer returns "no effect" and the
  caller never assumes one.

The AROS C boundary carries these together with positioned 64-bit read and
write, `CloneFile`, `CloneRange` and atomic replace as the `API_V2`
entry-point group of [`afsplus_aros.h`](../api/afsplus_aros.h). They operate
on the same locks and file handles as the DOS calls, and the positioned calls
neither use nor move the DOS file position.

### observation

- watch
- query capabilities
- query limits

## 4. Compatibility adapters

A legacy handler can expose v2 through an adapter.

Unsupported capabilities return a precise `NOT_SUPPORTED` result.

Applications must not infer capability from filesystem name.

### The AROS transport

On AROS an application reaches these operations in a running handler through
one DOS packet type, `ACTION_AFSPLUS_EXT`
([`afsplus_ext_packet.h`](../api/afsplus_ext_packet.h)). Its first argument
is a request block in the sender's memory: a magic, a version, the size of
the block as the sender knows it, an operation number, and one set of fields
every operation reads its own part of. The block has the same 112-byte layout
on every target, so pointers occupy eight bytes. A handler accepts a longer
block from a newer client and serves the part it knows; it refuses a shorter
one, a foreign magic or version, and an unassigned operation with
`ERROR_BAD_NUMBER`, and an operation of an entry-point group its library
lacks with `ERROR_ACTION_NOT_KNOWN`.

Objects travel as the application holds them: a lock as its `BPTR`, a file as
the `fh_Arg1` of its `FileHandle`. Names are single components; the transport
resolves no path. A report struct in a caller buffer declares its size in its
first field, and the buffer must hold that much.

The block and the buffers are application memory, which another task can
change while the packet is with the handler. The handler copies the block
once and checks and uses only its copy; it reads the declared size of a
report once, has the entry point fill a struct of its own, and copies back
exactly the size it read. It writes back three output fields and nothing else
of the block.

A handler that does not know the packet type answers
`ERROR_ACTION_NOT_KNOWN`, as every handler does, so the absence of the
transport costs a client one request. The client library
([`afsplus_client.h`](../native/aros/client/afsplus_client.h)) falls back
only where a classic call produces the same result: positioned read and write
become seek, transfer and seek back, within what a classic `Seek` expresses
and without protection against another user of the same handle. Clone,
preallocation and the reports have no classic equivalent; the error is the
signal to copy, to do without, or to report nothing. The library never sends
objects of two handlers to one of them.

The packet number is provisional: packet numbers are an AROS-wide allocation,
and the value in the header lies outside every assigned range until one is
made.

## 5. Large files

Classic APIs that accept signed 32-bit offsets retain their historical limits.

The v2 API uses 64-bit offsets and lengths end to end.

`ExtentMap(handle, offset, length)` is the planning query of the
mmap-friendly path: the committed mapping of a byte range as written,
reserved or hole, clipped to the range, without physical addresses, sharing
hints or allocator state. It has the range shape of the backup and restore
allocation readback. It describes committed state only: with unpublished
writes pending it is `BUSY` and commits nothing, and the caller syncs first.
One call returns at most 64 ranges, found in one tree descent wherever the
offset lies; an incomplete answer names the offset to continue from. The
AROS C boundary carries it as the `EXTENT_MAP` entry-point group.

## 6. Rust

The AROS Rust `std` port should bind to v2 for modern file operations.

Rust `Metadata::len()` maps naturally to 64-bit size.

Rust applications must not need AFS+-specific code.

## 7. Zed

Zed-facing requirements include:

- canonical path resolution
- symlink-safe resolution
- stable object IDs
- reliable rename
- recursive watch behavior or equivalent
- 64-bit metadata
- efficient large-directory enumeration

These are platform API requirements, not AFS+ private APIs.

## 8. Trusted snapshot backup extension

[ADR-075](../adr/ADR-075-revocable-backup-capability.md) defines authorization
for the trusted backup service. The Rust extension in `afsplus_vfs::backup`
separates a trusted `SnapshotBackend`, host-owned `BackupService` and issuer,
and a consumer-facing `BackupClient` facade. Consumers use semantic view IDs,
revision IDs, object IDs, metadata, paged names and caller-buffer reads. No
allocation addresses, tree records or AFS+ management shortcuts cross this API.

The host receives `BackupAuthority` during service construction and issues opaque
`BackupGrant` values only after authenticating a privileged backup caller.
Grants are bound to that service instance, including its mount lifetime; knowing
a volume UUID, snapshot ID or reader ID confers no authority. A support feature
bit must never be treated as a grant. The host backend hook is privileged and
must not be exposed through the consumer facade or an IPC request.

Every create, delete, list, open, view-info, stat, inventory-inspection,
allocation-page, directory-page and data-read operation checks authority before invoking the backend. Readers carry the grant
under which they opened; a fresh grant does not reactivate an old reader. Cloned
grants share revocation, and cloned readers share their original grant and lease.
Wrong-service and revoked authority return `BackupError::Denied` without data
or metadata disclosure or backend calls. Provider errors retain their distinct
`VfsError` category.

Operation admission holds a shared grant lock for the entire backend call.
Revocation acquires the exclusive lock, marks the grant revoked, and returns
only after previously admitted operations have finished. Calls admitted before
that boundary can return data; subsequent calls fail. This cannot retract data
already delivered. A backend must not synchronously revoke its own in-flight
grant. Host cancellation, lock contention and OS authentication require adapter
qualification; no scheduler fairness or revocation latency bound is implied.

Reader admission requires an explicit positive maximum of simultaneous logical
reader opens. Copies share one provider reader allocation. The final close/drop
releases the provider view before returning its budget; cleanup does not require
authorization. Deletion stays busy while any provider reader lease remains,
including revoked readers. The host must clean up handles on client disconnect.
Cursors alone confer no authority and do not pin views.

### Captured metadata inventory knowledge

`metadata_inventory(reader, object)` reports independent attribute and security
inventory knowledge under [ADR-082](../adr/ADR-082-backup-object-metadata.md).
`Empty` requires complete provider inspection; `Present` requires separate
lossless enumeration and transport; `Uninspected` explicitly withholds a
completeness assertion. Absence of enumeration support must never return empty.
The default provider validates the captured object through `stat` and returns
both inventories uninspected. Missing objects and provider errors propagate.

The operation uses the original reader grant, with admission held throughout
provider execution. Live metadata changes must not alter captured inventory
knowledge. This fixed-size result allocates no inventory list and exposes no
storage addresses. Attribute values and security descriptors require separately
bounded enumeration/read operations before a consumer can preserve them.

This additive Rust trait method has a conservative default for existing
providers. It changes no C structure, native capability advertisement, disk
record or classic DOS interface. A foreign or legacy adapter without inspection
support reports uninspected; it must not claim full preservation on that basis.
Actual provider completeness and constrained/native integration require their
own qualification. Consumer-facing methods do not expose backend or issuer.

### Opaque captured metadata transport

`metadata_page(reader, object, class, after, limit)` enumerates attribute or
security entries separately. Each entry preserves an exact UTF-8 key, an opaque
encoding identifier and unsigned 64-bit value size. Keys are not host paths.
Unknown encoding identifiers are transported without interpretation. Full
restoration requires an equivalent destination or explicit refusal.

Keys use strict UTF-8 byte ordering. The optional cursor is the last returned
key, exclusive; it is meaningful only with the same reader, object and class.
Pages contain at most the requested 1 through 64 entries. An empty nonterminal
page, duplicate/out-of-order key, or key not after the cursor is corruption.
Keys contain 1 through 1024 bytes; encoding identifiers contain 1 through 128
bytes. Neither permits NUL. The service checks request bounds before provider
calls and validates every returned descriptor. Providers must enforce bounds
before allocating results. Smaller pages reduce memory without changing values.

`metadata_read(reader, object, class, key, offset, buffer)` streams arbitrary
binary bytes into caller storage. Reads may be short; zero indicates value EOF.
Consumers compare accumulated bytes with the captured descriptor size and reject
premature or excessive data. Requests must fit the unsigned 64-bit byte-address
space, including a final byte at offset `2^64-1`. Providers must not return a count
larger than the buffer. The service allocates no value-sized buffer.

Both operations hold the reader's revocable grant throughout provider execution.
Missing provider implementations return `NotSupported`, never successful empty
results. These additive Rust methods define no native ABI or filesystem record.
Inventory/descriptor/value consistency and lossless destination storage require
provider and consumer qualification; read admission is not ACL evaluation or
permission to install a descriptor on a destination.

### Captured allocation enumeration

`allocations(reader, object, start, limit)` enumerates the captured file's
allocation records by ordinal. A positive limit of at most 64 is checked before
provider invocation. Each record exposes byte offset, length and an unwritten
flag; the page returns the next ordinal and explicit end-of-enumeration.
Resumption uses the same retained snapshot and object. Providers without this
operation return `NotSupported`; an empty map cannot stand in for missing support.
Grant admission covers the complete provider call under the reader's original
authority, including after live mutation and remount through a new service.

Gaps are holes. Ranges include allocation-rounded tails and unwritten
reservations beyond EOF; logical size comes from `stat`. Consumers must clip
content reads to logical size and treat unwritten regions as zeros. The final
rounded allocation can end at 2^64: use wider arithmetic for offset plus length,
or clip length to `size - offset` after checking offset against logical size. Range
segmentation can vary with physical fragmentation; preservation compares
semantic coverage, never record counts or physical addresses. No sharing hint,
physical block number or allocator state crosses the interface.

The AFS+ mapping uses `snapshot_allocation_page`, captured object roots and a
bounded ordinal extent-tree traversal. It validates the preceding extent at a
page boundary to reject overlapping mappings across pages. Memory and traversal
are bounded by page size and tree height; a large logical hole is skipped.
The ordinary full checker retains ownership and whole-tree validation duties.

[ADR-078](../adr/ADR-078-backup-preservation-modes.md) distinguishes full
preservation of reservations from explicit content recovery with a loss report.
The exact archive profile and destination reservation operations must define
alignment, rounding and completion before claiming full restoration.

### Versioning and compatibility

This additive Rust module is an experimental source interface within version
0.0.1. Its generic types, locks and handles have no C layout or serialized grant
representation. It does not change existing API-v2 structs, result numbers,
capability identities, legacy DOS behavior or adapter advertisements. A C/IPC
bridge needs versioned extension negotiation, opaque server-side handle mapping,
explicit denial translation and host authentication tests before advertisement.
The existing C header is not a wire encoding of these Rust types.

The provider mapping uses existing `Stat`, `DirectoryEntry` and 64-bit offsets.
The same consumer must run against an independent provider and AFS+; grants and
backend types cannot be used to teach the consumer a disk layout. Exact archive
and restoration additionally require attributes, security-container transport, destination
reservation operations and a qualified preservation profile. This reader
interface alone does not establish a complete backup format or restore contract.


## 9. Destination-scoped restore extension

[ADR-077](../adr/ADR-077-separate-restore-authority.md) defines separate backup
and restore authority. The experimental Rust mapping is
[`afsplus_vfs::restore`](../crates/afsplus-vfs/src/restore.rs): a trusted host
owns `RestoreService`, selects its provider and destination, and retains
`RestoreAuthority`. The consumer receives `RestoreClient` and an opaque
`RestoreGrant`. Backup and restore grants are distinct public types; a job
requiring both roles receives both explicitly.

Every root, create, write, resize, hard-link, metadata, stat, read and sync
operation checks destination scope and revocation before calling the backend.
Object handles retain their original grant; a fresh grant cannot reactivate
revoked handles. Two-object linking checks both grants and holds admission
through the backend call. Identical grants use a single admission lock, avoiding
recursive read-lock acquisition while a revoker waits. Revocation drains
admitted calls; completed writes are durable partial work and are not undone.

The host supplies a positive logical-handle limit. Reserve a handle before a
provider root/open or create call; exhaustion must not create an unreachable
file. Clones share one logical handle. Final close drops the provider object
before returning its budget, including after revocation. Host disconnect
cleanup, cancellation and authentication need separate adapter qualification.
No lock fairness or revocation latency bound is implied.

Names are exact UTF-8 components. Reject empty names, dot, dot-dot, slash and
NUL before provider invocation. A provider must preserve accepted names as
literal components or refuse them; host path syntax cannot expand authority.
Opaque handles expose no constructor for importing arbitrary destination IDs.
Privileged backend hooks belong to the trusted host and must not enter consumer
IPC. A native bridge must also prevent external namespace races and symlink
escape within its selected destination.

`AfsRestoreDestination` owns a writable `Volume` and a host-selected empty
directory. Construction checks emptiness with a bounded directory page. It
creates fresh files/directories and hard links without overwriting existing
names. [Scoped created-entry lookup](#scoped-created-entry-lookup) reopens entries
inside that exclusively owned restore tree. It exposes no arbitrary object import
or authority over an unrelated namespace outside the selected directory.
Existing-destination merge, overwrite and persistent resume require an explicit
Q11 design and failure oracle before admitting objects present before the job.

`RestoreMetadata` carries protection and the three timestamps independently of
destination identities and storage layout. AFS+ refuses protection values that
cannot fit its 32-bit field and invalid nanoseconds before metadata writes.
Restore directory metadata after restoring children because namespace mutations
change timestamps. Logical zero gaps and hard-link identity have separate
preservation checks. Attributes, security containers, data policy and a complete
sparse-range transport require additional operations with explicit refusal for
unsupported required metadata.

`reserve(object, offset, length, time)` preserves exact reservation coverage
without extending logical size or altering written contents. Providers return
`NotSupported` for unavailable semantics or unrepresentable alignment. Requests
have positive length and a mathematical end no greater than 2^64. The AFS+
provider accepts block-aligned ranges, including the final rounded block; it
refuses silent outward rounding. Reserve before final metadata restoration
because reservation changes ctime. Existing written coverage stays written.

Reservation admission is disabled until the trusted host configures a positive
per-operation byte limit using `set_reservation_limit`. The checked consumer
cannot raise it. Zero length, range overflow, excessive requests and revoked
handles are rejected before provider invocation. Admitted reservations hold
revocation exclusion throughout the backend call. This additive source API
changes no disk layout, C ABI or native capability advertisement.

The AFS+ restore provider admits writes touching at most 64 filesystem blocks
and 64 local extent records, including boundary neighbors and result mappings.
It uses `write_file_at_bounded`; callers split larger transfers into separately
durable requests. Unaligned writes count every touched block. Exceeding the
provider budget reports a limit error without a partial write. These limits
bound data buffers and local extent vectors; total allocator and retention
memory require independent qualification.

The AFS+ restore provider uses `truncate_file_bounded` for resizing. Shrinking
admits at most 64 retired allocated blocks and 64 local records, counting a
replaced partial tail and reservations beyond EOF. Growth preserves existing
tree mappings without enumeration. Excessive shrinking returns a limit error
before writes; the service does not silently split an atomic resize.

The byte limit bounds requested allocation per operation. The AFS+ provider
uses the [bounded local extent editor](06-files-and-extents.md#4-preallocation)
with a 64-record limit, including boundary neighbors, allocated runs and the
resulting local mappings. A limit refusal leaves the volume unchanged; a
consumer may retry smaller separately durable requests. Total core memory and
metadata headroom require separate measurement. Private unwritten consumption follows
[ADR-079](../adr/ADR-079-initialize-private-unwritten-reservations.md); sustained
near-full workloads and metadata headroom need separate qualification. Full archive preservation needs both
reservation-consumption and destination alignment/capacity evidence under
[ADR-078](../adr/ADR-078-backup-preservation-modes.md).

The [archive envelope](../spec/backup-envelope.md) supplies stream-integrity
receipts under [ADR-081](../adr/ADR-081-ordinary-pax-completion-member.md).
Profile validation binds those receipts to source enumeration, required
metadata and the selected preservation/loss contract before a consumer reports
complete backup or restoration.

`sync` reports filesystem durability; it does not certify a complete archive
restore. The PAX consumer must separately verify its preservation profile,
integrity, completion and interrupted/partial-work outcome under
[ADR-076](../adr/ADR-076-pax-backup-interchange.md). Authorization alone cannot
satisfy those checks.

This additive experimental Rust interface follows the version 0.0.1 source-only
compatibility boundary in section 8. It changes no disk encoding, API-v2 C
layout, capability number, legacy DOS behavior or native advertisement.
A native C/IPC extension requires version negotiation, server-owned opaque
handles, denial translation and destination/authentication qualification.
The [restore harness](../testing/security-model-conformance.md#14-destination-restore-authority)
runs the same consumer against an independent provider and AFS+.

### Committed destination allocation readback

Under [ADR-089](../adr/ADR-089-restore-allocation-readback.md),
`allocations(object, start, limit)` returns committed semantic byte ranges with
written/unwritten state, an entry-ordinal next cursor and EOF. Its range shape
matches [captured allocation enumeration](#captured-allocation-enumeration),
including rounded tails and reservations beyond logical EOF, without physical
addresses or sharing hints. Logical size comes from stat.

The original object's restore grant is checked and held through every provider
call. Limits are 1 through 64 entries. Responses must fit the limit, advance by
the returned entry count and contain ordered positive-length ranges with a
mathematical end at most 2^64. A nonterminal empty page, overlapping ranges,
overflow or inconsistent cursor is corruption. Unsupported enumeration returns
`NotSupported`, never an empty successful result.

The AFS+ implementation shares bounded extent-page reads with the snapshot
reader. It refuses an open intent-log mutation window with `Busy`; the caller
must explicitly commit that window before requesting committed allocation.
The query performs no implicit commit, flush or repair. Pagination does not
create a snapshot or freeze a complete sequence: the host must serialize edits
and enumeration of its owned destination objects and restart after any edit.

Full allocation preservation requires comparing exact byte coverage and
written/unwritten state, independently of adjacent extent segmentation, then
checking logical size. Matching total allocated bytes is insufficient. A
mismatch or unavailable readback cannot justify preservation success. This is
an additive Rust source API, with native/C ABI and provider qualification kept
separate. See the [readback gate](../testing/security-model-conformance.md#20-destination-allocation-readback).

### Staged opaque metadata restoration

[ADR-083](../adr/ADR-083-staged-opaque-metadata-restore.md) defines the optional
`OpaqueRestoreBackend` extension. `begin_opaque(object, class, entry)` binds a
provider-private upload to the exact destination object, key, encoding and size.
The host enables a per-value byte limit through `set_metadata_limit`; `None`
disables uploads and `Some(0)` admits only empty values. Descriptor text limits
match captured metadata transport. Each upload consumes a separate handle-budget
unit and retains the original object's lease and restore grant.

`write_opaque(upload, bytes)` accepts sequential chunks, refuses excess bytes
before provider invocation and permanently fails the upload after an uncertain
provider write. `finish_opaque(upload)` consumes the upload, requires exactly
the declared length, rechecks the original grant, and invokes atomic provider
publication. No caller-supplied offset or replacement grant can change the target.
A successful finish preserves the exact opaque bytes and encoding; it does not
certify whole-restore completion or filesystem durability beyond the provider's
qualified contract. Final synchronization is separate.

Dropping or explicitly aborting an upload releases its private staging before
returning its budget and destination lease. Cleanup requires no grant and cannot
change active metadata. Providers implement resource cleanup in their upload
type. Failed publication permits only old or complete new active metadata and
requires reconciliation of uncertain state before another mutation. Unsupported
formats, preservation semantics or staging implementations cause explicit refusal.
Security installation does not bypass the host's enforcement policy.

The extension adds Rust types and methods without changing existing provider
implementations, C ABI, classic DOS interface or on-disk representation. The
AFS+ mapping explicitly refuses opaque upload until its storage implementation
qualifies this contract. Constrained providers may spool staging and accept small
chunks; whole-value allocation is not required by the interface. Actual native
staging, cleanup, durability and memory limits need provider evidence.


### Scoped created-entry lookup

[ADR-092](../adr/ADR-092-scoped-restore-namespace-lookup.md) adds optional
`lookup_created(parent, name)` to the restore provider and checked facade. The
provider resolves exactly one literal component within the given directory,
without following symlink targets. Missing support returns `NotSupported`.
Validate the original parent grant, component syntax and a free active-handle
slot before provider calls. Hold the admission permit while checking that the
parent is a directory, resolving the entry and constructing its handle. Invalid
or symlink parents return `NotDirectory`; a missing entry returns `NotFound`.
Failures release the reserved slot.

The returned handle owns an independent provider object and carries the parent's
original grant. Closing the parent does not close the child. Revoking that grant
blocks both handles and their clones on subsequent operations. A valid new grant
can obtain a new root and reopen entries inside the same authorized scope, but
cannot change or reactivate an old handle's grant. Foreign-service parents fail
before provider calls. Lookup performs no filesystem writes or implicit barrier.

The host maintains an initially empty destination and exclusive namespace
ownership for the restore service lifetime. Consumers cannot insert external
objects, rename/delete entries out of scope, traverse symlinks or pass raw IDs.
AFS+ maps this operation to a directory-entry lookup in the exclusively owned
Volume. Native providers must qualify their own isolation and no-follow behavior.
This operation does not establish merge, overwrite or persistent-resume semantics.

Walking a path can use two active handles by dropping each previous parent after
opening its child. Holding a primary file while walking to an alias parent needs
another slot. Reopening trades repeated lookup work for bounded active resources;
it does not bound archive indexes, path storage or metadata manifests. Those need
separate caller budgets or spooling. Enclosing restore orchestration must verify
hard-link identity and finalize metadata after all namespace mutations.

Run the [created-entry lookup gate](../testing/security-model-conformance.md#21-scoped-created-entry-lookup).
The additive Rust trait default preserves provider compilation; no C ABI, feature
identity or filesystem disk record is changed.

### Scoped directory emptiness

[ADR-093](../adr/ADR-093-directory-and-hardlink-archive-groups.md) adds the optional
`directory_empty` restore query. Hold the original destination grant through
kind validation and provider access. Require a directory; do not follow a
symlink or create another active restore handle. Return `NotSupported` when the
provider lacks the query, never assume an unsupported directory is empty.
The AFS+ provider reads a directory page limited to one entry. The operation
performs no mutation or durability barrier. Revocation and foreign handles
refuse access under the same rules as other restore queries.

[Directory archive groups](../spec/backup-namespace.md) use this query before
restoring metadata. It is an observation within the isolated restore namespace,
not a lock against unrelated actors; destination isolation is a host obligation.

### Scoped symlink target transport

[ADR-094](../adr/ADR-094-scoped-symlink-transport.md) requires explicit `read_link`
operations for captured backup and scoped restore objects. Validate symlink kind
under the same original grant held through provider access. Return the required
byte count and leave a short output buffer unchanged. Bytes are opaque UTF-8;
no API resolves the target or follows a directory component from it.

Restore `create_symlink` validates the grant, parent directory, component name,
timestamp and nonempty NUL-free target, and reserves a handle before creation.
The provider enforces its target-size limit and performs atomic creation without
overwrite. The returned object retains the same destination grant. Revocation
blocks further reads and creation even through existing handles. Optional
providers return NotSupported; unsupported functionality never becomes an empty
target, an ordinary file or silently truncated content.

Rust VFS symlink creation commits a pending logged-data window through the common
namespace path. Unlink dispatch retires symlink metadata without regular-file
orphan handling. Its SYMLINKS capability covers creation/read/unlink; it does not
extend atomic replacement or advertise unimplemented OS/C adapter operations.
Published C capability identities are independent and retain their numbers.
Archive preservation requires a separate target/profile binding and final
readback; these primitives alone do not establish whole-job completion.
