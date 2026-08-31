# ADR-040: Testable FUSE protocol boundary and host mount entry point

Status: Accepted for Mountable Alpha-0

## Context

The portable VFS slice from ADR-039 must be exercised through real host
filesystem operations, but tying all FUSE semantics to callback reply objects
would make most behavior untestable on machines without a compatible kernel
driver. The development Mac initially had Fuse-T rather than macFUSE; `fuser`
can compile its callback contract there, but its built-in macOS mount path
expects the macFUSE libfuse-2 compatibility package.

## Decision

The `afsplus-fuse` crate has two layers:

- `FuseAdapter` owns protocol semantics over `afsplus-vfs` without depending
  on a kernel driver;
- the optional `fuser-adapter` module translates fuser callbacks, newtypes,
  replies, open flags and errno values around that adapter.

AFS+ object IDs map directly to FUSE inode numbers. Object 1 is already the
root, matching the FUSE root inode convention. VFS handles map directly to
FUSE file handles. Names must be valid UTF-8; invalid host byte sequences fail
with `EINVAL` rather than being changed on disk.

Directory offset 0 starts a stream. Offsets 1 and 2 resume after synthesized
`.` and `..`; real entries use the VFS ordinal plus 3. Every returned entry
carries its own next offset, so a byte-limited FUSE reply can stop anywhere
and resume without duplicates. Parent IDs used for `..` are session metadata
learned from lookup, creation, rename and enumeration; they are not persisted
as a second namespace structure.

The adapter implements lookup/getattr/set-size, regular-file create/open/read/
write/release, mkdir/unlink/rmdir, rename and no-replace, hard links, directory
streams, fsync/fsyncdir and statfs. Unsupported POSIX metadata mutations and
special nodes return explicit errors. Writes are immediate durable AFS+
transactions; FUSE `flush` is therefore a no-op, while `fsync` reaches the VFS
sync contract.

`afsplus-mount` opens an image, widens sparse host-file geometry from the AFS+
identification block, applies the selected mount mode and enters the fuser
session. Linux uses fuser's native mount path. macOS ordinary builds retain
fuser's test-only no-mount mode. The `macfuse-mount` feature dynamically loads
macFUSE's `MFMount.framework` in a tiny audited `-sys` crate. A narrow vendored
fuser patch supplies `SessionTransport` and `Session::from_transport`, allowing
complete FUSE messages to cross the FSKit channel without pretending it is a
`/dev/fuse` byte-stream descriptor. The ordinary device-backed fuser paths are
unchanged. Dynamic loading keeps build and test machines independent of a
system installation.
On macOS 15.4 and newer, the CLI selects macFUSE's user-space FSKit backend
explicitly. FSKit requires mountpoints below `/Volumes`; macFUSE can create a
missing direct child there, so the CLI permits exactly that missing-path case
and derives presentation ownership from the image file. Existing mountpoints
retain their own ownership. This avoids the Apple Silicon kernel-extension and
reduced-security path while keeping the native boundary unchanged. FSKit sends
mount-time control requests as root, so the session admits root and the mounting
user while `default_permissions` enforces the attributes exposed by AFS+.
macOS also repeats mode, owner and empty BSD flags immediately after create and
mkdir; exact no-op repetitions are acknowledged, while real unsupported POSIX
metadata changes still fail explicitly.

Some macOS builds display inert File System Extensions switches. The reversible
workaround and its version-specific caveat are documented in
[`docs/macos-fskit-activation.md`](../docs/macos-fskit-activation.md); it edits only FSKit's per-user enabled-module
list, retains a backup, and never enables the legacy kernel extension.

A direct Fuse-T experiment reached INIT, STATFS and GETATTR, but Fuse-T then
closed the session. This follows from its architecture: Fuse-T translates
through an NFS transport and its descriptor lifecycle is driven by its own
libfuse loop; it is not a drop-in raw macFUSE/kernel protocol channel for a
separate fuser event loop. A Fuse-T backend would therefore need a dedicated
adapter rather than a symbol-name alias and is outside this Alpha-0 path.

## Consequences

The full operation slice, sparse reads, replacement, remount/checker behavior,
directory pagination and failure modes run in CI without mounting. Kernel
integration is a thin, independently compilable layer. The ignored
`host_mount` qualification has passed on macOS through macFUSE 5.3.3's FSKit
backend: it exercises create/read/write with a sparse gap/truncate/rename with
replacement/hard-link/fsync, unmounts, and requires a clean checker result.
It remains an explicit, opt-in Alpha-0 gate on machines with macFUSE installed.
