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

C1 comes first because ABI version 1 has no capability query and no way to
add an entry point that an older handler shell can refuse cleanly.

## C1. Rust/C integration boundary

Present: ABI version 1, 29 exported functions, size-checked structures,
allocation-free calls, numeric 64-bit lock and file identifiers.

Lacking:

1. a version and capability query callable before mount, so a packet layer
   built against a newer header refuses an older static library by value
   instead of by link failure;
2. a per-mount capability and limits query (the Rust `Capabilities` mask,
   `StatFs` name limit, case policy and Unicode version never cross the
   boundary);
3. a stable mapping from `VfsError` to a structured result for callers that
   are not DOS packets: every error is folded into an `ERROR_*` value, and
   `Limit`, `Corrupt` and `Io` lose their category;
4. a rule for additive growth: which functions a version-1 caller may rely
   on, and how `struct_size` admits a longer structure.

Hosted and QEMU: all four, L1 to L3 on the development host, L4 by relinking
the existing S0 gates. Apple hardware: none.

## C2. DOS compatibility

Present: the 40 actions listed in the
[packet mapping](../docs/aros-native-bridge.md#native-lifecycle), which carry
the S0 matrix and the S1 desktop session.

Lacking, in the order classic software meets them:

| Action | Missing piece | Layer where it starts |
|---|---|---|
| `ACTION_SET_PROTECT` | VFS and adapter setter over `Volume::set_object_protection`; DOS inverted RWED bits pass through as stored | L1 |
| `ACTION_SET_DATE` | VFS setter for the modification time; `DateStamp` to Unix conversion in the packet layer | L1 |
| `ACTION_SET_COMMENT`, comment in `FileInfoBlock` | a stored comment attribute; the format has no comment or extended-attribute record ([docs/12](../docs/12-metadata-and-xattrs.md)) | Stage B decision |
| `ACTION_MAKE_LINK` soft, `ACTION_READ_LINK` | adapter calls over `Vfs::create_symlink` and `read_link`; `ERROR_IS_SOFT_LINK` on traversal; soft-link delete | L1 |
| `ACTION_FH_FROM_LOCK`, `ACTION_CHANGE_MODE` | handle from lock, shared/exclusive conversion with conflict check | L1 |
| `ACTION_EXAMINE_ALL`, `ACTION_EXAMINE_ALL_END` | paged fill of `ExAllData` from `read_directory` with more than one entry per call | L3 |
| `ACTION_RENAME_DISK` | label rewrite in the identity block; needs a core label setter | L1, core |
| `ACTION_WRITE_PROTECT` | runtime switch to a read-only view with the pass key | L1 |
| `ACTION_LOCK_RECORD`, `ACTION_FREE_RECORD` | byte-range record table per object, with timeout handled by the handler loop | L1, L4 |
| `ACTION_ADD_NOTIFY`, `ACTION_REMOVE_NOTIFY` | C8 | C8 |
| `ACTION_FORMAT`, `ACTION_SERIALIZE_DISK` | in-handler mkfs through the mounted device; refused while locks are open | L1, L4 |
| exclusive-lock and open-mode interaction | `FINDINPUT` on an exclusively locked object, delete of an open file mapped to `ERROR_OBJECT_IN_USE` or to the orphan path by policy | L1 |
| directory enumeration across a commit | `STALE` cookie maps to `ERROR_INVALID_LOCK`; DOS expects `ExNext` to continue | L1 |

Hosted and QEMU: every row except the comment. Apple hardware: none.

## C3. Classic single-user security preservation adapter

Present: protection bits are stored and returned unmodified; no code path
inspects a security container.

Lacking: the adapter of
[docs/30 section 9](../docs/30-portable-security-model.md#9-classic-amiga-compatibility-profile):
the local session maps to the owner, the classic bits are a projection, and a
protection write that would drop richer metadata is refused unless the caller
requests the downgrade. The adapter needs (1) a VFS query "this object
carries security metadata the classic view cannot express", (2) the refusal
path in `ACTION_SET_PROTECT`, (3) preservation across rename, link, clone and
replace, (4) a mount option for the explicit downgrade.

The container is the Stage B item B5. Against the current executable
contract no object can carry such metadata, so the buildable part is the
policy seam and its refusal test driven by a test double; the on-disk query
waits on B5.

## C4. Filesystem API v2 and the modern 64-bit API

Present: the Rust VFS subset of [docs/13](../docs/13-filesystem-api-v2.md);
[`api/filesystem_v2.h`](../api/filesystem_v2.h) declares types and no
functions; the C boundary exposes DOS-shaped calls only.

Lacking:

1. positioned 64-bit read and write at the C boundary (the DOS calls carry an
   implicit position and a 32-bit count);
2. object-ID operations: stat by ID, lookup returning the ID, paged directory
   read with opaque cookies and more than one entry;
3. `statfs64` with case policy and Unicode version;
4. atomic replace (the adapter always passes `replace = false`);
5. capability and limits query (C1);
6. a transport that reaches a running handler from an application: a
   versioned extension packet with a refusal that older handlers give for
   free (`ERROR_ACTION_NOT_KNOWN`), and a client library that falls back;
7. the capability numbering: the Rust mask and `FSV2_Capability` assign
   different bits to the same capability, and one published numbering must
   win before any application reads it.

Hosted and QEMU: all. Apple hardware: none.

## C5. Clone and reflink capability API

Present: `Vfs::clone_file` and `clone_range` behind `CLONE_FILE` and
`CLONE_RANGE`.

Lacking: adapter and C entry points taking DOS locks and handles; the
extension packet of C4; a `Copy CLONE`-style consumer that falls back to a
byte copy on `NOT_SUPPORTED`; exclusive-lock and open-handle rules for the
destination. Metadata inheritance follows the executable behavior until
Q14 is answered.

## C6. Access-intent and preallocation mapping

Present: `Volume::preallocate_file` and its bounded form;
[`api/performance_hints.h`](../api/performance_hints.h) is a draft with
`void *` handles.

Lacking: a VFS `preallocate` behind a capability bit; adapter and C entry
points; a defined mapping of each access hint to an effect or to an explicit
"accepted, no effect" result, so callers never see a hint silently change
durability; the bounded edit limits of the restore provider applied to the
handler so one packet cannot hold the single handler task unbounded.

## C7. mmap-friendly large-file path

Present: caller-buffer I/O, one logical block per device callback.

Lacking: a block-aligned multi-block read and write path that transfers
whole extents without a bounce copy; an extent query by semantic range
(offset, length, written or unwritten) that a pager can use to plan faults,
reusing the shape of the restore allocation readback; a stable page-cache
coherence rule between mapped pages and `ACTION_WRITE`. AROS has no
file-backed memory mapping, so the target-side consumer is a pager probe; the
contract is proven at L1 and L2 and its throughput only on hardware.

Apple hardware: the zero-copy claim and its timing.

## C8. Notifications

Present: none; the change stream (M10) is not started and is not a
dependency of DOS notification.

Lacking: a bounded in-memory watch table in the adapter keyed by object and
by parent plus name (DOS notifies on names that do not exist yet); event
generation at each mutating adapter call; coalescing so one packet produces
at most one event per watch; a drain call at the C boundary; the packet layer
turning events into `NotifyMessage` or `Signal`, including
`NRF_NOTIFY_INITIAL` and the rule that an unreplied message suppresses the
next one; overflow reported as a rescan event. The v2 `watch` operation uses
the same table.

Hosted and QEMU: all. Apple hardware: none.

## C9. Health reporting

Present: the core flight recorder and checker findings; nothing reaches the
handler boundary.

Lacking: a fixed-size health snapshot (mount mode, generation, pending
intent records, pending orphans, free and available blocks, last device
error, degraded flags from
[docs/26 section 17](../docs/26-debug-observability.md#17-structured-healthevent-stream));
a bounded health event ring with loss counter; C entry points; a target
query path (C4 transport).

## C10. Trace streaming and developer attachment

Present: `FlightRecorder` with `LiveSink`, categories and loss counters on
the host; the trackdisk activity sink.

Lacking: a C trace sink matching
[`api/debug_observability.h`](../api/debug_observability.h) installed through
the boundary; category selection at run time; a target front-end (message
port for a following tool, serial for QEMU and m68k); the rule that a slow
consumer costs only counted loss. Forwarding from the M1 target to a
development host needs the hardware.

## C11. Structured management APIs

Present: `afsplus-check --json` schema 5 on the host.

Lacking: a versioned info structure served by the running handler (volume
identity, features, capabilities, health), a target `afsplus-info` that prints
it as JSON with its schema version, and machine-readable error codes. Tools
are thin clients of C9 and C4.

## C12. File-backed virtual block device

Present: the generic AROS `fdsk.device` carries S0 and S1 on Hosted; the
external `afsram.device` carries native QEMU.

Lacking in the generic device, as upstream patches with regression probes:

1. `CMD_UPDATE` and `ETD_UPDATE` are accepted and ignored, so a filesystem
   barrier never reaches the backing file;
2. upstream has no `TD_READ64`, `TD_WRITE64` or `NSCMD_TD_*64`; MacAROS
   carries that fix, and AFS+ images beyond 4 GiB depend on it;
3. attach and detach of a unit to a named file at run time
   (`AttachDisk`-style), instead of the fixed `FDSK:Unit<N>` convention.

Hosted: all three. Apple hardware: the persistent native device that
replaces the retained-RAM transport.

## C13. Native AROS benchmark runner

Present: the host measurement harness and
[benchmark-contract](../testing/benchmark-contract.md).

Lacking: handler counters (DOS packets by action, failures by code, device
reads, writes, bytes, barriers, peak and steady memory) behind C9; a target
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
