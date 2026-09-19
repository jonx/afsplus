# Testing a mounted volume

> **ADRs:** none · **Spec:** none ·
> **Tests:** [tools/check-mounted-usage.sh](../tools/check-mounted-usage.sh),
> [tools/check-mount-driver-death.py](../tools/check-mount-driver-death.py),
> [tools/check-mount-responsiveness.py](../tools/check-mount-responsiveness.py),
> [tools/check-mount-name-policy.py](../tools/check-mount-name-policy.py),
> [tools/check-mount-kill-durability.py](../tools/check-mount-kill-durability.py),
> [tools/check-mount-endurance.py](../tools/check-mount-endurance.py),
> [crates/afsplus-vfs/tests/space_under_load.rs](../crates/afsplus-vfs/tests/space_under_load.rs),
> [crates/afsplus-fuse/tests/process_death.rs](../crates/afsplus-fuse/tests/process_death.rs),
> [crates/afsplus-vfs/tests/background_maintenance.rs](../crates/afsplus-vfs/tests/background_maintenance.rs) ·
> **Milestones:** M08

This plan covers what a person can do with an AFS+ volume mounted through
macFUSE: create, copy, rename, link, archive, and unmount with everything
still there afterwards. Its gate is
[tools/check-mounted-usage.sh](../tools/check-mounted-usage.sh), which makes
its own image, mounts it, uses it with ordinary tools and reports each result
in the words a person would use.

It also covers how to test a mounted driver without taking the machine down
with it, which is the harder half.

<!-- toc -->

