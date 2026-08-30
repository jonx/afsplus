# ADR-039: Portable handle API and generation-bound directory cookies

Status: Accepted for Mountable Alpha-0

## Context

The core API originally returned whole files and whole directories in `Vec`s.
That was useful for prototype tests but unsuitable for FUSE, MacAROS DOS
packets, large files, or constrained machines. Host adapters also need one
stable error/capability/handle contract rather than separate wrappers that
quietly acquire different semantics.

## Decision

`afsplus-core` exposes two bounded primitives:

- `read_file_at(object, offset, destination)` reads into caller-owned memory
  and returns short at EOF; holes and unwritten extents read as zero;
- `read_directory_page(object, cursor, limit)` uses AFST subtree item counts
  to skip branches and returns work proportional to tree height plus the
  returned leaves.

A directory cursor is `(checkpoint_generation, ordinal)`. Any checkpoint
change makes it explicitly `Stale`; adapters restart enumeration instead of
silently combining entries from two namespace views. Page size is capped at
4096 entries. The traversal retains only a root-to-leaf path and the requested
result page.

The new `afsplus-vfs` crate owns portable handles and maps the core to:

- lookup/stat/statfs and capability discovery;
- file and directory open/close;
- 64-bit offset read/write/truncate;
- paged directory reads with opaque ordinal cookies;
- create/mkdir/unlink/rmdir/rename/atomic replace/hard link;
- file/directory fsync and filesystem sync;
- stable, filesystem-neutral error categories.

Immediate core mutations are durable checkpoint transactions. `fsync` calls
the explicit core sync hook; read-only modes succeed without issuing a device
flush, preserving `NO_CHANGES`. Symlinks, xattrs, comments, protection
mutation, orphaned-open-file semantics, and concurrent writers remain
unadvertised until implemented and tested.

## Consequences

FUSE and MacAROS adapters can now be thin protocol translators over the same
executable API. The generation-bound cookie is conservative—an unrelated
commit invalidates an iterator—but correct. A later snapshot/read-view layer
may extend iterator lifetime without changing the current stale-detection
contract.
