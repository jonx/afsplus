# Storage Media Profiles

> **ADRs:** [ADR-036](../adr/ADR-036-reclaim-queue.md),
> [ADR-057](../adr/ADR-057-plain-m68000-emulator-gate.md) · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Status: proposal — for team review before epoch 1. On acceptance this
becomes a numbered document under `docs/`. Decisions requested are marked
**M1–M5**; M1 is a format question and is expanded in
[`proposals/checkpoint-slot-rings.md`](checkpoint-slot-rings.md).

<!-- toc -->

- [1. Why this document exists](#1-why-this-document-exists)
- [2. What the existing design already provides](#2-what-the-existing-design-already-provides)
- [3. Per-medium analysis](#3-per-medium-analysis)
  - [NVMe / SATA SSD](#nvme--sata-ssd)
  - [Spinning disks (HDD)](#spinning-disks-hdd)
  - [SD, eMMC, CompactFlash, USB sticks](#sd-emmc-compactflash-usb-sticks)
  - [Classic Amiga storage (SCSI, IDE, trackdisk-class devices)](#classic-amiga-storage-scsi-ide-trackdisk-class-devices)
  - [SMR and zoned devices (ZNS)](#smr-and-zoned-devices-zns)
  - [Host files, loop devices, VM images](#host-files-loop-devices-vm-images)
- [4. The hot-slot problem](#4-the-hot-slot-problem)
- [5. Discard wiring (runtime, no format impact)](#5-discard-wiring-runtime-no-format-impact)
- [6. The media profile](#6-the-media-profile)
- [7. Decisions requested](#7-decisions-requested)

<!-- /toc -->

## 1. Why this document exists

AFS+ targets media with opposite failure and performance characteristics:
NVMe and SATA SSDs, spinning disks, SD/eMMC/CompactFlash and USB sticks,
classic Amiga storage, host files and virtual disks. The core design is
already friendly to most of them by construction, but three concerns are
media-specific and one of them touches the on-disk format. This document
records the per-medium analysis and proposes a **media profile**: a small
set of mkfs-time parameters, exactly like `region_size`, `log_slots`, and
`reclaim_caps` today.

A clean split governs everything below:

- **Format-affecting choices** (slot ring sizes, alignment) are chosen at
  mkfs and recorded on disk. They must be settled before epoch 1.
- **Runtime policy** (discard emission, flush trust, queue usage) is host
  behavior and must never leak into the format.

## 2. What the existing design already provides

- **COW suits flash.** Committed data is never overwritten in place, which
  is the access pattern FTLs handle best; everything is 4 KiB-aligned.
- **The provably safe discard moment exists.** `docs/07` already requires
  TRIM only after deallocation is durably safe. The reclaim queue makes
  that moment exact: **promotion out of quarantine** is, by the ADR-036
  invariant, the first instant at which no selectable checkpoint can
  reference the block. Bounded, batched discard falls out of the existing
  reclamation design for free.
- **The durability contract is already honest** about devices that lie
  about cache flushes (`docs/08` §9) — the chronic problem of consumer
  SD/USB media. We promise nothing such a device cannot deliver.
- **HDD locality is respected**: allocation regions, the roving pointer,
  extent growth, and barrier reduction through group commit and the intent
  log. Long-term COW fragmentation is the known cost, owned by the planned
  relocation/defragmentation work.
- **Classic media constraints** (bounded memory, tiny caches) are core
  design rules, not per-medium fixes.

## 3. Per-medium analysis

### NVMe / SATA SSD

Nothing structural to fix. The FTL absorbs our fixed-slot rewrites; discard
should be wired (§5); queue-depth parallelism is a later implementation
concern with no format impact.

### Spinning disks (HDD)

Fixed metadata slots at the volume head plus COW metadata elsewhere means a
seek per commit — the same pattern as a fixed journal, and acceptable.
Region-local allocation keeps related data close. The medium-term item is
read-side fragmentation under sustained COW, addressed by relocation, not
by this document.

### SD, eMMC, CompactFlash, USB sticks

The problem medium — and, through CF, the classic Amiga medium. All such
devices have an FTL, but cheap ones wear-level within small groups and are
tuned for FAT-style access. Two consequences:

1. **Hot fixed slots wear the medium** (§4, decision M1).
2. **Alignment matters disproportionately**: erase blocks and SD
   allocation units are typically 4–8 MiB, and a misaligned region layout
   can double effective write amplification. `region_size` is already a
   power of two; the profile should pin recommended sizes and require the
   volume to start AU-aligned (M4).

Discard on this class is sometimes counterproductive (synchronous erase
stalls); emission is runtime policy (M3).

### Classic Amiga storage (SCSI, IDE, trackdisk-class devices)

Bounded memory is already handled. The open item is **512-byte sectors**:
the prototype is fixed at 4 KiB logical blocks; the block provider was
always specified with `get_sector_size`, and the torn-write model already
tears at sub-block granularity, so recovery semantics are ready. Sector
support is implementation work, listed but not urgent (M5).

### SMR and zoned devices (ZNS)

Fixed-location slots are fundamentally incompatible with host-managed
zones; drive-managed SMR works but with performance cliffs on our commit
pattern. Cheap external disks are often SMR, so this must be stated
rather than discovered: **host-managed zoned support is explicitly out of
scope for epoch 1** and would arrive, if ever, as a distinct
log-structured layout profile — not as a stretch of this format.

### Host files, loop devices, VM images

Already first-class through the block-provider abstraction and the sparse
file backend. Discard maps to hole punching where the host supports it —
same runtime policy switch as physical discard.

## 4. The hot-slot problem

Every transaction writes one checkpoint slot: LBA 1 and 2 are the hottest
blocks on any AFS+ volume by orders of magnitude. The first intent-log
slot is rewritten at every window's first fsync. Region descriptor and
bitmap slots rotate across three fixed locations and are spread per
region, so they run far cooler, but they are still fixed.

On strong FTLs this is invisible. On weak-FTL media, sustained load
concentrates program/erase cycles on a handful of physical groups — the
failure mode that shaped F2FS and that has killed SD cards under naive
journaling filesystems.

The proposed fix is **slot rings** (see the companion proposal): `checkpoint_ring_slots ≥ 2`
checkpoint slots selected by `generation mod N`, and an intent-log start
offset rotated by base generation, both recorded in the identification
block. Mount selection generalizes unchanged (scan the ring, take the
newest structurally valid; same-generation ambiguity stays fatal), and the
quarantine contract still protects exactly the newest and second-newest
generations — older ring entries are wear-leveling artifacts, **not**
additional recovery points. Cost: ring-size reads at mount instead of two.
This is cheap while the format is unfrozen and INCOMPAT-painful after.

## 5. Discard wiring (runtime, no format impact)

- Add `discard(lba, count)` to the block-provider trait; backends without
  support report it cleanly and the filesystem ignores failures
  (`docs/07`: discard failure never affects consistency).
- Emit at **reclaim promotion**, batched with the existing bounded batch;
  optionally at mkfs for the whole device (large practical win on used
  SSDs).
- Emission policy (on/off/async) belongs to the media profile's runtime
  half, defaulting per preset.

## 6. The media profile

One mkfs argument selecting a named preset, each preset being nothing but
defaults for existing and proposed parameters:

| Preset | checkpoint ring | log ring rotation | region size guidance | alignment | discard default |
|---|---|---|---|---|---|
| `nvme` | 2 | off | large (≥ 64 Mi blocks…) | 4 KiB | on |
| `ssd` | 2 | off | large | erase-block | on |
| `hdd` | 2 | off | medium | 4 KiB | off |
| `sd-card` | 16 | on | = allocation unit | AU (4–8 MiB) | off (M3) |
| `classic` | 8 | on | small | 4 KiB | off |
| `host-file` | 2 | off | free | 4 KiB | hole-punch |

Presets are convenience only: every value remains individually
overridable, and nothing in the format records the preset name — only the
resulting parameters.

## 7. Decisions requested

- **M1 — Slot rings (format).** Adopt ADR-057 (checkpoint ring +
  log-offset rotation, sizes in the identification block) before epoch 1?
  *Recommendation: yes; the generalization is small and the alternative
  (trusting every FTL) contradicts the classic-hardware goal.*
- **M2 — Ring defaults.** Sizes per preset as in §6?
  *Recommendation: as tabled; only `sd-card` and `classic` deviate from 2.*
- **M3 — Discard policy defaults.** Wire discard now (trait + promotion +
  mkfs), default off for `sd-card`/`hdd`?
  *Recommendation: yes; emission is cheap to implement and entirely
  runtime.*
- **M4 — Alignment.** Document AU/erase-block alignment guidance and
  validate volume-start alignment at mkfs (warning, not error)?
  *Recommendation: yes, warning only — partitioners own placement.*
- **M5 — 512-byte sectors.** Schedule sector-size support with the classic
  media milestone rather than now?
  *Recommendation: yes; the torn-write model is already sub-block, nothing
  blocks on it.*
