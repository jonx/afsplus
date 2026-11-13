# ADR-137: A host mount may delay commits when its user asks

Status: Accepted (2026-11-13)
Amends: ADR-121

## Context

[ADR-121](ADR-121-delayed-group-commit.md) decision 7 keeps the macOS FUSE
mount `SYNC`, because macFUSE does not forward `fsync(2)` to the driver, and
lets it offer `DELAYED` once fsync reaches the driver. macFUSE 5.5.0's FSKit
backend still does not: a program's `fsync` and `F_FULLFSYNC` return with only
the file's dirty pages delivered as `WRITE` requests, and no `FSYNC` request.
The mount therefore makes every operation durable before answering it, and
every data write as well.

Populating a stopped image from the Mac, a Wine prefix of tens of thousands
of files for instance, paid for that per file. `rsync -a` sends about ten
mutating requests per file (create, owner, the provenance attribute, data,
size, rename, times, mode), `cp -R` about eight, and each committed on its
own: 8 to 9 checkpoints and 19 device flushes per file, 10 to 12 files per
second on the M5 whatever the file size. Most of those requests were metadata
edits, which the window of decision 8 did not stage yet, so even a `DELAYED`
mount committed them one by one.

For that use the durability point a person relies on is the unmount: the
image is checked after it, and a population interrupted by a crash is run
again.

## Decision

1. Metadata edits join the window as decision 8 foresaw: protection, owner,
   modification time, comment and extended attributes of files and
   directories. A symlink and the root still commit the window and then
   themselves. The intent log has no record for these edits, so a window
   holding one is committed, never logged, by the next fsync (decision 5).
   This is not a format change.
2. `afsplus-mount` takes `--commit=sync|delayed|<seconds>`, the counterpart of
   the AROS handler's `COMMIT=` control string, with the same range of 1 to
   60 seconds and the same 5-second default for `delayed`.
   `tools/afsplus-image-mount.sh mount` passes it through.
3. The default stays `sync` on every host. Decision 7 is unchanged for the
   default; `delayed` on macOS is an explicit request by the person mounting,
   and the usage text and the image workflow say what it gives up: fsync is
   not a durability point there, the commit timer and the unmount are.
4. A delayed mount keeps every guarantee of decision 3: a crash or a killed
   driver loses the uncommitted operations, whole and in order, and the
   volume is clean.

## Consequences

- Populating an image through a delayed mount costs the commits of the window
  bound and the timer instead of several per file. The measurements are in
  [testing/image-workflow.md](../testing/image-workflow.md#populating-an-image-with-many-files).
- The AROS handler, which defaults to `DELAYED`, gains the same staging for
  `SetProtection` and `SetComment`. Its `SetDate` goes through the exact
  metadata restore of `Vfs::set_modified`, which still commits on its own.
- On Linux, where FUSE forwards fsync, `delayed` keeps fsync a durability
  point; whether Linux should default to it is left open.
- `tools/afsplus-image-mount.sh` keeps `sync` as its default (owner's
  decision, 2026-11-13): it also serves small edits of an existing image,
  where a silent loss after a driver crash is the wrong surprise. Populating
  an image asks for `--commit=delayed`.
