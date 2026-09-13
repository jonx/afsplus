# 24. Filesystem Comparison Matrix

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

This table compares architectural capabilities, not marketing claims. AFS+ entries marked Planned or Proposed are not implemented yet.

<!-- toc -->

- [1. Why compare](#1-why-compare)
- [2. High-level comparison](#2-high-level-comparison)
- [3. Where AFS+ should clearly outperform classic Amiga filesystems](#3-where-afs-should-clearly-outperform-classic-amiga-filesystems)
- [4. Where AFS+ can offer something uncommon even among modern filesystems](#4-where-afs-can-offer-something-uncommon-even-among-modern-filesystems)
  - [4.1 Global object stream plus persistent change stream](#41-global-object-stream-plus-persistent-change-stream)
  - [4.2 Low-memory and workstation profiles in the same format](#42-low-memory-and-workstation-profiles-in-the-same-format)
  - [4.3 Observability as a filesystem feature for developers](#43-observability-as-a-filesystem-feature-for-developers)
  - [4.4 Derived accelerators that never become correctness dependencies](#44-derived-accelerators-that-never-become-correctness-dependencies)
  - [4.5 Clone semantics as part of the developer contract](#45-clone-semantics-as-part-of-the-developer-contract)
- [5. Important features we should not chase merely to win a table](#5-important-features-we-should-not-chase-merely-to-win-a-table)
- [6. Reference facts](#6-reference-facts)
- [BFS and BeOS: historical comparison](#bfs-and-beos-historical-comparison)

<!-- /toc -->

## 1. Why compare

AFS+ should not exist merely because AROS lacks a modern native filesystem. A new on-disk format is expensive to design, implement, test, document, and maintain. It is justified only if the resulting system offers a useful combination that existing choices do not provide.

The goal is not to beat every filesystem at every workload. ZFS will remain stronger for multi-device storage pools and enterprise data integrity. APFS will remain more deeply integrated with Apple platforms. XFS will remain extraordinarily mature at large-scale Linux storage.

AFS+ should be unusually strong in the combination that matters to AROS:

- low and bounded resource use
- modern 64-bit semantics
- strong crash consistency
- excellent development-tree performance
- cheap same-volume clones/reflinks
- fast full-volume enumeration
- persistent incremental change discovery
- portable implementation
- classic Amiga-friendly semantics
- easy third-party implementation
- first-class debugging and repairability

## 2. High-level comparison

Legend:

- YES: mature/native feature
- PARTIAL: limited, optional, variant-dependent, or provided indirectly
- NO: not normally provided by the filesystem itself
- PLAN: planned AFS+ baseline
- PROP: proposed AFS+ extension, not frozen

| Capability | FFS/AFS | PFS3 | SFS/SFS2 | exFAT | NTFS | ext4 | XFS | Btrfs | OpenZFS | APFS | ReFS | AFS+ |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Native 64-bit file/offset contract | NO | PARTIAL / variant dependent | PARTIAL / SFS2 | YES | YES | YES | YES | YES | YES | YES | YES | PLAN |
| Extent-based file data | NO | YES, anode chains | YES-ish internal allocation | cluster chains/extents | YES | YES | YES | YES | YES | YES | YES | PLAN |
| Scalable indexed directories | legacy hash/list | limited | B-tree based design | linear directory sets | indexed | HTree | B+ trees | B-trees | ZAP/tree structures | B-trees | B+ trees | PLAN B+ tree |
| Metadata crash consistency | validation model | atomic COW/root commit | transactional/journal-style | NO | journal | journal | journal | COW | COW transaction groups | COW | COW/checkpoint style | PLAN COW checkpoints candidate |
| Metadata checksums | NO | NO | NO | NO | limited/internal, not end-to-end | YES | YES | YES | YES | internal integrity mechanisms | YES | PLAN |
| User-data checksums | NO | NO | NO | NO | NO | NO | NO | YES by default | YES | not exposed as general end-to-end contract | optional Integrity Streams | PROP optional |
| Snapshots | NO | NO | NO | NO | external VSS, not NTFS-native snapshots | NO | NO native snapshots | YES | YES | YES | YES | PROP |
| Reflink / block clone | NO | NO | NO | NO | NO general reflink | NO | YES | YES | YES clones | YES clones | YES | PLAN epoch-1 shared extents |
| Transparent compression | NO | NO | NO | NO | YES | fs-level compression not standard | NO general transparent compression | YES | YES | YES on modern APFS deployments/platform features | YES on current ReFS | PROP provider |
| Encryption in filesystem | NO | NO | NO | NO | EFS | fscrypt | fscrypt integration | external/per-file mechanisms depending stack | native dataset encryption | YES | YES in current ReFS/Windows stack | PROP, likely block/file policy layer |
| Hard links | YES | YES | YES | NO | YES | YES | YES | YES | YES | YES | YES | PLAN |
| Symlinks | YES/variant | YES | YES | NO | YES | YES | YES | YES | YES | YES | YES | PLAN |
| Extended attributes | limited Amiga metadata | Amiga metadata | Amiga metadata | NO native rich xattrs | YES | YES | YES | YES | YES | YES | YES | PLAN |
| Per-directory case policy | NO | NO | NO | NO | optional case-sensitive dirs on Windows | YES casefold dirs | NO common per-dir policy | filesystem policy variants | dataset policy | case behavior is platform-controlled | YES/Windows policy dependent | PLAN |
| Stable file/object ID | legacy lock/object concepts | anode identity | object nodes | synthesized | YES | inode+generation | inode+generation | object/inode IDs | object IDs | YES | YES | PLAN explicit 64-bit object ID |
| Persistent change stream | NO | NO | NO | NO | YES, USN | NO | NO general public journal | NO stable general API | no NTFS-like general change API | platform notification APIs, no public FS change journal | YES, USN-compatible ecosystem | PLAN optional change-stream |
| Fast global object enumeration | NO | NO | NO | directory walk | YES via MFT-oriented techniques | directory walk | directory/inode scan | tree scan | object traversal | private/internal | Windows metadata APIs | PLAN optional global catalog |
| Reverse physical->owner mapping | NO | NO | NO | NO | internal tooling | NO general | YES, rmap | internal trees | block birth/ownership metadata internally | private | internal | PROP rebuildable reverse-map |
| Online scrub | NO | limited tools | limited | NO | chkdsk mostly offline/online phases | e2scrub limited | YES | YES | YES | fsck largely system-managed | YES scrubber | PROP targeted scrub |
| Online repair | NO | tools/offline | limited | NO | limited | limited | YES, modern XFS | limited depending damage | self-heal with redundancy, tools | system-managed | self-heal with redundancy | PROP |
| Portable reference core | handler-specific | current portable work exists, historical driver OS-coupled | multiple ports but distinct implementations | many independent implementations | proprietary | Linux-specific core | Linux-specific core | Linux-specific core | multi-OS but large integrated stack | proprietary | proprietary | PLAN Rust reference core + C ABI |
| Low-memory implementation profile | YES | YES, excellent | YES | YES | moderate | moderate | moderate/high | higher | high | not a target | moderate/high | PLAN explicit profile |
| Built-in machine-readable feature API | legacy packets | private packets | private APIs | simple | rich Windows APIs | ioctl/statx mix | rich ioctl/tooling | ioctl/tooling | properties/ioctl tooling | Foundation/API stack | Windows APIs | PLAN Filesystem API v2 |
| Structured admin/tool API, no screen scraping | NO | NO | NO | NO | PARTIAL | fragmented | improving | fragmented | strong CLI/property model but still tool-specific | strong APIs | Windows APIs | PLAN |
| First-class developer trace / explain mode | NO | NO | NO | NO | ETW ecosystem, filesystem-specific internals | kernel tracing, not filesystem contract | tracing/debug tools | tracepoints/debug | extensive diagnostics, not portable FS contract | private Apple tooling | ETW/Windows diagnostics | PLAN |
| Strict zero-write forensic mount | not formalized | read-only behavior | read-only behavior | possible | possible | `noload`/RO combinations, semantics vary | RO semantics | RO semantics | readonly datasets/import options | system-controlled | readonly | PLAN `NO_CHANGES` contract |

## 3. Where AFS+ should clearly outperform classic Amiga filesystems

AFS+ should provide all of these simultaneously:

- no 2 GiB / 4 GiB era file-size boundary
- no legacy partition-size architecture limit
- UTF-8 long names
- scalable large directories
- stable object IDs
- crash-safe metadata transactions
- metadata checksums
- 64-bit offsets throughout the modern API
- sparse files
- symlinks and hard links
- xattrs plus native AROS comments/protection
- modern file notifications
- machine-readable capabilities
- portable checking and repair tools
- same-volume reflink cloning without copying file data

PFS3 remains a major inspiration for low-memory atomic-update design. SFS remains an important benchmark for responsiveness and transparent optimization. AFS+ should preserve those strengths rather than merely add modern features.

## 4. Where AFS+ can offer something uncommon even among modern filesystems

### 4.1 Global object stream plus persistent change stream

NTFS demonstrates the value of centralized metadata and the USN journal. Linux filesystem developers have repeatedly discussed NTFS-like persistent change journals because transient watchers do not solve backup/indexing catch-up after application downtime.

AFS+ intends to expose filesystem-neutral semantic APIs:

```text
EnumerateObjects()
GetChangesSince(sequence)
```

The fast path may be backed by the AFS+ catalog. The correctness path remains normal directory/object traversal.

### 4.2 Low-memory and workstation profiles in the same format

AFS+ deliberately treats small-machine implementability as an architectural metric, not as a separate legacy format.

A reader can stream B+ tree pages and allocation-region bitmaps with bounded memory while a modern workstation can cache aggressively.

### 4.3 Observability as a filesystem feature for developers

Most mature filesystems gained tracing, fsck tooling, fault injectors, and health reporting after years of painful debugging.

AFS+ plans these interfaces before the format is frozen. See [`docs/26-debug-observability.md`](26-debug-observability.md).

### 4.4 Derived accelerators that never become correctness dependencies

The global catalog, change stream, directory statistics, and reverse mapping should be rebuildable or safely discardable whenever practical.

This makes aggressive performance and maintenance features less dangerous to portability and recovery.

### 4.5 Clone semantics as part of the developer contract

AFS+ treats same-volume cloning as an explicit semantic API rather than only a hidden optimization.

```text
CloneFile()
CloneRange()
```

allow build tools, package managers, backup tools, VM/image workflows, and editors to request cheap independent copies and fall back to physical copy on other filesystems. Shared-extent support is therefore designed into the epoch-1 extent and reclamation model rather than retrofitted later.

## 5. Important features we should not chase merely to win a table

AFS+ 1.0 does not need to beat ZFS/Btrfs at:

- RAID
- multi-device pools
- deduplication
- send/receive replication
- complex snapshot hierarchies

Those features substantially increase implementation and recovery complexity.

They belong only if a real AROS workload justifies them and the extension framework can add them without destabilizing the core.

## 6. Reference facts

Key design references:

- OpenZFS checksums and snapshots: https://openzfs.github.io/openzfs-docs/
- Btrfs checksumming: https://btrfs.readthedocs.io/en/stable/Checksumming.html
- ext4 journal and metadata checksums: https://www.kernel.org/doc/html/latest/filesystems/ext4/
- XFS online repair design: https://www.kernel.org/doc/html/latest/filesystems/xfs/xfs-online-fsck-design.html
- APFS fundamentals: https://support.apple.com/guide/security/role-of-apple-file-system-seca6147599e/web
- ReFS overview/integrity/block clone: https://learn.microsoft.com/windows-server/storage/refs/
- NTFS filesystem functionality / USN: https://learn.microsoft.com/windows/win32/fileio/
- AmigaOS filesystem comparison: https://wiki.amigaos.net/wiki/UserDoc:AmigaOS_File_Systems

Exact limits and implementation status must be revalidated whenever this comparison is used for release claims.

## BFS and BeOS: historical comparison

The [complete Practical File System Design review](33-practical-filesystem-design-review.md)
covers the 1999 book chapter by chapter. This is a historical comparison,
not a statement about current Haiku implementations. AFS+ delivery status
remains in [milestones](../implementation/milestones.md).

| Area | Historical BFS described in the book | AFS+ requirement and decision |
|---|---|---|
| Metadata discovery | Typed attributes, secondary indexes and live queries | Optional catalog and persistent catch-up with complete backfill, explicit freshness and rescan. |
| Duplicate keys and queries | Duplicate lists; predicate-dependent search cost | Measure duplicate distributions; exact OR is not inherently a scan. Require an authoritative scan oracle. |
| Namespace identity | Single-parent assumptions and no hard-link implementation | Keep stable objects separate from every parent/name link and open handle. |
| Transactions | Metadata journal and grouped commits | COW checkpoints plus bounded intent records, data barriers and restartable recovery. |
| Allocation | Preallocation with fragmentation fallback | Invocation-local fallback cap; authoritative bitmap verification and safe delayed reuse. |
| Cache | Cache integration, bypass and lifetime concerns | Host-neutral coherence and durability contracts; measured policy and bounded memory. |
| Encoding | Alignment-sensitive implementations | Explicit endian codecs and checked ranges, independent of native struct layout. |
| Qualification | Synthetic, real-world and prolonged stress | Seeded byte oracles plus corruption/crash matrices, applications and native qualification. |

The chapter review identifies what is already covered, the allocation change,
and the owned experiments still required before optional facilities can be
advertised. Old throughput numbers and implementation omissions are not
permanent architectural limits.
