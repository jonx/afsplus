# Stage C gap list

What the external `L:` handler with its DOSDriver
([ADR-050](../adr/ADR-050-external-aros-handler-lifecycle.md)) lacks for each
[Stage C](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability)
item, in dependency order. The state of M06 and M07 is in
[milestones](milestones.md); this page is the work inventory those cells
summarise. Portable C work belongs to Stage D and is outside this list.

<!-- toc -->

- [Layers and proof targets](#layers-and-proof-targets)
- [Dependency order](#dependency-order)
- [C1. Rust/C integration boundary](#c1-rustc-integration-boundary)
- [C2. DOS compatibility](#c2-dos-compatibility)
- [C3. Classic single-user security preservation adapter](#c3-classic-single-user-security-preservation-adapter)
- [C4. Filesystem API v2 and the modern 64-bit API](#c4-filesystem-api-v2-and-the-modern-64-bit-api)
- [C5. Clone and reflink capability API](#c5-clone-and-reflink-capability-api)
- [C6. Access-intent and preallocation mapping](#c6-access-intent-and-preallocation-mapping)
- [C7. mmap-friendly large-file path](#c7-mmap-friendly-large-file-path)
- [C8. Notifications](#c8-notifications)
- [C9. Health reporting](#c9-health-reporting)
- [C10. Trace streaming and developer attachment](#c10-trace-streaming-and-developer-attachment)
- [C11. Structured management APIs](#c11-structured-management-apis)
- [C12. File-backed virtual block device](#c12-file-backed-virtual-block-device)
- [C13. Native AROS benchmark runner](#c13-native-aros-benchmark-runner)
- [C14. AROS handler qualification ladder](#c14-aros-handler-qualification-ladder)
- [Stage B dependencies](#stage-b-dependencies)

<!-- /toc -->

## Layers and proof targets

Every item crosses the same four layers, and each layer has its own proof:

| Layer | Source | Proof | Needs |
|---|---|---|---|
| L1 adapter semantics | [`crates/afsplus-aros`](../crates/afsplus-aros/src/lib.rs) over [`afsplus-vfs`](../crates/afsplus-vfs/src/lib.rs) | `cargo test -p afsplus-aros` on the development host, checker-clean image after each scenario | stable Rust |
| L2 C boundary | [`api/afsplus_aros.h`](../api/afsplus_aros.h), [`crates/afsplus-aros-ffi`](../crates/afsplus-aros-ffi/src/lib.rs) | `cargo test -p afsplus-aros-ffi` through the exported functions, C11 header check with layout assertions | stable Rust, host Clang |
| L3 packet translation | [`native/aros/afsplus_packet.c`](../native/aros/afsplus_packet.c) | host `DosPacket` matrix ([`packet_stub.c`](../native/aros/tests/packet_stub.c)) compiled with AROS headers | AROS SDK headers, or the AROS source headers through [`tools/dev-packet-matrix.sh`](../tools/dev-packet-matrix.sh) |
| L4 target runtime | [`native/aros/afsplus_handler.c`](../native/aros/afsplus_handler.c), target probes under [`native/aros/tests`](../native/aros/tests) | the gates of [aros-system-volume-qualification](../testing/aros-system-volume-qualification.md) | built Hosted MacAROS, QEMU with the `apple-aarch64` SDK, FS-UAE with the m68k SDK, `nightly-2026-06-27` |

The "Present" paragraph of an item names the layers that carry it. A piece
present at L1 to L3 and absent at L4 is built and proven on the development
host and waits for a target run; nothing in this list is a target claim
beyond the gates of the qualification document.

L1 to L3 are provable on a development host that carries only stable Rust,
Clang and the AROS source tree. L4 needs the MacAROS development machine
named in [aros-native-bridge](../docs/aros-native-bridge.md). Apple hardware is
needed only where a row says so: a reset-durable transport, physical device
timing and the performance budget.

## Dependency order

```text
C1 boundary v2 ──> C2 DOS compatibility ──> C3 classic security adapter
      │                  │
      │                  └──> C8 notifications
      ├──> C4 API v2 ──> C5 clone ──> C6 intent/preallocation ──> C7 mmap path
      ├──> C9 health ──> C10 trace streaming ──> C11 management
      └──> C12 block device ──> C13 benchmark runner ──> C14 ladder S2, S3
```

C1 comes first because every later entry point is an additive group that an
older library must be able to refuse by value.

## C1. Rust/C integration boundary

Present (L1 to L3): ABI version 1 with interface revision 14.
`afsplus_aros_interface` answers without a mount with the revision and a
mask of entry-point groups; the packet layer asks it at creation and answers
`ERROR_ACTION_NOT_KNOWN` for an action of a missing group.
`afsplus_aros_capabilities` reports the published `FSV2_CAP_*` mask, mount
mode, name limit, case policy, Unicode version, pending intent records and
block counts. Query structures share one size-negotiated growth rule. The
Rust mask and the C identities are separate numberings joined by one table.

Lacking:

1. L4: the Hosted, QEMU and m68k gates relinked against revision 7, with the
   exported-symbol list of [`check-aros-ffi.sh`](../tools/check-aros-ffi.sh);
2. a structured result for callers that are not DOS packets: every error is
   an `ERROR_*` value, and `Limit`, `Corrupt` and `Io` share codes with
   ordinary results.

## C2. DOS compatibility

Present: the 40 actions of the Alpha-0 and S1 gates (L1 to L4), and at L1 to
L3 `ACTION_SET_PROTECT`, `ACTION_SET_DATE`, soft `ACTION_MAKE_LINK`,
`ACTION_READ_LINK` with `ERROR_IS_SOFT_LINK` on traversal, DOS open-mode
locking (`MODE_NEWFILE` exclusive, held objects not deletable, refused
`DupLockFromFH` on an exclusive handle), `ExNext` that continues across
namespace changes, `ACTION_FH_FROM_LOCK`, `ACTION_CHANGE_MODE`, `ACTION_WRITE_PROTECT`, `ACTION_RENAME_DISK`, `ACTION_SET_COMMENT` with the comment in `FileInfoBlock` and `ED_COMMENT` records, immediate `ACTION_LOCK_RECORD` with
`ACTION_FREE_RECORD`, and at L3 `ACTION_EXAMINE_ALL`
with `ACTION_EXAMINE_ALL_END`.

Lacking, in the order classic software meets them:

| Action | Missing piece | Layer where it starts |
|---|---|---|
| all of the above | target run: a probe extension for the S0 matrix on Hosted, QEMU and m68k | L4 |
| record lock self-overlap | a handle does not collide with its own range; unverified against rom/dos `LockRecord` and a reference handler until a target run | L4 |
| `ACTION_LOCK_RECORD` waiting modes | honouring the `dp_Arg5` timeout: a queue of deferred packets in the handler loop, retried when a range is freed | L4 |
| `ACTION_FORMAT`, `ACTION_SERIALIZE_DISK` | in-handler mkfs through the mounted device; refused while locks are open | L1, L4 |
| `ExNext` resume cost | one resume reads O(log n) single-entry pages; a core seek-by-key page read makes it one descent | core |

Hosted and QEMU: every row. Apple hardware: none.

## C3. Classic single-user security preservation adapter

Present (L1, L2): the policy seam of
[docs/30 section 9](../docs/30-portable-security-model.md#9-classic-amiga-compatibility-profile).
The adapter asks one question through `RichSecurityProbe`; a protection write
on an object carrying metadata the classic projection cannot express is
`ERROR_WRITE_PROTECTED` with the stored state untouched, and the mount flag
`AFSPLUS_AROS_MOUNT_FLAG_SECURITY_DOWNGRADE` is the explicit downgrade. An
on-disk security descriptor takes the preserving path instead: the write
lands, the bytes stay and the divergence is marked, unless
`AFSPLUS_AROS_MOUNT_FLAG_STRICT_SECURITY_PROJECTION` selects the refusal.
Rename and hard link keep the object and its metadata.

Lacking: the on-disk answer to the probe, which is the Stage B item B5 (the
default probe answers no, which is exact for a format that stores protection
bits only); preservation across clone and atomic replace, which follows the
container's inheritance rule; a DOSDriver keyword that sets the mount flag.

## C4. Filesystem API v2 and the modern 64-bit API

Present (L1, L2): the `API_V2` entry-point group on the same locks and
handles as the DOS calls: positioned 64-bit read and write that leave the DOS
position alone, atomic replace over an unheld target, and the capability and
limits query of C1 with one published numbering (`FSV2_CAP_*`), and the
`OBJECT_IDS` group: lookup and stat by object ID and a paged directory walk
whose position survives namespace changes between pages.

Lacking:

1. a transport that reaches a running handler from an application: a
   versioned extension packet that older handlers refuse for free
   (`ERROR_ACTION_NOT_KNOWN`), its packet-layer cases, and a client library
   that falls back. The packet number is an AROS-wide allocation;
2. a consumer: the AROS Rust `std` port binding to the group.

Hosted and QEMU: all. Apple hardware: none.

## C5. Clone and reflink capability API

Present (L1, L2): `afsplus_aros_clone_file` from a lock and
`afsplus_aros_clone_range` between two handles, advertised separately and
answering `ERROR_ACTION_NOT_KNOWN` on a volume without shared extents.

Lacking: the extension packet of C4; a `Copy CLONE`-style consumer that falls
back to a byte copy. Metadata inheritance follows the executable behavior
until Q14 is answered.

## C6. Access-intent and preallocation mapping

Present (L1, L2): `Vfs::preallocate` behind the `PREALLOCATE` capability with
a caller-supplied block budget and the 64-record edit limit;
`afsplus_aros_preallocate` with the budget as a mount setting, an oversized
request reserving nothing; `afsplus_aros_advise` admitting every hint of
[`performance_hints.h`](../api/performance_hints.h) and returning the effect
it had, which is none.

Lacking: hints with an effect (sequential read-ahead, temporary-file
placement) once a cache exists to steer; `AFSPLUS_PREALLOC_CONTIGUOUS_PREFERRED`
and placement hints, which need allocator support; the extension packet of C4.

## C7. mmap-friendly large-file path

Present (L1, L2): caller-buffer positioned I/O, and `ExtentMap`, the planning
query of a pager: the committed mapping of a byte range as written, reserved
or hole, clipped to the range, without physical addresses, bounded per call,
one tree descent wherever the offset lies (`EXTENT_MAP` entry-point group).

Lacking: a block-aligned multi-block read and write path that transfers
whole extents without a bounce copy (the device boundary moves one logical
block per callback); a stated page-cache coherence rule between mapped pages
and `ACTION_WRITE`. AROS has no file-backed memory mapping, so the
target-side consumer is a pager probe; the contract is proven at L1 and L2
and its throughput only on hardware.

Apple hardware: the zero-copy claim and its timing.

## C8. Notifications

Present (L1, L2): a bounded watch table keyed by parent directory and
comparison key, covering names that do not exist yet and directory watches;
one pending flag per watch, so events coalesce and memory does not follow the
change rate; writes reported at close; `watch_add`, `watch_remove` and
`watch_drain` at the C boundary.

At L3 `ACTION_ADD_NOTIFY` and `ACTION_REMOVE_NOTIFY` pair each
`NotifyRequest` with a watch and deliver fired watches through a callback of
the packet configuration after every packet.

The handler shell delivers through its own reply port (L4 source, type-checked
against the SDK include tree, never run).

Lacking: the target run of the delivery path, which must show a change made
during an unreplied `NRF_WAIT_REPLY` message arriving after the reply, and a
dismount succeeding with a message never replied; the v2 `watch` operation
over the same table.

Hosted and QEMU: all. Apple hardware: none.

## C9. Health reporting

Present (L1, L2): every failed call on a mounted instance that describes the
volume or its device enters a health log with counters, degraded-state flags
and a bounded event ring whose sequence numbers expose loss;
`afsplus_aros_health` adds generation, pending intent records, pending
orphans and block counts and flags a read-only view of an unreplayed log.

Lacking: the remaining events of
[docs/26 section 17](../docs/26-debug-observability.md#17-structured-healthevent-stream)
that the core does not raise as errors (checkpoint fallback, reclaim backlog,
free-count mismatch); a target query path (C4 transport).

## C10. Trace streaming and developer attachment

Present (L1, L2): `afsplus_aros_set_trace_sink` attaches the core flight
recorder to a callback of
[`debug_observability.h`](../api/debug_observability.h) with a run-time
category mask; `afsplus_aros_trace_counters` reports delivered, missed,
filtered and dropped.

Lacking: a timestamp source (the core has no clock, so the field is zero); a
target front-end that owns the preallocated queue (message port for a
following tool, serial for QEMU and m68k); stable event codes (the draft
header publishes the core's event order). Forwarding from the M1 target to a
development host needs the hardware.

## C11. Structured management APIs

Present (L1, L2): `afsplus_aros_info_json` serves one versioned JSON
document (`afsplus-handler-info`, version 1) from the mounted instance:
identity, feature masks, mount state, capability names, health, handle usage.

Lacking: the target `afsplus-info` client and its transport (C4);
machine-readable error documents for failed management calls; dry-run
planning for destructive operations, which starts with `ACTION_FORMAT`.

## C12. File-backed virtual block device

Present: the generic AROS `fdsk.device` carries S0 and S1 on Hosted; the
external `afsram.device` carries native QEMU.

[`native/aros/upstream`](../native/aros/upstream/README.md) holds a patch and
a regression probe for item 1; neither has been compiled.

Lacking in the generic device, as upstream patches with regression probes:

1. `CMD_UPDATE` and `ETD_UPDATE` are answered inside `BeginIO`, so the reply
   overtakes queued writes and the barrier never reaches the backing file;
   the hosted `emul-handler` in turn implements no `ACTION_FLUSH`;
2. upstream has no `TD_READ64`, `TD_WRITE64` or `NSCMD_TD_*64`; MacAROS
   carries that fix, and AFS+ images beyond 4 GiB depend on it;
3. attach and detach of a unit to a named file at run time
   (`AttachDisk`-style), instead of the fixed `FDSK:Unit<N>` convention.

Hosted: all three. Apple hardware: the persistent native device that
replaces the retained-RAM transport.

## C13. Native AROS benchmark runner

Present: the host measurement harness and
[benchmark-contract](../testing/benchmark-contract.md).

`afsplus_aros_counters` (L1, L2) reports completed and failed calls and
device reads, writes, barriers, bytes and failures since mount.

Lacking: DOS packets by action and failures by code, which belong to the
packet layer; peak and steady handler memory; a target
runner executing a fixed operation trace against AFS+ and the AFS/FFS
baseline with manifest verification before and structural check after; a
result bundle in the contract format. Hosted gives software cost; the
emulators give determinism; the budget of
[section 2](../testing/aros-system-volume-qualification.md#2-what-macaros-performance-proves)
needs the M1 and the A500.

## C14. AROS handler qualification ladder

Open after the items above: S2 boot-selected volume (handler resident before
boot-volume selection, bootable priority, recorded bootstrap dependencies),
S3 repeated boot and recovery, on every platform; the Apple hardware run with
a reset-durable transport; the physical A500.

## Stage B dependencies

| Need | Item | Question or stage item |
|---|---|---|
| stored file comment or extended attribute record | C2 | metadata and xattr record, [docs/12](../docs/12-metadata-and-xattrs.md) |
| security container presence query | C3 | B5 |
| clone metadata inheritance | C5 | Q14 |
| header-flag and extension admission, which decides how an unknown security or attribute extension is preserved by a classic writer | C2, C3 | Q13 |
| ordinary-user access to historical views, which bounds what a v2 snapshot capability may advertise | C4 | Q5 |
