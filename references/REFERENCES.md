# Design References

These are design inspirations, not sources to copy blindly.

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

Useful ideas:
- checksummed journal records
- explicit feature negotiation within the journal

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
- explicit no-change/degraded recovery concepts
- avoiding divergent fsck logic

## AROS

https://github.com/aros-development-team/AROS

Relevant existing concepts:
- filesystem handlers
- DOS API
- modular filesystem support
- AFS implementation
- native boot filesystem package

## PFS3 All-In-One

https://github.com/tonioni/pfs3aio

Primary Amiga-native reference. Study:
- atomic update behavior
- anodes
- allocation
- directory handling
- low-memory design
- fragmentation behavior
- recovery and maintenance code

The source is available under a BSD-4-Clause license.

## Original PFS3 5.3 release

https://aminet.net/package/disk/misc/PFS3_53

Useful for historical documentation and original author provenance.

## PFS4 design comments by Michiel Pelt

https://forum.amiga.org/index.php?topic=52358.180

Historical design reference. Public comments by PFS author Michiel Pelt describe planned PFS4 ideas including B+ tree directories, redesigned atomic commit, grouping small files, and better fragmentation management.

Treat the forum as historical design evidence, not as a normative specification.