- [A driver under test must not be able to stop the desktop](#a-driver-under-test-must-not-be-able-to-stop-the-desktop)
- [What leaves a dead mount behind, and what clears it](#what-leaves-a-dead-mount-behind-and-what-clears-it)
- [Simulating a filesystem that stops answering](#simulating-a-filesystem-that-stops-answering)
- [One request at a time](#one-request-at-a-time)
- [Hours of use by several programs](#hours-of-use-by-several-programs)
- [What a killed driver keeps](#what-a-killed-driver-keeps)
- [macFUSE releases](#macfuse-releases)
- [What macFUSE's FSKit backend does to a listing and to `df`](#what-macfuses-fskit-backend-does-to-a-listing-and-to-df)
- [What belongs below the mount](#what-belongs-below-the-mount)

<!-- /toc -->

## A driver under test must not be able to stop the desktop

A macFUSE mountpoint whose filesystem process died can be left behind in a
state nothing can clear. It is no longer in the mount table, every `stat` on
it blocks for ever, `umount -f` blocks on it too, and only a reboot removes it.

Anything under `/Volumes` is visited by the whole desktop: the Finder,
Spotlight, file managers and `diskutil` all enumerate it. One dead entry there
stops each of them the next time it starts, even though nothing is wrong with
any real disk.

Three rules follow.

1. **Mount outside `/Volumes`.** Use a directory the test owns, such as its
   own work directory. macFUSE mounts on any existing directory, the desktop
   never sees it, and a failure cannot leave an entry that other programs
   will walk into.
2. **Never call `diskutil` from a script, and do not trust `mount` either.**
   Both ask every mounted filesystem for its state, so one dead mountpoint
   anywhere makes them block, whatever volume they were asked about. `|| true`
   does not rescue a command that never returns. Unmount with `umount`, then
   `umount -f`. To read the mount table, use `getmntinfo` with `MNT_NOWAIT`,
   which reads the cached table and asks no filesystem anything;
   [tools/check-mount-driver-death.py](../tools/check-mount-driver-death.py)
   shows how from Python. Resolve a mountpoint's real path before mounting,
   because resolving it through a dead mount blocks too.
3. **Probe a mountpoint with a bounded check.** `-d` and `-e` block on a dead
   mountpoint instead of returning false. Run the check in the background and
   treat no answer within a few seconds as a mountpoint that does not respond.

## What leaves a dead mount behind, and what clears it

macFUSE's FSKit backend relays every request from the kernel to the driver
through a helper process, `io.macfuse.app.fsmodule.macfuse`, one per mount,
running as the user who mounted. When the driver dies while a request is in
flight, that helper waits for an answer that never comes. The program that
made the request cannot be killed, `umount` blocks the same way, and the
mountpoint stays behind with every `stat` on it blocking.

A client killed on its own leaves nothing behind. It takes the driver dying
mid-request.

`afsplus-mount` therefore runs as a supervisor over the process that serves
the volume. When that process ends any way other than a clean unmount, the
supervisor unmounts the volume by its canonical path, and terminates the
helper of that mount only if the unmount blocks, which releases everything
waiting on it with an I/O error and lets the unmount finish. Stopping the
supervisor with Ctrl-C, `kill` or a closing terminal unmounts cleanly instead
of killing the driver under its clients.

The order matters. macOS keeps its own record of FSKit mounts, in
`/Library/Application Support/livefsd/settings.plist`, and removes an entry
only when the mount ends through an unmount. A mount whose helper is
terminated first disappears from the kernel but stays recorded, and macOS then
refuses every later mount at the same path, as `mount(8) returned 69`, until
the machine restarts. The driver-death check mounts again at the same path to
hold this. A refused mount also never returns from macFUSE's mount call, so
the driver gives up after thirty seconds and says why instead of waiting for
ever. Entries left by older builds are harmless except at their own paths;
mount somewhere else.

For a dead mount left by an older build, terminate that mount's helper by
hand; the program stuck on it is released at once and no reboot is needed:

```sh
ps -axo pid=,comm= | grep '/io.macfuse.app.fsmodule.macfuse$'   # one per mount
kill -TERM <pid of the dead mount's helper>
```

Match the helper on its executable path, as above, and never with
`pgrep -f`, which searches whole command lines and also matches any shell
that happens to mention the name.

## Simulating a filesystem that stops answering

Stop the process that serves the volume instead of killing it. With the
supervisor in place that is the child of `afsplus-mount`, not the process
you started:

```sh
serving=$(pgrep -P "$afsplus_mount_pid")
kill -STOP "$serving"   # every request to the volume now blocks
# ... observe what the client does while the filesystem is stalled ...
kill -CONT "$serving"   # the filesystem answers again
```

Stopping the supervisor itself stalls nothing: it is not on the request path.

While the process is stopped, the volume behaves exactly as it would during a
stall or a hang: every access blocks. Continuing it lets everything resume, so
the scenario can be repeated as often as needed without a reboot.

Killing the serving process while a client waits is safe now, because the
supervisor releases the mount, and
[tools/check-mount-driver-death.py](../tools/check-mount-driver-death.py) does
it on purpose. Two things can still leave a dead mountpoint: killing the
supervisor itself with `SIGKILL`, which no process can intercept, and killing
the driver of a build older than the supervisor. Start `umount` on the
canonical path first, then terminate the helper as shown above, so that the
unmount completes and macOS forgets the mount.

## One request at a time

On macOS, fuser serves a mount from a single thread, so a request that takes
long makes every other program using the volume wait for it. The work that
used to make requests long is the maintenance a delete leaves behind:
cleaning the orphan and returning its blocks to the free pool, which grows
with how fragmented the file was.

The driver therefore turns inline maintenance off in the portable layer and
runs it on a thread of its own, one transaction at a time, taking the
volume's lock only between requests. A request now waits for at most one
transaction.
[crates/afsplus-vfs/tests/background_maintenance.rs](../crates/afsplus-vfs/tests/background_maintenance.rs)
checks, below the mount, that with inline maintenance off an unlink does none
of the work and the steps drain all of it.
[tools/check-mount-responsiveness.py](../tools/check-mount-responsiveness.py)
checks the mounted result: another program's worst wait while a fragmented
file is deleted, and that the space still comes back.

A request that blocks inside the driver itself still stops the whole volume;
only more serving threads would change that, and fuser offers them on Linux
only.

## Hours of use by several programs

The checks above last seconds each. Leaks, drift and contention between
programs show only over time, so
[tools/check-mount-endurance.py](../tools/check-mount-endurance.py) keeps
three programs at work on one volume for half an hour: one writes, rereads,
renames and deletes files of 5 to 40 MiB, one builds and reshuffles folders of
small files, one lists and reads everything the way a file browser does. Every
file's bytes follow from its name, so every read is checked. It requires no
error in any program, a worst wait under a second for one listing, stat or
small read, the space of everything deleted back, and a clean image.

Its first runs found three faults that no short check could.

- Maintenance was starved. It cannot run beside an open data window, and a
  driver that makes every write durable keeps one open whenever anybody
  writes, so orphan cleanup was refused for as long as writing went on.
  Maintenance publishes the window first when it has work, and the
  maintenance thread queues for the lock instead of waiting for a free
  moment.
- Deleted space ran out before it came back. Free space swung between four
  hundred and forty megabytes, and a write in the trough failed for want of
  space that was only waiting to be reclaimed, which also lost the data
  window. An operation that needs space reclaims toward a low-water mark
  first, a few transactions at a time, with room kept for publishing the
  window; a write that still finds no space reclaims everything and tries
  once more.
  [crates/afsplus-vfs/tests/space_under_load.rs](../crates/afsplus-vfs/tests/space_under_load.rs)
  holds this below the mount.
- One request carried up to sixteen megabytes, committed before it was
  answered, and every other program waited behind it. Requests carry at
  most a megabyte.

Listings were the last to wait: FSKit looked up every name a listing returned,
one request each behind the writes in flight, until listings carried inode
numbers again (see below).

## What a killed driver keeps

A program that calls `fsync` and gets success is owed its data. On macOS the
FSKit backend does not pass `fsync` on to the driver, but it does not return
before the kernel has handed the driver every dirty byte of the file either,
and the driver makes each write durable before it answers it. A killed
driver therefore keeps everything whose `fsync` returned.

Two tests hold it. Below the mount,
[crates/afsplus-fuse/tests/process_death.rs](../crates/afsplus-fuse/tests/process_death.rs)
records every block the adapter writes and checks the image a kill would
leave after every prefix of those writes: the checker must find it clean and
every file whose durability was acknowledged must read back, with a
program's `fsync` and in the macOS driver's configuration. With durability
never asked for, the same harness must find a loss. On the mount,
[tools/check-mount-kill-durability.py](../tools/check-mount-kill-durability.py)
kills the driver with `SIGKILL` at a different instant in each of eight
rounds while a program writes, fsyncs and pauses, then requires the checker
to pass and every file reported after its `fsync` to read back byte for byte
on the next mount. A driver that commits nothing before unmounting fails it.

macFUSE's relay does not notice that the process serving a volume has died:
`umount` and every access then wait for ever (reported upstream as
[macfuse/macfuse#1201](https://github.com/macfuse/macfuse/issues/1201)). The
supervisor therefore terminates the relay when the unmount does not finish,
and gives it thirty seconds to end on its own before killing it: a relay that
ends normally has macOS forget the mount, while one killed first can leave the
path recorded, and macOS then refuses the next mount there until its file
system daemon restarts. macOS also forgets most records some time after the
relay has gone. The kill test mounts at a new path every time and reports how
many records are still there half a minute after each kill.

A panic in the driver unwinds: the request that met it is answered with an
error, the channel closes, and the volume is unmounted with nothing recorded.

## macFUSE releases

macFUSE 5.3 delivered every write of one to fourteen bytes to the driver as
zeros (macFUSE issue 1188). The file looked right while the volume was
mounted, because the kernel served it from its own cache, and held zeros once
the volume came back. macFUSE 5.4.0 fixes it and is required; the usage
battery rereads twenty small files after the remount to hold it. The same
release turns an empty reply to a read into a read that reports every byte and
fills none (issue 1196), so the driver answers such a read, which only a race
with a truncation can produce, with an I/O error.

## What macFUSE's FSKit backend does to a listing and to `df`

Two things a person sees on the mounted volume come from the relay rather
than from the volume, and each was first taken for a driver fault.

**A listing must carry each entry's inode number.** macFUSE 5.3's relay
handed every entry that carried one to the kernel twice, and the driver sent
the unknown inode number instead, as macFUSE's own libfuse does. That made the
relay look up every name of every listing, one request each, and under load
each of those waited behind the durable writes in flight: a folder of a few
hundred files took seconds to list, long enough for entries to go missing
from what a program saw. macFUSE 5.4 lists each entry once, and a listing
with inode numbers needs no lookups at all, so the driver sends them. The
battery checks that a program reading a folder sees each name once.

**`df` shows nothing used.** macOS `df` takes its Used column from the volume
attribute `ATTR_VOL_SPACEUSED`, not from `statfs`. The relay reports the
total, free and available space the driver gives it, but leaves the used
space at zero for every filesystem it serves: a minimal libfuse filesystem
that reports half its blocks free shows the same zero. The battery measures
used space from `statfs`, which is the figure the driver answers for.

**Whether names fold case is declared at mount.** The kernel learns it from a
flag in the reply to its first request and passes it on to programs through
`pathconf(_PC_CASE_SENSITIVE)`. fuser set that flag for every macOS
filesystem, so a case-sensitive volume, which keeps `Name.txt` and
`NAME.TXT` apart, reported the opposite. The driver sets it only for a
case-insensitive volume;
[tools/check-mount-name-policy.py](../tools/check-mount-name-policy.py)
mounts one volume of each policy and requires the answer and the behaviour to
agree.

## What belongs below the mount

Most behaviour reproduces at the portable interface, with no kernel in the
way, and runs in a fraction of a second. Recovery from a full volume
(`crates/afsplus-vfs/tests/full_volume.rs`) and space returned by a delete
(`crates/afsplus-vfs/tests/no_leak.rs`) are examples. Put a behaviour there
whenever it can be reached there.

The mount is for what only the transport shows: which owner and group the host
asks for, the attributes the kernel sends after a create, and directory
listings as the kernel requests them. Those cannot be reached below the mount,
and they are where the mounted battery earns its place.
