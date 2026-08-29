# Design References

These are design inspirations, not sources to copy blindly.

## PFS3 All-In-One source

https://github.com/tonioni/pfs3aio

Primary Amiga-native reference. Study:

- atomic copy-on-write metadata update
- root-last commit point
- deferred freeing of old metadata
- anodes/extents
- allocation bitmap and roving allocation
- cache/LRU behavior
- postponed operations
- deldir
- low-memory implementation

The source is distributed under a BSD-4-Clause license. AFS+ remains an independently specified new format.

## Current pfs3aio reliability work

The repository received substantial corruption-hardening and test-rig work in August 2026. Particularly useful lessons include:

- never free old COW metadata before commit
- prefer a conservative block leak to early reuse
- pin cache objects across nested operations that may evict
- add destination before removing source during fallible cross-directory rename
- treat unreadable allocation metadata as allocated/unknown, not free
- deterministic fault and black-box tests are essential

These changes are useful failure-case references even where AFS+ uses a different data structure.

## Original PFS3 5.3 release

https://aminet.net/package/disk/misc/PFS3_53

Historical documentation and original author provenance.

## PFS4 design comments by Michiel Pelt

https://forum.amiga.org/index.php?topic=52358.180

PFS4 existed only on paper. Public comments describe planned B+ tree directories, redesigned atomic commit, automatic grouping of small files, and improved fragmentation prevention/automatic defragmentation.

Treat these comments as historical design direction, not a normative specification.

## libpfs3 (Rust)

https://docs.rs/libpfs3/latest/libpfs3/

A 2026 pure-Rust PFS3 library providing read, write, format, check, RDB parsing, deldir, and an OS-independent block-device abstraction.

Useful as an additional reference for portable filesystem-core architecture and for understanding PFS3 independently of the historical Amiga handler implementation.

## OpenZFS feature flags

https://openzfs.github.io/openzfs-docs/Basic%20Concepts/Pool%20Structure/Feature%20Flags.html

Useful ideas:

- independent feature flags
- read-only compatibility
- compatibility feature sets
- explicit feature dependencies/states

## ext4 superblock and feature flags

https://www.kernel.org/doc/html/latest/filesystems/ext4/super.html

Useful ideas:

- COMPAT / RO_COMPAT / INCOMPAT distinction
- checksummed superblock
- explicit feature discovery
- redundant superblock strategy

## ext4 journal

https://www.kernel.org/doc/html/latest/filesystems/ext4/journal.html

Useful as the conventional redo-journal comparison point for AFS+ transaction experiments.

## NTFS USN change journal

https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/fsutil-usn

Useful idea:

- bounded persistent volume change stream for indexers and backup tools

## NTFS MFT

https://learn.microsoft.com/en-us/windows/win32/devnotes/master-file-table

Useful idea:

- compact volume-wide metadata access for fast enumeration
- AFS+ intentionally keeps its catalog derived rather than authoritative

## F2FS

https://www.kernel.org/doc/html/latest/filesystems/f2fs.html

Useful ideas:

- treat write amplification as a first-class design constraint
- optimize for flash behavior without making every feature mandatory

## bcachefs principles

https://bcachefs.org/bcachefs-principles-of-operation.pdf

Useful ideas:

- explicit degraded/recovery concepts
- integrity-first behavior
- avoiding divergent filesystem/checker logic

## AROS

https://github.com/aros-development-team/AROS

Relevant existing concepts:

- filesystem handlers
- DOS API
- modular filesystem support
- AFS implementation
- native boot filesystem package
