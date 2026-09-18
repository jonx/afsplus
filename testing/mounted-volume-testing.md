# Testing a mounted volume

> **ADRs:** none · **Spec:** none ·
> **Tests:** [tools/check-mounted-usage.sh](../tools/check-mounted-usage.sh),
> [tools/check-mount-driver-death.py](../tools/check-mount-driver-death.py) · **Milestones:** M08

This plan covers what a person can do with an AFS+ volume mounted through
macFUSE: create, copy, rename, link, archive, and unmount with everything
still there afterwards. Its gate is
[tools/check-mounted-usage.sh](../tools/check-mounted-usage.sh), which makes
its own image, mounts it, uses it with ordinary tools and reports each result
in the words a person would use.

It also covers how to test a mounted driver without taking the machine down
with it, which is the harder half.

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
supervisor terminates the helper of that mount, which releases everything
waiting on it with an I/O error, and removes the entry by its canonical path.
Stopping the supervisor with Ctrl-C, `kill` or a closing terminal unmounts
cleanly instead of killing the driver under its clients.

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
the driver of a build older than the supervisor. The helper can then be
terminated by hand as shown above.

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
