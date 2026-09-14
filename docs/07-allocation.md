# 07. Allocation

> **ADRs:** [ADR-035](../adr/ADR-035-allocation-root-reserved-pool.md) · **Spec:** none ·
> **Tests:** [crash-testing](../testing/crash-testing.md) · **Milestones:** M03

<!-- toc -->

- [1. Allocation regions](#1-allocation-regions)
- [2. Proposed region size](#2-proposed-region-size)
- [3. Epoch-1 blocker: allocation metadata under COW](#3-epoch-1-blocker-allocation-metadata-under-cow)
  - [3.1 Multi-page region binding experiment](#31-multi-page-region-binding-experiment)
- [4. Allocation strategy](#4-allocation-strategy)
- [5. Free-space summaries](#5-free-space-summaries)
- [6. Metadata reservation](#6-metadata-reservation)
- [7. Discard](#7-discard)
- [8. Allocation integrity](#8-allocation-integrity)

<!-- /toc -->

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
checkpoint allocation-root pointer
        |
        v
shared AFST: region -> descriptor slot/generation/free count
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
The typed allocation-root leaves keep one record per region: descriptor slot,
descriptor generation, and region free count. The checkpoint itself keeps one
root LBA plus the total free count used by cheap status queries.

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
one page plus one descriptor. The G1/G2/G3/G4 quarantine crash matrix checks descriptor ordering before
the metadata barrier and previous-checkpoint preservation under
[ADR-074](../adr/ADR-074-protect-previous-checkpoint.md). Corruption tests require
descriptor/page damage to be reported by the checker or first allocator access
without turning normal mount into a bitmap scan.

The formatter no longer loops over every logical block merely to seal reserved
bits; it touches reserved ranges and bitmap pages directly. An explicit host
qualification now formats and bounded-mounts a sparse 1 TiB image (1,024 full
regions), performs two small commits, and runs the exhaustive checker in about
3.57 seconds total on the development Apple-Silicon/APFS host. The latest
commits measured about 24.5 ms and 12.1 ms; each changed one region record and
at most three allocation-root nodes. The checker took about 2.28 seconds. Host physical
allocation varied from roughly 112 MiB to 2.13 GiB between runs, so the test
enforces a relative sparse bound below 1% rather than making APFS allocation an
AFS+ format guarantee.

This remains an experiment, not an epoch-1 commitment. It preserves page-level
write amplification, on-demand loading, crash safety, and repairability. If
the descriptor indirection performs poorly or becomes too complex, the
delta-log/spacemap candidates remain open.

The earlier inline checkpoint array has now been replaced by ADR-035's bounded
allocation-root tree. Its nodes live in a permanent triple-version pool, so
updating the allocator does not recursively allocate from the free space being
described. Transactions load current/older region records on demand and the
allocation-root mutation emits upserts only for dirty regions, therefore
writing only their COW paths. The measured 1 TiB commits fetched one current
record, then one current plus one retained-checkpoint record. Pool protection
still walks the small set of retained allocation-tree nodes, without
materializing all typed records.

Checker bitmap equality is exhaustive but sparse-aware: it verifies every
expected reserved/reachable/retired block, then walks set bits byte-wise to find
unowned allocations. It no longer performs one map lookup for every logical
LBA, which is what makes the 1 TiB checker qualification practical.

The first leaf-capacity boundary is crash-qualified separately: 145 regions
force a two-level allocation root, and the every-write/every-flush matrix
accepts only the complete pre- or post-commit state. The power-cut harness
streams each cloned image to the verifier, preserving exhaustive subset/torn
coverage without retaining the whole matrix in RAM.

## 4. Allocation strategy

Preferred policy, independent of exact free-space encoding:

1. extend adjacent file extent if possible
2. allocate within the object's current locality region
3. use a nearby region with sufficient free run
4. fall back to general free regions

Directories and their small child objects should have locality hints, not hard placement requirements.

Extent allocation retains a reduced request size for the remainder of one
allocation call after a failed large-run search. The cap is runtime-only and
resets on the next call, so freed contiguous space can be used immediately.
This avoids repeated oversized searches on fragmented volumes without
changing the authoritative bitmap or reuse rules. The
[book-review qualification](../testing/book-review-qualification.md) measures
allocation searches and bitmap positions examined independently of I/O.

## 5. Free-space summaries

Global/per-region free-space summaries may be accelerators.

They must never be the only proof that a block is free.

If a summary disagrees with authoritative allocation state:

- ignore/rebuild the summary
- do not mark a block free based only on the summary

The authoritative source itself is selected by the prototype in section 3.

## 6. Metadata reservation

A small emergency metadata reserve prevents the filesystem from becoming
impossible to commit or repair when nearly full. The current implementation
uses a **soft, format-neutral floor**, not a separately formatted partition:

```text
total volume below 64 blocks -> 0 (minimal/test geometry only)
otherwise                   -> clamp(ceil(total blocks / 32), 8, 64)
```

Ordinary growth transactions set this raw-free floor on their transaction
allocator. Every allocation made by that transaction — user data, COW tree
nodes, object records and reclaim structures — is refused before I/O if it
would cross the floor. File/directory creation, writes, preallocation,
reflinks, hard links, policy changes and growth inside an intent-log window
are normal growth. Destructive namespace transitions, truncation shrink,
orphan setup/cleanup, reclaim and replay may consume the floor so they can
make progress at ENOSPC.

Filesystem-facing final unlink and replacement use ADR-066's orphan
transition even when no process handle remains. The visible operation is
therefore bounded independently of file fragmentation; idle or sync cleanup
then removes at most its configured extent budget. Orphan cleanup promotes at
least 16 reclaim blocks per step even when a diagnostic caller selected a
smaller general reclaim budget, avoiding a maintenance loop that consumes COW
metadata faster than it recycles prior generations.

`free_blocks` is the checkpoint's authoritative raw free count.
`available_blocks = free_blocks - emergency_headroom` (saturating at zero) is
the capacity advertised for normal growth. VFS `statfs`, `afsplus-info` and
`afsplus-dump` report the distinction. The floor may already be partly
consumed after an emergency operation; in that state available capacity is
zero until bounded reclaim restores it.

This rule is runtime policy and adds no on-disk bit or bitmap ownership class.
[ADR-067](../adr/ADR-067-epoch1-allocation-state.md) closes Q3 by selecting the
triple-version region structures, fixed `3N` allocation-root pool, bitmap
authority and segmented reclaim queue. Exact byte layout and the global wire
epoch remain subject to M14 review.

## 7. Discard

Discard/TRIM is issued through the block provider only after deallocation becomes durably safe for reuse.

Discard failure does not make the filesystem inconsistent.

## 8. Allocation integrity

No committed allocation state may allow two live objects to own the same physical block unless an explicitly enabled shared-extent feature defines and accounts for that sharing.

On uncertainty, quarantine/leak space rather than making a possibly referenced block allocatable.
