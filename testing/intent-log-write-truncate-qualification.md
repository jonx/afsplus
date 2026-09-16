# Intent-log existing-file update qualification

> **ADRs:** [ADR-063](../adr/ADR-063-intent-log-epoch1.md),
> [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md) ·
> **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** `crates/afsplus-check/tests/intent_log.rs`,
> `crates/afsplus-check/tests/shared_crash.rs`,
> `crates/afsplus-check/tests/fsync_workloads.rs`,
> `crates/afsplus-vfs/tests/api.rs`, `crates/afsplus-fuse/tests/protocol.rs`,
> `crates/afsplus-fuse/tests/host_mount.rs`,
> `crates/afsplus-aros/tests/adapter.rs`,
> `crates/afsplus-aros-ffi/tests/ffi.rs` · **Milestones:** M04, M07, M14

This gate qualifies experimental intent-log version 3 for writes and
truncates of already committed files through the Rust core, portable VFS,
host FUSE protocol and packet-neutral AROS adapter. It does not freeze the wire
format. Portable C now validates the complete data/namespace view and can
append a namespace-only no-replace rename; it does not yet allocate or emit
version-3 data updates.

<!-- toc -->

- [Durability oracle](#durability-oracle)
- [Window ownership on preflight refusal](#window-ownership-on-preflight-refusal)
- [Compatibility oracle](#compatibility-oracle)
- [Reproduction](#reproduction)
  - [macFUSE installation and activation on macOS](#macfuse-installation-and-activation-on-macos)
- [Crash matrix](#crash-matrix)
- [Remaining boundary](#remaining-boundary)

<!-- /toc -->

## Durability oracle

An existing-file update in an open window follows this order:

1. construct complete replacement blocks from the current logical view;
2. allocate fresh physical blocks and write the replacement data;
3. flush that data before writing a record that names it;
4. write and flush one checksummed record for the complete fsync group;
5. later materialize all logged groups through one ordinary COW checkpoint.

Before fsync completes, a crash may recover the old file. After it completes,
mount must replay the record and recover the new file. No state may expose a
mixture of old and new logical blocks. Successive completed groups may recover
only a monotone prefix of their record sequence.

The data extents named by every active record must be allocatable but FREE in
the base checkpoint bitmap, pairwise disjoint across the valid record prefix,
within the resulting logical file size and protected by a CRC over every
complete replacement block. A partial shrinking truncate may name exactly one
zero-tailed block; sparse growth, aligned shrink and a shrink whose retained
tail is a hole carry no data.

## Window ownership on preflight refusal

Cancellation lookup for a delete or replacing rename precedes mutation. A
failed name check, absent/non-directory parent, or failed device read at that
point must preserve the entire open window, including every logged group and
unlogged operation. It performs no writes or barriers. A later valid operation
and fsync must preserve the same bytes as execution without the refused request.

`window_write_file_at` and `window_truncate_file` share that boundary. Each
builds its complete replacement blocks and allocates fresh physical extents
before it writes anything, so a refusal that reaches neither the first
replacement-block write nor the staged bookkeeping preserves the entire open
window and performs no write and no barrier. A resource refusal of either call
belongs to this class.

After mutation, failure to construct the corresponding log record requires
remount before further mutation; a writable window cannot discard that mutated
operation's bookkeeping. This follows the existing uncertain-window rule and
changes no intent-log encoding or filesystem API v2 ABI.

The `cancellation_preflight` tests in
[intent_log.rs](../crates/afsplus-check/tests/intent_log.rs) cover all four tree
cache profiles, with and without an acknowledged fsync prefix. They check
unchanged pending counts and write/flush counts for `NotDirectory`, `NotFound`,
invalid replacement names and an injected transient read error. Successful retry,
fsync and remount must recover the exact file contents with a clean checker.
Run the focused gate with:

```text
cargo test -p afsplus-check --test intent_log cancellation_preflight
```

## Compatibility oracle

Any group containing `Write` or `Truncate` uses record version 3 and requires
`org.aros.afsplus:intent-log-data-updates` (`INCOMPAT` bit 1), which itself
requires the base intent-log bit. A version-3 record without bit 1 is
corruption. A volume carrying only the historical bit 0 remains usable for
namespace-only version-2 records, while its existing-file window API returns
`FeatureDisabled`.

This fail-closed rule is essential: the historical scanner treats an unknown
record version as an invalid tail, which would otherwise let an older writer
silently discard an acknowledged fsync.

## Reproduction

Run codec, intent-log and shared-reference correctness gates:

```text
cargo test -p afsplus-format --test roundtrip
cargo test -p afsplus-check --test intent_log
cargo test -p afsplus-check --test shared_crash
cargo test -p afsplus-vfs --test api
cargo test -p afsplus-fuse --test protocol
cargo test -p afsplus-aros --test adapter
cargo test -p afsplus-aros-ffi --test ffi
```

With macFUSE's FSKit modules enabled, run the real host boundary:

```text
AFSPLUS_FUSE_MOUNT_TEST=1 \
  cargo test -p afsplus-fuse --all-features --test host_mount \
  -- --ignored --nocapture
```

### macFUSE installation and activation on macOS

Use the signed installer from the [official macFUSE installation guide](https://github.com/macfuse/macfuse/wiki/Getting-Started).
The FSKit backend requires macOS 15.4 or later. This procedure uses FSKit and
does not require enabling the kernel backend or changing Recovery security.

1. Download the official macFUSE disk image, open it and run **Install macFUSE**.
   Approve the installation with an administrator account when macOS asks.
2. Open **System Settings → General → Login Items & Extensions → File System
   Extensions** (select **By Category** if necessary). Enable both **macFUSE**
   and **macFUSE (local)**.
3. If the first mount fails during extension startup or registration, run the
   official registration command below. Complete the administrator password
   dialog it presents; command invocation alone is not evidence of completion.
4. Run the explicit real-mount test above. Require a passing operation matrix,
   successful unmount and final image check before calling the setup qualified.

```text
/Library/Filesystems/macfuse.fs/Contents/Resources/macfuse.app/Contents/MacOS/macfuse install --force
```

Installation and activation are separate prerequisites. A missing
`MFMount.framework` is an installation failure; an enabled-module refusal or
ExtensionKit startup error occurs before the test can qualify filesystem
operations. Inspect the macFUSE and FSKit system logs to distinguish them.

Record the macOS build, macFUSE version, backend, exact test command and exit
status with the qualification. A successful installation or registration does
not replace a successful operation matrix and final image check. This gate
uses a temporary image and mount point; it is not native-device qualification.

Before any close, rename or unmount can mask the result, this gate opens the
backing image through a second descriptor after host `fsync` and requires a
checker-clean replayable record or newer checkpoint. A host stack that returns
from the syscall without delivering FUSE `FSYNC` uses the macOS FSKit
write-through fallback: every delivered `WRITE` or size-changing `SETATTR` is
made durable before its reply. The backing-image oracle verifies the fallback
at the syscall boundary; the protocol test separately crashes immediately
after write and truncate replies without issuing `FSYNC`.

Run the optimized 4,000-operation measurement:

```text
cargo test -p afsplus-check --test fsync_workloads --release \
  fsync_workload_qualification -- --ignored --nocapture
```

The release gate reports reads, writes and flushes per operation for logged
append, logged database-hotset rewrite and checkpointed append. It requires
the logged paths to beat the checkpoint flush floor. Retained measurements
and their interpretation live in the
[fsync baseline](../implementation/fsync-intent-log-baseline.md); these
memory-backend structural counts make no hardware-latency claim.

## Crash matrix

The executable cases prove:

- every cut around a logged existing-file write yields the exact old or new
  content and a checker-clean image;
- a write and partial truncate replay in order, preserving timestamps and
  content generation semantics;
- a write followed by rename is one all-or-nothing fsync group;
- restarting after every modeled write/flush of recovery itself converges on
  the same final checkpoint;
- successive writes recover only record-prefix contents;
- sparse growth, aligned shrink and partial shrink choose the correct
  data-free or one-tail-block representation;
- a logged write to one reflink owner splits shared-reference accounting while
  the other owner retains its original bytes, including crashes during
  recovery;
- bit-0-only volumes retain namespace replay but reject data updates, and a
  version-3 record with no bit 1 fails closed in both mount and checker.
- VFS reads and stats observe staged bytes before checkpoint, a successful
  VFS `fsync` writes no checkpoint slot, and remount replays its durable record;
- log-free volumes do not advertise `LOGGED_DATA_FSYNC` and retain the
  checkpoint path; FUSE, AROS and its current C ABI bridge exercise the same
  read-your-writes path;
- the macFUSE FSKit mount strengthens each data-mutation reply into a durability
  point, while host-neutral FUSE and the AROS adapters retain ordinary deferred
  writeback plus explicit `fsync`.

## Remaining boundary

The gate covers the Rust core APIs and recovery path. A log record is bounded
to one block and at most 16 physical extents per data operation.
The VFS may still publish a data window before immediate namespace/reflink
transactions, but recovery now also handles a logged write followed by final
delete/replacement: the final victim and its resulting layout enter ADR-066
orphan state in the replay checkpoint. The independent C writer exposes
data-free truncate, delete and both rename modes. Its truncate covers sparse
growth and aligned shrink without allocation; unaligned shrink still requires
a replacement tail block. Portable-C allocation/data emission, real-storage
flush testing and final numeric wire allocation remain M14 work.
