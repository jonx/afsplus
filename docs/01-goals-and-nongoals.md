# 01. Goals and Non-goals

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
- journal transaction numbers
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

An implementation must be able to operate with bounded memory. No core structure may require loading metadata proportional to total volume size or file count.

### G5. Portable independent implementations

A developer must be able to implement a conforming reader using only the published specification.

### G6. Safe evolution

Future format additions must not require a new global filesystem version whenever possible.

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

- snapshots
- data deduplication
- filesystem RAID
- send/receive
- transparent data compression
- transparent data encryption
- reflinks
- full data checksumming

The architecture must permit carefully specified future extensions.

### N6. Application-specific filesystem behavior

Zed, Ferail, Cargo, Git, or any other application must not receive filesystem-specific private hacks. They use stable system APIs.
