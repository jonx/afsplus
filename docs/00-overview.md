# 00. Overview

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

## 1. Scope

AFS+ is a new on-disk filesystem designed primarily for modern AROS systems while remaining practical to implement on other Amiga-family operating systems and general-purpose operating systems.

It is not a binary-compatible extension of classic AFS/FFS. Existing AFS/FFS volumes remain supported by their existing handlers.

It is not an extension of exFAT. exFAT remains the preferred interchange filesystem when transparent compatibility with macOS, Windows, Linux, and other systems is more important than AROS-native semantics.

## 2. System architecture

```text
                Applications
                     |
          +----------+----------+
          |                     |
     Classic DOS API       Modern FS API
          |                     |
          +----------+----------+
                     |
             Filesystem API v2
                     |
      +--------------+---------------+
      |              |               |
     AFS+           exFAT          legacy adapter
      |                              |
  libafsplus                      FFS/SFS/etc.
      |
  block device API
```

AFS+ has no knowledge of `SYS:`, `Work:`, `PROGDIR:`, POSIX mount points, or Windows drive letters. Those are namespace-layer concepts.

## 3. Design priorities

In order:

1. correctness after power loss or system crash
2. recoverability and diagnosability
3. simple, documented on-disk invariants
4. compatibility and evolvability
5. bounded-memory implementation
6. good metadata and source-tree performance
7. large-file and large-volume scalability
8. portability
9. optional advanced features

AFS+ does not prioritize benchmark wins that weaken recovery semantics or make the format difficult to implement independently.

## 4. Target workloads

AFS+ should perform well for:

- AROS system volumes
- software-development trees
- Rust/C/C++ build trees
- Git repositories
- code editors such as Zed
- millions of small files
- package caches
- desktop user data
- large media files
- external SSDs
- internal NVMe storage
- indexing tools such as Ferail
- backup and synchronization tools

## 5. Compatibility philosophy

Old software compatibility is provided above the filesystem through `dos.library`.

Old operating-system support is enabled by keeping the core disk format small, explicit, and memory-bounded.

A classic implementation is not required to implement every optional AFS+ feature. Unsupported features are negotiated through feature flags and compatibility profiles.

## 6. Canonical components

The project should ultimately contain:

```text
libafsplus-reader   minimal read-only parser
libafsplus          full portable core
afsplus.handler     native AROS handler
mkafsplus           formatter
afsplus-info        safe inspector
afsplus-check       verifier/repair tool
afsplus-resize      filesystem resize tool
afsplus-dump        forensic metadata dump
afsplus-catalog     catalog inspection/rebuild
afsplus-fuse        macOS/Linux FUSE front-end
```

The tools must not independently reinvent on-disk parsing.
