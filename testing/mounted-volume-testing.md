# Testing a mounted volume

> **ADRs:** none · **Spec:** none ·
> **Tests:** [tools/check-mounted-usage.sh](../tools/check-mounted-usage.sh) · **Milestones:** M08

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
2. **Never call `diskutil` from a script.** It enumerates every volume before
   acting, so one dead mountpoint anywhere makes every call block, whatever
   volume it was asked about. `|| true` does not rescue a command that never
   returns. Use `umount`, then `umount -f`, and confirm the result through
   `mount`, not through an exit status.
3. **Probe a mountpoint with a bounded check.** `-d` and `-e` block on a dead
   mountpoint instead of returning false. Run the check in the background and
   treat no answer within a few seconds as a mountpoint that does not respond.

## Simulating a filesystem that stops answering

Stop the mount process instead of killing it:

```sh
kill -STOP "$mount_pid"   # every request to the volume now blocks
# ... observe what the client does while the filesystem is stalled ...
kill -CONT "$mount_pid"   # the filesystem answers again
```

While the process is stopped, the volume behaves exactly as it would during a
stall or a hang: every access blocks. Continuing it lets everything resume, so
the scenario can be repeated as often as needed without a reboot.

Do not reproduce a stall on a real mount by killing the driver while a client
is waiting on it, or by filling the volume until the driver refuses. Both can
leave a dead mountpoint behind.

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
