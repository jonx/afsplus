# 01. Goals and Non-goals

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

## Goals

### G1. Native AROS semantics

AFS+ must represent AROS concepts without sidecar files:

- protection bits
- file comments
- volume labels
- links
- file identity
- timestamps
- extensible attributes

### G2. 64-bit storage

All format-level quantities that can reasonably grow must use 64-bit representation:

- logical block numbers
- object IDs
- file offsets
- file sizes
- volume block counts
- allocation generation numbers
- transaction/checkpoint generation numbers
- change-stream sequence numbers

### G3. Modern developer workloads

The format and API must support assumptions made by current software:

- atomic rename
- robust canonicalization
- symlinks
- hard links
- stable file identity
- 64-bit stat information
- Unicode
- file watching
- durable flush semantics
- large directories
- millions of files
- efficient recursive enumeration
- efficient incremental change discovery

### G4. Lightweight implementations

The format and core algorithms must permit an implementation to operate with
bounded memory. No core structure may *require* loading metadata proportional
to total volume size or file count.

This is a portability capability, not a performance ceiling for every host.
A classic or `reader-minimal` implementation may stream pages through a tiny
cache, while Macaros Native and other modern systems may cache allocation and
metadata state aggressively, prefetch, and parallelize. Both strategies must
produce the same on-disk results and preserve the same correctness rules.

### G5. Portable independent implementations

A developer must be able to implement a conforming reader using only the published specification.

### G6. Safe evolution and retirement

Future format additions must not require a new global filesystem version whenever possible.

Experimental features must also be able to become deprecated or retired without reusing their identifiers or making existing volumes silently unreadable. Active incompatible state may be removed only through an explicit conversion/migration that leaves no remaining dependency on the retired feature.

### G7. Repairability

A corrupted optional accelerator must never make user data unreachable when the authoritative structures are intact.

## Non-goals

### N1. Binary compatibility with classic FFS

AFS+ is a new format.

### N2. Full POSIX emulation in the disk format

POSIX compatibility belongs in the OS layer.

### N3. Replacing exFAT for interchange

AFS+ and exFAT serve different purposes.

### N4. Mandatory support for every classic Amiga configuration

The format should permit small implementations, but modern AROS is not limited to 1980s hardware constraints.

### N5. ZFS/Btrfs feature parity

AFS+ 1.0 does not require:

- general user-visible snapshot management/history
- data deduplication
- filesystem RAID
- send/receive
- transparent data compression
- transparent data encryption
- mandatory full data checksumming

Shared extents/reflinks are no longer listed as a non-goal because the epoch-1 extent architecture is intended to support them. Optional data checksums have a reserved extension path but are not required for the first production profile.

Retaining one or more previous checkpoints, or temporarily retaining an exact object/content generation for correctness, recovery, testing, or a privileged scan handle, does **not** by itself commit AFS+ to a general snapshot product/API. The storage/versioning cost of those guarantees must still be proven by the prototype.

### N6. Application-specific filesystem behavior

Zed, Ferail, Cargo, Git, or any other application must not receive filesystem-specific private hacks. They use stable system APIs.
