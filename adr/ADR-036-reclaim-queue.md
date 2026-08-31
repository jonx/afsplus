# ADR-036: Segmented reclaim queue with bounded, resumable batches

Status: Accepted for the prototype (Reclaim Scale-2); wire format experimental

<!-- toc -->

- [Context](#context)
- [Quarantine invariant (normative for this prototype)](#quarantine-invariant-normative-for-this-prototype)
- [Decision](#decision)
  - [Entries are runs](#entries-are-runs)
  - [Three tiers, append-only sealing](#three-tiers-append-only-sealing)
  - [Why there is no self-recursion](#why-there-is-no-self-recursion)
  - [Bounded, resumable reclamation](#bounded-resumable-reclamation)
  - [Bounded mount](#bounded-mount)
  - [Checker obligations](#checker-obligations)
- [Format impact](#format-impact)
- [Rejected alternative](#rejected-alternative)

<!-- /toc -->

## Context

The first prototype recorded quarantined storage in a single retired-list
block: at most 253 entries, one block per quarantined LBA, fully re-read and
fully promoted by every transaction. Two consequences became blocking once
data COW landed:

1. **Capacity.** A COW rewrite of 1 MiB already retires 256 data blocks
   before counting replaced metadata. Large truncates and unlinks retire
   thousands to millions of blocks. `(4096 − 32 − 8) / 16 = 253` entries is a
   hard failure, not a slow path.
2. **Unbounded per-transaction work.** Even a multi-block list would leave
   `TxAllocator::begin` promoting the entire backlog in one transaction,
   violating ADR-021's bounded-batch requirement. Capacity and bounded
   reclamation must be solved together; a chained list solves only the first.

## Quarantine invariant (normative for this prototype)

A physical block run enters quarantine when a committed transaction makes it
unreachable. For a run retired by the transaction that committed generation
`R`:

- the run is unreachable from every committed state with generation ≥ `R`;
- its allocation bits remain **set** in every bitmap state from `R` onward,
  until reclaimed;
- it is recorded in the reclaim queue with `retire_generation = R`;
- it may be cleared to FREE (and thus reallocated) only by a transaction
  committing generation `N > R`. Since checkpoint selection takes the newest
  structurally valid slot, the only checkpoint selectable while transaction
  `N` is in flight is `N − 1 ≥ R`, which does not reference the run.

Blocks allocated by an in-flight transaction and discarded before
publication are **not** retired: no committed state can reference them, so
they are released back to the transaction's free pool immediately
(`release`, distinct from `retire`). Only committed storage enters
quarantine.

On any uncertainty — a promotion that finds a bit already clear, a run
overlapping reachable state — the implementation must fail or leak, never
reuse early.

## Decision

The checkpoint references a **reclaim queue**: a FIFO of retired runs with a
persistent consumption cursor, stored in three tiers of checksummed blocks.

### Entries are runs

An entry is `(start LBA, block count, retire generation)` — 20 bytes. Extent
COW, truncate, and unlink retire contiguous runs, so a million-block unlink
costs a handful of entries, not a million.

### Three tiers, append-only sealing

```text
reclaim root (1 block, COW'd every transaction)
  fixed header: totals, cursor, per-volume area capacities
  table refs      -> sealed table blocks (oldest first)
  segment refs    -> sealed segment blocks not yet grouped into a table
  inline entries  -> newest appended runs
sealed segment block: up to 202 entries, immutable once written
sealed table block:   up to 338 segment refs, immutable once written
```

New runs are appended to the root's inline area, which the transaction
rewrites anyway. When the inline area overflows, its oldest entries are
sealed into a fresh segment block; when the segment-ref area overflows, its
oldest refs are sealed into a fresh table block. Sealed blocks are **never
rewritten**: consumption progress lives entirely in the root's cursor, and a
fully consumed segment or table block is dropped from the root and retired
through the queue itself.

### Why there is no self-recursion

Two distinct recursion hazards are addressed:

1. **Allocator ↔ reclaim structure.** Queue blocks are ordinary allocatable
   blocks obtained from the region allocator. Allocating them only flips
   bitmap bits, and bitmap/descriptor state lives in reserved generational
   slots (ADR-035 layering), so recording an allocation never allocates.
2. **Append cascade.** Because sealed blocks are immutable, appending never
   COWs existing queue blocks. The blocks a transaction retires *of the
   queue itself* — the previous root, plus any segments/tables fully
   consumed by this transaction's batch — are all known **before** the new
   root is encoded, so the final entry set is computed in one pass: no
   fixpoint. Sealing during that pass only allocates fresh blocks; each seal
   empties a full area and appends nothing, so the seal loop terminates.

### Bounded, resumable reclamation

Each transaction consumes at most `K` **blocks** (not entries) from the
queue head before allocating: it clears their bits, records fully consumed
segments/tables for retirement, and persists the advanced cursor in the new
root. `K` is a runtime policy knob, not format state. The cursor supports
resumption mid-run (`head block offset`) and mid-segment (`head entry
offset`), so a crash or reboot continues exactly where the last committed
generation stopped — the backlog and cursor are ordinary committed state
published through the checkpoint like everything else.

Reclamation is therefore incremental across transactions and across reboots;
a maintenance operation (`reclaim_step`) drains the backlog with otherwise
empty transactions when the volume is idle or near-full.

Multiple retire generations coexist in the queue. FIFO order means older
generations drain first; per-entry generations are retained for the checker
and for future retention policies that keep more than two checkpoints.

### Bounded mount

Normal mount reads and validates only the reclaim root (one block; its
inline areas are bounded by the recorded capacities). Walking tables and
segments is batch/checker work.

### Checker obligations

The exhaustive checker walks the full queue and verifies:

- structure: counts within capacities, cursor offsets within bounds, exact
  `appended − reclaimed = pending` accounting, per-block checksums;
- every unconsumed run: inside allocatable bounds, outside the permanent
  allocation-root pool, retire generation in `1..=checkpoint generation`,
  every covered block allocated in the bitmaps and unreachable from the
  checkpoint's state;
- ownership: root, table, and segment blocks claimed uniquely as reachable
  metadata;
- bitmap ⟺ accounting equality now includes quarantined runs.

## Format impact

- The checkpoint's retired-list reference becomes the reclaim-root
  reference and is always nonzero; mkfs writes an empty root as a fourth
  bootstrap metadata block (the allocation-root pool derivation shifts by
  one).
- Three new block types (reclaim root, sealed segment, sealed table). The
  single-block `AFSR` codec remains transitional test coverage only.
- Root-area capacities are recorded per volume so crash tests can force
  sealing and consumption with tiny areas; sealed-block capacities derive
  from the block size.

This is prototype wire format under ADR-034's experimental rules, not an
epoch-1 freeze. Measured amplification (structure blocks per retired run,
reclaim I/O per batch) is part of the standing measurement suite; ADR-021's
resumable-reclamation contract is the governing requirement.

## Rejected alternative

Chaining multiple `AFSR` blocks was rejected explicitly: it removes the
capacity limit but keeps whole-backlog promotion in one transaction, and
appending to a chained tail COWs sealed blocks, reintroducing the append
cascade this design eliminates.
