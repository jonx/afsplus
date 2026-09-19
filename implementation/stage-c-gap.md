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
- [What Stage C still waits on elsewhere](#what-stage-c-still-waits-on-elsewhere)

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
named in [aros-native-bridge](../docs/aros-native-bridge.md), whose built AROS
tree needs six things a build of it does not make; they are listed there under
"Reproduce the Hosted target run" and applied by
[`prepare-hosted-aros.sh`](../tools/prepare-hosted-aros.sh), which is what to
run after every rebuild of that tree. Apple hardware is
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

Present (L1 to L3): ABI version 1 with interface revision 15.
`afsplus_aros_interface` answers without a mount with the revision and a
mask of entry-point groups; the packet layer asks it at creation and answers
`ERROR_ACTION_NOT_KNOWN` for an action of a missing group.
`afsplus_aros_capabilities` reports the published `FSV2_CAP_*` mask, mount
mode, name limit, case policy, Unicode version, pending intent records and
block counts. Query structures share one size-negotiated growth rule. The
Rust mask and the C identities are separate numberings joined by one table.

Lacking:

1. L4 on the emulators and on m68k. The Hosted gates were relinked and run
   against revision 15 on 2026-09-17: the handler that passed S0, S1 and
   [`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh) is built
   from the whole boundary, whose exported-symbol list
   [`check-aros-ffi.sh`](../tools/check-aros-ffi.sh) checks at link time, and
   those runs reach the DOS, soft-link, notify, record, volume-label,
   comment, attribute and extension-packet groups through a real
   dos.library, and the paged object-ID walk and the health-event ring
   through the extension packet. What no Hosted run reaches: the trace sink,
   which no target path can call at all. QEMU and m68k stand where they were,
   because neither toolchain is on the machine that ran the rest;
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
| all of the above | the QEMU and m68k sequences do not run [`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh); Hosted does, and passes | L4 |
| one instance per unit on an SMP kernel | the claim relies on `Forbid()` for the port list | L4 |
| `ACTION_FORMAT`, `ACTION_SERIALIZE_DISK` | in-handler mkfs through the mounted device; refused while locks are open | L1, L4 |
| `ExNext` resume cost | one resume reads O(log n) single-entry pages; a core seek-by-key page read makes it one descent | core |

Proven on Hosted darwin-aarch64 by [`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh): the setters, the comment, soft links, `ExAll`, `OpenFromLock`, `ChangeMode`, record locks with a grant by a second task's release, the owed `NRF_WAIT_REPLY` notification, a dismount with a message never replied, `Relabel`, and one instance per unit under dos.library's double start. QEMU and m68k: every row. Apple hardware: none.

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

The on-disk half arrived with the security preservation container
([ADR-101](../adr/ADR-101-security-preservation-container.md)) and its
admission rule ([ADR-105](../adr/ADR-105-security-reference-admission.md)): a
descriptor is preserved across a classic protection write, and across a clone
([ADR-102](../adr/ADR-102-clone-metadata-inheritance.md)). `RichSecurityProbe`
stays the seam for metadata the projection cannot express and that is not a
descriptor, of which this format has none, so its default answer of no is
exact. The `Control` string of the DOSDriver selects the policy
(`SECURITY=PRESERVE|STRICT|DOWNGRADE`).

Lacking: nothing at L1 to L4 for what the format stores. The open question is
policy, not code: who may read a historical view after live permissions
change ([Q5](open-questions.md), format half closed, evaluation semantics and
historical access open), which bounds what a v2 snapshot capability may
advertise.

## C4. Filesystem API v2 and the modern 64-bit API

Present (L1, L2): the `API_V2` entry-point group on the same locks and
handles as the DOS calls: positioned 64-bit read and write that leave the DOS
position alone, atomic replace over an unheld target, and the capability and
limits query of C1 with one published numbering (`FSV2_CAP_*`), and the
`OBJECT_IDS` group: lookup and stat by object ID and a paged directory walk
whose position survives namespace changes between pages.

Lacking:

1. the transport ([docs/13](../docs/13-filesystem-api-v2.md#the-aros-transport))
   is present at L3: `ACTION_AFSPLUS_EXT`, its fifteen packet-layer
   operations and a client library with fallback, proven by the packet and
   client host matrices and cross-compiled. Open: its target run, which
   [`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh) drives
   through `AFSPlusInfo`, and the AROS-wide allocation of the packet number;
2. a consumer: the AROS Rust `std` port binding to the group.

Hosted and QEMU: all. Apple hardware: none.

## C5. Clone and reflink capability API

Present (L1, L2): `afsplus_aros_clone_file` from a lock and
`afsplus_aros_clone_range` between two handles, advertised separately and
answering `ERROR_ACTION_NOT_KNOWN` on a volume without shared extents.

`AFSPlusClone` (L3, cross-compiled) clones through the transport of C4 and
falls back to a byte copy across handlers or without the capability; its
target run is part of [`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh).

Lacking: the same choice inside `C:Copy`, which is an AROS change. Metadata
inheritance is settled ([ADR-102](../adr/ADR-102-clone-metadata-inheritance.md),
extended by [ADR-106](../adr/ADR-106-stored-object-comment.md) for the comment
and [ADR-108](../adr/ADR-108-extended-attributes.md) for the attribute set):
`CloneFile` carries the modification time, the protection word, the security
descriptor, the comment and the attributes; `CloneRange` is a content write.

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

The handler shell delivers through its own reply port, and that path ran on
Hosted darwin-aarch64 on 2026-09-17 inside
[`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh): against a
real dos.library, no message arrives while one is unreplied, the change made
in between arrives after the reply, and a second boot dismounts a volume
whose notification was never replied, with the shell still running
afterwards.

The v2 watch travels as `WATCH_ADD`, `WATCH_TAKE` and `WATCH_REMOVE` of
the extension packet ([`afsplus_ext_packet.h`](../api/afsplus_ext_packet.h)),
over the same table: the handler's drain after every packet marks an
extension watch as fired, and a take reads and clears the mark, so changes
between two takes are one. A NotifyRequest's watch is out of the transport's
reach, and an extension watch holds no port, so `ACTION_DIE` discards what
is left. The packet matrix proves the separation both ways, and
[`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh) runs a watch
of a name created after it through a real dos.library.

QEMU: all. Apple hardware: none.

## C9. Health reporting

Present (L1, L2): every failed call on a mounted instance that describes the
volume or its device enters a health log with counters, degraded-state flags
and a bounded event ring whose sequence numbers expose loss;
`afsplus_aros_health` adds generation, pending intent records, pending
orphans and block counts and flags a read-only view of an unreplayed log.

Present (L1, L2): the three events of
[docs/26 section 17](../docs/26-debug-observability.md#17-structured-healthevent-stream)
that no failed call can report, because nothing fails. They carry `dos_error`
0, set no degraded-state flag, and each has its own counter in
`AfsplusArosHealth`, in the handler's info JSON and in the event ring. The
VFS gathers them as notes and the adapter drains them into the log after
mount and after every call, which keeps AROS types out of the core.

- `CHECKPOINT_FALLBACK` (5): selection rejected a slot that claims a
  generation newer than the chosen one, so the volume reads the older
  checkpoint of the pair and the last commit before this mount is not in what
  it shows. A slot that was never written claims nothing. Proven by
  `crates/afsplus-aros/tests/health_events.rs`: two commits use both slots,
  the newer slot's payload is damaged so its checksum fails, and the mount is
  clean, one generation back, with exactly one event; the controls are the
  same image undamaged and a freshly formatted volume whose second slot is
  empty.
- `RECLAIM_BACKLOG_HIGH` (6): the blocks quarantined in the reclaim queue
  plus the deleted files still waiting to be cleaned crossed a sixteenth of
  the volume, at least 4096 blocks; the next crossing is only noted after the
  backlog has fallen below half of it. Proven by
  `crates/afsplus-vfs/tests/health_notes.rs`: retiring a large file at once
  is one note, four further maintenance rounds over the same backlog are
  none, draining it below half arms it again, and a second large delete is a
  second note; the control is an ordinary small delete on the same volume,
  which notes nothing. `crates/afsplus-aros/tests/health_events.rs` carries
  that backlog into the handler's log as event 6.
- `REGION_FREECOUNT_MISMATCH` (7): the mounted checkpoint's
  `free_blocks_total` against the sum over its own allocation-root region
  records, one bounded read at the first health drain. The bitmap pages
  themselves are not read: a normal mount never reads them, and the region
  records are what the descriptors and the pages are checked against on the
  full load path and by `afsplus-check`. Proven by
  `crates/afsplus-aros/tests/health_events.rs`: the checkpoint is re-encoded
  with a free count off by one and a checksum that covers the lie, the mount
  believes the record and records one event; the control is the same volume
  untouched.

The target query path exists: `HEALTH_EVENTS` over the
extension packet of C4 empties the ring and reports what it dropped.

## C10. Trace streaming and developer attachment

Present (L1, L2): `afsplus_aros_set_trace_sink` attaches the core flight
recorder to a callback of
[`debug_observability.h`](../api/debug_observability.h) with a run-time
category mask; `afsplus_aros_trace_counters` reports delivered, missed,
filtered and dropped.

The target front-end is the handler: with `TRACE=<events>` in the DOSDriver
`Control` string it preallocates a ring, attaches the sink, stamps each event
from its own clock, and a tool drains the ring through the extension packet
(`AFSPlusInfo <path> TRACE`). Proven on Hosted: 46 events with a ring of 64
and nothing lost, and with a ring of 8 the last 8 events with the other 85
counted as overwritten.

Every event carries a stable code, `enum afsp_trace_event_code` of
[`debug_observability.h`](../api/debug_observability.h): the same numbers the
diagnostic bundles use, appended and never reused, and a test holds the header
to the core's table.

Lacking: a serial front-end for QEMU and m68k, where no tool can run beside
the filesystem; a clock better than the system tick, which today gives every
event of one operation the same stamp. Forwarding from the M1 target to a
development host needs the hardware.

## C11. Structured management APIs

Present (L1, L2): `afsplus_aros_info_json` serves one versioned JSON
document (`afsplus-handler-info`, version 1) from the mounted instance:
identity, feature masks, mount state, capability names, health, handle usage.

Present (L3, cross-compiled): `AFSPlusInfo`, which prints the report of the
handler behind a path through the transport of C4; not yet run on a target.

Lacking:
machine-readable error documents for failed management calls; dry-run
planning for destructive operations, which starts with `ACTION_FORMAT`.

## C12. File-backed virtual block device

Present: the generic AROS `fdsk.device` carries S0 and S1 on Hosted; the
external `afsram.device` carries native QEMU.

[`native/aros/upstream`](../native/aros/upstream/README.md) holds a patch and
a regression probe for item 1, both now built and run on a target.

Lacking in the generic device, as upstream patches with regression probes:

1. `CMD_UPDATE` and `ETD_UPDATE` are answered inside `BeginIO`, so the reply
   overtakes queued writes and the barrier never reaches the backing file.
   Both the defect and the fix are now shown on a target by
   [`fdsk_update_probe.c`](../native/aros/tests/fdsk_update_probe.c); the
   patch is not applied in the gate tree, and
   [`check-aros-fdsk-ordering.sh`](../tools/check-aros-fdsk-ordering.sh) is
   the ONLY gate that requires a patched AROS: it applies the fix, rebuilds
   that one device, proves the ordering and restores the tree. The second
   half, a host `fsync`,
   waits on a generic AROS decision named in
   [`native/aros/upstream/README.md`](../native/aros/upstream/README.md).
   `hostdisk.device` had the same defect, found by
   [`AFSPlusDriverProbe`](../native/aros/tools/afsplus_driver_probe.c)
   ([`check-hosted-aros-driver.sh`](../tools/check-hosted-aros-driver.sh)),
   and passes the barrier with the local patch
   [`native/aros/aros-patches/hostdisk-unit-pattern.patch`](../native/aros/aros-patches/hostdisk-unit-pattern.patch):
   `CMD_UPDATE` goes to the unit thread like `CMD_WRITE`, and the unit thread
   syncs the host file, `fcntl(fd, F_FULLFSYNC)` on Darwin with `fsync()` as
   the fallback and on every other host, so both halves hold there. A failing
   sync is reported as a non-zero `io_Error`, never a silent success;
2. upstream has no `TD_READ64`, `TD_WRITE64` or `NSCMD_TD_*64`; MacAROS
   carries that fix, and AFS+ images beyond 4 GiB depend on it. Without it
   the handler does not misaddress such a volume, it refuses to mount it: the
   startup probe finds neither the NSD commands nor `TD_READ64`, and the
   trackdisk adapter answers `ERROR_OBJECT_TOO_LARGE` for a partition ending
   above 4 GiB. Shown on Hosted with a DOSDriver of 1,099,999 cylinders of
   4 KiB: `mount failed at trackdisk-adapter: error 207`, `List` fails with
   `RETURN_FAIL`, no handler task is left and nothing traps;
3. attach and detach of a unit to a named file at run time
   (`AttachDisk`-style), instead of the fixed `FDSK:Unit<N>` convention.

Hosted: all three. Apple hardware: the persistent native device that
replaces the retained-RAM transport.

## C13. Native AROS benchmark runner

Present: the host measurement harness and
[benchmark-contract](../testing/benchmark-contract.md).

`afsplus_aros_counters` (L1, L2) reports completed and failed calls and
device reads, writes, barriers, bytes and failures since mount.

The packet layer counts answered packets by type and failures by error code
in bounded tables (L3), read through the transport of C4 with
`afsplus_client_packet_counts`.

`afsplus_aros_counters` also reports the library's Rust heap: the bytes its
allocations hold and the most they held at once since it started, counted by
a metering allocator around the system one.

`STEADY <rounds>` of the DOS probe (L4) runs a round of paired operations —
create, open, read, lock and unlock a record, close, lock, examine, unlock,
start and end a notification, set the comment and the protection, delete —
five times as a warm-up, then the given number of times, and reports the
system's free memory and the handler's heap before and after, each read after
an `ACTION_FLUSH` so that a delayed mount is measured at a durable point.
Under [`check-hosted-aros-dos.sh`](../tools/check-hosted-aros-dos.sh) a
hundred rounds may move the heap or its peak by at most 256 bytes. System
free memory alone cannot show a leak: the allocator's pools absorb it. The
heap counters found one it hid, a cache of parents that grew by about 94
bytes a round.

The meter is the library's allocator, and at the C boundary the tests' disk
is a sparse in-memory device that allocates each block the first time it is
written: a test that makes the volume write new blocks sees them in the heap.
Two earlier findings were that device and not the library, and a disk whose
blocks all exist beforehand shows neither: pending orphans cost no memory,
and rewriting one file 1,500 times holds the same heap throughout. What
remains open is a growth of a few dozen bytes over thousands of commits, in
sync mode too, by steps that suggest a slowly growing collection not yet
identified.

The target runner is
[`tools/bench-hosted-aros.sh`](../tools/bench-hosted-aros.sh) with
`AFSPlusBench` on Hosted: a fixed seeded workload against AFS+ and the Fast
File System in one boot, the package manifest and the image checked before,
the AFS+ image checked after, and a `results.json` bundle in the contract
format ([benchmark contract](../testing/benchmark-contract.md#native-aros-runner)).

Lacking: the same runner on the emulators and on m68k, where the clock and
the budget of
[section 2](../testing/aros-system-volume-qualification.md#2-what-macaros-performance-proves)
apply; the M1 and the A500 for hardware numbers.

## C14. AROS handler qualification ladder

S2 on Hosted: AROS boots from an AFS+ partition
([`check-hosted-aros-s2.sh`](../tools/check-hosted-aros-s2.sh)). The handler is
a boot module and registers DosType AFS+ in `FileSystem.resource`; the
partition is an AROS GPT partition of that type ([ADR-122](../adr/ADR-122-aros-partition-identity.md));
the boot scan finds it through partition.library and hostdisk.device, whose
unit pattern comes from a kernel argument (a local AROS patch in
`native/aros/aros-patches`). The recorded bootstrap dependencies are those
three modules and nothing else: the handler serves packets without
stdc.library, posixc.library or stdcio.library, which cannot start before
the boot volume (`afsplus_bootlibc.c`, `afsplus_bootposix.c`), and a volume
needs `AROS.boot` naming its CPU to be bootable.

S3 on Hosted: a machine booted from that partition is cut off and booted
again, round after round
([`check-hosted-aros-s3.sh`](../tools/check-hosted-aros-s3.sh)). Each round
lets the Startup-Sequence reach Wanderer and a workload that churns files,
saves a preference under ENVARC: and writes one marker it makes durable with
ACTION_FLUSH and one it only closes; then `aros-ctl kill` takes the machine
away at the moment the schedule names, the partition is taken out and checked,
and a second boot has to reach the Startup-Sequence on AFS+ and read every
marker back.

The default run, 24 rounds in 24 minutes: 4 cuts inside the Startup-Sequence,
5 in the workload's writes, 5 within 400 ms of a marker being made durable, 5
in the idle time the mount cleans its deletes in, and 5 clean shutdowns as the
reference. 24 boots after a cut, all reaching AFS+; 24 partitions checked
after the cut and 24 after the mount that followed, all clean and all with
`log_records_pending` 0; 25 markers kept by the last round, every one byte for
byte; 0 torn, 0 lost. Boot time held: 21.8 s mean over the first three rounds
against 20.4, 25.7 and 25.4 s over the last three, against a leak clause that
fails at twice the mean.

A boot mount found through the boot scan has no DOSDriver to say a stack size,
and S3 found the handler task overrunning partition.library's 40 KiB while it
replayed an intent-log record, twice in a row: the handler's
`FileSystem.resource` entry now carries the 262144 bytes every AFS+ mountlist
here asks for (`native/aros/afsplus.conf`). The same mount runs SYNC rather
than with the COMMIT=5 default, because it is made before the handler can open
timer.device; that is why the gate's negative control claims a marker it has
not written rather than merely skipping the flush.

Open: S2 on QEMU and Native, S3 on QEMU and Native; the Apple hardware run
with a reset-durable transport; the physical A500.

## What Stage C still waits on elsewhere

Stage B is struck: the stored comment, the extended attribute set, the
security container and the clone inheritance rule all landed, and the rows
that named them are gone. One question remains, and it is policy rather than
format or code.

| Need | Item | Question |
|---|---|---|
| ordinary-user access to historical views, which bounds what a v2 snapshot capability may advertise | C3, C4 | [Q5](open-questions.md), evaluation semantics and historical access |
