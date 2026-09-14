# ADR-067: Select the epoch-1 authoritative allocation-state architecture

Status: Accepted
Amends: ADR-035, ADR-036

## Context

Q3 asks whether AFS+ should retain its reserved triple-slot bitmap pages,
triple-slot region descriptors and fixed-topology allocation-root pool as the
epoch-1 authoritative allocation representation, or replace them with a delta
log or spacemap before format freeze.

The prototype now exercises the deciding boundaries rather than only the
small-image happy path. Allocation-root records and bitmap pages load on
demand; clean scan pages are evicted; only dirty paths stay resident. A
runtime rover carried between transactions and free-count summaries skip
exhausted regions. The allocation root has a fixed region-key topology whose
`3N` deterministic pool can represent two retained checkpoints plus a
complete new image without allocating from the space it describes.

The remaining liveness gap was not a defect in that representation. Normal
growth could consume the last raw blocks needed to publish destructive
metadata, and direct final deletion could make work proportional to a file's
fragmentation. ADR-066 supplies a bounded persistent orphan transition and
restartable cleanup. The runtime allocator now keeps a soft emergency floor
for normal growth while destructive, reclaim and recovery transactions may
use it. This adds no disk field or separately owned metadata partition.

## Decision

Select the following as the epoch-1 authoritative allocation-state
architecture:

1. Each allocation region has one logical descriptor and one or more logical
   bitmap pages. Descriptors and pages each use three deterministic physical
   generations; a transaction writes a slot referenced by neither retained
   checkpoint.
2. The checkpoint names one AFST allocation root and stores the exact global
   raw-free total. Allocation-root leaves map every region number to its
   selected descriptor slot, descriptor generation and free count.
3. The allocation-root region-key set and bulk-built topology are fixed by
   immutable geometry. If it contains `N` logical nodes, its deterministic
   permanently allocated pool contains `3N` physical blocks. It never
   allocates from ordinary free space and never enters the reclaim queue.
4. The bitmap remains the authority for whether an ordinary physical block is
   allocated. Global, region and page free counts are checked summaries, not
   independent proof that a block is reusable.
5. Blocks removed from live ownership enter ADR-036's checksummed segmented
   reclaim queue. They remain allocated until retained-checkpoint safety makes
   bounded promotion legal. Transaction-local allocations discarded before
   publication return directly to the working free state.
6. Normally available capacity excludes a runtime emergency-metadata floor.
   The floor, cleanup/reclaim batch sizes, rover and locality hints are policy,
   not epoch-1 wire fields. They may be tuned without changing the allocation
   encoding, provided growth cannot consume protected headroom and emergency
   operations remain bounded.

A future allocation delta, spacemap or free-run index may exist only as a
rebuildable accelerator behind an explicitly negotiated feature. It cannot
silently replace the bitmap/descriptor/allocation-root authority within epoch
1. Changing that authority requires a new incompatible format decision.

Acceptance closes Q3 at the architecture level. Exact byte offsets and the
repository's still-experimental global wire epoch remain subject to the final
M14 independent layout review; this ADR does not by itself declare format
epoch 1 shipped.

## Compatibility and classic-system consequences

The selected representation is identical for classic and modern profiles.
Classic implementations can load one descriptor and bitmap page at a time,
choose small cleanup batches and omit optional features; modern ports can add
larger caches without changing media. At the 1 GiB maximum region size, the
authoritative bitmap payload is 32 KiB split across nine independently checked
4 KiB pages. Ordinary small commits dirty only touched pages, one descriptor
per changed region and the corresponding fixed allocation-root paths.

The soft emergency floor consumes no permanently dedicated data blocks. The
current reference policy ranges from 8 to 64 blocks on supported geometries,
or 32 KiB to 256 KiB at 4 KiB blocks. Raw free and normally available counts
are distinct API/tool values; an emergency transaction may temporarily reduce
raw free below the nominal floor, leaving normal availability at zero until
bounded reclaim restores it.

An unaware epoch-1 implementation cannot substitute another allocation
authority. Existing compatibility negotiation remains responsible for other
features, while allocation-state codec changes after M14 require an
incompatible format generation rather than a profile-specific interpretation.

## Evidence

The executable qualification currently proves:

- exact bitmap equality with reserved, reachable, retired and pool ownership;
- G1/G2/G3 quarantine and reuse with every modeled write/flush cut;
- independent bitmap-page and descriptor corruption detection;
- a multi-node allocation-root commit and crash matrix across its first leaf
  boundary;
- sparse 1 TiB format, bounded mount and exhaustive check;
- one maximum 1 GiB region updated with nine bitmap pages and 32 KiB peak
  allocator bitmap RAM;
- failed ENOSPC growth performs zero writes and barriers, including failure
  caused only by finish-time metadata allocation;
- raw-free/emergency/available reporting through the VFS, FUSE, AROS mapping
  and official tools;
- near-full destructive progress on single- and multi-region geometries; and
- a deep-namespace, highly fragmented final unlink whose visible transition is
  extent-count independent and whose bounded cleanup converges even when the
  caller selects a one-block general reclaim budget.

The reproducible commands and current machine-readable measurements live in
the [allocation qualification](../testing/allocation-qualification.md). Full
repository gates include the checker, crash matrices, Rust/C fuzzing,
strict/sanitized/static-analyzed C and the configured m68k compiler.

## Rejected alternatives

- **Make an allocation delta log authoritative.** It adds replay, compaction
  and repair states to an encoding whose measured bitmap paths are already
  bounded. The existing intent log remains logical-operation durability, not
  a second allocation authority.
- **Use a spacemap/log-structured authority.** It can improve large free-run
  discovery, but authoritative history and condensation complicate classic
  recovery. A rebuildable free-run accelerator remains possible later.
- **Dedicate a persistent emergency partition.** It taxes small media, adds a
  second allocator and still does not bound an operation whose work scales
  with file fragmentation. The soft floor plus ADR-066 avoids all three.
- **Keep the representation provisional indefinitely.** That would make every
  writer, checker, repair tool and portable implementation target a moving
  allocation authority despite the blocker tests now passing.

## Consequences

Implementations and review can treat the triple-version region structures,
fixed `3N` allocation-root pool, authoritative bitmap and segmented reclaim
queue as one coherent epoch-1 architecture. Performance work can concentrate
on locality, caches and optional accelerators without reopening the
self-referential COW problem.

The cost is a permanent commitment to deterministic reserved slot overhead
and bitmap authority for epoch 1. M14 must still audit exact layouts, overflow
bounds, feature negotiation and independent cross-reading before setting the
wire epoch stable. Any qualification regression reopens acceptance rather
than being explained away as implementation tuning.
