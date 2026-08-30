# ADR-040: Testable FUSE protocol boundary and host mount entry point

Status: Accepted for Mountable Alpha-0

## Context

The portable VFS slice from ADR-039 must be exercised through real host
filesystem operations, but tying all FUSE semantics to callback reply objects
would make most behavior untestable on machines without a compatible kernel
driver. The development Mac currently has Fuse-T rather than macFUSE; `fuser`
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
session. Linux can use fuser's native mount path. macOS builds use fuser's
test-only no-mount mode until either macFUSE is present or a separately
qualified Fuse-T bridge supplies the mounted FUSE descriptor.

## Consequences

The full operation slice, sparse reads, replacement, remount/checker behavior,
directory pagination and failure modes run in CI without mounting. Kernel
integration is a thin, independently compilable layer. A successful build on
macOS does not yet claim a successful host mount; that qualification remains
an explicit Alpha-0 gate.
