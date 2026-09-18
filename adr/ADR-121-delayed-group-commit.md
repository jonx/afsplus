# ADR-121: Changes are durable within seconds, or at once on request

Status: Accepted
Amends: ADR-063

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
   window is committed. A commit cleans only what exceeds 4,096 waiting
   objects, so a volume that is never idle still has a bound. An operation
   that needs space reclaims it first, counting what the open window has
   already allocated.

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
