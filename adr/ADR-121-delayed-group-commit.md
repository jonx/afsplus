# ADR-121: Changes are durable within seconds, or at once on request

Status: Accepted
Amends: ADR-063

<!-- toc -->

- [Context](#context)
- [Decision](#decision)
- [Consequences](#consequences)

<!-- /toc -->

## Context

[ADR-063](ADR-063-intent-log-epoch1.md) chose checkpoint copy-on-write with
bounded group commit and an intent log. Group commit exists as an explicit
batch and as the open window of file-data writes, but every namespace and
metadata operation of the AROS handler still commits its own checkpoint
before it returns. On the Hosted benchmark of
[C13](../implementation/stage-c-gap.md#c13-native-aros-benchmark-runner)
that is 51,734 barriers and 828 MB written for 6.3 MB of files. The
measured batch of
[`fsync-intent-log-baseline`](../implementation/fsync-intent-log-baseline.md)
cut barriers 63 times and writes 5.9 times on the same kind of workload.

Other file systems settle the same trade by time: ext4 commits its journal
every 5 seconds by default, ZFS a transaction group every 5 seconds, Btrfs
every 30; every one of them makes an explicit sync immediate. A checkpoint
file system can offer a stronger version of that trade, because what a crash
loses is a whole suffix of operations, never a torn one.

## Decision

1. A mount has a durability mode. `SYNC` is the behaviour before this
   decision: every operation is durable when it returns. `DELAYED` gathers
   operations in the open window and commits them together.
2. `DELAYED` commits when the first of these happens: the volume has had no
   operation for one second; the oldest uncommitted operation is older than
   the mount's maximum age, five seconds by default; the window reaches its
   bound in operations or memory; or something asks for durability: fsync of
   a handle, `ACTION_FLUSH`, inhibit, dismount, format, write protection,
   disk change, and a lack of space the commit would relieve.
3. A crash loses at most the uncommitted operations, as a whole and in order.
   The volume mounts at its last checkpoint plus the fsync groups of the
   intent log. An operation is never torn, and a later operation never
   survives an earlier one it followed.
4. Every operation is visible to every caller of the mount at once,
   committed or not.
5. An operation the intent log cannot record makes the next fsync commit the
   window as a checkpoint instead of appending a log record.
6. The AROS handler defaults to `DELAYED`. The DOSDriver `Control` string
   selects `COMMIT=SYNC` or `COMMIT=<seconds>`, the maximum age from 1 to 60.
7. The macOS FUSE mount stays `SYNC`: macFUSE does not forward `fsync(2)`,
   so a delayed mount would silently lose what an application synced. It may
   offer `DELAYED` once fsync reaches the driver.
8. Operations join the window as the core learns to stage them. One the
   window does not stage commits the window first and then itself, as
   before: correct, only slower.
9. `DELAYED` commits a delete as the removal of the name. The object's
   blocks come back in idle time, a bounded batch per idle tick, after the
   window is committed. A commit cleans what exceeds 4,096 waiting
   objects, and cleans until an eighth of the volume, and at least 1,024
   blocks, is available: a volume that is never idle still has a bound, and
   the next commit has room. An operation that needs space reclaims it
   first, counting what the open window has already allocated.

**Amendment, 2026-09-19.** Directories join the window (decision 8). A
`CreateDir` was an immediate transaction: it committed the open window and
then ran a transaction of its own, four device flushes for every drawer, and
a `Copy ALL` of a source tree paid that for every directory it made. The
window now stages the new directory's record, the block its entry tree will
be rooted at and the entry in its parent, and the directory is usable at once:
a file created in it in the same window, a lookup, a stat, a rename into it.
The intent log has Create, Delete, Rename and Write and no record that makes
a directory, so a window holding one is unloggable and the next fsync
commits it as a checkpoint rather than appending, which is decision 5. That
is not a format change; a log record kind for a directory would be, and is
not proposed here. Removing a directory is still not stageable: the
immediate path commits the window first, as this decision allows. Measured
at the C boundary, 90 drawers of 32 files: 3.98 flushes per drawer before,
0.38 after, which is the window commits of the whole phase spread over the
drawers.

**Amendment, 2026-09-19.** The AROS handler was committing on every close of
a written file, which decision 2 never asked for: `ACTION_END` called
`afsplus_aros_fsync`, left there from before this decision, when every
operation was durable when it returned. Under `DELAYED` that is a whole
checkpoint for each created file, because a write to a file the open window
created makes the window unloggable: 2.00 flushes and 12 block writes per
created file against 0.01 and 2.1 without it. A close is not in the list of
decision 2 and is not added to it. AmigaDOS `Close()` promises nothing about
the medium, the Fast File System does not flush there, and a program that
needs its bytes on the disk asks for them, with `ACTION_FLUSH` or an fsync of
the handle before it closes. `ACTION_END` now closes the handle and nothing
more; the dismount path still synchronises the files it finds open, because a
dismount is a durability point.

**Amendment, 2026-09-19 (cleanup is batched per transaction).** The bounded
batch of decision 9 was a batch of transactions rather than a transaction.
Each waiting object cost two commits, one to release its data and one to
remove its name, so an idle tick of 32 objects was 67 commits and 134 device
flushes, and on the hosted benchmark the delete phase paid 2,170 flushes for
2,650 deletes: the delete itself is a window operation, the flushes were the
cleanup behind it. Cleanup is now batched per transaction. One transaction
takes up to 32 orphans from the orphan directory, releases their data,
removes the entries of the ones it finished and commits once; the same tick
is 4 commits and 8 flushes, and the delete-then-create loop of the profiling
harness falls from 2.20 to 0.04 device flushes and 12.93 to 1.79 block writes
per operation. What bounds a batch is its count, 32 objects, and its extent
budget, which is the volume's per-orphan budget times that count: exactly
what that many single steps spent, so a batch does their work and only the
commits are fewer. An orphan whose data outlives the budget stays in the
orphan directory, shrunk, and the batch stops there. An orphan a caller still
holds open is passed over rather than waited for, and no longer stops the
cleanup of the orphans behind it. A cut in the middle of a batch leaves every
orphan in it whole or gone, because the batch is one checkpoint.

**Amendment, 2026-09-20 (a drawer is removed in the window too).** The
amendment above left removing a directory on the immediate path, which
decision 8 allows. Measured on the host with the benchmark's own tree, ten
trees of eight drawers of 32 files on a 64 MiB volume, that was the whole
cost of the delete phase: the 2,560 file deletes cost 10 device flushes, the
five commits of the window bound, and the 91 drawer removals cost 910, five
checkpoints and ten flushes each. The five are the open window committed
first, the removal's own transaction, and the three maintenance transactions
behind it. A directory now joins the window as `RemoveDirectory`. Empty means
empty as the window sees it: an entry the window deleted is gone, an entry it
created or renamed in keeps the directory occupied and the removal is refused
exactly as the immediate path refuses it, without committing anything to find
out. A directory the same window created cancels out, record, parent entry
and the block its entry tree was to be rooted at together, and nothing of it
reaches the disk. A committed one gives its record block, its entry-tree
blocks and its security and attribute blocks to the window's transaction to
retire, which is what `Volume::remove_directory` does. As for `CreateDir` the
intent log has no record kind for it, so a window holding one is unloggable
and the next fsync commits it as a checkpoint, which is decision 5; that is
not a format change. The same phase now costs 10 flushes and 5 checkpoints in
all, and 91 drawer removals cost nothing beyond the window commits.

## Consequences

- An unprotected crash or power cut on AROS loses up to the maximum age of
  work, whole operations only. Classic Amiga file systems buffer too; users
  know to wait for the drive before a reset, and `COMMIT=SYNC` is there for
  a volume that must not wait.
- Programs that need durability ask for it, as on every other system; the
  handler commits at once for them.
- The crash matrices gain delayed windows: every cut must mount at a prefix
  of the operations, with no torn state.
- A delete costs the removal of its name; the space of a deleted file is
  not available at once, but a write that needs it takes it back.
- The read cache is separate and changes no durability; see
  [performance](../docs/20-performance.md#cache-integration-qualification).
