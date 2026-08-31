# Checkpoint slot rings for weak-FTL media

> **ADRs:** none · **Spec:** none ·
> **Tests:** none · **Milestones:** none

Status: proposal — for team review (decision M1 of
`storage-media-profiles.md`). Format-affecting: on acceptance this becomes
a numbered ADR and the change lands before epoch 1.

## Context

Every transaction writes one of the two checkpoint slots at LBA 1–2: they
are the hottest blocks on any AFS+ volume by orders of magnitude. The
first intent-log slot is rewritten at every window's first fsync. On
NVMe/SATA SSDs the FTL disperses this invisibly. On weak-FTL media —
SD, eMMC, USB sticks, and the CompactFlash that carries most classic
Amiga installations — wear leveling is grouped or shallow, and sustained
load concentrates program/erase cycles on a handful of physical groups.
This failure mode shaped F2FS and has destroyed SD cards under naive
journaling filesystems; a design that names classic hardware as a target
cannot delegate it to FTL quality.

Region descriptor and bitmap slots also sit at fixed locations, but they
rotate across three slots, are written only when their region is dirty,
and are spread across the volume; they run orders of magnitude cooler and
are left at three slots until measurements say otherwise.

## Proposed decision

Generalize both hot areas into **rings sized at mkfs** and recorded in the
identification block:

1. **Checkpoint ring.** `checkpoint_ring_slots = N ≥ 2` contiguous
   reserved slots. A commit for generation `g` writes slot `g mod N`.
   Mount scans the ring and selects the newest structurally valid
   checkpoint — the existing selection logic unchanged except for the scan
   width. Two structurally valid checkpoints with the same generation
   remain fatal ambiguity. Today's layout is exactly the `N = 2` case.
2. **Intent-log offset rotation.** A window whose base generation is `g`
   starts its records at slot `g mod log_slots`, wrapping; the scan
   derives the start deterministically from the selected generation.
   Record sequence and binding rules are unchanged.

Defaults per media profile: 2 everywhere except `sd-card` (16) and
`classic` (8).

## What does not change

- **Crash semantics.** The commit invariant is "never overwrite the
  newest valid checkpoint"; with `slot = g mod N` the newest (generation
  `g`) and the in-flight write (`g + 1`) always land in different slots
  for any `N ≥ 2`. A crash-and-retry rewrites the same slot it targeted
  before, which was never the newest valid. The existing crash matrices
  apply with the ring width as a parameter.
- **Retention.** Quarantine and the reserved-pool exclusion still protect
  exactly the newest and second-newest generations. Older ring entries
  are wear-leveling artifacts, **not** additional recovery points: their
  referenced metadata may already be reused. The checker reports them as
  stale, never selects them, and mount falls back at most one generation,
  as today.
- **Bitmap-slot protection.** The "avoid the slots referenced by both
  retained checkpoints" rule already takes the two newest checkpoints
  from the scan; unchanged.

## Costs

- Mount reads `N` slots instead of 2 — bounded and small (≤ 16 per the
  profile table), and only identity-record reads.
- `N` reserved blocks instead of 2. At `N = 16`: 56 KiB.
- The identification block gains a ring-size field (a reserved area
  exists); mkfs and the checker learn the scan width.

## Alternatives considered

- **Trust the FTL** — correct for NVMe/SSD, wrong for the project's
  stated classic-hardware target; rejected as the sole answer.
- **Fully log-structured superblock (F2FS-style)** — solves wear
  thoroughly, but replaces a two-line selection loop with a new subsystem;
  rejected for scope.
- **Checkpoints in allocator-managed space** — creates a locator problem
  (mount must find them without the allocator), which forces a fixed
  locator block anyway; the ring is that idea reduced to its minimum.

## Qualification gates

- Existing crash matrices re-run at `N ∈ {2, 8, 16}`, including the
  mis-ordered-commit negative control and same-generation ambiguity.
- A wear distribution test: run 10k transactions, assert checkpoint
  writes spread uniformly (±1) across the ring, and intent-log writes
  across the log area.
- Mount-cost assertion: reads grow by exactly `N − 2` on the 1 TiB image.
