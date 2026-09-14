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
  - [observation](#observation)
- [4. Compatibility adapters](#4-compatibility-adapters)
- [5. Large files](#5-large-files)
- [6. Rust](#6-rust)
- [7. Zed](#7-zed)
- [8. Trusted snapshot backup extension](#8-trusted-snapshot-backup-extension)
  - [Captured allocation enumeration](#captured-allocation-enumeration)
  - [Versioning and compatibility](#versioning-and-compatibility)
- [9. Destination-scoped restore extension](#9-destination-scoped-restore-extension)

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
adapter inherit it through the shared VFS. The version-1 AROS C bridge has no
capability-query structure to extend and keeps its ABI unchanged: its existing
write/truncate/fsync entry points receive the same semantics transparently.
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

### observation

- watch
- query capabilities
- query limits

## 4. Compatibility adapters

A legacy handler can expose v2 through an adapter.

Unsupported capabilities return a precise `NOT_SUPPORTED` result.

Applications must not infer capability from filesystem name.

## 5. Large files

Classic APIs that accept signed 32-bit offsets retain their historical limits.

The v2 API uses 64-bit offsets and lengths end to end.

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

Every create, delete, list, open, view-info, stat, directory-page and data-read
operation checks authority before invoking the backend. Readers carry the grant
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
names; the consumer has no lookup or import operation for pre-existing objects.
An unrelated namespace outside the selected directory receives no authority.
Existing-destination merge, overwrite and resume require an explicit Q11 design
and failure oracle before adding operations that expose existing objects.

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

The byte limit bounds requested allocation per operation. It does not bound
all core memory: preallocation loads the existing extent layout. Constrained
qualification must replace or bound that traversal and measure metadata
headroom on fragmented files. Private unwritten consumption follows
[ADR-079](../adr/ADR-079-initialize-private-unwritten-reservations.md); sustained
near-full workloads and metadata headroom need separate qualification. Full archive preservation needs both
reservation-consumption and destination alignment/capacity evidence under
[ADR-078](../adr/ADR-078-backup-preservation-modes.md).

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
