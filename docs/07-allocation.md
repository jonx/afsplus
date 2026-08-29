# 07. Allocation

## 1. Allocation regions

The volume is divided into fixed-size allocation regions.

Each region has bounded free-space/accounting state and locality information such as:

- free-block state
- free-block count
- largest-known-free-run hint
- metadata/data allocation hints
- generation
- checksum/integrity information where applicable

The region architecture is intended to bound memory use and repair scope.

**The exact authoritative on-disk representation of free-space state is not yet frozen.** A flat COW bitmap was the initial proposal, but the transaction prototype must first solve the self-reference problem described below.

## 2. Proposed region size

Initial default: 1 GiB of address space per region.

At 4 KiB blocks, a flat bitmap representation would be:

```text
1 GiB / 4 KiB = 262,144 blocks
bitmap = 262,144 bits = 32 KiB
```

This remains a useful memory-budget reference even if the final authoritative representation includes deltas/logs rather than only a flat bitmap.

Region size is stored in the superblock and must be a power-of-two multiple of the logical block size.

## 3. Epoch-1 blocker: allocation metadata under COW

Naively copy-on-writing an authoritative allocation bitmap is recursive:

```text
need to update bitmap
 -> allocate new block for COW bitmap
 -> allocation changes bitmap
 -> need to update bitmap again
```

This is not an implementation footnote. It can determine the allocation metadata format.

The first allocator prototype must compare at least these families:

1. **PFS3-like reserved metadata allocation**: metadata COW allocations come from a separately managed reserve whose commit rules break the recursion.
2. **Region bitmap plus bounded transactional delta/log**: the checkpoint references a compact base plus committed allocation changes.
3. **Log/space-map-like authoritative free-space history with periodic condensed bitmap rebuild**.
4. A hybrid in which a tiny fixed bootstrap/reserve allocator is used to update ordinary region allocation structures.

The chosen design must preserve:

- no double allocation
- bounded mount recovery
- bounded RAM
- safe behavior at nearly full volume
- repairability
- low write amplification
- explicit crash ordering

No epoch-1 bitmap layout should be frozen before this prototype passes power-cut tests.

The current reserved-slot prototype also keeps transaction RAM independent of
the total region count: it loads bitmap pages on demand, evicts unsuccessful
clean scan pages, and pins only dirty/touched pages until commit. Modern ports
may layer a larger persistent bitmap cache over the same semantics. The
checker deliberately retains the option to load all pages for an exhaustive
whole-volume comparison.

### 3.1 Multi-page region binding experiment

The executable prototype now implements the descriptor indirection below.
The proposed 1 GiB region needs nine 4 KiB bitmap pages: the raw bits occupy
32 KiB, but every independently verifiable page also needs its common header
and page identity. A naïve checkpoint record for every bitmap page would make
the checkpoint grow with
the number of pages and would quickly turn the checkpoint block itself into a
volume-size limit.

The prototype instead uses one reserved, triple-buffered region descriptor
plus triple-buffered bitmap pages:

```text
checkpoint region record
        |
        v
region descriptor slot A/B/C
        |
        +-- bitmap page 0 slot A/B/C
        +-- bitmap page 1 slot A/B/C
        +-- ...
```

The region descriptor records, for each logical bitmap page, its selected
physical slot, generation, free count, and any required integrity binding.
The checkpoint keeps only one bounded record per region: descriptor slot,
descriptor generation, and region free count.

A transaction changing one allocation page:

1. write that page to a slot referenced by neither retained descriptor
2. write a new region descriptor to its unused slot, reusing bindings for
   unchanged pages
3. include both writes in the pre-checkpoint metadata barrier
4. publish the alternate checkpoint

For a 1 GiB region with nine bitmap pages, three descriptor blocks plus three
slots for each page reserve 30 blocks, or 120 KiB (about 0.0114% of the
region). Region 0 also contains the identification block and two global
checkpoint slots. These blocks remain outside the allocator they describe,
so the COW self-reference stays broken.

The host tests format the full 262,144-block region geometry sparsely, force a
single allocation run across a bitmap-page boundary, and verify that exactly
two pages plus one descriptor are dirtied. Ordinary small transactions dirty
one page plus one descriptor. The G1/G2/G3 quarantine crash matrix passes with
the descriptor write included before the metadata barrier, and separate
corruption tests prove that descriptor/page damage is found by the checker or
the first allocator access without turning normal mount into a bitmap scan.

This remains an experiment, not an epoch-1 commitment. It preserves page-level
write amplification, on-demand loading, crash safety, and repairability. If
the descriptor indirection performs poorly or becomes too complex, the
delta-log/spacemap candidates remain open.

The current single-block checkpoint also limits the number of region records.
Larger-volume work must eventually replace that prototype limit with a
bounded allocation-root structure rather than silently increasing mount I/O
in proportion to the volume.

## 4. Allocation strategy

Preferred policy, independent of exact free-space encoding:

1. extend adjacent file extent if possible
2. allocate within the object's current locality region
3. use a nearby region with sufficient free run
4. fall back to general free regions

Directories and their small child objects should have locality hints, not hard placement requirements.

## 5. Free-space summaries

Global/per-region free-space summaries may be accelerators.

They must never be the only proof that a block is free.

If a summary disagrees with authoritative allocation state:

- ignore/rebuild the summary
- do not mark a block free based only on the summary

The authoritative source itself is selected by the prototype in section 3.

## 6. Metadata reservation

A small emergency metadata reserve is required so the filesystem does not become impossible to commit/repair when nearly full.

Whether the reserve also becomes a core part of the allocation transaction algorithm is intentionally part of the section-3 experiment.

User-visible free-space reporting must distinguish normally allocatable space from emergency reserved metadata space and, where applicable, pending-reclaim capacity.

## 7. Discard

Discard/TRIM is issued through the block provider only after deallocation becomes durably safe for reuse.

Discard failure does not make the filesystem inconsistent.

## 8. Allocation integrity

No committed allocation state may allow two live objects to own the same physical block unless an explicitly enabled shared-extent feature defines and accounts for that sharing.

On uncertainty, quarantine/leak space rather than making a possibly referenced block allocatable.
