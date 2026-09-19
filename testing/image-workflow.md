# The image workflow: create, fork, mount, diff, replay

> **ADRs:** none · **Spec:** none ·
> **Tests:** [tools/check-image-workflow.sh](../tools/check-image-workflow.sh) ·
> **Milestones:** M08

How a developer works with AFS+ volumes as files on the host, and the gate
that proves each step does what it says. An AFS+ image is a plain sparse
file; nothing in the tools rewrites its identification block, so a copy of
the file is a complete, independent volume.

| Step | Command | What it gives you |
|---|---|---|
| create | `mkafsplus --size-mib 256 --label Work base.afsp` | a 256 MiB volume that costs about 80 KiB on disk until written |
| fork | `cp -c base.afsp fork.afsp` (macOS, APFS clone); `cp --reflink=auto` on Linux | a second volume, instantly, sharing blocks with the first until either writes; the base stays untouched |
| mount | `afsplus-mount fork.afsp mnt` (a directory of your own, never under `/Volumes`) | the fork as a folder; `umount mnt` when done |
| check | `afsplus-check fork.afsp --json` | `clean: true` or what is wrong |
| diff | `afsplus-image-diff base.afsp fork.afsp --json` | every object created, removed or modified, every link and rename, and the changed byte ranges |
| replay | the same session on another fork, then `afsplus-image-diff fork1.afsp fork2.afsp` | the same names, links, sizes and bytes; timestamps and commit counters differ |

Mounting follows [mounted-volume-testing](mounted-volume-testing.md): a
mountpoint outside `/Volumes`, `umount` and never `diskutil`, and a mount
table read that cannot block.

## What the gate checks

`tools/check-image-workflow.sh` (`make image-workflow`) runs the six steps
on a fresh image, ten checks: the host file is sparse; the fork is byte for
byte the base and the base is untouched after a session on the fork; the
checker is clean; the diff lists exactly the drawer, the note and the 3 MiB
file the session made; the replayed session gives the same content.
Without macFUSE the mount, diff and replay steps report *not run*.

Two things it established, kept as facts rather than surprises: how many
commits a session takes is not deterministic (the commit timer and idle-time
cleanup decide), so `content_generation` and the volume's generation differ
between two replays of one session; and a deferred delete may or may not
have been cleaned at unmount, so `free_blocks` can differ by the blocks of
that file until the next mount's idle time.

## Open

Replaying a recorded scenario bundle
([developer-harness](developer-harness.md)) onto a file image: the scenario
runner formats its own in-memory volume, so a bundle cannot be applied to
a fork. Replay here is a rerun of the session's commands.
