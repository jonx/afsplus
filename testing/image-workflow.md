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
| mount to populate | `tools/afsplus-image-mount.sh mount --commit=delayed Wine.afs ~/mnt/wine` |
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

### Populating an image with many files

By default the mount makes every operation durable before it answers it, and
every data write too, because macFUSE does not pass a program's `fsync(2)` or
`F_FULLFSYNC` to the driver (checked on macFUSE 5.5.0: only the file's dirty
pages arrive, as `WRITE` requests, and no `FSYNC`). A copy pays for that per
file: `rsync -a` sends about ten changing requests per file (create, owner,
`com.apple.provenance`, data, size, rename, times, mode) and `cp -R` about
eight, which cost 8 to 9 commits and 19 device flushes per file.

`mount --commit=delayed` (or `--commit=SECONDS`, 1 to 60) mounts the image
with [ADR-121](../adr/ADR-121-delayed-group-commit.md)'s delayed commit, as
the AROS handler runs: operations, metadata edits included, gather in one
window committed after a second of idleness, at the maximum age (5 seconds
for `delayed`), every 512 operations and at unmount
([ADR-137](../adr/ADR-137-host-mount-delayed-commit-option.md)). On macOS a
program's fsync is not a durability point under it; the unmount is. A crash
or a killed driver loses the operations not yet committed, whole and in
order, and leaves the image clean: a file `rsync` was copying is absent or
complete under its final name, possibly with its temporary name left beside
it; a file `cp` was copying can be present and shorter than its source, as
after any interrupted copy. Run the copy again after such a crash.

Measured on the M5 (macOS, macFUSE 5.5.0), files of 20 KB or empty, copied
from APFS into a fresh 2 GiB image:

| Workload | sync (default) | `--commit=delayed` |
|---|---|---|
| 500 × 20 KB, `rsync -a` | 11 files/s, 8.2 commits per file | 134 files/s, 0.03 commits per file |
| 500 × 20 KB, `cp -R` | 10 files/s, 9.3 commits per file | 113 files/s, 0.04 commits per file |
| 5,000 × 20 KB, `rsync -a` | 12 files/s | not measured |
| 5,000 × 20 KB, `cp -R` | 10 files/s | not measured |
| 5,000 empty, `rsync -a` | 20 files/s | not measured |
| 5,000 empty, `cp -R` | 14 files/s | not measured |

With delayed commit the per-file cost is the driver's request round trips
through macFUSE, not the disk.
[crates/afsplus-fuse/tests/delayed_population.rs](../crates/afsplus-fuse/tests/delayed_population.rs)
replays both copy patterns through the adapter, cuts the image after every
block write, and requires a clean checker and exactly the state after some
earlier request, never less than the last sync;
[crates/afsplus-vfs/tests/delayed_metadata.rs](../crates/afsplus-vfs/tests/delayed_metadata.rs)
counts the flushes: 12 for 200 files with their mode, owner, times, comment
and an attribute.

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
