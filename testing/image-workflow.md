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

## Copying files in and out of a stopped image

An image a VM uses as a disk, such as the Wine prefix of the AROS lab, is
edited on the Mac while the VM is stopped, the way `mtools` edits a FAT
image: mount it, copy with `cp`, `rsync` or the Finder, unmount.
[tools/afsplus-image-mount.sh](../tools/afsplus-image-mount.sh) wraps
`afsplus-mount` for this and builds it on first use.

| Step | Command |
|---|---|
| mount read-write | `tools/afsplus-image-mount.sh mount Wine.afs ~/mnt/wine` |
| mount read-only | `tools/afsplus-image-mount.sh mount --read-only Wine.afs ~/mnt/wine` |
| copy | `rsync -a ~/payload/ ~/mnt/wine/drive_c/` or `cp`, or `open ~/mnt/wine` for the Finder |
| unmount | `tools/afsplus-image-mount.sh unmount ~/mnt/wine` |

`mount` refuses an image that any process holds open, a mountpoint under
`/Volumes` and a non-empty mountpoint, runs `afsplus-check` first and
refuses an image that is not clean, then starts `afsplus-mount` in the
background and returns once the volume is in the mount table. The volume
stays mounted after the terminal closes. `unmount` unmounts, waits for the
driver to commit its last state and exit, and runs `afsplus-check` again;
it refuses while a program still uses the volume, and `unmount --force`
unmounts anyway at the cost of I/O errors in those programs.

Prerequisites on macOS: macFUSE 5.4.0 or later with its FSKit module enabled
([macos-fskit-activation](../docs/macos-fskit-activation.md)).
`afsplus-mount` refuses an older macFUSE: 5.3.x stores writes of one to
fourteen bytes as zeros and lists entries several times
([mounted-volume-testing](mounted-volume-testing.md#macfuse-releases)).
Installing or upgrading macFUSE asks for an administrator password, and a
first install also asks for the module to be enabled in System Settings.

Never mount an image a running VM has open, read-only included. QEMU and
the driver would each write blocks the other believes it owns, and a
read-only mount of an image being written reads a torn state. The lock
check catches a VM that is running; it cannot know about a VM started while
the volume is mounted, so start the VM only after `unmount` has reported the
image clean. Never stop the driver with `kill -9`: unmount it.

A case-insensitive volume (`mkafsplus --case-insensitive`, every AROS
volume) behaves like APFS: `Name.txt` and `NAME.TXT` are one file, the
spelling first written is kept, and a rename that changes only case renames
it. `pathconf(_PC_CASE_SENSITIVE)` answers 0.

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
