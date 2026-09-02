# Shared-Extent and Reflink Qualification

> **ADRs:** [ADR-027](../adr/ADR-027-reflink-clones.md),
> [ADR-061](../adr/ADR-061-shared-extent-references.md) · **Spec:**
> [invariants](../spec/invariants.md) · **Tests:**
> [crash testing](crash-testing.md) · **Milestones:** M03, M05, M14

<!-- toc -->

- [Purpose](#purpose)
- [Required observability](#required-observability)
- [Functional matrix](#functional-matrix)
- [Canonical reference oracle](#canonical-reference-oracle)
- [Corruption matrix](#corruption-matrix)
- [Crash matrix](#crash-matrix)
- [Resource and compatibility gates](#resource-and-compatibility-gates)
- [Completion evidence](#completion-evidence)

<!-- /toc -->

## Purpose

This gate proves that `CloneFile` and `CloneRange` share physical data without
allowing a write, truncate, unlink, replacement or crash to expose one
object's changes through another object or to return still-referenced storage
to the allocator.

The executable targets are:

```sh
cargo test -p afsplus-check --test shared_extents
cargo test -p afsplus-check --test shared_clone
cargo test -p afsplus-check --test shared_crash
```

## Required observability

The test uses public diagnostic loaders rather than parsing private bytes in
the application layer. For the selected checkpoint it must be able to obtain:

- every file extent, including its flags and whether the object used direct or
  extent-tree layout;
- every shared-reference record and every metadata block belonging to the
  shared-reference tree;
- the reclaim runs, allocation bitmaps and both selectable checkpoints;
- the recorded block writes and flushes for one transaction;
- the mounted generation and file contents after recovery.

The loader and overlap resolver belong in the core verification layer, so the
mount shadow verifier and `afsplus-check` cannot acquire different
interpretations of the format.

## Functional matrix

Every successful row is followed by unmount, remount and a clean exhaustive
checker result.

| Case | Operation | Required post-state |
|---|---|---|
| F1 | clone a direct-layout file | source and destination are promoted to extent trees; both map the same data; the reference tree contains the exact shared runs |
| F2 | clone an extent-tree file | the destination preserves holes and unwritten ranges; allocated source runs are shared canonically |
| F3 | clone a representable sub-range | only the requested physical sub-runs gain references; destination bytes outside the range retain their specified state |
| F4 | clone overlapping ranges into independent destinations | counts split at every physical boundary and adjacent equal-count records merge |
| F5 | make a third clone | the covered records move from count two to count three without duplicating physical ownership |
| F6 | write one block in a shared range | only the writer receives replacement blocks; its new bytes are visible; every peer's bytes remain unchanged |
| F7 | truncate through a shared range | removed mappings decrement counts; blocks with a surviving peer remain allocated and outside reclaim |
| F8 | unlink one of exactly two clones | the count-two record disappears, the survivor becomes private, and **no covered block enters reclaim or becomes free** |
| F9 | unlink the final survivor later | the now-private run enters reclaim and becomes reusable only after the ordinary generation quarantine |
| F10 | rename with replacement over a shared target | replacement removes exactly the target's references while preserving every other peer |
| F11 | stale `EXTENT_SHARED` after peers disappear | the flagged private run is legal, requires a bounded overlap lookup, and never collapses to direct layout |
| F12 | clone on a profile without the feature | returns `NotSupported` with generation, namespace, free count and recorded I/O unchanged |

For every row the test also asserts:

- logical size, allocated size and file bytes;
- reference records equal the canonical interval sweep;
- every shared physical block is allocated but accounted only once by the
  bitmap equality check;
- shared-reference tree blocks are reachable metadata and cannot intersect
  data, free space, reclaim or another metadata owner;
- the operation never writes data or metadata reachable from the checkpoint
  that remains selected if the transaction aborts. Blocks reachable only from
  the older checkpoint may be recycled while its slot is the publication
  target; a crash before publication still selects the intact newer state.

## Canonical reference oracle

The checker derives expected records from all live file maps. It creates two
events for each allocated physical extent, sorts the interval endpoints and
sweeps them while maintaining the number of live mappings. It emits only
maximal intervals whose count is at least two, merging adjacent intervals with
the same count.

This is a semantic oracle, not a prescribed implementation. Its resource
bound is proportional to the number of extent boundaries, not the numerical
size of the volume and not one counter per physical block. Checked arithmetic
rejects zero-length runs, overflowing ends and reference counts above the wire
limit.

The synthetic oracle cases are:

1. no maps and one private map produce no shared record;
2. two identical maps produce one count-two record;
3. partial overlaps split at both ends;
4. three-way overlap produces count-two/count-three/count-two partitions;
5. touching partitions with the same count merge even when their peer sets
   differ;
6. a gap between shared intervals remains absent;
7. zero length, end overflow and count overflow fail closed.

## Corruption matrix

Each mutation keeps the containing block's CRC valid where possible so the
test reaches the semantic invariant rather than stopping only at checksum
validation.

| Case | Forged state | Required verdict |
|---|---|---|
| C1 | reference record count below two | checker error; mutation paths fail closed |
| C2 | record end overflow or outside allocatable geometry | checker error |
| C3 | overlapping or non-maximal adjacent reference records | checker error |
| C4 | missing record for a physical interval with two live mappings | checker error |
| C5 | extra record for an interval with only one live mapping | checker error |
| C6 | wrong count for an otherwise correct interval | checker error |
| C7 | unflagged extent overlaps a reference record | checker error |
| C8 | shared flag or non-zero root while the feature bit is clear | structural corruption; normal mutation never frees through the unknown state |
| C9 | feature enabled with a zero root before first clone | legal enabled-unused state |
| C10 | direct-layout object participates in a shared record | checker error |
| C11 | shared-tree block intersects data, reclaim or free space | checker error |
| C12 | unreadable shared root or descendant | bounded mount may defer unrelated damage, exhaustive checker reports it, and the first dependent mutation fails without freeing or publishing |

## Crash matrix

For each operation, record all block writes and flushes and inject power loss
after every operation. For every unflushed tail enumerate all full-write
subsets plus the representative torn writes supplied by
`afsplus_block::powercut`.

| Case | Recorded transaction | Post-state-specific oracle |
|---|---|---|
| P1 | first `CloneFile` including direct-layout promotion | either only the source exists privately, or both files exist with complete shared records |
| P2 | `CloneRange` causing multiple reference boundaries | either the exact old destination or the complete partitioned result; never a hybrid map/count pair |
| P3 | write-COW splitting one shared run | either old bytes in both files, or new bytes only in the writer and old bytes in every peer |
| P4 | unlink at count three | either count three with all names or count two with the removed name absent |
| P5 | unlink at count two | either both mappings and count two, or one intact private survivor with no record and no premature reclaim |
| P6 | truncate crossing private and shared sub-runs | exact old size/map or exact new size/map; surviving peer bytes remain intact |
| P7 | rename replacement whose target is shared | exact pre-rename namespace or exact post-rename namespace with reference counts matching it |
| P8 | fsynced shared unlink replayed from the intent log | recovered state is a monotone durable prefix and reference changes occur in the replay transaction |
| P9 | fsynced rename-replacement replayed from the intent log | no namespace state can be selected without its matching reference-tree state |
| P10 | later reclaim and reuse of the final private owner | no selectable older checkpoint observes the reused contents through the old mapping |

Every recovered image must satisfy all of these, regardless of whether the
old or new generation wins:

1. the exhaustive checker is clean;
2. the selected generation is exactly the pre-transaction or
   post-transaction generation allowed by the operation;
3. every visible file has byte-for-byte expected contents;
4. no free or reclaim block is reachable by a live mapping;
5. reference-tree records equal the canonical oracle in both directions;
6. both the pre-state and post-state occur somewhere in the enumerated matrix,
   proving that the test did not accidentally exercise only one outcome.

Mount success alone is never an acceptance result.

## Resource and compatibility gates

- Run the canonical oracle with a large block address and few intervals to
  prove it does not allocate by volume size.
- Exercise the smallest supported metadata cache; overlap resolution retains
  only the bounded tree path and result page promised by ADR-034.
- A volume whose feature is disabled remains writable through all pre-clone
  operations and rejects clone without changing state.
- A reader that understands the `ro_compat` bit but not cloning may mount
  read-only; a writer lacking support must not mount read-write.
- Classic DOS applications and the existing Alpha-0 operation matrix remain
  unchanged because clone is an additional semantic operation.

## Completion evidence

The milestone is not complete until one retained report records:

- the commit containing format, core, checker and tests;
- `cargo test -p afsplus-check --test shared_extents` passing;
- `make check` passing;
- the explicit qualification tests, if any, run with `--ignored` in release;
- a clean worktree and the pushed `origin/main` revision.

Hardware qualification is not claimed by this host-side gate. MacAROS bare
metal and the physical A500 remain separate M06 evidence.
